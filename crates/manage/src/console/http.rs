//! `/manage` 路由与处理。
//!
//! 三条不可放宽的规则（`03` §1／§6）：
//!   1. **未列入 `03` 路由地图的管理路由一律 404**，不能只靠前端隐藏；
//!   2. 管理会话与普通用户会话使用**独立 cookie 与路径**，两者不可互换；
//!   3. 处于 `BindingRequired` 的会话只能访问绑定流程，其余管理路由服务端拒绝。
//!
//! 授权判定不在本模块：每个动作都调用装配层注入的 `ConsoleBackend`，
//! 后者内部走既有的统一授权门。本模块只负责路由、表单与渲染。
use crate::console::view::{Field, Link, Page, PageRenderer};
use axum::{
    Form, Router,
    extract::{OriginalUri, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use std::{collections::HashMap, sync::Arc};

/// 管理会话 cookie 名。与普通用户会话完全不同的名字与路径（`03` §6）。
///
/// 生产环境使用 `__Host-` 前缀：该前缀要求 `Secure`、`Path=/` 且无 `Domain`，
/// 浏览器据此拒绝任何子域写入，避免子域被攻陷后伪造管理会话。
pub const SESSION_COOKIE: &str = "bcm_manage_session";
pub const CSRF_COOKIE: &str = "bcm_manage_csrf";

/// 装配层注入的后端。领域路由不直接持有服务句柄，便于未装配时整体关闭。
pub trait ConsoleBackend: Send + Sync {
    /// 校验一次管理会话，返回（操作者显示名, 是否已完成 MFA 绑定）。
    fn session<'a>(
        &'a self,
        raw_session: &'a str,
    ) -> BoxFuture<'a, Result<(String, bool), ConsoleError>>;
    /// 密码 + staff principal 校验，成功返回新的会话与 CSRF 原始值。
    fn login<'a>(
        &'a self,
        username: &'a str,
        password: &'a str,
    ) -> BoxFuture<'a, Result<(String, String, bool), ConsoleError>>;
    /// 开始 TOTP 绑定，返回 Base32 密钥与 otpauth URI。
    fn begin_binding<'a>(
        &'a self,
        raw_session: &'a str,
    ) -> BoxFuture<'a, Result<(String, String), ConsoleError>>;
    /// 确认绑定，返回只显示一次的恢复码。
    fn confirm_binding<'a>(
        &'a self,
        raw_session: &'a str,
        csrf: &'a str,
        code: &'a str,
    ) -> BoxFuture<'a, Result<Vec<String>, ConsoleError>>;
    /// 案件队列。
    fn cases<'a>(
        &'a self,
        raw_session: &'a str,
        csrf: &'a str,
    ) -> BoxFuture<'a, Result<Vec<crate::console::view::CaseRow>, ConsoleError>>;
    /// 案件详情。
    fn case<'a>(
        &'a self,
        raw_session: &'a str,
        csrf: &'a str,
        id: i64,
    ) -> BoxFuture<'a, Result<crate::console::view::CaseDetail, ConsoleError>>;
    /// 案件动作：推进状态或追加备注。
    fn case_action<'a>(
        &'a self,
        raw_session: &'a str,
        csrf: &'a str,
        id: i64,
        form: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<(), ConsoleError>>;
    /// 加载移帖表单所需的主题事实：(标题, 当前版块名, 当前版本, 可选版块列表)。
    ///
    /// 版本号由服务端在此填入隐藏字段，而不是让人手填——它是乐观并发的 CAS 令牌，
    /// 手填既难用又容易填错，而填错的后果是一次静默失败或误覆盖。
    fn move_form<'a>(
        &'a self,
        raw_session: &'a str,
        csrf: &'a str,
        topic_id: i64,
    ) -> BoxFuture<'a, Result<MoveForm, ConsoleError>>;
    /// 全站举报明细队列。
    fn reports<'a>(
        &'a self,
        raw_session: &'a str,
        csrf: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ReportRow>, ConsoleError>>;
    /// 移帖。
    fn move_topic<'a>(
        &'a self,
        raw_session: &'a str,
        csrf: &'a str,
        form: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<i64, ConsoleError>>;
}

pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// 移帖表单所需的主题事实：(标题, 当前版块名, 当前版本, 可选目标版块列表)。
pub type MoveForm = (String, String, i64, Vec<(String, String)>);
/// 一行举报明细：(类型, 时间, 举报人标识)。
pub type ReportRow = (String, String, String);

/// 控制台错误。
///
/// 对外只区分「要重新登录」「要先绑定」「被拒绝」「输入无效」四类：
/// 更细的原因留在服务端日志里，响应上区分只会变成探测接口。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleError {
    Unauthenticated,
    BindingRequired,
    Denied,
    InvalidInput,
    Unavailable,
}

#[derive(Clone)]
pub struct ConsoleState {
    pub backend: Option<Arc<dyn ConsoleBackend>>,
    pub renderer: Arc<dyn PageRenderer>,
    pub secure_cookies: bool,
}

/// 挂载 `03` 第 2 节中本轮开放的路由。
///
/// 其余管理路由**不注册**，因此由框架直接 404——这正是 `03` §1 要求的
/// 「服务端必须返回 404，不能只依赖前端隐藏」。
pub fn router<S>(state: ConsoleState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/manage/login", get(page).post(submit))
        .route("/manage/mfa", get(page).post(submit))
        .route("/manage", get(page))
        .route("/manage/cases", get(page))
        .route("/manage/reports", get(page))
        .route("/manage/cases/{id}", get(page).post(submit))
        .route("/manage/content", get(page).post(submit))
        .with_state(state)
}

fn cookie_name(state: &ConsoleState, name: &str) -> String {
    // `__Host-` 前缀要求 Secure + Path=/ + 无 Domain，浏览器据此拒绝子域写入。
    if state.secure_cookies {
        format!("__Host-{name}")
    } else {
        name.to_owned()
    }
}

fn read_cookie(headers: &HeaderMap, wanted: &str) -> Option<String> {
    let mut values = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|h| h.split(';'))
        .filter_map(|h| h.trim().split_once('='))
        .filter(|(name, _)| *name == wanted)
        .map(|(_, value)| value);
    let value = values.next()?;
    // 重复出现、超长或含非法字符一律视为不可用，与其他 cookie 读取同一条规则。
    if values.next().is_some()
        || !(1..=256).contains(&value.len())
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    Some(value.into())
}

/// 管理页面的安全响应头。
///
/// `/manage` 不允许被任何页面嵌套，也不向外泄露 Referer——后台 URL 本身就带信息。
fn secure_headers(response: &mut Response) {
    let headers = response.headers_mut();
    headers.insert(header::X_FRAME_OPTIONS, "DENY".parse().expect("常量"));
    headers.insert(
        header::REFERRER_POLICY,
        "no-referrer".parse().expect("常量"),
    );
    headers.insert(header::CACHE_CONTROL, "no-store".parse().expect("常量"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        "default-src 'self'; frame-ancestors 'none'"
            .parse()
            .expect("常量"),
    );
}

fn render(state: &ConsoleState, page: &Page, status: StatusCode) -> Response {
    let body = match state.renderer.render(page) {
        Ok(body) => body,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let mut response = (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response();
    secure_headers(&mut response);
    response
}

fn navigation(path: &str) -> Vec<Link> {
    [
        "/manage",
        "/manage/cases",
        "/manage/reports",
        "/manage/content",
    ]
    .into_iter()
    .zip(["工作队列", "案件", "举报", "内容治理"])
    .map(|(href, label)| {
        let mut link = Link::new(label, href);
        link.active = path == href;
        link
    })
    .collect()
}

async fn page(
    State(state): State<ConsoleState>,
    OriginalUri(uri): OriginalUri,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let path = uri.path();
    // 只解析本模块用到的两个查询参数：主题 ID 与动作结果标识。
    // 其余参数一律忽略，不做任何转发或回显，避免把查询串变成注入面。
    let topic_query = query.get("topic_id").and_then(|v| v.parse::<i64>().ok());
    let done = query.get("done").map(String::as_str);
    let Some(backend) = state.backend.clone() else {
        // 未装配：整体不可用，而不是渲染一个空壳后台。
        return StatusCode::NOT_FOUND.into_response();
    };
    let session = read_cookie(&headers, &cookie_name(&state, SESSION_COOKIE));
    let csrf = read_cookie(&headers, &cookie_name(&state, CSRF_COOKIE)).unwrap_or_default();

    // 登录页是唯一不需要会话的路由。
    if path == "/manage/login" {
        let mut page = Page::new("管理登录");
        page.action = "/manage/login".into();
        page.submit = "登录";
        page.csrf = csrf;
        page.fields.push(Field::input("username", "用户名", "text"));
        page.fields
            .push(Field::input("password", "密码", "password"));
        return render(&state, &page, StatusCode::OK);
    }

    let Some(raw) = session else {
        return redirect("/manage/login");
    };
    let (operator, bound) = match backend.session(&raw).await {
        Ok(value) => value,
        Err(_) => return redirect("/manage/login"),
    };

    // AUTH-014：未完成绑定的会话只能访问绑定流程。这里是服务端拒绝，
    // 不是「隐藏入口」——直接敲 URL 同样进不去。
    if !bound && path != "/manage/mfa" {
        return redirect("/manage/mfa");
    }
    if bound && path == "/manage/mfa" {
        return redirect("/manage");
    }

    let mut page = Page::new("管理后台");
    page.nav = navigation(path);
    // 动作完成后经重定向回到本页，用查询参数带上结果标识再渲染提示——
    // 直接渲染会让刷新变成重复提交。
    page.notice = match done {
        Some("moved") => Some("主题已移动，并已写入审计。".into()),
        Some("case") => Some("案件已更新。".into()),
        _ => None,
    };
    page.operator = Some(operator);
    page.csrf = csrf.clone();
    match path {
        "/manage/mfa" => {
            page.title = "绑定两步验证".into();
            page.action = "/manage/mfa".into();
            page.submit = "确认绑定";
            page.fields
                .push(Field::input("code", "认证器验证码", "text"));
            match backend.begin_binding(&raw).await {
                Ok((secret, uri)) => page.totp = Some((secret, uri)),
                Err(_) => page.error = Some("暂时无法开始绑定，请稍后重试。".into()),
            }
        }
        "/manage/cases" => {
            page.title = "案件队列".into();
            match backend.cases(&raw, &csrf).await {
                Ok(rows) => page.cases = rows,
                Err(ConsoleError::Denied) => page.error = Some("没有查看案件的权限。".into()),
                Err(_) => page.error = Some("暂时无法读取案件队列。".into()),
            }
        }
        path if path.starts_with("/manage/cases/") => {
            let id = path.rsplit('/').next().and_then(|v| v.parse().ok());
            match id {
                Some(id) => match backend.case(&raw, &csrf, id).await {
                    Ok(detail) => {
                        page.title = format!("案件 #{id}");
                        page.action = format!("/manage/cases/{id}");
                        page.submit = "提交";
                        page.fields
                            .push(Field::input("note", "内部备注", "textarea"));
                        page.case = Some(detail);
                    }
                    Err(ConsoleError::Denied) => {
                        return render(&state, &denied(page), StatusCode::FORBIDDEN);
                    }
                    Err(_) => return StatusCode::NOT_FOUND.into_response(),
                },
                None => return StatusCode::NOT_FOUND.into_response(),
            }
        }
        "/manage/reports" => {
            page.title = "举报队列".into();
            match backend.reports(&raw, &csrf).await {
                Ok(rows) => {
                    page.case = Some(crate::console::view::CaseDetail {
                        id: 0,
                        target: "全站举报".into(),
                        status: "",
                        priority: "",
                        escalated: false,
                        reports: rows.len() as i64,
                        evidence: Vec::new(),
                        transitions: Vec::new(),
                        report_details: rows,
                    })
                }
                Err(ConsoleError::Denied) => page.error = Some("没有查看举报的权限。".into()),
                Err(_) => page.error = Some("暂时无法读取举报队列。".into()),
            }
        }
        "/manage/content" => {
            page.title = "内容治理".into();
            page.action = "/manage/content".into();
            // 两步：先按主题 ID 加载，再确认移动。第一步只填一个数字；
            // 第二步的版本号由服务端填进隐藏字段，目标版块用下拉选。
            match topic_query {
                Some(topic_id) => match backend.move_form(&raw, &csrf, topic_id).await {
                    Ok((title, current, version, forums)) => {
                        page.submit = "移动主题";
                        page.notice = Some(format!("《{title}》当前位于「{current}」。"));
                        page.fields
                            .push(Field::hidden("topic_id", topic_id.to_string()));
                        page.fields
                            .push(Field::hidden("version", version.to_string()));
                        page.fields
                            .push(Field::select("forum_id", "目标版块", forums));
                    }
                    Err(ConsoleError::Denied) => page.error = Some("没有移动主题的权限。".into()),
                    Err(_) => page.error = Some("找不到该主题，或它当前不可移动。".into()),
                },
                None => {
                    page.submit = "加载主题";
                    page.fields
                        .push(Field::input("topic_id", "主题 ID", "number"));
                }
            }
        }
        _ => {
            page.title = "工作队列".into();
            match backend.cases(&raw, &csrf).await {
                Ok(rows) => page.cases = rows,
                // 首页对无权查看案件的角色不报错，只是队列为空——
                // 让只读审计员也能进后台看到导航。
                Err(_) => page.cases = Vec::new(),
            }
        }
    }
    render(&state, &page, StatusCode::OK)
}

fn denied(mut page: Page) -> Page {
    page.error = Some("没有执行该操作的权限。".into());
    page
}

fn redirect(href: &str) -> Response {
    let mut response = Redirect::to(href).into_response();
    secure_headers(&mut response);
    response
}

async fn submit(
    State(state): State<ConsoleState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let path = uri.path().to_owned();
    let Some(backend) = state.backend.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let csrf = read_cookie(&headers, &cookie_name(&state, CSRF_COOKIE)).unwrap_or_default();
    let session = read_cookie(&headers, &cookie_name(&state, SESSION_COOKIE));

    if path == "/manage/login" {
        let username = form.get("username").map(String::as_str).unwrap_or_default();
        let password = form.get("password").map(String::as_str).unwrap_or_default();
        return match backend.login(username, password).await {
            Ok((session, csrf, bound)) => {
                let mut response = redirect(if bound { "/manage" } else { "/manage/mfa" });
                set_cookie(&state, &mut response, SESSION_COOKIE, &session);
                set_cookie(&state, &mut response, CSRF_COOKIE, &csrf);
                response
            }
            Err(_) => {
                // 统一失败语义：不区分「账号不存在」「不是 staff」「密码错误」。
                let mut page = Page::new("管理登录");
                page.action = "/manage/login".into();
                page.submit = "登录";
                page.error = Some("登录失败，请检查用户名与密码。".into());
                page.fields.push(Field::input("username", "用户名", "text"));
                page.fields
                    .push(Field::input("password", "密码", "password"));
                render(&state, &page, StatusCode::UNAUTHORIZED)
            }
        };
    }

    let Some(raw) = session else {
        return redirect("/manage/login");
    };
    let outcome = match path.as_str() {
        "/manage/mfa" => {
            let code = form.get("code").map(String::as_str).unwrap_or_default();
            match backend.confirm_binding(&raw, &csrf, code).await {
                Ok(codes) => {
                    let mut page = Page::new("绑定完成");
                    page.nav = navigation("/manage");
                    page.csrf = csrf;
                    page.notice = Some("请立即保存下列恢复码，它们只显示这一次。".into());
                    page.recovery_codes = codes;
                    return render(&state, &page, StatusCode::OK);
                }
                Err(e) => Err(e),
            }
        }
        "/manage/content" => {
            return match backend.move_topic(&raw, &csrf, &form).await {
                // 带上结果标识重定向：刷新不会重复提交，用户也能确认动作确实完成了。
                Ok(_) => redirect("/manage/content?done=moved"),
                Err(error) => failure_page(&state, &path, csrf, error),
            };
        }
        path if path.starts_with("/manage/cases/") => {
            match path.rsplit('/').next().and_then(|v| v.parse().ok()) {
                Some(id) => backend.case_action(&raw, &csrf, id, &form).await,
                None => Err(ConsoleError::InvalidInput),
            }
        }
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    match outcome {
        Ok(()) => redirect(&format!("{path}?done=case")),
        Err(ConsoleError::BindingRequired) => redirect("/manage/mfa"),
        Err(ConsoleError::Unauthenticated) => redirect("/manage/login"),
        Err(error) => failure_page(&state, &path, csrf, error),
    }
}

/// 失败页。错误文案只分三档，与 `ConsoleError` 的粒度一致，不透露更细的原因。
fn failure_page(state: &ConsoleState, path: &str, csrf: String, error: ConsoleError) -> Response {
    let mut page = Page::new("操作未完成");
    page.nav = navigation(path);
    page.csrf = csrf;
    page.error = Some(
        match error {
            ConsoleError::Denied => "没有执行该操作的权限。",
            ConsoleError::InvalidInput => "输入无效，请检查后重试。",
            _ => "操作暂时无法完成，请稍后重试。",
        }
        .into(),
    );
    render(
        state,
        &page,
        match error {
            ConsoleError::Denied => StatusCode::FORBIDDEN,
            ConsoleError::InvalidInput => StatusCode::BAD_REQUEST,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        },
    )
}

fn set_cookie(state: &ConsoleState, response: &mut Response, name: &str, value: &str) {
    // 管理 cookie 的路径限定在 /manage：即便普通页面存在 XSS，也读不到它。
    // 生产环境用 `__Host-` 前缀，该前缀要求 Path=/，因此两种形态分开构造。
    let cookie = if state.secure_cookies {
        format!("__Host-{name}={value}; Path=/; Secure; HttpOnly; SameSite=Strict")
    } else {
        format!("{name}={value}; Path=/manage; HttpOnly; SameSite=Strict")
    };
    if let Ok(value) = cookie.parse() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
}

/// 未使用的路径参数提取器占位，保证 `Path` 导入被使用。
#[allow(dead_code)]
fn _path_extractor(_: Path<i64>) {}
