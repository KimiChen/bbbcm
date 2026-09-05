//! 路由挂载点：只组合领域 router，不写业务。
//! 领域路由随对应工作包逐个填充 handler。
//!
//! `with_state` 必须留在全部 `merge`/`nest` 之后：领域 router 是 `Router<S>`，
//! 只有在此处统一注入状态后才收敛为 `Router<()>`。

use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::{Path, Query};
use axum::response::Html;
use axum::routing::get;
use platform::config::Environment;
use platform::error::AppError;

use crate::app_state::AppState;
use crate::contract::{HomeTemplate, PageFrame};
use crate::error::ApiError;
use crate::fixtures::{self, FixtureQuery};

pub fn router(state: Arc<AppState>) -> Router {
    let app = Router::new()
        .route("/", get(home))
        .route("/health", get(health))
        .merge(identity::http::router())
        .merge(content::http::router())
        .merge(media::http::router())
        .merge(interaction::http::router())
        .merge(moderation::http::router())
        .merge(support::http::router())
        .merge(trust::http::router())
        .merge(privacy::http::router());

    // 正向环境白名单；staging/production 完全不挂载夹具与源资产。
    // 不依赖业务 Feature Flag，也不会因路径或查询参数而扩大可达范围。
    let app = if matches!(
        state.config.server.environment,
        Environment::Development | Environment::Test
    ) {
        app.route("/__fixtures/a00/{page}", get(fixture))
            .merge(crate::assets::router())
    } else {
        app
    };
    app.with_state(state)
}

/// 渲染模板。渲染失败返回 500 并记录原因，不在请求内 panic——
/// handler 里的 panic 会中断该连接，且把内部细节暴露给客户端。
fn render<T: Template>(template: &T) -> Result<Html<String>, ApiError> {
    template
        .render()
        .map(Html)
        .map_err(|err| ApiError(AppError::Internal(format!("模板渲染失败: {err}"))))
}

async fn home() -> Result<Html<String>, ApiError> {
    render(&HomeTemplate {
        frame: PageFrame::default(),
    })
}

async fn fixture(
    Path(page): Path<String>,
    Query(query): Query<FixtureQuery>,
) -> Result<Html<String>, ApiError> {
    let template = fixtures::page(&page, query.theme)
        .ok_or_else(|| ApiError(AppError::NotFound("没有这个视觉夹具".into())))?;
    render(&template)
}

async fn health() -> &'static str {
    "ok"
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header};
    use axum::response::Response;
    use platform::config::Config;
    use tower::ServiceExt;

    use super::*;

    fn app(environment: Environment) -> Router {
        let mut config = Config::default();
        config.server.environment = environment;
        config.features.redis = environment.is_production_like();
        router(Arc::new(AppState::new(config)))
    }

    async fn request(app: &Router, path: &str) -> Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("测试路径合法"),
            )
            .await
            .expect("路由应返回响应")
    }

    async fn body(response: Response) -> String {
        String::from_utf8(
            to_bytes(response.into_body(), 1_000_000)
                .await
                .expect("夹具响应大小应小于 1 MB")
                .to_vec(),
        )
        .expect("夹具应是 UTF-8")
    }

    #[tokio::test]
    async fn every_fixture_is_html_and_deterministic_in_local_environments() {
        for environment in [Environment::Development, Environment::Test] {
            let app = app(environment);
            for page in fixtures::PAGES {
                for theme in ["light", "dark", "system"] {
                    let path = format!("/__fixtures/a00/{page}?theme={theme}");
                    let first = request(&app, &path).await;
                    assert_eq!(first.status(), StatusCode::OK, "{path}");
                    assert_eq!(
                        first.headers().get(header::CONTENT_TYPE).unwrap(),
                        "text/html; charset=utf-8"
                    );
                    let first = body(first).await;
                    let second = body(request(&app, &path).await).await;
                    assert_eq!(first, second, "{path} 必须重复输出相同字节");
                    assert!(first.contains("草莓论坛"), "{path}");
                    assert!(first.contains(fixtures::FIXTURE_VERSION), "{path}");
                    for remote in [
                        "src=\"http:",
                        "src=\"https:",
                        "src=\"//",
                        "href=\"http:",
                        "href=\"https:",
                        "href=\"//",
                        "srcset=\"http",
                        "url(http",
                    ] {
                        assert!(!first.contains(remote), "{path}: 禁止运行时外链 {remote}");
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn production_like_environments_never_mount_fixtures() {
        for environment in [Environment::Staging, Environment::Production] {
            let app = app(environment);
            for page in fixtures::PAGES {
                for suffix in ["", "?theme=light", "?theme=dark", "?theme=system"] {
                    let path = format!("/__fixtures/a00/{page}{suffix}");
                    assert_eq!(
                        request(&app, &path).await.status(),
                        StatusCode::NOT_FOUND,
                        "{environment}: {path}"
                    );
                }
            }
            let home = request(&app, "/").await;
            assert_eq!(home.status(), StatusCode::OK);
            let home = body(home).await;
            assert!(home.contains("骨架首页"));
            assert!(!home.contains(fixtures::FIXTURE_VERSION));
        }
    }

    #[tokio::test]
    async fn fixture_assets_are_allowlisted_and_local_only() {
        const ASSETS: [(&str, &str); 30] = [
            ("site.css", "text/css; charset=utf-8"),
            ("pages.css", "text/css; charset=utf-8"),
            ("profile.css", "text/css; charset=utf-8"),
            ("fonts.css", "text/css; charset=utf-8"),
            ("brand-mark.svg", "image/svg+xml"),
            ("ui-user.svg", "image/svg+xml"),
            ("ui-document.svg", "image/svg+xml"),
            ("ui-forum.svg", "image/svg+xml"),
            ("oauth-google.svg", "image/svg+xml"),
            ("oauth-github.svg", "image/svg+xml"),
            ("oauth-x.svg", "image/svg+xml"),
            ("brand-symbol.svg", "image/svg+xml"),
            ("favicon-16.png", "image/png"),
            ("favicon-32.png", "image/png"),
            ("apple-touch-icon.png", "image/png"),
            ("favicon.ico", "image/x-icon"),
            ("og-image.svg", "image/svg+xml"),
            ("og-image.png", "image/png"),
            ("email-header.svg", "image/svg+xml"),
            ("theme-sun.svg", "image/svg+xml"),
            ("theme-moon.svg", "image/svg+xml"),
            ("ui-eye.svg", "image/svg+xml"),
            ("ui-chat.svg", "image/svg+xml"),
            ("ui-search.svg", "image/svg+xml"),
            ("ui-menu.svg", "image/svg+xml"),
            ("ui-chevron-up.svg", "image/svg+xml"),
            ("space-mono-latin-400-normal.woff2", "font/woff2"),
            ("space-mono-latin-700-normal.woff2", "font/woff2"),
            ("space-mono-latin-ext-400-normal.woff2", "font/woff2"),
            ("space-mono-latin-ext-700-normal.woff2", "font/woff2"),
        ];
        for environment in [
            Environment::Development,
            Environment::Test,
            Environment::Staging,
            Environment::Production,
        ] {
            let app = app(environment);
            for (name, mime) in ASSETS {
                let response = request(&app, &format!("/__fixtures/a00/assets/{name}")).await;
                if environment.is_production_like() {
                    assert_eq!(response.status(), StatusCode::NOT_FOUND, "{name}");
                } else {
                    assert_eq!(response.status(), StatusCode::OK, "{name}");
                    assert_eq!(response.headers()[header::CONTENT_TYPE], mime);
                    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
                    assert_eq!(
                        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
                        "nosniff"
                    );
                }
            }
            for name in ["unknown.css", "%2e%2e%2fCargo.toml", "..%2f..%2f.env"] {
                assert_eq!(
                    request(&app, &format!("/__fixtures/a00/assets/{name}"))
                        .await
                        .status(),
                    StatusCode::NOT_FOUND,
                    "{environment}: {name}"
                );
            }
        }
    }

    #[tokio::test]
    async fn unknown_pages_and_invalid_themes_fail_closed() {
        let app = app(Environment::Test);
        for path in [
            "/__fixtures/a00",
            "/__fixtures/a00/",
            "/__fixtures/a00/unlisted",
            "/__fixtures/a00/home/extra",
        ] {
            assert_eq!(request(&app, path).await.status(), StatusCode::NOT_FOUND);
        }
        assert_eq!(
            request(&app, "/__fixtures/a00/home?theme=unknown")
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        let default = body(request(&app, "/__fixtures/a00/home").await).await;
        let system = body(request(&app, "/__fixtures/a00/home?theme=system").await).await;
        assert_eq!(default, system);
        let untrusted_query = body(
            request(
                &app,
                "/__fixtures/a00/home?q=%3Cscript%3Ealert(1)%3C%2Fscript%3E",
            )
            .await,
        )
        .await;
        assert_eq!(default, untrusted_query, "夹具不得反射请求文本");
        let light = body(request(&app, "/__fixtures/a00/home?theme=light").await).await;
        let dark = body(request(&app, "/__fixtures/a00/home?theme=dark").await).await;
        assert_ne!(light, dark, "主题选项必须实际进入 SSR 输出");
    }

    #[tokio::test]
    async fn authentication_fixtures_have_no_submission_endpoint() {
        let app = app(Environment::Test);
        for page in ["login", "register", "reset"] {
            let path = format!("/__fixtures/a00/{page}");
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(&path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED, "{page}");
            let html = body(request(&app, &path).await).await;
            assert!(!html.contains("name=\"password\""));
            assert!(!html.contains("name=\"email\""));
            assert!(html.contains("只展示控件，不收集或提交账号和密码。"));
        }
    }

    #[test]
    fn page_frames_and_topic_text_are_html_escaped() {
        const UNSAFE: &str = "合成攻击字符串<script>alert(1)</script>\"&'";
        let mut fixture = fixtures::page("home", fixtures::Theme::Light).unwrap();
        fixture.frame = PageFrame {
            flash: Some(UNSAFE),
            error: Some(UNSAFE),
            empty: Some(UNSAFE),
        };
        fixture.title = UNSAFE;
        fixture.topics.items[0].title = UNSAFE;
        fixture.topics.items[0].author = UNSAFE;
        let html = render(&fixture).unwrap_or_else(|_| panic!("合成文本应能渲染"));
        assert!(html.0.contains("合成攻击字符串"));
        assert!(!html.0.contains(UNSAFE));
        assert!(!html.0.contains("<script>alert(1)</script>"));

        let home = render(&HomeTemplate {
            frame: PageFrame {
                flash: Some(UNSAFE),
                error: Some(UNSAFE),
                empty: Some(UNSAFE),
            },
        })
        .unwrap_or_else(|_| panic!("骨架页错误块应能渲染"));
        assert!(home.0.contains("合成攻击字符串"));
        assert!(!home.0.contains(UNSAFE));
    }
}
