use super::*;
use askama::Template;
use axum::body::{Body, to_bytes};
use axum::http::Request;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
use tower::ServiceExt;

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "bbbcm-assets-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, bytes: &[u8]) {
        let file = self.0.join(name);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, bytes).unwrap();
    }
    fn manifest(&self, value: &Value) {
        self.write("manifest.json", &serde_json::to_vec(value).unwrap());
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

pub(crate) fn local() -> Assets {
    Assets::local_sources()
}
fn fixture() -> (Temp, Value) {
    let temp = Temp::new();
    let entries: Vec<_> = SOURCES
        .iter()
        .map(|s| {
            let sha = format!("{:x}", Sha256::digest(s.bytes));
            let (stem, extension, mime) = logical_parts(s.logical).unwrap();
            let name = format!("{stem}.{sha}{extension}");
            temp.write(&name, s.bytes);
            json!({"logical":s.logical,"path":name,"sha256":sha,"size":s.bytes.len(),"mime":mime})
        })
        .collect();
    let manifest = json!({"version":1,"assets":entries});
    temp.manifest(&manifest);
    (temp, manifest)
}
fn app(environment: Environment, assets: Assets) -> Router {
    let mut config = platform::config::Config::default();
    config.server.environment = environment;
    config.features.redis = environment.is_production_like();
    crate::router::router(Arc::new(AppState::new(config, assets)))
}
async fn request(app: &Router, method: &str, path: &str) -> Response {
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
async fn body(response: Response) -> Bytes {
    to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap()
}

#[test]
fn manifest_load_checks_hashes_names_mime_and_duplicate_entries() {
    let (temp, valid) = fixture();
    assert!(Assets::load(&temp.0).is_ok());
    for (key, value) in [
        ("logical", json!("../site.css")),
        ("path", json!("../secret")),
        ("sha256", json!("00")),
        ("size", json!(0)),
        ("mime", json!("text/html")),
        ("extra", json!(true)),
    ] {
        let mut invalid = valid.clone();
        invalid["assets"][0][key] = value;
        temp.manifest(&invalid);
        assert!(Assets::load(&temp.0).is_err(), "{key}");
    }
    let mut invalid = valid.clone();
    invalid["assets"]
        .as_array_mut()
        .unwrap()
        .push(valid["assets"][0].clone());
    temp.manifest(&invalid);
    assert!(Assets::load(&temp.0).is_err());
    for invalid in [
        json!({"version":2,"assets":[]}),
        json!({"version":1,"assets":[]}),
        json!({"version":1,"assets":[],"unknown":true}),
    ] {
        temp.manifest(&invalid);
        assert!(Assets::load(&temp.0).is_err());
    }
    temp.write("manifest.json", br#"{"version":1,"version":1,"assets":[]}"#);
    assert!(Assets::load(&temp.0).is_err());
    temp.manifest(&valid);
    let file = valid["assets"][0]["path"].as_str().unwrap();
    temp.write(file, b"corrupt");
    assert!(Assets::load(&temp.0).is_err());
    fs::remove_file(temp.0.join(file)).unwrap();
    assert!(Assets::load(&temp.0).is_err());
}

#[test]
fn only_never_built_default_local_dist_can_use_source_mode() {
    let temp = Temp::new();
    let missing = temp.0.join("never-built");
    for env in [
        Environment::Development,
        Environment::Test,
        Environment::Staging,
        Environment::Production,
    ] {
        assert_eq!(
            Assets::load_for(env, &missing, false).is_ok(),
            !env.is_production_like()
        );
        assert!(Assets::load_for(env, &missing, true).is_err());
        // Existing dist with missing manifest, and malformed JSON, always fail.
        assert!(Assets::load_for(env, &temp.0, false).is_err());
    }
    temp.write("manifest.json", b"{");
    assert!(Assets::load_for(Environment::Test, &temp.0, false).is_err());
    let (complete, mut manifest) = fixture();
    manifest["assets"].as_array_mut().unwrap().pop();
    complete.manifest(&manifest);
    assert!(Assets::load_for(Environment::Production, &complete.0, true).is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_at_root_manifest_directory_and_file_are_rejected() {
    use std::os::unix::fs::symlink;
    let (temp, manifest) = fixture();
    let outside = Temp::new();
    symlink(&temp.0, outside.0.join("root")).unwrap();
    assert!(Assets::load(&outside.0.join("root")).is_err());
    fs::rename(
        temp.0.join("manifest.json"),
        outside.0.join("manifest.json"),
    )
    .unwrap();
    symlink(
        outside.0.join("manifest.json"),
        temp.0.join("manifest.json"),
    )
    .unwrap();
    assert!(Assets::load(&temp.0).is_err());
    fs::remove_file(temp.0.join("manifest.json")).unwrap();
    temp.manifest(&manifest);
    let name = manifest["assets"][0]["path"].as_str().unwrap();
    fs::remove_file(temp.0.join(name)).unwrap();
    outside.write("file", b"outside");
    symlink(outside.0.join("file"), temp.0.join(name)).unwrap();
    assert!(Assets::load(&temp.0).is_err());
    fs::remove_dir_all(temp.0.join("css")).unwrap();
    symlink(&outside.0, temp.0.join("css")).unwrap();
    assert!(Assets::load(&temp.0).is_err());
}

#[tokio::test]
async fn fingerprint_get_head_and_failure_cache_contract() {
    let (temp, manifest) = fixture();
    let assets = Assets::load_for(Environment::Production, &temp.0, true).unwrap();
    let url = assets.url("css/site.css").unwrap();
    let expected = assets.files[&url[8..]].bytes.clone();
    // Mutating/removing the disk after loading cannot change immutable responses.
    fs::remove_dir_all(temp.0.join("css")).unwrap();
    temp.write("secret.txt", b"must not be served");
    let app = app(Environment::Production, assets);
    for method in ["GET", "HEAD"] {
        let response = request(&app, method, &url).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "public, max-age=31536000, immutable"
        );
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/css; charset=utf-8"
        );
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        assert_eq!(
            response.headers()[header::CONTENT_LENGTH],
            expected.len().to_string()
        );
        let bytes = body(response).await;
        if method == "HEAD" {
            assert!(bytes.is_empty());
        } else {
            assert_eq!(bytes, expected);
        }
    }
    assert_eq!(
        body(request(&app, "GET", &format!("{url}?v=ignored")).await).await,
        expected
    );
    for path in [
        "/assets/unknown.css",
        "/assets/manifest.json",
        "/assets/secret.txt",
        "/assets/css/site.css",
    ] {
        let response = request(&app, "GET", path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    for path in [
        "/assets",
        "/assets/",
        "/assets/../Cargo.toml",
        "/assets/css/./site.css",
        "/assets//site.css",
        "/assets/%2e%2e%2f.env",
        "/assets/%252e%252e%252f.env",
        "/assets/css%2fsite.css",
        "/assets/css%5csite.css",
        "/assets/%00",
        "/assets/%FF",
        "/assets/\\secret",
    ] {
        let response = request(&app, "GET", path).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    assert_eq!(
        request(&app, "POST", &url).await.status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(manifest["assets"].as_array().unwrap().len(), SOURCES.len());
}

#[tokio::test]
async fn ssr_uses_real_manifest_helper_and_production_isolation_is_preserved() {
    let (temp, _) = fixture();
    for env in [
        Environment::Development,
        Environment::Test,
        Environment::Staging,
        Environment::Production,
    ] {
        let assets = Assets::load_for(env, &temp.0, true).unwrap();
        let css = assets.url("css/site.css").unwrap();
        let app = app(env, assets);
        assert_eq!(request(&app, "GET", &css).await.status(), StatusCode::OK);
        if !env.is_production_like() {
            for page in crate::fixtures::PAGES {
                let response = request(&app, "GET", &format!("/__fixtures/a00/{page}")).await;
                assert_eq!(response.status(), StatusCode::OK);
                let html = String::from_utf8(body(response).await.to_vec()).unwrap();
                assert!(html.contains(&css));
                assert!(!html.contains("/__fixtures/a00/assets/"));
            }
        } else {
            for path in [
                "/__fixtures/a00/home",
                "/__fixtures/a00/profile",
                "/__fixtures/a00/assets/site.css",
                "/static/css/site.css",
                "/static/dist/manifest.json",
                "/css/site.css",
                "/Cargo.toml",
                "/.env",
                "/manage/config",
            ] {
                assert_eq!(
                    request(&app, "GET", path).await.status(),
                    StatusCode::NOT_FOUND,
                    "{path}"
                );
            }
            let home =
                String::from_utf8(body(request(&app, "GET", "/").await).await.to_vec()).unwrap();
            assert!(home.contains("骨架首页"));
            assert!(!home.contains("a00-synthetic"));
        }
    }
}

#[tokio::test]
async fn missing_helper_context_and_unknown_template_assets_are_errors() {
    let template = crate::fixtures::page("home", crate::fixtures::Theme::Light).unwrap();
    assert!(
        template.render().is_err(),
        "未注入 assets 不得静默输出源 URL"
    );
    let (temp, _) = fixture();
    let mut assets = Assets::load(&temp.0).unwrap();
    assert!(assets.url("unknown.css").is_err());
    assets.urls.remove("css/site.css");
    let response = request(
        &app(Environment::Test, assets),
        "GET",
        "/__fixtures/a00/home",
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body(response).await, "服务出现内部错误，请稍后重试");
}

#[test]
fn runtime_brand_contract_is_restricted_versioned_and_has_priority() {
    #[derive(Debug)]
    struct Provider {
        valid: bool,
    }
    impl BrandAssetOverrides for Provider {
        fn resolve(&self, logical: &str) -> Option<BrandOverride> {
            assert!(
                is_brand_slot(logical),
                "CSS/UI/OG/fonts must never consult the provider"
            );
            let sha256 = "a".repeat(64);
            Some(BrandOverride {
                url: if self.valid {
                    format!("/media/brand/{sha256}.svg")
                } else {
                    "https://invalid.example/logo.svg".into()
                },
                sha256,
                effective_version: 3,
            })
        }
    }
    let (temp, _) = fixture();
    let mut assets = Assets::load(&temp.0).unwrap();
    let default = assets.url("img/brand-mark.svg").unwrap();
    assert!(default.starts_with("/assets/"));
    assets.brand_overrides = Arc::new(Provider { valid: true });
    assert_eq!(
        assets.url("img/brand-mark.svg").unwrap(),
        format!("/media/brand/{}.svg", "a".repeat(64))
    );
    for logical in [
        "css/site.css",
        "img/ui-menu.svg",
        "img/og-image.png",
        "fonts/space-mono-latin-400-normal.woff2",
    ] {
        assert!(assets.url(logical).unwrap().starts_with("/assets/"));
    }
    assets.brand_overrides = Arc::new(Provider { valid: false });
    assert!(assets.url("img/brand-mark.svg").is_err());
    assert!(assets.url("img/unknown.svg").is_err());
}
