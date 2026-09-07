//! A00 source allowlist, mounted only in development/test. Never a production fallback.
use crate::app_state::AppState;
use axum::{
    Router,
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use std::sync::Arc;

pub struct SourceAsset {
    pub name: &'static str,
    pub logical: &'static str,
    pub mime: &'static str,
    pub bytes: &'static [u8],
}

pub const SOURCES: &[SourceAsset] = &[
    SourceAsset {
        name: "site.css",
        logical: "css/site.css",
        mime: "text/css; charset=utf-8",
        bytes: include_bytes!("../static/css/site.css"),
    },
    SourceAsset {
        name: "pages.css",
        logical: "css/pages.css",
        mime: "text/css; charset=utf-8",
        bytes: include_bytes!("../static/css/pages.css"),
    },
    SourceAsset {
        // 无障碍叠加层（DEC-067）：必须排在三份参考样式之后加载才能生效。
        name: "a11y.css",
        logical: "css/a11y.css",
        mime: "text/css; charset=utf-8",
        bytes: include_bytes!("../static/css/a11y.css"),
    },
    SourceAsset {
        name: "profile.css",
        logical: "css/profile.css",
        mime: "text/css; charset=utf-8",
        bytes: include_bytes!("../static/css/profile.css"),
    },
    SourceAsset {
        name: "fonts.css",
        logical: "css/fonts.css",
        mime: "text/css; charset=utf-8",
        bytes: include_bytes!("../static/css/fonts.css"),
    },
    SourceAsset {
        name: "ui-user.svg",
        logical: "img/ui-user.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/ui-user.svg"),
    },
    SourceAsset {
        name: "ui-document.svg",
        logical: "img/ui-document.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/ui-document.svg"),
    },
    SourceAsset {
        name: "ui-forum.svg",
        logical: "img/ui-forum.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/ui-forum.svg"),
    },
    SourceAsset {
        name: "oauth-google.svg",
        logical: "img/oauth-google.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/oauth-google.svg"),
    },
    SourceAsset {
        name: "oauth-github.svg",
        logical: "img/oauth-github.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/oauth-github.svg"),
    },
    SourceAsset {
        name: "oauth-x.svg",
        logical: "img/oauth-x.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/oauth-x.svg"),
    },
    SourceAsset {
        name: "brand-symbol.svg",
        logical: "img/brand-symbol.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/brand-symbol.svg"),
    },
    SourceAsset {
        name: "favicon-16.png",
        logical: "img/favicon-16.png",
        mime: "image/png",
        bytes: include_bytes!("../static/img/favicon-16.png"),
    },
    SourceAsset {
        name: "favicon-32.png",
        logical: "img/favicon-32.png",
        mime: "image/png",
        bytes: include_bytes!("../static/img/favicon-32.png"),
    },
    SourceAsset {
        name: "apple-touch-icon.png",
        logical: "img/apple-touch-icon.png",
        mime: "image/png",
        bytes: include_bytes!("../static/img/apple-touch-icon.png"),
    },
    SourceAsset {
        name: "favicon.ico",
        logical: "img/favicon.ico",
        mime: "image/x-icon",
        bytes: include_bytes!("../static/img/favicon.ico"),
    },
    SourceAsset {
        name: "og-image.svg",
        logical: "img/og-image.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/og-image.svg"),
    },
    SourceAsset {
        name: "og-image.png",
        logical: "img/og-image.png",
        mime: "image/png",
        bytes: include_bytes!("../static/img/og-image.png"),
    },
    SourceAsset {
        name: "email-header.svg",
        logical: "img/email-header.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/email-header.svg"),
    },
    SourceAsset {
        name: "ui-eye.svg",
        logical: "img/ui-eye.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/ui-eye.svg"),
    },
    SourceAsset {
        name: "ui-chat.svg",
        logical: "img/ui-chat.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/ui-chat.svg"),
    },
    SourceAsset {
        name: "ui-search.svg",
        logical: "img/ui-search.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/ui-search.svg"),
    },
    SourceAsset {
        name: "ui-menu.svg",
        logical: "img/ui-menu.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/ui-menu.svg"),
    },
    SourceAsset {
        name: "ui-chevron-up.svg",
        logical: "img/ui-chevron-up.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/ui-chevron-up.svg"),
    },
    SourceAsset {
        name: "theme-sun.svg",
        logical: "img/theme-sun.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/theme-sun.svg"),
    },
    SourceAsset {
        name: "theme-moon.svg",
        logical: "img/theme-moon.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/theme-moon.svg"),
    },
    SourceAsset {
        name: "brand-mark.svg",
        logical: "img/brand-mark.svg",
        mime: "image/svg+xml",
        bytes: include_bytes!("../static/img/brand-mark.svg"),
    },
    SourceAsset {
        name: "space-mono-latin-400-normal.woff2",
        logical: "fonts/space-mono-latin-400-normal.woff2",
        mime: "font/woff2",
        bytes: include_bytes!("../static/fonts/space-mono-latin-400-normal.woff2"),
    },
    SourceAsset {
        name: "space-mono-latin-700-normal.woff2",
        logical: "fonts/space-mono-latin-700-normal.woff2",
        mime: "font/woff2",
        bytes: include_bytes!("../static/fonts/space-mono-latin-700-normal.woff2"),
    },
    SourceAsset {
        name: "space-mono-latin-ext-400-normal.woff2",
        logical: "fonts/space-mono-latin-ext-400-normal.woff2",
        mime: "font/woff2",
        bytes: include_bytes!("../static/fonts/space-mono-latin-ext-400-normal.woff2"),
    },
    SourceAsset {
        name: "space-mono-latin-ext-700-normal.woff2",
        logical: "fonts/space-mono-latin-ext-700-normal.woff2",
        mime: "font/woff2",
        bytes: include_bytes!("../static/fonts/space-mono-latin-ext-700-normal.woff2"),
    },
];

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/__fixtures/a00/assets/{name}", get(asset))
}

async fn asset(Path(name): Path<String>) -> Response {
    let Some(source) = SOURCES.iter().find(|source| source.name == name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    (
        [
            (header::CONTENT_TYPE, source.mime),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        source.bytes,
    )
        .into_response()
}
