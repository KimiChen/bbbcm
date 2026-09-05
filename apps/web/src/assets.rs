//! A01: validated, content-addressed snapshots. Requests never access the filesystem.
//! DEC-035: a trusted, audited brand provider may override only named brand slots;
//! bootstrap uses an empty provider until ADMIN-005 implements its write workflow.
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{OriginalUri, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use platform::config::Environment;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::app_state::AppState;
use crate::fixture_assets::SOURCES;

pub const DEFAULT_DIST: &str = "apps/web/static/dist";
const MAX_ASSET: u64 = 16 * 1024 * 1024;
const MAX_TOTAL: usize = 128 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("资产文件读取失败: {0}")]
    Io(#[from] io::Error),
    #[error("manifest JSON 无效: {0}")]
    Json(#[from] serde_json::Error),
    #[error("静态资产校验失败: {0}")]
    Invalid(&'static str),
    #[error("未登记的模板资产: {0}")]
    Unknown(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    assets: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    logical: String,
    path: String,
    sha256: String,
    size: usize,
    mime: String,
}

#[derive(Debug)]
struct LoadedAsset {
    mime: &'static str,
    bytes: Bytes,
}

/// Provider input comes only from the future audited ADMIN-005 read model, never
/// from request parameters. It owns format/dimension/box validation and records
/// the effective version and hash in the same audited replacement workflow.
pub trait BrandAssetOverrides: std::fmt::Debug + Send + Sync {
    fn resolve(&self, logical: &str) -> Option<BrandOverride>;
}

#[derive(Debug)]
pub struct BrandOverride {
    /// Same-origin content-addressed object URL; future media service owns serving.
    pub url: String,
    pub sha256: String,
    pub effective_version: u64,
}

#[derive(Debug)]
pub struct NoBrandOverrides;
impl BrandAssetOverrides for NoBrandOverrides {
    fn resolve(&self, _: &str) -> Option<BrandOverride> {
        None
    }
}

#[derive(Debug)]
pub struct Assets {
    urls: BTreeMap<String, String>,
    files: BTreeMap<String, LoadedAsset>,
    brand_overrides: Arc<dyn BrandAssetOverrides>,
}

impl Assets {
    pub fn from_environment(
        environment: Environment,
        brand_overrides: Arc<dyn BrandAssetOverrides>,
    ) -> Result<Self, AssetError> {
        let override_path = std::env::var_os("BBBCM_ASSET_DIST");
        let dist = override_path
            .as_deref()
            .map(Path::new)
            .unwrap_or(Path::new(DEFAULT_DIST));
        let mut assets = Self::load_for(environment, dist, override_path.is_some())?;
        assets.brand_overrides = brand_overrides;
        Ok(assets)
    }

    pub fn load_for(
        environment: Environment,
        dist: &Path,
        explicit: bool,
    ) -> Result<Self, AssetError> {
        // A never-built default dist is the ONLY fallback case, and only locally.
        // A present directory with a missing/corrupt manifest always fails closed.
        if !explicit
            && !environment.is_production_like()
            && matches!(fs::symlink_metadata(dist), Err(ref e) if e.kind() == io::ErrorKind::NotFound)
        {
            tracing::warn!(
                "默认 dist 尚未构建：使用 development/test 的 A00 源资产模式；运行 scripts/assets.py build 可验证指纹 SSR"
            );
            return Ok(Self::local_sources());
        }
        let assets = Self::load(dist)?;
        for source in SOURCES {
            if !assets.urls.contains_key(source.logical) {
                return Err(AssetError::Invalid("manifest 缺少必需的模板/源资产"));
            }
        }
        Ok(assets)
    }

    fn local_sources() -> Self {
        Self {
            urls: SOURCES
                .iter()
                .map(|s| {
                    (
                        s.logical.into(),
                        format!("/__fixtures/a00/assets/{}", s.name),
                    )
                })
                .collect(),
            files: BTreeMap::new(),
            brand_overrides: Arc::new(NoBrandOverrides),
        }
    }

    fn load(dist: &Path) -> Result<Self, AssetError> {
        let stat = fs::symlink_metadata(dist)?;
        if !stat.is_dir() || stat.is_symlink() {
            return Err(AssetError::Invalid("dist 不是普通目录"));
        }
        let manifest: Manifest =
            serde_json::from_slice(&read_regular(dist, "manifest.json", 2 * 1024 * 1024)?)?;
        if manifest.version != 1 || manifest.assets.is_empty() || manifest.assets.len() > 4096 {
            return Err(AssetError::Invalid("manifest 版本或条目数量无效"));
        }
        let mut result = Self {
            urls: BTreeMap::new(),
            files: BTreeMap::new(),
            brand_overrides: Arc::new(NoBrandOverrides),
        };
        let mut total = 0;
        for entry in manifest.assets {
            let Some((stem, extension, mime)) = logical_parts(&entry.logical) else {
                return Err(AssetError::Invalid("逻辑资产名无效"));
            };
            let expected = format!("{stem}.{}{extension}", entry.sha256);
            if !is_digest(&entry.sha256)
                || entry.path != expected
                || entry.mime != mime
                || entry.size == 0
                || entry.size > MAX_ASSET as usize
            {
                return Err(AssetError::Invalid("指纹文件名、类型或大小无效"));
            }
            if result.urls.contains_key(&entry.logical) || result.files.contains_key(&entry.path) {
                return Err(AssetError::Invalid("重复逻辑名或输出路径"));
            }
            total += entry.size;
            if total > MAX_TOTAL {
                return Err(AssetError::Invalid("资产快照超过 128 MiB"));
            }
            let bytes = read_regular(dist, &entry.path, MAX_ASSET)?;
            if bytes.len() != entry.size || format!("{:x}", Sha256::digest(&bytes)) != entry.sha256
            {
                return Err(AssetError::Invalid("资产内容与 manifest 摘要不一致"));
            }
            result
                .urls
                .insert(entry.logical, format!("/assets/{}", entry.path));
            result.files.insert(
                entry.path,
                LoadedAsset {
                    mime,
                    bytes: Bytes::from(bytes),
                },
            );
        }
        Ok(result)
    }

    /// Only Logo/favicon/app icons participate in DEC-035. CSS/UI/fonts/OG never
    /// consult the provider. Invalid provider output fails rendering, not fallback.
    pub fn url(&self, logical: &str) -> Result<String, AssetError> {
        let base = self
            .urls
            .get(logical)
            .ok_or_else(|| AssetError::Unknown(logical.into()))?;
        if is_brand_slot(logical)
            && let Some(asset) = self.brand_overrides.resolve(logical)
        {
            let extension = logical
                .rsplit_once('.')
                .map(|(_, ext)| ext)
                .unwrap_or_default();
            let expected = format!("/media/brand/{}.{}", asset.sha256, extension);
            if !is_digest(&asset.sha256) || asset.effective_version == 0 || asset.url != expected {
                return Err(AssetError::Invalid("运行时品牌覆盖缺少有效指纹或生效版本"));
            }
            return Ok(asset.url);
        }
        Ok(base.clone())
    }
}

fn is_brand_slot(logical: &str) -> bool {
    matches!(
        logical,
        "img/brand-mark.svg"
            | "img/brand-symbol.svg"
            | "img/favicon-16.png"
            | "img/favicon-32.png"
            | "img/favicon.ico"
            | "img/apple-touch-icon.png"
    )
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn safe_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 240
        && value.split('/').all(|part| {
            part.as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        })
}

fn logical_parts(logical: &str) -> Option<(&str, &'static str, &'static str)> {
    if !safe_path(logical) {
        return None;
    }
    let (category, _) = logical.split_once('/')?;
    let (stem, extension) = logical.rsplit_once('.')?;
    let (extension, mime) = match (category, extension) {
        ("css", "css") => (".css", "text/css; charset=utf-8"),
        ("js", "js" | "ts") => (".js", "text/javascript; charset=utf-8"),
        ("img", "svg") => (".svg", "image/svg+xml"),
        ("img", "png") => (".png", "image/png"),
        ("img", "ico") => (".ico", "image/x-icon"),
        ("img", "jpg") => (".jpg", "image/jpeg"),
        ("img", "jpeg") => (".jpeg", "image/jpeg"),
        ("img", "webp") => (".webp", "image/webp"),
        ("img", "gif") => (".gif", "image/gif"),
        ("fonts", "woff") => (".woff", "font/woff"),
        ("fonts", "woff2") => (".woff2", "font/woff2"),
        ("fonts", "ttf") => (".ttf", "font/ttf"),
        ("fonts", "otf") => (".otf", "font/otf"),
        _ => return None,
    };
    Some((stem, extension, mime))
}

fn read_regular(root: &Path, relative: &str, limit: u64) -> Result<Vec<u8>, AssetError> {
    if !safe_path(relative) {
        return Err(AssetError::Invalid("非法文件路径"));
    }
    let mut file = root.to_path_buf();
    for component in relative.split('/') {
        file.push(component);
        if fs::symlink_metadata(&file)?.is_symlink() {
            return Err(AssetError::Invalid("资产路径不允许符号链接"));
        }
    }
    let stat = fs::symlink_metadata(&file)?;
    if !stat.is_file() || stat.len() > limit {
        return Err(AssetError::Invalid("文件类型或大小无效"));
    }
    let mut bytes = Vec::new();
    File::open(file)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(AssetError::Invalid("读取期间文件超限"));
    }
    Ok(bytes)
}

pub mod filters {
    #[askama::filter_fn]
    pub fn asset_url(logical: &str, values: &dyn askama::Values) -> askama::Result<String> {
        let assets = askama::get_value::<super::Assets>(values, "assets")?;
        assets.url(logical).map_err(askama::Error::custom)
    }
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/assets", get(invalid_path))
        .route("/assets/", get(invalid_path))
        .route("/assets/{*path}", get(asset))
}

async fn invalid_path() -> Response {
    rejection(StatusCode::BAD_REQUEST)
}
fn rejection(status: StatusCode) -> Response {
    (
        status,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
    )
        .into_response()
}

async fn asset(State(state): State<Arc<AppState>>, OriginalUri(uri): OriginalUri) -> Response {
    // Validate the raw path, before URL-decoding or filesystem normalization.
    // Percent escapes (including encoded slashes/dots), backslashes, empty and dot
    // segments are rejected rather than normalized into another allowed resource.
    let Some(path) = uri.path().strip_prefix("/assets/").filter(|p| safe_path(p)) else {
        return rejection(StatusCode::BAD_REQUEST);
    };
    let Some(asset) = state.assets.files.get(path) else {
        return rejection(StatusCode::NOT_FOUND);
    };
    (
        [
            (header::CONTENT_TYPE, asset.mime),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        asset.bytes.clone(),
    )
        .into_response()
}

#[cfg(test)]
#[path = "assets_tests.rs"]
pub(crate) mod tests;
