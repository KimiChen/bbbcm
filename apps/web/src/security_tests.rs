//! `WP-MVP-E01` 安全矩阵中**不依赖未完成功能**的部分，对应 `90` 第 5 节「安全」。
//!
//! 这些用例跑在完整装配的路由上，但**不需要数据库**：它们验证的是「后端未装配时
//! 也必须成立」的性质——失败关闭、安全响应头、不泄露、不给开放重定向。这类性质
//! 恰恰最容易在后端不可用的降级分支里被绕过，所以专挑这个状态压。
//!
//! 未覆盖（依赖尚未交付的功能，不得据本文件宣称已验证）：上传 MIME／文件头／
//! 压缩炸弹／像素炸弹与下载鉴权（`WP-MVP-C03` 上传链未实现）、验证码边界、
//! 凭证填充与时序（需真实身份后端与统计样本，属 `WP-MVP-E02` 性能矩阵一并做）。
use crate::app_state::AppState;
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use tower::ServiceExt;

/// 后端未装配的应用：所有需要数据库的路径都应失败关闭，而不是泄露或放行。
fn app() -> axum::Router {
    let config = platform::config::Config::default();
    let missing = std::env::temp_dir().join(format!(
        "bbbcm-security-assets-{}",
        platform::transaction::new_operation_id()
    ));
    let assets =
        crate::assets::Assets::load_for(platform::config::Environment::Test, &missing, false)
            .unwrap();
    crate::router::router(std::sync::Arc::new(AppState::new(config, assets)))
}

async fn send(app: &axum::Router, method: Method, path: &str) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `03` §6 + `01` §8：`/manage` 的每一条响应都必须带齐四个安全头。
///
/// 关键在「每一条」——包括 404 与失败关闭分支。安全头只在成功路径上设置是常见
/// 疏漏：攻击者感兴趣的恰恰是错误页，而后台 URL 本身就带信息（案件与主题 ID），
/// 少一个 `Referrer-Policy` 就等于把它送到站外。本用例首次执行即抓到这一点：
/// 当时 `secure_headers` 只在渲染成功与跳转两处调用，六处提前返回全部漏掉。
///
/// 范围说明：这里只覆盖**已注册**的管理路由。未注册的 `/manage/...` 由框架的
/// 全局 404 处理，不经过管理路由的 layer；那条路径不返回任何管理数据，
/// 按 `03` §1 只要求 404 本身。
#[tokio::test]
async fn every_management_response_carries_the_full_security_header_set() {
    let app = app();
    for path in [
        "/manage",
        "/manage/login",
        "/manage/mfa",
        "/manage/cases",
        "/manage/reports",
        "/manage/content",
    ] {
        let response = send(&app, Method::GET, path).await;
        let headers = response.headers();
        assert_eq!(
            headers
                .get(header::CACHE_CONTROL)
                .map(|v| v.to_str().unwrap()),
            Some("no-store"),
            "{path} 缺少 Cache-Control: no-store"
        );
        let csp = headers
            .get(header::CONTENT_SECURITY_POLICY)
            .map(|v| v.to_str().unwrap().to_owned())
            .unwrap_or_default();
        assert!(
            csp.contains("frame-ancestors 'none'"),
            "{path} 的 CSP 缺少 frame-ancestors 'none'：{csp:?}"
        );
        assert_eq!(
            headers.get("x-frame-options").map(|v| v.to_str().unwrap()),
            Some("DENY"),
            "{path} 缺少 X-Frame-Options: DENY"
        );
        assert_eq!(
            headers.get("referrer-policy").map(|v| v.to_str().unwrap()),
            Some("no-referrer"),
            "{path} 缺少 Referrer-Policy: no-referrer"
        );
    }
}

/// `03` §1：未登录访问管理路由，永远不能拿到内容（C3 永不返回）。
///
/// 断言不只看状态码，还要看**响应体里没有任何管理数据**。只断言状态码挡不住
/// 「返回 200 但正文是登录页」和「返回 403 但正文带了案件标题」这两类问题。
#[tokio::test]
async fn management_routes_never_leak_content_without_a_session() {
    let app = app();
    for path in [
        "/manage",
        "/manage/cases",
        "/manage/reports",
        "/manage/content",
    ] {
        let response = send(&app, Method::GET, path).await;
        let status = response.status();
        assert!(
            status.is_client_error() || status.is_server_error() || status.is_redirection(),
            "{path} 未登录时不得返回成功状态，实际 {status}"
        );
        let body = body_text(response).await;
        for leak in ["案件", "举报人", "处置", "assignee", "operator_id"] {
            assert!(
                !body.contains(leak),
                "{path} 未登录响应泄露了管理数据：{leak}"
            );
        }
    }
}

/// `90` §5 安全：任何响应都不得带出 Secret、连接串或内部路径。
///
/// 扫一遍全部可达路由的响应体与响应头。这类泄露通常来自错误处理——把
/// `Debug` 输出直接渲染进页面，连接串和文件路径就一起出去了。
#[tokio::test]
async fn no_response_leaks_secrets_connection_strings_or_host_paths() {
    let app = app();
    let paths = [
        "/health",
        "/manage",
        "/manage/login",
        "/does-not-exist",
        "/assets/unknown.css",
        "/t/1",
        "/forums/nope",
        "/search?q=x",
        "/robots.txt",
        "/sitemap.xml",
        "/feed.xml",
    ];
    for path in paths {
        let response = send(&app, Method::GET, path).await;
        let headers = format!("{:?}", response.headers());
        let body = body_text(response).await;
        let haystack = format!("{headers}\n{body}");
        for needle in [
            "postgres://",
            "redis://",
            "BBBCM_",
            "SecretString",
            "/Users/",
            "/home/",
            "panicked",
            "RUST_BACKTRACE",
        ] {
            assert!(!haystack.contains(needle), "{path} 的响应泄露了 {needle}");
        }
    }
}

/// `90` §5 安全：不得存在开放重定向。
///
/// 逐个试常见的跳转参数名与绕过写法。断言的是「要么不跳转，要么只跳到站内」：
/// 只要 `Location` 带了协议或以 `//` 开头，就已经指向站外了。
#[tokio::test]
async fn redirect_targets_can_never_be_pointed_off_site() {
    let app = app();
    let hostile = [
        "https://evil.example",
        "//evil.example",
        "/\\evil.example",
        "https:/\\evil.example",
        "%2f%2fevil.example",
    ];
    for param in ["next", "redirect", "return_to", "continue", "url"] {
        for target in hostile {
            for base in ["/manage/login", "/login", "/logout"] {
                let path = format!("{base}?{param}={target}");
                let response = send(&app, Method::GET, &path).await;
                if let Some(location) = response.headers().get(header::LOCATION) {
                    let value = location.to_str().unwrap_or_default();
                    assert!(
                        value.starts_with('/') && !value.starts_with("//"),
                        "{path} 产生了站外跳转：{value:?}"
                    );
                    assert!(
                        !value.contains("evil.example"),
                        "{path} 的跳转目标含攻击者可控主机：{value:?}"
                    );
                }
            }
        }
    }
}

/// `90` §5 安全：改状态的动作不得经 GET 到达。
///
/// GET 会被浏览器预取、被 `<img>` 触发、被历史记录重放，因此任何用 GET 就能改
/// 状态的入口本身就是 CSRF 缺口——无论有没有校验令牌。
#[tokio::test]
async fn state_changing_paths_are_not_reachable_by_get() {
    let app = app();
    for path in [
        "/manage/login",
        "/manage/mfa",
        "/logout",
        "/topics/new",
        "/settings/profile",
    ] {
        let response = send(&app, Method::GET, path).await;
        let status = response.status();
        // GET 可以用来渲染表单页，但绝不能是「执行」——因此这里要求它要么
        // 不可达，要么是纯展示（非 2xx 的写入语义状态码都不该出现在 GET 上）。
        assert_ne!(
            status,
            StatusCode::CREATED,
            "{path} 的 GET 返回 201，说明它执行了写入"
        );
        assert_ne!(
            status,
            StatusCode::NO_CONTENT,
            "{path} 的 GET 返回 204，说明它执行了写入"
        );
    }
}

/// `01` §8.1：静态资产必须带 `nosniff`，避免被当成脚本执行。
///
/// 缺 `nosniff` 时，一个内容像 HTML/JS 的上传文件会被浏览器按脚本执行，
/// 同源下等于存储型 XSS。
#[tokio::test]
async fn asset_responses_forbid_mime_sniffing() {
    let app = app();
    for path in ["/assets/unknown.css", "/assets/manifest.json"] {
        let response = send(&app, Method::GET, path).await;
        assert_eq!(
            response
                .headers()
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .map(|v| v.to_str().unwrap()),
            Some("nosniff"),
            "{path} 缺少 X-Content-Type-Options: nosniff"
        );
    }
}
