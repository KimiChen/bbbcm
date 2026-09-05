//! A00 development-only, embedded static allowlist. This is not the A01 asset pipeline.
//! The parent router mounts this module only in development/test; source assets
//! never become production routes. A01 will implement esbuild content fingerprints.

use std::sync::Arc;

use axum::Router;
use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::app_state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/__fixtures/a00/assets/{name}", get(asset))
}

async fn asset(Path(name): Path<String>) -> Response {
    let (mime, bytes): (&str, &[u8]) = match name.as_str() {
        "site.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../static/css/site.css"),
        ),
        "pages.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../static/css/pages.css"),
        ),
        "profile.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../static/css/profile.css"),
        ),
        "fonts.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../static/css/fonts.css"),
        ),
        "ui-user.svg" => ("image/svg+xml", include_bytes!("../static/img/ui-user.svg")),
        "ui-document.svg" => (
            "image/svg+xml",
            include_bytes!("../static/img/ui-document.svg"),
        ),
        "ui-forum.svg" => (
            "image/svg+xml",
            include_bytes!("../static/img/ui-forum.svg"),
        ),
        "oauth-google.svg" => (
            "image/svg+xml",
            include_bytes!("../static/img/oauth-google.svg"),
        ),
        "oauth-github.svg" => (
            "image/svg+xml",
            include_bytes!("../static/img/oauth-github.svg"),
        ),
        "oauth-x.svg" => ("image/svg+xml", include_bytes!("../static/img/oauth-x.svg")),
        "brand-symbol.svg" => (
            "image/svg+xml",
            include_bytes!("../static/img/brand-symbol.svg"),
        ),
        "favicon-16.png" => ("image/png", include_bytes!("../static/img/favicon-16.png")),
        "favicon-32.png" => ("image/png", include_bytes!("../static/img/favicon-32.png")),
        "apple-touch-icon.png" => (
            "image/png",
            include_bytes!("../static/img/apple-touch-icon.png"),
        ),
        "favicon.ico" => ("image/x-icon", include_bytes!("../static/img/favicon.ico")),
        "og-image.svg" => (
            "image/svg+xml",
            include_bytes!("../static/img/og-image.svg"),
        ),
        "og-image.png" => ("image/png", include_bytes!("../static/img/og-image.png")),
        "email-header.svg" => (
            "image/svg+xml",
            include_bytes!("../static/img/email-header.svg"),
        ),
        "ui-eye.svg" => ("image/svg+xml", include_bytes!("../static/img/ui-eye.svg")),
        "ui-chat.svg" => ("image/svg+xml", include_bytes!("../static/img/ui-chat.svg")),
        "ui-search.svg" => (
            "image/svg+xml",
            include_bytes!("../static/img/ui-search.svg"),
        ),
        "ui-menu.svg" => ("image/svg+xml", include_bytes!("../static/img/ui-menu.svg")),
        "ui-chevron-up.svg" => (
            "image/svg+xml",
            include_bytes!("../static/img/ui-chevron-up.svg"),
        ),
        "theme-sun.svg" => (
            "image/svg+xml",
            include_bytes!("../static/img/theme-sun.svg"),
        ),
        "theme-moon.svg" => (
            "image/svg+xml",
            include_bytes!("../static/img/theme-moon.svg"),
        ),
        "brand-mark.svg" => (
            "image/svg+xml",
            include_bytes!("../static/img/brand-mark.svg"),
        ),
        "space-mono-latin-400-normal.woff2" => (
            "font/woff2",
            include_bytes!("../static/fonts/space-mono-latin-400-normal.woff2"),
        ),
        "space-mono-latin-700-normal.woff2" => (
            "font/woff2",
            include_bytes!("../static/fonts/space-mono-latin-700-normal.woff2"),
        ),
        "space-mono-latin-ext-400-normal.woff2" => (
            "font/woff2",
            include_bytes!("../static/fonts/space-mono-latin-ext-400-normal.woff2"),
        ),
        "space-mono-latin-ext-700-normal.woff2" => (
            "font/woff2",
            include_bytes!("../static/fonts/space-mono-latin-ext-700-normal.woff2"),
        ),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response()
}
