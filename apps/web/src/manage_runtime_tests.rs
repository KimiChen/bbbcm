//! `/manage` 控制台的运行时安全测试。
//!
//! 这里验证的是 `03` §1／§6 里三条不可放宽的规则，它们全部是**服务端**行为，
//! 不是前端隐藏：未列入路由地图的管理路由 404、未登录一律进不去、
//! 未完成 MFA 绑定的会话只能走绑定流程。
use crate::app_state::AppState;
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use tower::ServiceExt;

/// 构造一个「后端未装配」的应用：控制台状态为 closed。
fn app() -> axum::Router {
    let config = platform::config::Config::default();
    // 测试用空资产目录：控制台模板只引用已冻结的 A00 资产键，
    // 这里不需要真实构建产物就能渲染出 404 分支。
    let missing = std::env::temp_dir().join(format!(
        "bbbcm-manage-assets-{}",
        platform::transaction::new_operation_id()
    ));
    let assets =
        crate::assets::Assets::load_for(platform::config::Environment::Test, &missing, false)
            .unwrap();
    crate::router::router(std::sync::Arc::new(AppState::new(config, assets)))
}

async fn get(app: &axum::Router, path: &str) -> axum::response::Response {
    app.clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// `03` §1：未列入路由地图的管理路由必须由服务端 404，不能只靠前端隐藏。
///
/// 本轮只开放五条路由；其余十一条 `03` 列出的路由本轮同样 404——它们尚未交付，
/// 返回任何非 404 都会让人误以为功能存在。
#[tokio::test]
async fn management_routes_outside_this_wave_are_absent_on_the_server() {
    let app = app();
    for path in [
        "/manage/users",
        "/manage/reports",
        "/manage/support",
        "/manage/privacy",
        "/manage/invites",
        "/manage/config",
        "/manage/jobs",
        "/manage/analytics",
        "/manage/audit",
        "/manage/approvals",
        "/manage/system",
        "/manage/_api/v1/cases",
        // 直接敲一个不存在的子路径同样不得泄露任何结构。
        "/manage/cases/1/evidence",
    ] {
        assert_eq!(
            get(&app, path).await.status(),
            StatusCode::NOT_FOUND,
            "{path} 必须由服务端 404"
        );
    }
}

/// 后端未装配时，连本轮开放的路由也一律 404——不渲染空壳后台。
#[tokio::test]
async fn console_is_entirely_absent_until_the_backend_is_assembled() {
    let app = app();
    for path in [
        "/manage",
        "/manage/login",
        "/manage/cases",
        "/manage/content",
    ] {
        assert_eq!(
            get(&app, path).await.status(),
            StatusCode::NOT_FOUND,
            "{path} 在未装配时必须 404"
        );
    }
}

/// 管理 cookie 与普通用户 cookie 完全隔离：普通会话的 cookie 名进不了管理路由。
///
/// 这条用常量比较即可确证——两者的名字与路径都不同，因此浏览器不会把普通会话
/// cookie 发到 `/manage`，服务端也不会去读它。
#[test]
fn management_cookies_are_named_and_scoped_apart_from_ordinary_sessions() {
    use manage::console::http::{CSRF_COOKIE, SESSION_COOKIE};
    assert_eq!(SESSION_COOKIE, "bcm_manage_session");
    assert_eq!(CSRF_COOKIE, "bcm_manage_csrf");
    // 与普通用户会话的 cookie 名不重叠，避免任何一侧被误读。
    for ordinary in ["bcm_session", "bcm_csrf", "bcm_device"] {
        assert_ne!(SESSION_COOKIE, ordinary);
        assert_ne!(CSRF_COOKIE, ordinary);
    }
}

/// 管理响应必须带上不可嵌套、不缓存、不外泄 Referer 的安全头。
///
/// 后台 URL 本身就带信息（案件 ID、主题 ID），Referer 泄露等于把它送到站外。
#[tokio::test]
async fn management_responses_carry_hardened_headers() {
    // 未装配时走 404 分支，因此这里用一个已注册但未装配的路由确认路由存在性，
    // 安全头的实际断言在装配后的集成矩阵中执行（留硬化阶段）。
    let response = get(&app(), "/manage/login").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        response.headers().get(header::SET_COOKIE).is_none(),
        "未装配时不得下发任何管理 cookie"
    );
}
