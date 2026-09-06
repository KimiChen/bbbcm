//! HTTP parses untrusted transport, invokes core services and renders safe projections.
use crate::{
    contract::*,
    core::{IdentityService, random_secret},
    rate::RateProtection,
    web_view::{Field, Page, PageRenderer, Row},
};
use axum::{
    Form, Router,
    extract::{ConnectInfo, DefaultBodyLimit, OriginalUri, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use platform::flags::{Flag, FlagReader};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};

#[derive(Clone)]
pub struct IdentityWebState {
    pub service: Option<Arc<IdentityService>>,
    pub flags: Option<Arc<dyn FlagReader>>,
    pub renderer: Arc<dyn PageRenderer>,
    pub secure_cookies: bool,
    pub rate: Option<RateProtection>,
    pub risk_signals: Option<Arc<crate::rate::RiskSignalKey>>,
}
pub fn router<S>(state: IdentityWebState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let mut router = Router::new();
    for route in [
        "/login",
        "/register",
        "/verify-email",
        "/resend-verification",
        "/forgot-password",
        "/reset-password",
        "/confirm-email",
        "/reauth",
        "/logout",
        "/settings/profile",
        "/settings/email",
        "/settings/username",
        "/settings/password",
        "/settings/sessions",
        "/settings/privacy",
        "/settings/invitations",
        "/me/invites",
    ] {
        router = router.route(route, get(page).post(submit));
    }
    router
        .route("/settings/sessions/revoke", axum::routing::post(submit))
        .route("/settings/invitations/revoke", axum::routing::post(submit))
        .route("/auth/operations", axum::routing::post(operation))
        .route("/u/{id}", get(public_profile))
        .route("/terms", get(policy))
        .route("/privacy", get(policy))
        .route("/rules", get(policy))
        .layer(DefaultBodyLimit::max(16_384))
        .with_state(state)
}
fn canonical_path(path: &str) -> &str {
    if path == "/me/invites" {
        "/settings/invitations"
    } else {
        path
    }
}
fn cookie_name(state: &IdentityWebState, name: &str) -> String {
    format!(
        "{}bcm_{name}",
        if state.secure_cookies { "__Host-" } else { "" }
    )
}
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let mut values = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .filter(|(key, _)| *key == name)
        .map(|(_, value)| value);
    let value = values.next()?;
    if values.next().is_some()
        || value.len() > 256
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    Some(value.into())
}
fn set_cookie(
    response: &mut Response,
    state: &IdentityWebState,
    name: &str,
    value: &str,
    max_age: Option<i64>,
) {
    let mut text = format!(
        "{}={value}; Path=/; HttpOnly; SameSite=Lax",
        cookie_name(state, name)
    );
    if state.secure_cookies {
        text.push_str("; Secure")
    }
    if let Some(seconds) = max_age {
        text.push_str(&format!("; Max-Age={seconds}"))
    }
    if let Ok(value) = text.parse() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
}
fn response(state: &IdentityWebState, page: &Page, status: StatusCode) -> Response {
    let mut result = match state.renderer.render(page) {
        Ok(body) => (status, Html(body)).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "服务暂不可用").into_response(),
    };
    let h = result.headers_mut();
    h.insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
    h.insert("referrer-policy", "no-referrer".parse().unwrap());
    h.insert(
        "x-robots-tag",
        "noindex, nofollow, noarchive".parse().unwrap(),
    );
    h.insert("x-content-type-options", "nosniff".parse().unwrap());
    h.insert("content-security-policy","default-src 'self'; script-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; object-src 'none'".parse().unwrap());
    result
}
fn failure(state: &IdentityWebState, error: IdentityError) -> Response {
    let (status, message) = match error {
        IdentityError::Disabled => (StatusCode::NOT_FOUND, "功能暂未开放"),
        IdentityError::Unauthenticated => (StatusCode::UNAUTHORIZED, "请先登录"),
        IdentityError::Csrf => (StatusCode::FORBIDDEN, "请求校验失败，请重新打开表单"),
        IdentityError::ReauthenticationRequired => {
            (StatusCode::PRECONDITION_REQUIRED, "请先重新验证密码")
        }
        IdentityError::PolicyPending => (StatusCode::SERVICE_UNAVAILABLE, "账号服务尚未开放"),
        IdentityError::CommitUnknown { .. } => {
            (StatusCode::ACCEPTED, "提交结果尚待确认，请查询原操作")
        }
        IdentityError::ChallengeRequired { .. } => (
            StatusCode::PRECONDITION_REQUIRED,
            "需要完成本地测试挑战后再提交原请求。",
        ),
        IdentityError::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "请求暂未确认，请使用原请求重试",
        ),
        IdentityError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "请求过于频繁，请稍后再试"),
        IdentityError::Rejected => (StatusCode::BAD_REQUEST, "请求未能完成，请检查输入后重试"),
    };
    let mut page = Page::new("账号服务", "");
    page.show_form = false;
    page.frame.error = Some(message);
    response(state, &page, status)
}
async fn gate(
    state: &IdentityWebState,
    path: &str,
    write: bool,
) -> Result<Arc<IdentityService>, IdentityError> {
    let flags = state
        .flags
        .as_ref()
        .ok_or(IdentityError::Disabled)?
        .load()
        .await
        .map_err(|_| IdentityError::Unavailable)?;
    let path = canonical_path(path);
    let policy_read = matches!(path, "/terms" | "/privacy" | "/rules");
    let flag = if path == "/register" {
        Flag::Registration
    } else if matches!(
        path,
        "/verify-email" | "/confirm-email" | "/resend-verification"
    ) {
        Flag::EmailVerification
    } else {
        Flag::PublicRead
    };
    if !(flags.get(flag).enabled || (policy_read && flags.get(Flag::Registration).enabled))
        || (path == "/register"
            && (!flags.get(Flag::EmailVerification).enabled
                || flags.get(Flag::RegistrationLock).enabled))
        || (path.contains("invitations") && flags.get(Flag::InvitationLock).enabled)
        || (write && flags.get(Flag::ReadOnlyMode).enabled)
    {
        return Err(IdentityError::Disabled);
    }
    state.service.clone().ok_or(IdentityError::Unavailable)
}
async fn session(
    state: &IdentityWebState,
    service: &IdentityService,
    headers: &HeaderMap,
) -> Result<SessionIdentity, IdentityError> {
    let raw =
        cookie(headers, &cookie_name(state, "session")).ok_or(IdentityError::Unauthenticated)?;
    service
        .authenticate(&raw, chrono::Utc::now())
        .await?
        .ok_or(IdentityError::Unauthenticated)
}
fn row(label: impl Into<String>, value: impl Into<String>) -> Row {
    Row {
        label: label.into(),
        value: value.into(),
        action: None,
        action_label: "",
        reference: String::new(),
    }
}
fn trust_label(level: Option<&str>) -> &'static str {
    match level {
        Some("t0") => "T0",
        Some("t1") => "T1",
        _ => "待确认",
    }
}
fn value<'a>(form: &'a HashMap<String, String>, name: &str) -> Result<&'a str, IdentityError> {
    form.get(name)
        .map(String::as_str)
        .ok_or(IdentityError::Rejected)
}
fn origin(headers: &HeaderMap, secure: bool) -> Result<(), IdentityError> {
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v == "cross-site")
    {
        return Err(IdentityError::Csrf);
    }
    if let Some(got) = headers.get(header::ORIGIN) {
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .ok_or(IdentityError::Csrf)?;
        let expected = format!("{}://{host}", if secure { "https" } else { "http" });
        if got.to_str().ok() != Some(expected.as_str()) {
            return Err(IdentityError::Csrf);
        }
    }
    Ok(())
}
fn context(
    state: &IdentityWebState,
    headers: &HeaderMap,
    form: &HashMap<String, String>,
) -> Result<RequestContext, IdentityError> {
    Ok(RequestContext::new(
        cookie(headers, &cookie_name(state, "preauth"))
            .ok_or(IdentityError::Csrf)?
            .into(),
        value(form, "csrf")?.to_owned().into(),
        value(form, "idempotency_key")?.into(),
    ))
}
fn device_summary(headers: &HeaderMap) -> &'static str {
    let Some(value) = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.len() <= 1024)
    else {
        return "unknown";
    };
    let value = value.to_ascii_lowercase();
    if value.contains("ipad") || value.contains("tablet") {
        "tablet"
    } else if value.contains("mobi") {
        "mobile"
    } else if value.contains("windows") || value.contains("macintosh") || value.contains("x11") {
        "desktop"
    } else {
        "unknown"
    }
}
async fn page(
    State(state): State<IdentityWebState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    match build_page(&state, uri.path(), &headers, &query).await {
        Ok(result) => result,
        Err(error) => failure(&state, error),
    }
}
async fn build_page(
    state: &IdentityWebState,
    path: &str,
    headers: &HeaderMap,
    _query: &HashMap<String, String>,
) -> Result<Response, IdentityError> {
    let path = canonical_path(path);
    let service = gate(state, path, false).await?;
    let (title, action) = match path {
        "/login" => ("登录", "/login"),
        "/register" => ("邀请注册", "/register"),
        "/verify-email" => ("验证邮箱", "/verify-email"),
        "/resend-verification" => ("重发验证邮件", "/resend-verification"),
        "/forgot-password" => ("找回密码", "/forgot-password"),
        "/reset-password" => ("重置密码", "/reset-password"),
        "/confirm-email" => ("确认新邮箱", "/confirm-email"),
        "/reauth" => ("重新验证密码", "/reauth"),
        "/logout" => ("退出登录", "/logout"),
        "/settings/profile" => ("个人资料", "/settings/profile"),
        "/settings/email" => ("修改邮箱", "/settings/email"),
        "/settings/username" => ("修改用户名", "/settings/username"),
        "/settings/password" => ("修改密码", "/settings/password"),
        "/settings/sessions" => ("登录设备", "/settings/sessions"),
        "/settings/privacy" => ("隐私设置", "/settings/privacy"),
        "/settings/invitations" => ("我的邀请", "/me/invites"),
        _ => return Err(IdentityError::Disabled),
    };
    let mut view = Page::new(title, action);
    let signed = path.starts_with("/settings/") || matches!(path, "/reauth" | "/logout");
    let mut grant = None;
    if signed {
        let identity = session(state, &service, headers).await?;
        view.csrf = cookie(headers, &cookie_name(state, "csrf")).ok_or(IdentityError::Csrf)?;
        crate::core::check_csrf(&identity, &view.csrf)?;
        if path.starts_with("/settings/") {
            let profile = service.profile(&identity, chrono::Utc::now()).await?;
            view.fields
                .push(Field::hidden("version", profile.version.to_string()));
            match path {
                "/settings/profile" => {
                    let mut bio = Field::input("bio", "简介", "textarea");
                    bio.required = false;
                    bio.value = profile.public.bio;
                    view.fields.push(bio);
                    view.preset_avatar = true;
                    view.rows
                        .push(row("信任等级", trust_label(profile.public.trust_level)));
                    view.rows.push(row("用户名", profile.public.username));
                    view.rows.push(row("邮箱", profile.email));
                    view.rows
                        .push(row("内容资料", "主题、回帖和贡献统计尚未接入"));
                }
                // PRIV-001／PRIV-002 的用户自助入口。两项分列渲染，勾选即公开；
                // DEC-054 批准的默认值是两项都公开，这里回显的是账号当前实际取值。
                "/settings/privacy" => {
                    view.fields.push(Field::checkbox(
                        "profile_lists_public",
                        "允许未登录访客查看我的主题和回帖列表",
                        profile.profile_lists_public,
                    ));
                    view.fields.push(Field::checkbox(
                        "topic_body_public",
                        "允许未登录访客查看我的主题正文",
                        profile.topic_body_public,
                    ));
                    view.rows.push(row(
                        "说明",
                        "关闭后未登录访客看不到对应内容，登录成员与本人不受影响；标题按版块规则保留。",
                    ));
                }
                "/settings/email" => view.fields.push(Field::input("email", "新邮箱", "email")),
                "/settings/username" => {
                    view.fields
                        .push(Field::input("username", "新用户名", "text"));
                    view.rows
                        .push(row("修改规则", "修改后冷却30天，旧用户名保留90天防抢注。"));
                }
                "/settings/password" => {
                    view.fields
                        .push(Field::input("password", "新密码", "password"))
                }
                "/settings/sessions" => {
                    view.submit = "退出其他设备";
                    for device in service.sessions(&identity, chrono::Utc::now()).await? {
                        view.rows.push(Row {
                            label: match device.summary.as_str() {
                                "desktop" => "桌面浏览器",
                                "mobile" => "手机浏览器",
                                "tablet" => "平板浏览器",
                                _ => "其他浏览器",
                            }
                            .into(),
                            value: format!(
                                "最近使用：{}{}",
                                device.last_seen_at,
                                if device.current {
                                    " · 当前设备"
                                } else {
                                    ""
                                }
                            ),
                            action: Some("/settings/sessions/revoke"),
                            action_label: "退出此设备",
                            reference: device.session_ref,
                        });
                    }
                }
                "/settings/invitations" => {
                    view.submit = "生成邀请码";
                    let mut days = Field::input("days", "有效天数（1至7天）", "number");
                    days.value = "3".into();
                    view.fields.push(days);
                    let invites = service.invitations(&identity, chrono::Utc::now()).await?;
                    view.rows.push(row(
                        "今日生成",
                        format!("{} / {}", invites.generated_today, invites.daily_limit),
                    ));
                    view.rows.push(row(
                        "未使用额度",
                        format!("{} / {}", invites.unused, invites.unused_limit),
                    ));
                    for invite in invites.records {
                        let available =
                            invite.state == "available" && invite.expires_at > chrono::Utc::now();
                        let label = match invite.state.as_str() {
                            "available" if available => "可使用",
                            "available" | "expired" => "已过期",
                            "used" => "已使用",
                            "revoked" => "已撤销",
                            "frozen" => "已冻结",
                            _ => "不可使用",
                        };
                        view.rows.push(Row {
                            label: invite.invitation_ref.clone(),
                            value: format!("{label} · 到期 {}", invite.expires_at),
                            action: available.then_some("/settings/invitations/revoke"),
                            action_label: "撤销邀请码",
                            reference: invite.invitation_ref,
                        });
                    }
                }
                _ => {}
            }
        } else if path == "/reauth" {
            view.fields
                .push(Field::input("password", "当前密码", "password"));
        }
    } else {
        let existing = cookie(headers, &cookie_name(state, "preauth"))
            .zip(cookie(headers, &cookie_name(state, "preauth_csrf")));
        if let Some((token, csrf)) = existing {
            match service
                .validate_public_context(
                    &RequestContext::new(
                        token.into(),
                        csrf.clone().into(),
                        view.idempotency_key.clone(),
                    ),
                    chrono::Utc::now(),
                )
                .await
            {
                Ok(()) => view.csrf = csrf,
                Err(IdentityError::Csrf) => {}
                Err(error) => return Err(error),
            }
        }
        if view.csrf.is_empty() {
            let preauth = service.create_preauth(chrono::Utc::now()).await?;
            view.csrf = preauth.csrf.expose().into();
            grant = Some(preauth);
        }
        match path {
            "/register" => {
                view.fields.extend([
                    Field::input("invitation", "邀请码", "text"),
                    Field::input("username", "用户名", "text"),
                    Field::input("email", "邮箱", "email"),
                    Field::input("password", "密码（8至256字符）", "password"),
                ]);
                let versions = service.current_policies().await?;
                for (kind, id) in versions.references() {
                    view.fields.push(Field::hidden(kind, id.to_string()));
                    view.rows.push(row(kind, format!("版本引用 {id}")));
                }
                view.policy_consent = true;
            }
            "/login" => view.fields.extend([
                Field::input("identifier", "用户名或邮箱", "text"),
                Field::input("password", "密码", "password"),
            ]),
            "/forgot-password" | "/resend-verification" => {
                view.fields
                    .push(Field::input("identifier", "用户名或邮箱", "text"))
            }
            "/verify-email" | "/confirm-email" | "/reset-password" => {
                view.fields
                    .push(Field::input("token", "邮件中的一次性凭据", "password"));
                if path == "/reset-password" {
                    view.fields
                        .push(Field::input("password", "新密码", "password"));
                }
            }
            _ => {}
        }
    }
    let mut result = response(state, &view, StatusCode::OK);
    if let Some(grant) = grant {
        set_cookie(
            &mut result,
            state,
            "preauth",
            grant.token.expose(),
            Some(7200),
        );
        set_cookie(
            &mut result,
            state,
            "preauth_csrf",
            grant.csrf.expose(),
            Some(7200),
        );
    }
    if cookie(headers, &cookie_name(state, "device")).is_none() {
        set_cookie(&mut result, state, "device", random_secret().expose(), None);
    }
    Ok(result)
}
async fn submit(
    State(state): State<IdentityWebState>,
    OriginalUri(uri): OriginalUri,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    headers: HeaderMap,
    form: Result<Form<HashMap<String, String>>, axum::extract::rejection::FormRejection>,
) -> Response {
    let Ok(Form(form)) = form else {
        return failure(&state, IdentityError::Rejected);
    };
    let result = async {
        origin(&headers, state.secure_cookies)?;
        let service = gate(&state, uri.path(), true).await?;
        submit_inner(
            &state,
            &service,
            uri.path(),
            peer.ok().map(|v| v.0),
            &headers,
            &form,
        )
        .await
    }
    .await;
    match result {
        Ok(result) => result,
        Err(IdentityError::CommitUnknown { operation_id }) => {
            pending(&state, &operation_id, scope_for_path(uri.path()), &form)
        }
        Err(IdentityError::ChallengeRequired { challenge_id }) => {
            challenge_page(&state, uri.path(), &challenge_id, &form)
        }
        Err(error) => failure(&state, error),
    }
}
async fn submit_inner(
    state: &IdentityWebState,
    service: &IdentityService,
    path: &str,
    peer: Option<SocketAddr>,
    headers: &HeaderMap,
    form: &HashMap<String, String>,
) -> Result<Response, IdentityError> {
    let path = canonical_path(path);
    let now = chrono::Utc::now();
    let mut view = Page::new("账号操作", "");
    view.show_form = false;
    let csrf = value(form, "csrf")?;
    let key = value(form, "idempotency_key")?;
    if matches!(
        path,
        "/login"
            | "/register"
            | "/forgot-password"
            | "/resend-verification"
            | "/reset-password"
            | "/verify-email"
            | "/confirm-email"
    ) {
        service
            .validate_public_context(&context(state, headers, form)?, now)
            .await?;
    }
    let mut challenge_required = false;
    let mut hard_rate_limited = false;
    if let Some(action) = match path {
        "/register" => Some("registration"),
        "/login" | "/reauth" => Some("login"),
        "/forgot-password" | "/reset-password" | "/resend-verification" => Some("recovery"),
        "/settings/invitations" => Some("invitation"),
        _ => None,
    } {
        let rate = state.rate.as_ref().ok_or(IdentityError::Unavailable)?;
        let account = if path == "/register" {
            value(form, "email")?
        } else {
            form.get("identifier").map(String::as_str).unwrap_or("")
        };
        let rate_session = if matches!(path, "/reauth" | "/settings/invitations") {
            Some(session(state, service, headers).await?)
        } else {
            None
        };
        if let Some(identity) = rate_session.as_ref() {
            crate::core::check_csrf(identity, csrf)?;
        }
        let normalized = if path == "/register" {
            service.registration_subject(account)
        } else {
            service
                .rate_subject(
                    account,
                    if path == "/reset-password" {
                        Some(value(form, "token")?)
                    } else {
                        None
                    },
                    rate_session.as_ref(),
                )
                .await?
        };
        let device = cookie(headers, &cookie_name(state, "device")).ok_or(IdentityError::Csrf)?;
        if rate.redis_enabled {
            let ip = peer.ok_or(IdentityError::Unavailable)?.ip();
            let hard = rate.check(action, &normalized, ip, &device).await;
            if matches!(
                path,
                "/login" | "/register" | "/forgot-password" | "/resend-verification"
            ) {
                challenge_required = rate.check_soft(action, ip, &device).await?;
            }
            match hard {
                Err(IdentityError::RateLimited)
                    if matches!(
                        path,
                        "/login" | "/forgot-password" | "/register" | "/resend-verification"
                    ) =>
                {
                    if challenge_required || form.contains_key("challenge_id") {
                        // A private account bucket must not alter the challenge response
                        // selected by public device/IP state. Core prepares/validates the
                        // challenge but cannot run a business command with this marker.
                        hard_rate_limited = true;
                    } else {
                        // Account aliases share a private bucket. A limited account must not
                        // expose that mapping through a distinct public status or work cost.
                        service.passwords.verify("limited-fixed-cost", None);
                        match path {
                            "/login" => return Err(IdentityError::Rejected),
                            "/register" => {
                                view.frame.flash =
                                    Some("请求已受理；符合条件时将生成邮箱验证邮件。")
                            }
                            "/resend-verification" => {
                                view.frame.flash =
                                    Some("请求已受理；符合条件时将生成邮箱验证邮件。")
                            }
                            _ => view.frame.flash = Some("请求已受理；符合条件时将生成恢复邮件。"),
                        }
                        return Ok(response(state, &view, StatusCode::OK));
                    }
                }
                result => {
                    result?;
                }
            }
        }
    }
    let signals = if matches!(path, "/register" | "/login" | "/settings/invitations") {
        if let Some(key) = state.risk_signals.as_ref() {
            let signed = if path == "/settings/invitations" {
                Some(session(state, service, headers).await?)
            } else {
                None
            };
            let subject = if path == "/register" {
                service.registration_subject(value(form, "email")?)
            } else {
                service
                    .rate_subject(
                        if path == "/register" {
                            value(form, "email")?
                        } else {
                            form.get("identifier").map(String::as_str).unwrap_or("")
                        },
                        None,
                        signed.as_ref(),
                    )
                    .await?
            };
            let device = cookie(headers, &cookie_name(state, "device"));
            key.signals(
                &subject,
                peer.map(|v| v.ip()),
                device.as_deref(),
                form.get("invitation").map(String::as_str),
            )?
        } else {
            trust::risk::RiskSignals::default()
        }
    } else {
        trust::risk::RiskSignals::default()
    };
    let request_context = || {
        let submission = match form.get("challenge_id") {
            Some(id) if !id.is_empty() => Some(crate::challenge::ChallengeSubmission {
                challenge_id: id.clone(),
                proof: value(form, "challenge_proof")?.to_owned().into(),
            }),
            _ => None,
        };
        context(state, headers, form).map(|c| {
            c.with_risk_signals(signals.clone())
                .with_challenge(challenge_required, submission)
                .with_hard_rate_limited(hard_rate_limited)
        })
    };
    let mut session_grant = None;
    let mut clear = false;
    match path {
        "/register" => {
            if value(form, "consent")? != "yes" {
                return Err(IdentityError::Rejected);
            }
            let input = RegistrationInput {
                username: value(form, "username")?.into(),
                email: value(form, "email")?.into(),
                password: value(form, "password")?.to_owned().into(),
                invitation: value(form, "invitation")?.to_owned().into(),
                policies: ["terms", "privacy", "community"]
                    .iter()
                    .map(|kind| Ok(((*kind).into(), value(form, kind)?.into())))
                    .collect::<Result<_, IdentityError>>()?,
            };
            service.register(&request_context()?, input, now).await?;
            view.frame.flash = Some("请求已受理；符合条件时将生成邮箱验证邮件。");
        }
        "/login" => {
            let result = service
                .login(
                    &request_context()?,
                    LoginInput {
                        identifier: value(form, "identifier")?.into(),
                        password: value(form, "password")?.to_owned().into(),
                        device_summary: device_summary(headers).into(),
                    },
                    now,
                )
                .await?;
            session_grant = result.grant;
            if session_grant.is_none() {
                view.frame.error = Some("账号或密码不正确，或当前无法登录。");
            } else {
                view.frame.flash = Some("登录成功，可前往账号设置。");
            }
        }
        "/forgot-password" => {
            service
                .request_reset(&request_context()?, value(form, "identifier")?, now)
                .await?;
            view.frame.flash = Some("请求已受理；符合条件时将生成恢复邮件。");
        }
        "/resend-verification" => {
            service
                .resend_verification(&request_context()?, value(form, "identifier")?, now)
                .await?;
            view.frame.flash = Some("请求已受理；符合条件时将生成邮箱验证邮件。");
        }
        "/verify-email" => {
            service
                .consume_verification(&request_context()?, value(form, "token")?, now)
                .await?;
            view.frame.flash = Some("邮箱验证完成。");
        }
        "/reset-password" => {
            service
                .reset_password(
                    &request_context()?,
                    value(form, "token")?,
                    value(form, "password")?,
                    now,
                )
                .await?;
            clear = true;
            view.frame.flash = Some("密码已重置，请重新登录。");
        }
        "/confirm-email" => {
            service
                .confirm_email_change(&request_context()?, value(form, "token")?, now)
                .await?;
            clear = true;
            view.frame.flash = Some("邮箱已更新，请重新登录。");
        }
        _ => {
            let identity = session(state, service, headers).await?;
            let version = || {
                value(form, "version")?
                    .parse::<i64>()
                    .map_err(|_| IdentityError::Rejected)
            };
            match path {
                "/reauth" => {
                    service
                        .reauthenticate(&identity, csrf, value(form, "password")?, now)
                        .await?;
                }
                "/logout" => {
                    service.logout(&identity, csrf, key, now).await?;
                    clear = true;
                }
                "/settings/profile" => {
                    service
                        .update_bio(&identity, csrf, key, version()?, value(form, "bio")?, now)
                        .await?;
                }
                // 未勾选的复选框浏览器不会提交，因此缺失即显式的「关闭」。
                "/settings/privacy" => {
                    let checked = |name: &str| form.get(name).map(String::as_str) == Some("yes");
                    service
                        .update_privacy(
                            &identity,
                            csrf,
                            key,
                            version()?,
                            checked("profile_lists_public"),
                            checked("topic_body_public"),
                            now,
                        )
                        .await?;
                }
                "/settings/email" => {
                    service
                        .request_email_change(
                            &identity,
                            csrf,
                            key,
                            version()?,
                            value(form, "email")?,
                            now,
                        )
                        .await?;
                }
                "/settings/username" => {
                    service
                        .change_username(
                            &identity,
                            csrf,
                            key,
                            version()?,
                            value(form, "username")?,
                            now,
                        )
                        .await?;
                }
                "/settings/password" => {
                    service
                        .change_password(
                            &identity,
                            csrf,
                            key,
                            version()?,
                            value(form, "password")?,
                            now,
                        )
                        .await?;
                    clear = true;
                }
                "/settings/sessions" => {
                    service
                        .revoke_other_sessions(&identity, csrf, key, now)
                        .await?;
                }
                "/settings/sessions/revoke" => {
                    let reference = value(form, "reference")?;
                    service
                        .revoke_session(&identity, csrf, key, reference, now)
                        .await?;
                    clear = reference == identity.session_ref();
                }
                "/settings/invitations" => {
                    let days = form
                        .get("days")
                        .map(|v| v.parse::<u8>().map_err(|_| IdentityError::Rejected))
                        .transpose()?;
                    let grant = service
                        .generate_invitation_with_signals(&identity, csrf, key, days, signals, now)
                        .await?;
                    view.rows
                        .push(row("邀请码（仅显示一次）", grant.code.expose()));
                }
                "/settings/invitations/revoke" => {
                    service
                        .revoke_invitation(&identity, csrf, key, value(form, "reference")?, now)
                        .await?;
                }
                _ => return Err(IdentityError::Disabled),
            }
            view.frame.flash = Some("操作已完成。");
        }
    }
    let mut result = response(state, &view, StatusCode::OK);
    if let Some(grant) = session_grant {
        set_cookie(
            &mut result,
            state,
            "session",
            grant.token.expose(),
            Some(30 * 86400),
        );
        set_cookie(
            &mut result,
            state,
            "csrf",
            grant.csrf.expose(),
            Some(30 * 86400),
        );
    }
    if clear {
        set_cookie(&mut result, state, "session", "", Some(0));
        set_cookie(&mut result, state, "csrf", "", Some(0));
    }
    Ok(result)
}
async fn public_profile(State(state): State<IdentityWebState>, Path(id): Path<String>) -> Response {
    let result = async {
        let service = gate(&state, "/u", false).await?;
        let profile = service
            .public_profile(&id, chrono::Utc::now())
            .await?
            .ok_or(IdentityError::Disabled)?;
        let mut page = Page::new("用户资料", "");
        page.show_form = false;
        page.preset_avatar = true;
        page.public_content_root = Some(format!("/u/{}", profile.public_ref));
        page.rows = vec![
            row("信任等级", trust_label(profile.trust_level)),
            row("用户名", profile.username),
            row("简介", profile.bio),
            row("加入时间", profile.joined_at.to_string()),
            row("贡献统计", "贡献统计尚未接入"),
        ];
        Ok(response(&state, &page, StatusCode::OK))
    }
    .await;
    match result {
        Ok(result) => result,
        Err(error) => failure(&state, error),
    }
}
async fn policy(State(state): State<IdentityWebState>, OriginalUri(uri): OriginalUri) -> Response {
    let result = async {
        let service = gate(&state, uri.path(), false).await?;
        let kind = match uri.path() {
            "/terms" => "terms",
            "/privacy" => "privacy",
            _ => "community",
        };
        let doc = service
            .policy_documents()
            .await?
            .into_iter()
            .find(|v| v.kind == kind)
            .ok_or(IdentityError::PolicyPending)?;
        let mut page = Page::new("政策版本", "");
        page.show_form = false;
        page.rows.push(row(
            format!(
                "{} · 版本 {}{}",
                kind,
                doc.version,
                if doc.synthetic {
                    "（合成测试政策）"
                } else {
                    ""
                }
            ),
            doc.body,
        ));
        Ok(response(&state, &page, StatusCode::OK))
    }
    .await;
    match result {
        Ok(result) => result,
        Err(error) => failure(&state, error),
    }
}
fn challenge_page(
    state: &IdentityWebState,
    path: &str,
    challenge_id: &str,
    form: &HashMap<String, String>,
) -> Response {
    let action = match path {
        "/login" => "/login",
        "/register" => "/register",
        "/forgot-password" => "/forgot-password",
        "/resend-verification" => "/resend-verification",
        _ => return failure(state, IdentityError::Rejected),
    };
    let mut page = Page::new("本地测试挑战 · LOCAL TEST", action);
    page.csrf = form.get("csrf").cloned().unwrap_or_default();
    page.idempotency_key = form.get("idempotency_key").cloned().unwrap_or_default();
    page.frame.flash = Some("请完成本地测试挑战，并重新填写原请求中的秘密字段。这是开发测试能力。");
    page.fields
        .push(Field::hidden("challenge_id", challenge_id));
    page.rows.push(row("本地挑战引用", challenge_id));
    // Reflect only these non-secret values, never passwords, invitation codes or proof.
    for (name, label, kind) in if path == "/register" {
        vec![
            ("invitation", "原邀请码", "text"),
            ("username", "原用户名", "text"),
            ("email", "原邮箱", "email"),
            ("password", "原密码", "password"),
        ]
    } else if path == "/login" {
        vec![
            ("identifier", "原用户名或邮箱", "text"),
            ("password", "原密码", "password"),
        ]
    } else {
        vec![("identifier", "原用户名或邮箱", "text")]
    } {
        let mut field = Field::input(name, label, kind);
        if matches!(name, "username" | "email" | "identifier") {
            field.value = form.get(name).cloned().unwrap_or_default();
        }
        page.fields.push(field);
    }
    if path == "/register" {
        for kind in ["terms", "privacy", "community"] {
            page.fields.push(Field::hidden(
                kind,
                form.get(kind).cloned().unwrap_or_default(),
            ));
        }
        page.policy_consent = true;
    }
    page.fields.push(Field::input(
        "challenge_proof",
        "本地测试挑战凭据",
        "password",
    ));
    response(state, &page, StatusCode::PRECONDITION_REQUIRED)
}
fn scope_for_path(path: &str) -> &'static str {
    match canonical_path(path) {
        "/register" => "identity.public.register",
        "/login" => "identity.public.login",
        "/forgot-password" => "identity.public.recovery",
        "/resend-verification" => "identity.public.resend_verification",
        "/verify-email" => "identity.public.verify",
        "/reset-password" => "identity.public.reset",
        "/confirm-email" => "identity.public.email_confirm",
        "/logout" => "identity.logout",
        "/settings/profile" => "identity.profile.bio",
        "/settings/username" => "identity.profile.username",
        "/settings/email" => "identity.profile.email_request",
        "/settings/password" => "identity.profile.password",
        "/settings/sessions" => "identity.sessions.revoke_others",
        "/settings/sessions/revoke" => "identity.sessions.revoke",
        "/settings/invitations" => "identity.invitation.generate",
        "/settings/invitations/revoke" => "identity.invitation.revoke",
        _ => "",
    }
}
fn pending(
    state: &IdentityWebState,
    id: &str,
    scope: &str,
    form: &HashMap<String, String>,
) -> Response {
    let mut page = Page::new("查询提交结果", "/auth/operations");
    page.submit = "查询原操作";
    page.csrf = form.get("csrf").cloned().unwrap_or_default();
    page.frame.flash = Some("提交结果尚未确认。请查询原操作，不要创建新的提交。");
    page.fields.extend([
        Field::hidden("operation_id", id),
        Field::hidden("scope", scope),
        Field::hidden(
            "original_key",
            form.get("original_key")
                .or_else(|| form.get("idempotency_key"))
                .cloned()
                .unwrap_or_default(),
        ),
    ]);
    page.rows.push(row("操作引用", id));
    response(state, &page, StatusCode::ACCEPTED)
}
async fn operation(
    State(state): State<IdentityWebState>,
    headers: HeaderMap,
    form: Result<Form<HashMap<String, String>>, axum::extract::rejection::FormRejection>,
) -> Response {
    let Ok(Form(form)) = form else {
        return failure(&state, IdentityError::Rejected);
    };
    let result = async {
        origin(&headers, state.secure_cookies)?;
        let scope = value(&form, "scope")?;
        let id = value(&form, "operation_id")?;
        let key = value(&form, "original_key")?;
        let gate_path = match scope {
            "identity.public.register" => "/register",
            "identity.public.verify"
            | "identity.public.email_confirm"
            | "identity.public.resend_verification" => "/verify-email",
            _ => "/login",
        };
        let service = gate(&state, gate_path, false).await?;
        let status = if scope.starts_with("identity.public.") {
            service
                .public_operation(
                    &context(&state, &headers, &form)?,
                    scope,
                    key,
                    id,
                    chrono::Utc::now(),
                )
                .await?
        } else {
            let raw = cookie(&headers, &cookie_name(&state, "session"))
                .ok_or(IdentityError::Unauthenticated)?;
            service
                .session_operation_by_token(
                    &raw,
                    value(&form, "csrf")?,
                    scope,
                    key,
                    id,
                    chrono::Utc::now(),
                )
                .await?
        };
        use platform::transaction::OperationStatus;
        let message = match status {
            OperationStatus::Completed(result) => {
                if result.status_code == 200 {
                    "原操作已确认完成。登录凭据和邀请码只展示一次；若未收到，请重新进入对应流程。"
                } else {
                    "原操作已确认未成功。"
                }
            }
            OperationStatus::NotObserved | OperationStatus::Pending => {
                return Ok(pending(&state, id, scope, &form));
            }
            OperationStatus::ResponseExpired => {
                "原操作的响应已过期，不能据此重新执行，请联系支持核查。"
            }
        };
        let mut page = Page::new("操作结果", "");
        page.show_form = false;
        page.frame.flash = Some(message);
        Ok(response(&state, &page, StatusCode::OK))
    }
    .await;
    match result {
        Ok(result) => result,
        Err(error) => failure(&state, error),
    }
}
