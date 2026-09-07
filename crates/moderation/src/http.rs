//! 治理领域路由：举报入口（`MOD-001`／`MOD-002`）。
//!
//! 装配方式与 `manage::console` 一致：领域 crate 只定义路由、视图模型与两个由
//! `apps/web` 实现的 trait，不依赖模板引擎，也不反向依赖 identity／content。
//! 这样 `moderation` 既能校验会话、又不必知道会话是怎么存的。
//!
//! **未装配时整组路由不注册**，因此由框架直接 404——与 `03` §1「服务端必须返回
//! 404，不能只依赖前端隐藏」同一口径：没有后端就没有这个入口，而不是一个会回
//! 500 的入口。
use crate::cases::ReportCategory;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Form, Router};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 对外只区分「能不能提交」，不区分「为什么不能」。
/// 更细的原因留在服务端；在响应上区分会把举报入口变成内容存在性探测器——
/// 「目标不存在」与「你无权举报它」一旦可分辨，就能用来枚举私有内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportError {
    Unauthenticated,
    Rejected,
    InvalidInput,
    Unavailable,
}

/// 举报页视图模型。
#[derive(Debug, Clone)]
pub struct ReportPage {
    pub title: String,
    pub csrf: String,
    pub target_kind: String,
    pub target_id: i64,
    /// `(取值, 显示名)`，顺序即表单里的顺序。
    pub categories: Vec<(&'static str, &'static str)>,
    pub notice: Option<String>,
    pub error: Option<String>,
}

/// 页面渲染器。由 `apps/web` 实现。
pub trait ReportRenderer: Send + Sync {
    fn render(&self, page: &ReportPage) -> Result<String, RenderError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderError;

pub trait ReportBackend: Send + Sync {
    /// 只判断这条会话是否属于一个可以看到举报表单的登录用户。
    /// 不返回账号 ID——渲染表单不需要它，少传一个身份标识就少一处泄露面。
    fn viewer<'a>(&'a self, raw_session: &'a str) -> BoxFuture<'a, Result<(), ReportError>>;
    /// 提交一条举报，返回所属案件 ID。
    ///
    /// `session` 与 `csrf` 原样传给实现方一并校验：CSRF 采用与内容写路径同一套
    /// 双提交（cookie + 表单字段），由 identity 侧统一判定，这里不另立一套。
    ///
    /// 目标可见性、举报资格与限流同样由实现方在同一事务内校验——领域 crate 不
    /// 知道内容可见性规则，把它放这里会变成第二套权限判断。
    fn submit<'a>(
        &'a self,
        session: &'a str,
        csrf: &'a str,
        kind: &'a str,
        target_id: i64,
        category: ReportCategory,
        note: &'a str,
    ) -> BoxFuture<'a, Result<i64, ReportError>>;
}

#[derive(Clone)]
pub struct ReportState {
    pub backend: Arc<dyn ReportBackend>,
    pub renderer: Arc<dyn ReportRenderer>,
    pub secure_cookies: bool,
}

/// `MOD-002` 的八类举报类型。顺序即表单顺序，「其他」殿后。
const CATEGORIES: &[(&str, &str, ReportCategory)] = &[
    ("spam", "垃圾广告", ReportCategory::Spam),
    ("harassment", "辱骂骚扰", ReportCategory::Harassment),
    ("illegal", "违法违规", ReportCategory::Illegal),
    ("privacy", "隐私泄露", ReportCategory::Privacy),
    ("infringement", "侵权", ReportCategory::Infringement),
    ("fraud", "诈骗", ReportCategory::Fraud),
    ("malware", "恶意文件", ReportCategory::Malware),
    ("other", "其他", ReportCategory::Other),
];

/// `MOD-001` 当前可举报的目标类型。附件与私信随各自能力上线再加。
const TARGET_KINDS: &[&str] = &["topic", "post", "profile"];

fn categories() -> Vec<(&'static str, &'static str)> {
    CATEGORIES.iter().map(|(v, l, _)| (*v, *l)).collect()
}

fn category(value: &str) -> Option<ReportCategory> {
    CATEGORIES
        .iter()
        .find(|(v, _, _)| *v == value)
        .map(|(_, _, c)| *c)
}

pub fn router<S>(state: Option<ReportState>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let Some(state) = state else {
        return Router::new();
    };
    Router::new()
        .route("/reports/new", get(form))
        .route("/reports", axum::routing::post(submit))
        .with_state(state)
}

/// 会话 cookie 名与 `content`／`identity` 保持一致；`secure_cookies` 打开时用
/// `__Host-` 前缀，与其他领域同一套规则，避免出现只在举报路径上生效的别名。
fn cookie(headers: &HeaderMap, secure: bool, name: &str) -> Option<String> {
    let full = if secure {
        format!("__Host-bcm_{name}")
    } else {
        format!("bcm_{name}")
    };
    let mut found = None;
    for value in headers.get_all(header::COOKIE) {
        for pair in value.to_str().ok()?.split(';') {
            let (key, val) = pair.trim().split_once('=')?;
            if key != full {
                continue;
            }
            // 同名 cookie 出现两次时一律不认：无法判断浏览器会送哪一个，
            // 猜一个等于把会话固定攻击的窗口留着。
            if found.is_some() {
                return None;
            }
            found = Some(val.to_owned());
        }
    }
    found
}

/// 常量时间比较：按字节提前返回会把「前缀对了几位」泄露出去。
/// 与 `identity::core::constant_equal` 同一实现；不跨 crate 引用是因为
/// `moderation` 不依赖 `identity`（依赖方向由 `apps/web` 装配层承担）。
fn constant_equal(a: &str, b: &str) -> bool {
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.bytes().zip(b.bytes()) {
        diff |= (x ^ y) as usize
    }
    diff == 0
}

fn render(state: &ReportState, page: &ReportPage, status: StatusCode) -> Response {
    match state.renderer.render(page) {
        Ok(body) => (
            status,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            body,
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct TargetQuery {
    kind: Option<String>,
    id: Option<i64>,
}

async fn form(
    State(state): State<ReportState>,
    headers: HeaderMap,
    Query(query): Query<TargetQuery>,
) -> Response {
    let kind = query.kind.unwrap_or_default();
    let id = query.id.unwrap_or_default();
    if !TARGET_KINDS.contains(&kind.as_str()) || id <= 0 {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(raw) = cookie(&headers, state.secure_cookies, "session") else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    // CSRF 取自 cookie，与内容写路径同一套双提交方案；表单里回填同一个值，
    // 提交时由 identity 侧比对，不在本层再造一套判定。
    let Some(csrf) = cookie(&headers, state.secure_cookies, "csrf") else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if state.backend.viewer(&raw).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let page = ReportPage {
        title: "举报内容".into(),
        csrf,
        target_kind: kind,
        target_id: id,
        categories: categories(),
        notice: None,
        error: None,
    };
    render(&state, &page, StatusCode::OK)
}

async fn submit(
    State(state): State<ReportState>,
    headers: HeaderMap,
    Form(form): Form<BTreeMap<String, String>>,
) -> Response {
    let Some(raw) = cookie(&headers, state.secure_cookies, "session") else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(csrf) = cookie(&headers, state.secure_cookies, "csrf") else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let get = |key: &str| form.get(key).map(String::as_str).unwrap_or_default();
    // 双提交的第一半在本层比对（表单字段 vs cookie），第二半由 identity 侧把
    // cookie 值与会话绑定核对。两半缺一不可：只比 cookie 与表单，攻击者只要能
    // 写 cookie 就能同时控制两边。
    if !constant_equal(get("csrf"), &csrf) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let kind = get("target_kind").to_owned();
    let id: i64 = get("target_id").parse().unwrap_or_default();
    let note = get("note").to_owned();
    let invalid = |state: &ReportState, message: &str| {
        let page = ReportPage {
            title: "举报内容".into(),
            csrf: csrf.clone(),
            target_kind: kind.clone(),
            target_id: id,
            categories: categories(),
            notice: None,
            error: Some(message.to_owned()),
        };
        render(state, &page, StatusCode::BAD_REQUEST)
    };
    if !TARGET_KINDS.contains(&kind.as_str()) || id <= 0 {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(category) = category(get("category")) else {
        return invalid(&state, "请选择举报类型。");
    };
    if note.chars().count() > 2_000 {
        return invalid(&state, "补充说明不能超过 2000 字。");
    }
    match state
        .backend
        .submit(&raw, &csrf, &kind, id, category, &note)
        .await
    {
        Ok(_) => {
            let page = ReportPage {
                title: "举报已提交".into(),
                csrf,
                target_kind: kind,
                target_id: id,
                categories: categories(),
                notice: Some(
                    "举报已提交，我们会尽快处理。同一目标的重复举报会合并到同一案件。".into(),
                ),
                error: None,
            };
            render(&state, &page, StatusCode::OK)
        }
        Err(ReportError::Unauthenticated) => StatusCode::UNAUTHORIZED.into_response(),
        Err(ReportError::InvalidInput) => invalid(&state, "举报内容不合法。"),
        // Rejected 与 Unavailable 都回同一句：区分「目标不存在」与「你无权举报」
        // 会让这个入口变成内容存在性探测器。
        Err(_) => invalid(&state, "暂时无法提交举报，请稍后重试。"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mod002_category_is_offered_and_other_is_last() {
        // `MOD-002` 要求八类：垃圾广告、辱骂骚扰、违法违规、隐私泄露、侵权、
        // 诈骗、恶意文件、其他。少一类就是需求没满足，这里逐项断言。
        let offered: Vec<_> = categories().into_iter().map(|(v, _)| v).collect();
        for expected in [
            "spam",
            "harassment",
            "illegal",
            "privacy",
            "infringement",
            "fraud",
            "malware",
            "other",
        ] {
            assert!(offered.contains(&expected), "缺少举报类型 {expected}");
        }
        assert_eq!(offered.len(), 8);
        assert_eq!(offered.last(), Some(&"other"), "「其他」必须殿后");
        // 表单取值必须能反解回领域枚举，否则提交时会静默落到默认分类。
        for value in &offered {
            assert!(category(value).is_some(), "{value} 无法反解为领域类型");
        }
        assert!(category("nonexistent").is_none());
    }

    #[test]
    fn duplicate_session_cookie_is_refused() {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, "bcm_session=abc".parse().unwrap());
        assert_eq!(cookie(&headers, false, "session").as_deref(), Some("abc"));
        // 打开 secure 前缀后旧名不再被接受，避免出现降级别名。
        assert!(cookie(&headers, true, "session").is_none());
        headers.append(header::COOKIE, "bcm_session=second".parse().unwrap());
        assert!(cookie(&headers, false, "session").is_none());
    }
}
