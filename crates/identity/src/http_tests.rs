//! HTTP integration against real isolated SQLite and the ordinary-account core.
//! A projection renderer tests transport and view contracts independently of the A00 HTML.
use crate::{
    contract::IdentityError,
    core_tests::{Fixture, at},
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
        .register(&f.context().await, f.registration(), at())
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
    let f = Fixture::new().await;
    let app = app(&f);
    let mut browser = Browser::default();
    for path in [
        "/login",
        "/register",
        "/forgot-password",
        "/reset-password",
        "/verify-email",
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
    let f = Fixture::new().await;
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
    let f = Fixture::new().await;
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
    let f = Fixture::new().await;
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
    let f = Fixture::new().await;
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
        _: RateLimitPolicy,
    ) -> BoxFuture<'a, Result<RateLimitDecision, CacheError>> {
        self.keys.lock().unwrap().push(key.clone());
        let allowed = !self.limited.load(Ordering::SeqCst);
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
    let f = Fixture::new().await;
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
        assert_eq!(limiter.take_keys().len(), 4);
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
        assert_eq!(limiter.take_keys().len(), 4);
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
    let f = Fixture::new().await;
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
        assert_eq!(keys.len(), 4);
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
    assert_eq!(changed.len(), 4);
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
    assert_eq!(invalid_mailbox.len(), 4);
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
    let f = Fixture::new().await;
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
    let f = Fixture::new().await;
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
    // Explicit synthetic capability fixture only: no HTTP or production grant path is exercised.
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let snapshot =
        f.r.table(tx.connection().backend(), "trust", "identity_snapshots")
            .unwrap();
    tx.connection().execute(&format!("UPDATE {snapshot} SET level = ?, email_verified = ?, invitation_allowed = ?, policy_version = ?, reason_code = ? WHERE user_id = ?"), &["t1".into(), DbValue::Bool(true), DbValue::Bool(true), 1_i64.into(), "approved_policy".into(), identity.user_id().into()]).await.unwrap();
    let users =
        f.r.table(tx.connection().backend(), "identity", "users")
            .unwrap();
    tx.connection()
        .execute(
            &format!("UPDATE {users} SET email_verified = ? WHERE id = ?"),
            &[DbValue::Bool(true), identity.user_id().into()],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
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
    assert!(
        profile.page["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["label"] == "信任等级" && row["value"] == "t1")
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
            .any(|row| row["label"] == "信任等级" && row["value"] == "t1")
    );
}
