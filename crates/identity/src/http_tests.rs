//! HTTP integration against real isolated SQLite and the ordinary-account core.
//! A projection renderer tests transport and view contracts independently of the A00 HTML.
use crate::{
    contract::IdentityError,
    core_tests::Fixture,
    http::{IdentityWebState, router},
    rate::{RatePolicy, RateProtection},
    web_view::{Page, PageRenderer},
};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use chrono::{Duration, Utc};
use platform::{
    cache::{
        CacheError, DisabledRateLimiter, RateLimitDecision, RateLimitKey, RateLimitPolicy,
        RateLimiter,
    },
    flags::Flag,
    identifiers::SchemaRenderer,
    outbox::BoxFuture,
    policy::{PolicyError, PolicySet, PolicyVersion},
    transaction::new_operation_id,
};
use platform_db::{
    policy::PolicyReader,
    transaction::{DbTransaction, DbValue, TransactionConnection},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration as StdDuration,
};
use tower::ServiceExt;

struct ProjectionRenderer;
async fn http_fixture() -> Fixture {
    let f = Fixture::new().await;
    // HTTP owns its wall clock. Refresh only this isolated synthetic invitation so
    // route tests remain valid after the fixed core-test date has passed.
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let table = crate::repo::table(&tx.connection(), &f.r, "invitations").unwrap();
    tx.connection()
        .execute(
            &format!("UPDATE {table} SET created_at = ?, expires_at = ?"),
            &[
                DbValue::Time(Utc::now()),
                DbValue::Time(Utc::now() + Duration::days(2)),
            ],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    f
}
impl PageRenderer for ProjectionRenderer {
    fn render(&self, page: &Page) -> Result<String, IdentityError> {
        Ok(json!({
            "title": page.title, "action": page.action, "csrf": page.csrf,
            "key": page.idempotency_key, "show_form": page.show_form,
            "error": page.frame.error, "flash": page.frame.flash,
            "policy_consent": page.policy_consent, "preset_avatar": page.preset_avatar,
            "fields": page.fields.iter().map(|f| json!({"name":f.name,"value":f.value,"kind":f.kind})).collect::<Vec<_>>(),
            "rows": page.rows.iter().map(|r| json!({"label":r.label,"value":r.value,"action":r.action,"action_label":r.action_label,"reference":r.reference})).collect::<Vec<_>>(),
        }).to_string())
    }
}

/// Explicitly synthetic reader for this transport test, without adding a reverse domain dependency.
struct SyntheticDocuments;
impl PolicyReader for SyntheticDocuments {
    fn current<'a, 'c>(
        &'a self,
        _: &'a mut TransactionConnection<'c>,
        _: &'a SchemaRenderer,
    ) -> BoxFuture<'a, Result<PolicySet, PolicyError>> {
        Box::pin(async {
            Ok(PolicySet {
                terms: 1,
                privacy: 2,
                community: 3,
            })
        })
    }
    fn documents<'a, 'c>(
        &'a self,
        _: &'a mut TransactionConnection<'c>,
        _: &'a SchemaRenderer,
    ) -> BoxFuture<'a, Result<Vec<PolicyVersion>, PolicyError>> {
        Box::pin(async {
            Ok(["terms", "privacy", "community"]
                .into_iter()
                .enumerate()
                .map(|(index, kind)| PolicyVersion {
                    id: index as i64 + 1,
                    kind,
                    version: 1,
                    body: "合成测试政策，仅用于隔离数据库，无正式法律效力。".into(),
                    synthetic: true,
                })
                .collect())
        })
    }
}

fn app(f: &Fixture) -> Router {
    router(web_state(f))
}
fn web_state(f: &Fixture) -> IdentityWebState {
    let mut service = f.service.clone();
    service.policy_reader = Arc::new(SyntheticDocuments);
    IdentityWebState {
        service: Some(Arc::new(service)),
        flags: Some(Arc::new(platform_db::flags_repo::DbFlagReader::new(
            f.pool.clone(),
            f.r.clone(),
        ))),
        renderer: Arc::new(ProjectionRenderer),
        secure_cookies: false,
        risk_signals: None,
        rate: Some(RateProtection {
            redis_enabled: false,
            limiter: Arc::new(DisabledRateLimiter),
            registration: None,
            login: None,
            recovery: None,
            invitation: None,
        }),
    }
}

fn approved_state(f: &Fixture) -> IdentityWebState {
    let mut state = web_state(f);
    let mut service = state.service.as_deref().unwrap().clone();
    service.policy = Arc::new(crate::policy::ApprovedIdentityPolicy);
    service = service.with_risk_policy(Arc::new(trust::risk::ApprovedIdentityRiskPolicy));
    state.service = Some(Arc::new(service));
    state.rate = Some(RateProtection::approved_v1(
        false,
        Arc::new(DisabledRateLimiter),
    ));
    state
}

async fn latest_token(f: &Fixture, purpose: &str) -> (String, String) {
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let table = crate::repo::table(&tx.connection(), &f.r, "one_time_tokens").unwrap();
    let row = tx.connection().fetch_optional(&format!("SELECT token_id, token_nonce, user_id, target_email_key, expires_at, issued_credential_version FROM {table} WHERE purpose = ? AND consumed = 0 ORDER BY created_at DESC, token_id DESC"), &[purpose.into()]).await.unwrap().unwrap();
    let id = row.text("token_id").unwrap();
    let raw = f.issuer.derive(
        &id,
        &row.text("token_nonce").unwrap(),
        row.i64("user_id").unwrap(),
        purpose,
        &row.text("target_email_key").unwrap(),
        row.time("expires_at").unwrap(),
        row.i64("issued_credential_version").unwrap(),
    );
    tx.rollback().await.unwrap();
    (id, raw.expose().into())
}

async fn count(f: &Fixture, table: &str) -> i64 {
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let table = crate::repo::table(&tx.connection(), &f.r, table).unwrap();
    let count = tx
        .connection()
        .fetch_optional(&format!("SELECT COUNT(*) AS n FROM {table}"), &[])
        .await
        .unwrap()
        .unwrap()
        .i64("n")
        .unwrap();
    tx.rollback().await.unwrap();
    count
}

#[derive(Default, Clone)]
struct Browser {
    cookies: BTreeMap<String, String>,
}
struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    page: Value,
}
impl Browser {
    async fn get(&mut self, app: &Router, path: &str) -> Reply {
        self.request(app, Method::GET, path, &[], None).await
    }
    async fn post(&mut self, app: &Router, path: &str, form: &[(String, String)]) -> Reply {
        self.request(app, Method::POST, path, form, Some("http://localhost"))
            .await
    }
    async fn request(
        &mut self,
        app: &Router,
        method: Method,
        path: &str,
        form: &[(String, String)],
        origin: Option<&str>,
    ) -> Reply {
        let mut builder = Request::builder()
            .method(method.clone())
            .uri(path)
            .header(header::HOST, "localhost");
        if !self.cookies.is_empty() {
            builder = builder.header(
                header::COOKIE,
                self.cookies
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        let encoded = form
            .iter()
            .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
            .collect::<Vec<_>>()
            .join("&");
        if method == Method::POST {
            builder = builder.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let mut request = builder.body(Body::from(encoded)).unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "192.0.2.15:42300".parse::<SocketAddr>().unwrap(),
        ));
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        for cookie in headers.get_all(header::SET_COOKIE) {
            let text = cookie.to_str().unwrap();
            let (key, value) = text.split(';').next().unwrap().split_once('=').unwrap();
            if text.contains("Max-Age=0") {
                self.cookies.remove(key);
            } else {
                self.cookies.insert(key.into(), value.into());
            }
        }
        let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let page = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"raw":String::from_utf8_lossy(&bytes)}));
        Reply {
            status,
            headers,
            page,
        }
    }
}
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}
fn form(page: &Value, extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut fields = vec![
        ("csrf".into(), page["csrf"].as_str().unwrap().into()),
        (
            "idempotency_key".into(),
            page["key"].as_str().unwrap().into(),
        ),
    ];
    for field in page["fields"].as_array().unwrap() {
        if field["kind"] == "hidden" {
            fields.push((
                field["name"].as_str().unwrap().into(),
                field["value"].as_str().unwrap().into(),
            ));
        }
    }
    fields.extend(
        extra
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string())),
    );
    fields
}
async fn flag(f: &Fixture, flag: Flag, enabled: bool) {
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let table =
        f.r.table(tx.connection().backend(), "platform", "feature_flags")
            .unwrap();
    tx.connection()
        .execute(
            &format!("UPDATE {table} SET enabled = ?, version = version + 1 WHERE flag_key = ?"),
            &[DbValue::Bool(enabled), flag.key().into()],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
}
async fn registered(f: &Fixture) {
    f.enable().await;
    f.service
        .register(
            &{
                let grant = f.service.create_preauth(Utc::now()).await.unwrap();
                crate::contract::RequestContext::new(grant.token, grant.csrf, new_operation_id())
            },
            f.registration(),
            Utc::now(),
        )
        .await
        .unwrap();
}
async fn login(browser: &mut Browser, app: &Router) -> Reply {
    let page = browser.get(app, "/login").await;
    assert_eq!(page.status, StatusCode::OK);
    browser
        .post(
            app,
            "/login",
            &form(
                &page.page,
                &[
                    ("identifier", "ordinary_member"),
                    ("password", "Synthetic password 123"),
                ],
            ),
        )
        .await
}

#[tokio::test]
async fn default_flags_close_all_identity_routes_without_side_effects() {
    let f = http_fixture().await;
    let app = app(&f);
    let mut browser = Browser::default();
    for path in [
        "/login",
        "/register",
        "/forgot-password",
        "/reset-password",
        "/verify-email",
        "/resend-verification",
        "/confirm-email",
        "/settings/profile",
        "/settings/sessions",
        "/terms",
        "/privacy",
        "/rules",
    ] {
        let result = browser.get(&app, path).await;
        assert_eq!(result.status, StatusCode::NOT_FOUND, "{path}");
        assert!(result.headers.get(header::SET_COOKIE).is_none());
        assert_eq!(result.headers[header::CACHE_CONTROL], "private, no-store");
    }
    let result = browser.post(&app, "/register", &[]).await;
    assert_eq!(result.status, StatusCode::NOT_FOUND);
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let table =
        f.r.table(tx.connection().backend(), "identity", "request_principals")
            .unwrap();
    assert_eq!(
        tx.connection()
            .fetch_optional(&format!("SELECT COUNT(*) AS n FROM {table}"), &[])
            .await
            .unwrap()
            .unwrap()
            .i64("n")
            .unwrap(),
        0
    );
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn login_csrf_origin_cookie_attributes_and_owner_form_boundary() {
    let f = http_fixture().await;
    registered(&f).await;
    let app = app(&f);
    let mut browser = Browser::default();
    let page = browser.get(&app, "/login").await;
    let valid = form(
        &page.page,
        &[
            ("identifier", "ordinary_member"),
            ("password", "Synthetic password 123"),
        ],
    );
    let cross_origin = browser
        .request(
            &app,
            Method::POST,
            "/login",
            &valid,
            Some("https://other.example.invalid"),
        )
        .await;
    assert_eq!(cross_origin.status, StatusCode::FORBIDDEN);
    let mut invalid = valid.clone();
    invalid.iter_mut().find(|(key, _)| key == "csrf").unwrap().1 = "invalid_csrf".into();
    assert!(
        browser
            .post(&app, "/login", &invalid)
            .await
            .status
            .is_client_error()
    );
    assert!(!browser.cookies.contains_key("bcm_session"));
    let success = browser.post(&app, "/login", &valid).await;
    assert_eq!(success.status, StatusCode::OK);
    assert!(browser.cookies.contains_key("bcm_session"));
    let cookies: Vec<_> = success
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap())
        .collect();
    assert!(cookies.iter().all(|value| value.contains("HttpOnly")
        && value.contains("SameSite=Lax")
        && value.contains("Path=/")
        && value.contains("Max-Age=2592000")));
    assert_eq!(success.headers["referrer-policy"], "no-referrer");
    let profile = browser.get(&app, "/settings/profile").await;
    assert_eq!(profile.status, StatusCode::OK);
    let mut edit = form(&profile.page, &[("bio", "synthetic updated biography")]);
    edit.iter_mut().find(|(key, _)| key == "csrf").unwrap().1 = "wrong_owner_csrf".into();
    assert_eq!(
        browser.post(&app, "/settings/profile", &edit).await.status,
        StatusCode::FORBIDDEN
    );
    let updated = browser
        .post(
            &app,
            "/settings/profile",
            &form(&profile.page, &[("bio", "synthetic updated biography")]),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK);
    let mut anonymous = Browser::default();
    assert_eq!(
        anonymous.get(&app, "/settings/profile").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        anonymous
            .post(
                &app,
                "/settings/profile",
                &form(&profile.page, &[("bio", "forged_owner")])
            )
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn registration_policy_links_work_under_the_same_controlled_flag_state() {
    let f = http_fixture().await;
    flag(&f, Flag::Registration, true).await;
    flag(&f, Flag::EmailVerification, true).await;
    let app = app(&f);
    let mut browser = Browser::default();
    let registration = browser.get(&app, "/register").await;
    assert_eq!(registration.status, StatusCode::OK);
    assert_eq!(registration.page["policy_consent"], true);
    for path in ["/terms", "/privacy", "/rules"] {
        let doc = browser.get(&app, path).await;
        assert_eq!(doc.status, StatusCode::OK, "{path}");
        assert!(doc.page["rows"].to_string().contains("合成"));
    }
    let input = f.registration();
    let submission = form(
        &registration.page,
        &[
            ("invitation", input.invitation.expose()),
            ("username", &input.username),
            ("email", &input.email),
            ("password", input.password.expose()),
            ("consent", "yes"),
        ],
    );
    let result = browser.post(&app, "/register", &submission).await;
    assert_eq!(result.status, StatusCode::OK);
    assert!(!browser.cookies.contains_key("bcm_session"));
    flag(&f, Flag::RegistrationLock, true).await;
    assert_eq!(
        browser.get(&app, "/register").await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn expired_revoked_sessions_and_cross_owner_device_actions_are_rejected() {
    let f = http_fixture().await;
    registered(&f).await;
    let app = app(&f);
    let mut a = Browser::default();
    let mut a_second = Browser::default();
    assert_eq!(login(&mut a, &app).await.status, StatusCode::OK);
    assert_eq!(login(&mut a_second, &app).await.status, StatusCode::OK);
    let second_token = a_second.cookies["bcm_session"].clone();
    let second_identity = f
        .service
        .authenticate(&second_token, Utc::now())
        .await
        .unwrap()
        .unwrap();
    // The fixture's ordinary inviter is another account; it receives an isolated synthetic session only.
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let other = crate::repo::account_by_id(&mut tx.connection(), &f.r, 1)
        .await
        .unwrap()
        .unwrap();
    let other_ref = new_operation_id();
    crate::repo::create_session(
        &mut tx.connection(),
        &f.r,
        &other,
        &other_ref,
        &crate::core::secret_hash("synthetic_other_device"),
        &crate::core::secret_hash("synthetic_other_csrf"),
        "unknown",
        Utc::now(),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let devices = a.get(&app, "/settings/sessions").await;
    assert_eq!(devices.status, StatusCode::OK);
    assert!(!devices.page["rows"].to_string().contains(&other_ref));
    assert!(
        a.post(
            &app,
            "/settings/sessions/revoke",
            &form(&devices.page, &[("reference", &other_ref)])
        )
        .await
        .status
        .is_client_error()
    );
    assert_eq!(
        a.post(
            &app,
            "/settings/sessions/revoke",
            &form(
                &devices.page,
                &[("reference", second_identity.session_ref())]
            )
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        a_second.get(&app, "/settings/profile").await.status,
        StatusCode::UNAUTHORIZED
    );
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let sessions =
        f.r.table(tx.connection().backend(), "identity", "sessions")
            .unwrap();
    tx.connection()
        .execute(
            &format!("UPDATE {sessions} SET idle_expires_at = ? WHERE token_hash = ?"),
            &[
                DbValue::Time(Utc::now() - Duration::seconds(1)),
                crate::core::secret_hash(&a.cookies["bcm_session"]).into(),
            ],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        a.get(&app, "/settings/profile").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn login_and_recovery_responses_do_not_enumerate_accounts() {
    let f = http_fixture().await;
    registered(&f).await;
    let app = app(&f);
    let mut signatures = Vec::new();
    for (identifier, password) in [
        ("ordinary_member", "wrong_synthetic_password"),
        ("unknown_member", "wrong_synthetic_password"),
        ("ordinary_member", "tiny"),
    ] {
        let mut browser = Browser::default();
        let page = browser.get(&app, "/login").await;
        let result = browser
            .post(
                &app,
                "/login",
                &form(
                    &page.page,
                    &[("identifier", identifier), ("password", password)],
                ),
            )
            .await;
        assert!(!browser.cookies.contains_key("bcm_session"));
        assert!(!result.page.to_string().contains(identifier));
        signatures.push((
            result.status,
            result.page["error"].clone(),
            result.page["flash"].clone(),
        ));
    }
    assert!(signatures.windows(2).all(|values| values[0] == values[1]));
    let mut signatures = Vec::new();
    for identifier in ["ordinary_member", "unknown_member"] {
        let mut browser = Browser::default();
        let page = browser.get(&app, "/forgot-password").await;
        let result = browser
            .post(
                &app,
                "/forgot-password",
                &form(&page.page, &[("identifier", identifier)]),
            )
            .await;
        assert_eq!(result.status, StatusCode::OK);
        assert!(!result.page.to_string().contains(identifier));
        signatures.push((result.page["error"].clone(), result.page["flash"].clone()));
    }
    assert_eq!(signatures[0], signatures[1]);
}

struct RecordingLimiter {
    limited: AtomicBool,
    keys: Mutex<Vec<RateLimitKey>>,
}
impl RecordingLimiter {
    fn take_keys(&self) -> Vec<RateLimitKey> {
        std::mem::take(&mut *self.keys.lock().unwrap())
    }
}
impl RateLimiter for RecordingLimiter {
    fn check<'a>(
        &'a self,
        key: &'a RateLimitKey,
        policy: RateLimitPolicy,
    ) -> BoxFuture<'a, Result<RateLimitDecision, CacheError>> {
        self.keys.lock().unwrap().push(key.clone());
        // This fixture exercises the hard limiter path; soft pairs remain available.
        let allowed = !self.limited.load(Ordering::SeqCst)
            || policy != RateLimitPolicy::new(5, StdDuration::from_secs(10)).unwrap();
        Box::pin(async move {
            Ok(RateLimitDecision {
                allowed,
                retry_after: if allowed {
                    StdDuration::ZERO
                } else {
                    StdDuration::from_secs(1)
                },
            })
        })
    }
}
fn rate_app(f: &Fixture, limiter: Arc<RecordingLimiter>) -> Router {
    let mut state = web_state(f);
    let budget = RateLimitPolicy::new(5, StdDuration::from_secs(10)).unwrap();
    let policy = RatePolicy {
        account: budget,
        ip: budget,
        device: budget,
        network: budget,
        ipv4_prefix: 24,
        ipv6_prefix: 64,
    };
    state.rate = Some(RateProtection {
        redis_enabled: true,
        limiter,
        registration: Some(policy.clone()),
        login: Some(policy.clone()),
        recovery: Some(policy.clone()),
        invitation: Some(policy),
    });
    router(state)
}

#[tokio::test]
async fn limited_login_and_recovery_hide_account_mapping_and_do_one_real_argon_verification() {
    let f = http_fixture().await;
    registered(&f).await;
    let limiter = Arc::new(RecordingLimiter {
        limited: AtomicBool::new(true),
        keys: Mutex::new(Vec::new()),
    });
    let app = rate_app(&f, limiter.clone());
    let mut signatures = Vec::new();
    for (limited, identifier, password) in [
        (true, "ordinary_member", "Synthetic password 123"),
        (true, "member@example.invalid", "Synthetic password 123"),
        (false, "ordinary_member", "wrong_synthetic_password"),
        (false, "missing_account", "wrong_synthetic_password"),
    ] {
        limiter.limited.store(limited, Ordering::SeqCst);
        let mut browser = Browser::default();
        let page = browser.get(&app, "/login").await;
        let before = f.service.passwords.verification_count();
        let result = browser
            .post(
                &app,
                "/login",
                &form(
                    &page.page,
                    &[("identifier", identifier), ("password", password)],
                ),
            )
            .await;
        assert_eq!(f.service.passwords.verification_count(), before + 1);
        assert_eq!(limiter.take_keys().len(), 6);
        assert_eq!(result.status, StatusCode::BAD_REQUEST);
        assert!(!result.headers.contains_key(header::RETRY_AFTER));
        assert!(!browser.cookies.contains_key("bcm_session"));
        signatures.push((
            result.status,
            result.page["error"].clone(),
            result.page["flash"].clone(),
        ));
    }
    assert!(
        signatures
            .windows(2)
            .all(|results| results[0] == results[1])
    );
    let mut signatures = Vec::new();
    for (limited, identifier) in [
        (true, "ordinary_member"),
        (true, "member@example.invalid"),
        (false, "ordinary_member"),
        (false, "missing_account"),
    ] {
        limiter.limited.store(limited, Ordering::SeqCst);
        let mut browser = Browser::default();
        let page = browser.get(&app, "/forgot-password").await;
        let before = f.service.passwords.verification_count();
        let result = browser
            .post(
                &app,
                "/forgot-password",
                &form(&page.page, &[("identifier", identifier)]),
            )
            .await;
        assert_eq!(f.service.passwords.verification_count(), before + 1);
        assert_eq!(limiter.take_keys().len(), 6);
        assert_eq!(result.status, StatusCode::OK);
        assert!(!result.headers.contains_key(header::RETRY_AFTER));
        signatures.push((
            result.status,
            result.page["error"].clone(),
            result.page["flash"].clone(),
        ));
    }
    assert!(
        signatures
            .windows(2)
            .all(|results| results[0] == results[1])
    );
}

#[tokio::test]
async fn registration_rate_subject_uses_email_and_ignores_extra_identifier() {
    let f = http_fixture().await;
    registered(&f).await;
    let limiter = Arc::new(RecordingLimiter {
        limited: AtomicBool::new(true),
        keys: Mutex::new(Vec::new()),
    });
    let app = rate_app(&f, limiter.clone());
    let mut browser = Browser::default();
    let mut recorded = Vec::new();
    for identifier in ["ordinary_member", "missing_account"] {
        let page = browser.get(&app, "/register").await;
        let before = f.service.passwords.verification_count();
        let result = browser
            .post(
                &app,
                "/register",
                &form(
                    &page.page,
                    &[
                        ("email", "synthetic_candidate@example.invalid"),
                        ("identifier", identifier),
                    ],
                ),
            )
            .await;
        assert_eq!(result.status, StatusCode::OK);
        assert_eq!(f.service.passwords.verification_count(), before + 1);
        let keys = limiter.take_keys();
        assert_eq!(keys.len(), 6);
        recorded.push(keys);
    }
    assert_eq!(recorded[0], recorded[1]);
    // Changing the actual mailbox changes the account dimension while the stable device/IP/network remain shared.
    let page = browser.get(&app, "/register").await;
    let result = browser
        .post(
            &app,
            "/register",
            &form(
                &page.page,
                &[
                    ("email", "another_candidate@example.invalid"),
                    ("identifier", "ordinary_member"),
                ],
            ),
        )
        .await;
    assert_eq!(result.status, StatusCode::OK);
    let changed = limiter.take_keys();
    assert_eq!(changed.len(), 6);
    assert_ne!(recorded[0][0], changed[0]);
    assert_eq!(recorded[0][1..], changed[1..]);
    let page = browser.get(&app, "/register").await;
    assert_eq!(
        browser
            .post(
                &app,
                "/register",
                &form(
                    &page.page,
                    &[
                        ("email", "ordinary_member"),
                        ("identifier", "ordinary_member")
                    ]
                )
            )
            .await
            .status,
        StatusCode::OK
    );
    let invalid_mailbox = limiter.take_keys();
    assert_eq!(invalid_mailbox.len(), 6);
    assert_ne!(recorded[0][0], invalid_mailbox[0]);
    assert_eq!(recorded[0][1..], invalid_mailbox[1..]);
}

async fn operation_id(f: &Fixture, actor: i64, scope: &str, key: &str) -> String {
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let table =
        f.r.table(tx.connection().backend(), "platform", "idempotency_records")
            .unwrap();
    let id = tx.connection().fetch_optional(&format!("SELECT operation_id FROM {table} WHERE actor_id = ? AND scope = ? AND idempotency_key = ?"), &[actor.into(), scope.into(), key.into()]).await.unwrap().unwrap().text("operation_id").unwrap();
    tx.rollback().await.unwrap();
    id
}
fn query_form(browser: &Browser, scope: &str, key: &str, id: &str) -> Vec<(String, String)> {
    vec![
        ("csrf".into(), browser.cookies["bcm_csrf"].clone()),
        ("scope".into(), scope.into()),
        ("original_key".into(), key.into()),
        ("operation_id".into(), id.into()),
    ]
}
async fn synthetic_other_browser(f: &Fixture) -> (Browser, i64) {
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let account = crate::repo::account_by_id(&mut tx.connection(), &f.r, 1)
        .await
        .unwrap()
        .unwrap();
    let token = crate::core::random_secret();
    let csrf = crate::core::random_secret();
    crate::repo::create_session(
        &mut tx.connection(),
        &f.r,
        &account,
        &new_operation_id(),
        &crate::core::secret_hash(token.expose()),
        &crate::core::secret_hash(csrf.expose()),
        "unknown",
        Utc::now(),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    (
        Browser {
            cookies: BTreeMap::from([
                ("bcm_session".into(), token.expose().into()),
                ("bcm_csrf".into(), csrf.expose().into()),
            ]),
        },
        account.id,
    )
}

#[tokio::test]
async fn revoked_bearer_queries_only_its_bound_logout_and_password_operations() {
    let f = http_fixture().await;
    registered(&f).await;
    let app = app(&f);
    let mut browser = Browser::default();
    assert_eq!(login(&mut browser, &app).await.status, StatusCode::OK);
    let identity = f
        .service
        .authenticate(&browser.cookies["bcm_session"], Utc::now())
        .await
        .unwrap()
        .unwrap();
    let mut old = browser.clone();
    let logout = browser.get(&app, "/logout").await;
    let logout_key = logout.page["key"].as_str().unwrap();
    assert_eq!(
        browser
            .post(&app, "/logout", &form(&logout.page, &[]))
            .await
            .status,
        StatusCode::OK
    );
    assert!(!browser.cookies.contains_key("bcm_session"));
    let id = operation_id(&f, identity.user_id(), "identity.logout", logout_key).await;
    let query = query_form(&old, "identity.logout", logout_key, &id);
    let result = old.post(&app, "/auth/operations", &query).await;
    assert_eq!(result.status, StatusCode::OK);
    assert!(
        result.page["flash"]
            .as_str()
            .unwrap()
            .contains("已确认完成")
    );
    assert!(!result.headers.contains_key(header::SET_COOKIE));
    assert_eq!(
        old.get(&app, "/settings/profile").await.status,
        StatusCode::UNAUTHORIZED
    );
    let unknown_operation = new_operation_id();
    for (scope, key, operation) in [
        ("identity.logout", "incorrect_key", id.as_str()),
        ("identity.profile.password", logout_key, id.as_str()),
        ("identity.logout", logout_key, unknown_operation.as_str()),
    ] {
        let query = query_form(&old, scope, key, operation);
        assert_eq!(
            old.post(&app, "/auth/operations", &query).await.status,
            StatusCode::ACCEPTED
        );
    }
    let mut bad_csrf = query.clone();
    bad_csrf[0].1 = "invalid_csrf".into();
    assert_eq!(
        old.post(&app, "/auth/operations", &bad_csrf).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        old.request(
            &app,
            Method::POST,
            "/auth/operations",
            &query,
            Some("https://other.example.invalid")
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let (mut other, other_id) = synthetic_other_browser(&f).await;
    let other_logout = other.get(&app, "/logout").await;
    let other_key = other_logout.page["key"].as_str().unwrap();
    assert_eq!(
        other
            .post(&app, "/logout", &form(&other_logout.page, &[]))
            .await
            .status,
        StatusCode::OK
    );
    let other_operation = operation_id(&f, other_id, "identity.logout", other_key).await;
    let cross_owner = query_form(&old, "identity.logout", other_key, &other_operation);
    assert_eq!(
        old.post(&app, "/auth/operations", &cross_owner)
            .await
            .status,
        StatusCode::ACCEPTED
    );
    // Password changes revoke the querying bearer as well; its narrow receipt proof survives.
    assert_eq!(login(&mut browser, &app).await.status, StatusCode::OK);
    let mut old_password_session = browser.clone();
    let password_page = browser.get(&app, "/settings/password").await;
    let password_key = password_page.page["key"].as_str().unwrap();
    assert_eq!(
        browser
            .post(
                &app,
                "/settings/password",
                &form(
                    &password_page.page,
                    &[("password", "Synthetic changed password")]
                )
            )
            .await
            .status,
        StatusCode::OK
    );
    let password_id = operation_id(
        &f,
        identity.user_id(),
        "identity.profile.password",
        password_key,
    )
    .await;
    let receipt = query_form(
        &old_password_session,
        "identity.profile.password",
        password_key,
        &password_id,
    );
    let result = old_password_session
        .post(&app, "/auth/operations", &receipt)
        .await;
    assert_eq!(result.status, StatusCode::OK);
    assert!(!result.headers.contains_key(header::SET_COOKIE));
    assert!(
        !result
            .page
            .to_string()
            .contains("Synthetic changed password")
    );
    assert_eq!(
        old_password_session
            .get(&app, "/settings/profile")
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        old_password_session
            .post(
                &app,
                "/settings/profile",
                &form(&password_page.page, &[("bio", "revoked_write")])
            )
            .await
            .status
            .is_client_error()
    );
}

#[tokio::test]
async fn invitation_alias_revocation_action_and_existing_profile_projections_are_wired() {
    let f = http_fixture().await;
    registered(&f).await;
    let app = app(&f);
    let mut browser = Browser::default();
    assert_eq!(login(&mut browser, &app).await.status, StatusCode::OK);
    let identity = f
        .service
        .authenticate(&browser.cookies["bcm_session"], Utc::now())
        .await
        .unwrap()
        .unwrap();
    // Exercise the real verification transition; inviting does not depend on granting T1.
    let (_, raw) = latest_token(&f, "verify_email").await;
    let verification = browser.get(&app, "/verify-email").await;
    assert_eq!(
        browser
            .post(
                &app,
                "/verify-email",
                &form(&verification.page, &[("token", &raw)])
            )
            .await
            .status,
        StatusCode::OK
    );
    let invite_page = browser.get(&app, "/me/invites").await;
    assert_eq!(invite_page.status, StatusCode::OK);
    assert_eq!(invite_page.page["action"], "/me/invites");
    assert_eq!(
        browser
            .post(&app, "/me/invites", &form(&invite_page.page, &[]))
            .await
            .status,
        StatusCode::OK
    );
    let page = browser.get(&app, "/me/invites").await;
    let revoke = page.page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["action"] == "/settings/invitations/revoke")
        .unwrap();
    assert_eq!(revoke["action_label"], "撤销邀请码");
    let reference = revoke["reference"].as_str().unwrap();
    assert_eq!(
        browser
            .post(
                &app,
                "/settings/invitations/revoke",
                &form(&page.page, &[("reference", reference)])
            )
            .await
            .status,
        StatusCode::OK
    );
    let page = browser.get(&app, "/me/invites").await;
    let revoked = page.page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["reference"] == reference)
        .unwrap();
    assert!(revoked["action"].is_null());
    assert!(revoked["value"].as_str().unwrap().contains("已撤销"));
    let profile = browser.get(&app, "/settings/profile").await;
    assert_eq!(profile.page["preset_avatar"], true);
    // DEC-054：邮箱验证在同一事务内自动授予 T0，资料页因此显示 T0 而不再是「待确认」。
    assert!(
        profile.page["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["label"] == "信任等级" && row["value"] == "T0")
    );
    let own = f.service.profile(&identity, Utc::now()).await.unwrap();
    let public = Browser::default()
        .get(&app, &format!("/u/{}", own.public.public_ref))
        .await;
    assert_eq!(public.status, StatusCode::OK);
    assert_eq!(public.page["preset_avatar"], true);
    assert!(!public.page.to_string().contains("member@example.invalid"));
    assert!(
        public.page["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["label"] == "信任等级" && row["value"] == "T0")
    );
}

#[tokio::test]
async fn approved_preauth_reuses_two_hour_principal_without_sliding_or_cross_form_invalidation() {
    let f = http_fixture().await;
    registered(&f).await;
    let app = router(approved_state(&f));
    let mut browser = Browser::default();
    let first = browser.get(&app, "/login").await;
    let token = browser.cookies["bcm_preauth"].clone();
    let csrf = browser.cookies["bcm_preauth_csrf"].clone();
    assert_eq!(first.page["csrf"], csrf);
    for cookie in first.headers.get_all(header::SET_COOKIE) {
        let cookie = cookie.to_str().unwrap();
        if cookie.starts_with("bcm_preauth") {
            assert!(
                cookie.contains("Max-Age=7200")
                    && cookie.contains("HttpOnly")
                    && cookie.contains("SameSite=Lax")
            );
        }
    }
    for path in [
        "/register",
        "/forgot-password",
        "/resend-verification",
        "/verify-email",
    ] {
        let page = browser.get(&app, path).await;
        assert_eq!(page.status, StatusCode::OK);
        assert_eq!(page.page["csrf"], csrf);
        assert_eq!(browser.cookies["bcm_preauth"], token);
        assert!(!page.headers.contains_key(header::SET_COOKIE));
    }
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let table = crate::repo::table(&tx.connection(), &f.r, "request_principals").unwrap();
    let row = tx
        .connection()
        .fetch_optional(
            &format!("SELECT created_at, expires_at FROM {table} WHERE token_hash = ?"),
            &[crate::core::secret_hash(&token).into()],
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.time("expires_at").unwrap() - row.time("created_at").unwrap(),
        Duration::hours(2)
    );
    tx.rollback().await.unwrap();
    let first_form = form(
        &first.page,
        &[
            ("identifier", "ordinary_member"),
            ("password", "Synthetic password 123"),
        ],
    );
    assert_eq!(
        browser.post(&app, "/login", &first_form).await.status,
        StatusCode::OK
    );
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    tx.connection()
        .execute(
            &format!("UPDATE {table} SET expires_at = ? WHERE token_hash = ?"),
            &[
                DbValue::Time(Utc::now() - Duration::seconds(1)),
                crate::core::secret_hash(&token).into(),
            ],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        browser.post(&app, "/login", &first_form).await.status,
        StatusCode::FORBIDDEN
    );
    let replacement = browser.get(&app, "/login").await;
    assert_eq!(replacement.status, StatusCode::OK);
    assert_ne!(browser.cookies["bcm_preauth"], token);
    assert_ne!(browser.cookies["bcm_preauth_csrf"], csrf);
}

#[tokio::test]
async fn approved_registration_normalization_settings_and_invitation_expiry_are_real_routes() {
    let f = http_fixture().await;
    f.enable().await;
    let state = approved_state(&f);
    let service = state.service.clone().unwrap();
    let app = router(state);
    let mut browser = Browser::default();
    let input = f.registration();
    for username in ["Adm1n", "Orchard_42"] {
        let page = browser.get(&app, "/register").await;
        assert_eq!(
            browser
                .post(
                    &app,
                    "/register",
                    &form(
                        &page.page,
                        &[
                            ("invitation", input.invitation.expose()),
                            ("username", username),
                            ("email", "  Orchard.Name+Tag@EXAMPLE.INVALID  "),
                            ("password", input.password.expose()),
                            ("consent", "yes"),
                        ]
                    )
                )
                .await
                .status,
            if username == "Adm1n" {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::OK
            }
        );
        assert_eq!(
            count(&f, "users").await,
            if username == "Adm1n" { 1 } else { 2 }
        );
    }
    let page = browser.get(&app, "/login").await;
    assert_eq!(
        browser
            .post(
                &app,
                "/login",
                &form(
                    &page.page,
                    &[
                        ("identifier", "orchard.name+tag@example.invalid"),
                        ("password", input.password.expose())
                    ]
                )
            )
            .await
            .status,
        StatusCode::OK
    );
    let identity = service
        .authenticate(&browser.cookies["bcm_session"], Utc::now())
        .await
        .unwrap()
        .unwrap();
    let profile = service.profile(&identity, Utc::now()).await.unwrap();
    assert_eq!(profile.public.username, "Orchard_42");
    assert_eq!(profile.email, "Orchard.Name+Tag@example.invalid");
    let page = browser.get(&app, "/me/invites").await;
    assert_eq!(
        browser
            .post(&app, "/me/invites", &form(&page.page, &[("days", "3")]))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let (_, token) = latest_token(&f, "verify_email").await;
    let page = browser.get(&app, "/verify-email").await;
    assert_eq!(
        browser
            .post(
                &app,
                "/verify-email",
                &form(&page.page, &[("token", &token)])
            )
            .await
            .status,
        StatusCode::OK
    );
    // DEC-054：验证成功即自动授予 T0；T1 仍由 trust::core 的确定性规则单独评估。
    assert_eq!(
        service
            .profile(&identity, Utc::now())
            .await
            .unwrap()
            .public
            .trust_level,
        Some("t0")
    );
    for invalid in ["0", "8", "1.5", "-1", ""] {
        let page = browser.get(&app, "/me/invites").await;
        assert_eq!(
            browser
                .post(&app, "/me/invites", &form(&page.page, &[("days", invalid)]))
                .await
                .status,
            StatusCode::BAD_REQUEST
        );
    }
    for days in [None, Some("7")] {
        let page = browser.get(&app, "/me/invites").await;
        assert!(
            page.page["fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["name"] == "days" && v["value"] == "3")
        );
        let extra = days.map(|v| vec![("days", v)]).unwrap_or_default();
        assert_eq!(
            browser
                .post(&app, "/me/invites", &form(&page.page, &extra))
                .await
                .status,
            StatusCode::OK
        );
    }
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let table = crate::repo::table(&tx.connection(), &f.r, "invitations").unwrap();
    let rows = tx
        .connection()
        .fetch_all(
            &format!("SELECT created_at, expires_at FROM {table} WHERE creator_id = ? ORDER BY id"),
            &[identity.user_id().into()],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    for (row, days) in rows.iter().zip([3, 7]) {
        assert_eq!(
            row.time("expires_at").unwrap() - row.time("created_at").unwrap(),
            Duration::days(days)
        );
    }
    tx.rollback().await.unwrap();
    let page = browser.get(&app, "/settings/profile").await;
    let bio = "果".repeat(500);
    assert_eq!(
        browser
            .post(
                &app,
                "/settings/profile",
                &form(&page.page, &[("bio", &bio)])
            )
            .await
            .status,
        StatusCode::OK
    );
    let page = browser.get(&app, "/settings/profile").await;
    assert_eq!(
        browser
            .post(
                &app,
                "/settings/profile",
                &form(&page.page, &[("bio", &"果".repeat(501))])
            )
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn approved_resend_and_recovery_reissue_invalidate_old_tokens_but_same_key_does_not() {
    let f = http_fixture().await;
    registered(&f).await;
    let app = router(approved_state(&f));
    let mut browser = Browser::default();
    let (_, original) = latest_token(&f, "verify_email").await;
    let page = browser.get(&app, "/resend-verification").await;
    let request = form(&page.page, &[("identifier", "member@example.invalid")]);
    let first = browser.post(&app, "/resend-verification", &request).await;
    assert_eq!(first.status, StatusCode::OK);
    let mail_count = count(&f, "mail_requests").await;
    let (token_id, new) = latest_token(&f, "verify_email").await;
    assert_ne!(original, new);
    assert_eq!(
        browser
            .post(&app, "/resend-verification", &request)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(count(&f, "mail_requests").await, mail_count);
    assert_eq!(latest_token(&f, "verify_email").await.0, token_id);
    let page = browser.get(&app, "/resend-verification").await;
    let absent = browser
        .post(
            &app,
            "/resend-verification",
            &form(&page.page, &[("identifier", "absent_member")]),
        )
        .await;
    assert_eq!(
        (absent.status, &absent.page["flash"]),
        (first.status, &first.page["flash"])
    );
    assert_eq!(count(&f, "mail_requests").await, mail_count);
    for (token, status) in [
        (&original, StatusCode::BAD_REQUEST),
        (&new, StatusCode::OK),
        (&new, StatusCode::BAD_REQUEST),
    ] {
        let page = browser.get(&app, "/verify-email").await;
        assert_eq!(
            browser
                .post(
                    &app,
                    "/verify-email",
                    &form(&page.page, &[("token", token)])
                )
                .await
                .status,
            status
        );
    }
    assert_eq!(login(&mut browser, &app).await.status, StatusCode::OK);
    let mut old_session = browser.clone();
    let mut recovery = Vec::new();
    for _ in 0..2 {
        let page = browser.get(&app, "/forgot-password").await;
        let request = form(&page.page, &[("identifier", "ordinary_member")]);
        assert_eq!(
            browser
                .post(&app, "/forgot-password", &request)
                .await
                .status,
            StatusCode::OK
        );
        let token = latest_token(&f, "reset_password").await;
        let mail_count = count(&f, "mail_requests").await;
        assert_eq!(
            browser
                .post(&app, "/forgot-password", &request)
                .await
                .status,
            StatusCode::OK
        );
        assert_eq!(count(&f, "mail_requests").await, mail_count);
        assert_eq!(latest_token(&f, "reset_password").await.0, token.0);
        recovery.push(token.1);
    }
    assert_ne!(recovery[0], recovery[1]);
    for (token, status) in [
        (&recovery[0], StatusCode::BAD_REQUEST),
        (&recovery[1], StatusCode::OK),
        (&recovery[1], StatusCode::BAD_REQUEST),
    ] {
        let page = browser.get(&app, "/reset-password").await;
        assert_eq!(
            browser
                .post(
                    &app,
                    "/reset-password",
                    &form(
                        &page.page,
                        &[("token", token), ("password", "New synthetic password 42")]
                    )
                )
                .await
                .status,
            status
        );
    }
    assert_eq!(
        old_session.get(&app, "/settings/profile").await.status,
        StatusCode::UNAUTHORIZED
    );
    let page = browser.get(&app, "/login").await;
    assert_eq!(
        browser
            .post(
                &app,
                "/login",
                &form(
                    &page.page,
                    &[
                        ("identifier", "ordinary_member"),
                        ("password", "New synthetic password 42")
                    ]
                )
            )
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn exhausted_soft_pairs_hide_private_hard_account_buckets_and_never_execute_limited_commands()
{
    use platform::{
        cache::TokenBucketLimiter, config::Environment, integrations::local::LocalRoot,
    };
    let f = http_fixture().await;
    registered(&f).await;
    for (path, action, hard_capacity, soft_capacity) in [
        ("/login", "login", 10, 20),
        ("/forgot-password", "recovery", 3, 5),
        ("/resend-verification", "recovery", 3, 5),
    ] {
        let local = LocalRoot::new(
            &f.dir
                .join(format!("hard-soft-{}", path.trim_start_matches('/'))),
            Environment::Test,
        )
        .unwrap();
        let mut state = approved_state(&f);
        let service = state
            .service
            .as_deref()
            .unwrap()
            .clone()
            .with_challenge_provider(Arc::new(crate::challenge::LocalChallengeProvider::new(
                local.clone(),
                f.issuer.clone(),
            )));
        let account = service
            .rate_subject("ordinary_member", None, None)
            .await
            .unwrap();
        state.service = Some(Arc::new(service));
        let rate = RateProtection::approved_v1(true, Arc::new(TokenBucketLimiter::new()));
        state.rate = Some(rate.clone());
        let app = router(state);
        let mut browser = Browser::default();
        browser.get(&app, path).await;
        // Exhaust only the existing account's hard bucket, on a different device/IP/network.
        for _ in 0..hard_capacity {
            rate.check(
                action,
                &account,
                "192.0.3.17".parse().unwrap(),
                "seed-device",
            )
            .await
            .unwrap();
        }
        for _ in 0..soft_capacity {
            assert!(
                !rate
                    .check_soft(
                        action,
                        "192.0.2.15".parse().unwrap(),
                        &browser.cookies["bcm_device"]
                    )
                    .await
                    .unwrap()
            );
        }
        let mails = count(&f, "mail_requests").await;
        let sessions = count(&f, "sessions").await;
        let mut signatures = Vec::new();
        for identifier in ["ordinary_member", "member@example.invalid", "absent_member"] {
            let page = browser.get(&app, path).await;
            let mut values = vec![("identifier", identifier)];
            if path == "/login" {
                values.push(("password", "Synthetic password 123"));
            }
            let challenge = browser.post(&app, path, &form(&page.page, &values)).await;
            assert_eq!(
                challenge.status,
                StatusCode::PRECONDITION_REQUIRED,
                "{path} {identifier}"
            );
            signatures.push((
                challenge.status,
                challenge.page["title"].clone(),
                challenge.page["flash"].clone(),
                challenge.page["error"].clone(),
            ));
            let id = challenge.page["fields"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["name"] == "challenge_id")
                .unwrap()["value"]
                .as_str()
                .unwrap();
            let artifact: Value =
                serde_json::from_slice(&local.read("processed", id, "json", 16_384).await.unwrap())
                    .unwrap();
            values.push(("challenge_proof", artifact["proof"].as_str().unwrap()));
            let completed = browser
                .post(&app, path, &form(&challenge.page, &values))
                .await;
            assert_eq!(
                completed.status,
                if path == "/login" {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::OK
                }
            );
            assert!(!browser.cookies.contains_key("bcm_session"));
        }
        assert!(signatures.windows(2).all(|v| v[0] == v[1]));
        assert_eq!(count(&f, "mail_requests").await, mails);
        assert_eq!(count(&f, "sessions").await, sessions);
    }
}

#[tokio::test]
async fn expired_unconsumed_challenge_proof_has_same_response_with_or_without_private_hard_limit() {
    use platform::{
        cache::TokenBucketLimiter, config::Environment, integrations::local::LocalRoot,
    };
    let f = http_fixture().await;
    registered(&f).await;
    for path in ["/forgot-password", "/resend-verification"] {
        let local = LocalRoot::new(
            &f.dir
                .join(format!("expired-proof-{}", path.trim_start_matches('/'))),
            Environment::Test,
        )
        .unwrap();
        let mut state = approved_state(&f);
        let service = state
            .service
            .as_deref()
            .unwrap()
            .clone()
            .with_challenge_provider(Arc::new(crate::challenge::LocalChallengeProvider::new(
                local.clone(),
                f.issuer.clone(),
            )));
        let account = service
            .rate_subject("ordinary_member", None, None)
            .await
            .unwrap();
        state.service = Some(Arc::new(service));
        let rate = RateProtection::approved_v1(true, Arc::new(TokenBucketLimiter::new()));
        state.rate = Some(rate.clone());
        let app = router(state);
        let mut browser = Browser::default();
        browser.get(&app, path).await;
        for _ in 0..3 {
            rate.check(
                "recovery",
                &account,
                "192.0.3.17".parse().unwrap(),
                "seed-device",
            )
            .await
            .unwrap();
        }
        for _ in 0..5 {
            assert!(
                !rate
                    .check_soft(
                        "recovery",
                        "192.0.2.15".parse().unwrap(),
                        &browser.cookies["bcm_device"]
                    )
                    .await
                    .unwrap()
            );
        }
        let before = count(&f, "mail_requests").await;
        let mut signatures = Vec::new();
        for identifier in ["ordinary_member", "absent_member"] {
            let page = browser.get(&app, path).await;
            let challenge = browser
                .post(&app, path, &form(&page.page, &[("identifier", identifier)]))
                .await;
            assert_eq!(challenge.status, StatusCode::PRECONDITION_REQUIRED);
            let id = challenge.page["fields"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["name"] == "challenge_id")
                .unwrap()["value"]
                .as_str()
                .unwrap();
            let artifact: Value =
                serde_json::from_slice(&local.read("processed", id, "json", 16_384).await.unwrap())
                    .unwrap();
            let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
            let table = crate::repo::table(&tx.connection(), &f.r, "challenges").unwrap();
            tx.connection()
                .execute(
                    &format!("UPDATE {table} SET expires_at = ? WHERE challenge_id = ?"),
                    &[DbValue::Time(Utc::now() - Duration::seconds(1)), id.into()],
                )
                .await
                .unwrap();
            tx.commit().await.unwrap();
            let expired = browser
                .post(
                    &app,
                    path,
                    &form(
                        &challenge.page,
                        &[
                            ("identifier", identifier),
                            ("challenge_proof", artifact["proof"].as_str().unwrap()),
                        ],
                    ),
                )
                .await;
            assert_eq!(expired.status, StatusCode::BAD_REQUEST);
            signatures.push((
                expired.status,
                expired.page["error"].clone(),
                expired.page["flash"].clone(),
            ));
        }
        assert_eq!(signatures[0], signatures[1]);
        assert_eq!(count(&f, "mail_requests").await, before);
    }
}

#[tokio::test]
async fn registration_challenge_requires_reentering_invitation_password_and_same_policy_versions() {
    use platform::{
        cache::TokenBucketLimiter, config::Environment, integrations::local::LocalRoot,
    };
    let f = http_fixture().await;
    f.enable().await;
    let local = LocalRoot::new(
        &f.dir.join("registration-local-provider"),
        Environment::Test,
    )
    .unwrap();
    let mut state = approved_state(&f);
    let service = state
        .service
        .as_deref()
        .unwrap()
        .clone()
        .with_challenge_provider(Arc::new(crate::challenge::LocalChallengeProvider::new(
            local.clone(),
            f.issuer.clone(),
        )));
    state.service = Some(Arc::new(service));
    let rate = RateProtection::approved_v1(true, Arc::new(TokenBucketLimiter::new()));
    state.rate = Some(rate.clone());
    let app = router(state);
    let mut browser = Browser::default();
    let page = browser.get(&app, "/register").await;
    for _ in 0..5 {
        assert!(
            !rate
                .check_soft(
                    "registration",
                    "192.0.2.15".parse().unwrap(),
                    &browser.cookies["bcm_device"]
                )
                .await
                .unwrap()
        );
    }
    let input = f.registration();
    let values = [
        ("username", input.username.as_str()),
        ("email", input.email.as_str()),
        ("invitation", input.invitation.expose()),
        ("password", input.password.expose()),
        ("consent", "yes"),
    ];
    let challenge = browser
        .post(&app, "/register", &form(&page.page, &values))
        .await;
    assert_eq!(challenge.status, StatusCode::PRECONDITION_REQUIRED);
    assert_eq!(challenge.page["key"], page.page["key"]);
    assert_eq!(count(&f, "users").await, 1);
    assert_eq!(count(&f, "mail_requests").await, 0);
    let serialized = challenge.page.to_string();
    assert!(!serialized.contains(input.invitation.expose()));
    assert!(!serialized.contains(input.password.expose()));
    assert_eq!(challenge.page["policy_consent"], true);
    for secret in ["invitation", "password", "challenge_proof"] {
        assert!(
            challenge.page["fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["name"] == secret && v["value"] == "")
        );
    }
    let id = challenge.page["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "challenge_id")
        .unwrap()["value"]
        .as_str()
        .unwrap();
    let artifact: Value =
        serde_json::from_slice(&local.read("processed", id, "json", 16_384).await.unwrap())
            .unwrap();
    let mut values = values.to_vec();
    values.push(("challenge_proof", artifact["proof"].as_str().unwrap()));
    let request = form(&challenge.page, &values);
    assert_eq!(
        browser.post(&app, "/register", &request).await.status,
        StatusCode::OK
    );
    assert_eq!(count(&f, "users").await, 2);
    assert_eq!(count(&f, "mail_requests").await, 1);
    assert_eq!(
        browser.post(&app, "/register", &request).await.status,
        StatusCode::OK
    );
    assert_eq!(count(&f, "users").await, 2);
    assert_eq!(count(&f, "mail_requests").await, 1);
}

#[tokio::test]
async fn approved_soft_challenge_http_binds_original_request_and_local_proof_without_secret_reflection()
 {
    use platform::{
        cache::TokenBucketLimiter, config::Environment, integrations::local::LocalRoot,
    };
    let f = http_fixture().await;
    registered(&f).await;
    let local = LocalRoot::new(&f.dir.join("http-local-provider"), Environment::Test).unwrap();
    let mut state = approved_state(&f);
    let service = state
        .service
        .as_deref()
        .unwrap()
        .clone()
        .with_challenge_provider(Arc::new(crate::challenge::LocalChallengeProvider::new(
            local.clone(),
            f.issuer.clone(),
        )));
    state.service = Some(Arc::new(service));
    let rate = RateProtection::approved_v1(true, Arc::new(TokenBucketLimiter::new()));
    state.rate = Some(rate.clone());
    let app = router(state);
    let mut browser = Browser::default();
    let page = browser.get(&app, "/login").await;
    let device = browser.cookies["bcm_device"].clone();
    // Populate the same approved independent soft buckets; hard budgets stay available.
    // This models requests spread across account identifiers and a busy IP.
    for _ in 0..20 {
        assert!(
            !rate
                .check_soft("login", "192.0.2.15".parse().unwrap(), &device)
                .await
                .unwrap()
        );
    }
    let original = form(
        &page.page,
        &[
            ("identifier", "ordinary_member"),
            ("password", "Synthetic password 123"),
        ],
    );
    let challenge = browser.post(&app, "/login", &original).await;
    assert_eq!(challenge.status, StatusCode::PRECONDITION_REQUIRED);
    assert_eq!(challenge.page["key"], page.page["key"]);
    assert!(
        challenge.page["title"]
            .as_str()
            .unwrap()
            .contains("LOCAL TEST")
    );
    assert!(
        !challenge
            .page
            .to_string()
            .contains("Synthetic password 123")
    );
    assert!(!browser.cookies.contains_key("bcm_session"));
    let id = challenge.page["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "challenge_id")
        .unwrap()["value"]
        .as_str()
        .unwrap();
    let artifact: Value =
        serde_json::from_slice(&local.read("processed", id, "json", 16_384).await.unwrap())
            .unwrap();
    assert!(
        artifact["provider"]
            .as_str()
            .unwrap()
            .contains("LOCAL TEST")
    );
    let proof = artifact["proof"].as_str().unwrap();
    assert!(!challenge.page.to_string().contains(proof));
    let submission = form(
        &challenge.page,
        &[
            ("identifier", "ordinary_member"),
            ("password", "Synthetic password 123"),
            ("challenge_proof", proof),
        ],
    );
    for (field, value) in [
        ("password", "Changed synthetic password"),
        ("idempotency_key", "changed_key"),
        ("challenge_proof", "wrong_proof"),
    ] {
        let mut tampered = submission.clone();
        tampered
            .iter_mut()
            .find(|(name, _)| name == field)
            .unwrap()
            .1 = value.into();
        assert_eq!(
            browser.post(&app, "/login", &tampered).await.status,
            StatusCode::BAD_REQUEST
        );
    }
    let mut other = Browser::default();
    let other_page = other.get(&app, "/login").await;
    let mut cross_principal = submission.clone();
    cross_principal
        .iter_mut()
        .find(|(name, _)| name == "csrf")
        .unwrap()
        .1 = other_page.page["csrf"].as_str().unwrap().into();
    assert_eq!(
        other.post(&app, "/login", &cross_principal).await.status,
        StatusCode::BAD_REQUEST
    );
    let successful = browser.post(&app, "/login", &submission).await;
    assert_eq!(successful.status, StatusCode::OK);
    assert!(browser.cookies.contains_key("bcm_session"));
    let session_count = count(&f, "sessions").await;
    // A committed original receipt stays queryable/replayable after its proof expires.
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let table = crate::repo::table(&tx.connection(), &f.r, "challenges").unwrap();
    tx.connection()
        .execute(
            &format!("UPDATE {table} SET expires_at = ? WHERE challenge_id = ?"),
            &[DbValue::Time(Utc::now() - Duration::seconds(1)), id.into()],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        browser.post(&app, "/login", &submission).await.status,
        StatusCode::OK
    );
    assert_eq!(count(&f, "sessions").await, session_count);
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let table = crate::repo::table(&tx.connection(), &f.r, "challenges").unwrap();
    let row = tx
        .connection()
        .fetch_optional(
            &format!(
                "SELECT consumed, proof_hash, request_hash FROM {table} WHERE challenge_id = ?"
            ),
            &[id.into()],
        )
        .await
        .unwrap()
        .unwrap();
    assert!(row.boolean("consumed").unwrap());
    assert_ne!(row.text("proof_hash").unwrap(), proof);
    assert!(
        !row.text("request_hash")
            .unwrap()
            .contains("Synthetic password")
    );
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn challenged_recovery_fails_closed_without_local_provider_and_disabled_redis_is_unprotected()
{
    use platform::cache::TokenBucketLimiter;
    let f = http_fixture().await;
    registered(&f).await;
    let mut state = approved_state(&f);
    let rate = RateProtection::approved_v1(true, Arc::new(TokenBucketLimiter::new()));
    state.rate = Some(rate.clone());
    let app = router(state);
    let mut browser = Browser::default();
    let page = browser.get(&app, "/forgot-password").await;
    for _ in 0..5 {
        assert!(
            !rate
                .check_soft(
                    "recovery",
                    "192.0.2.15".parse().unwrap(),
                    &browser.cookies["bcm_device"]
                )
                .await
                .unwrap()
        );
    }
    let request = form(&page.page, &[("identifier", "ordinary_member")]);
    let before = count(&f, "mail_requests").await;
    assert_eq!(
        browser
            .post(&app, "/forgot-password", &request)
            .await
            .status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(count(&f, "mail_requests").await, before);
    let unprotected = router(approved_state(&f));
    assert_eq!(
        browser
            .post(&unprotected, "/forgot-password", &request)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(count(&f, "mail_requests").await, before + 1);
}
