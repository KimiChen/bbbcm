//! Application composition: ordinary identity services and the existing asset resolver.
use crate::{
    app_state::AppState,
    assets::{Assets, filters},
};
use askama::Template;
use identity::{
    contract::{IdentityError, PasswordScreen, Secret},
    core::{IdentityService, PasswordEngine},
    http::IdentityWebState,
    password_screen::{CommonPasswordScreen, OfflinePasswordScreen},
    policy::ApprovedIdentityPolicy,
    rate::{RateProtection, RiskSignalKey},
    web_view::{Page, PageFrame, PageRenderer},
};
use std::sync::Arc;

#[derive(Template)]
#[template(path = "auth/live.html")]
struct IdentityTemplate<'a> {
    page: &'a Page,
    frame: &'a PageFrame,
}
struct LocalBrandRenderer(Assets);
impl PageRenderer for LocalBrandRenderer {
    fn render(&self, page: &Page) -> Result<String, IdentityError> {
        IdentityTemplate {
            page,
            frame: &page.frame,
        }
        .render_with_values(&("assets", &self.0 as &dyn std::any::Any))
        .map_err(|_| IdentityError::Unavailable)
    }
}
pub fn closed(config: &platform::config::Config, assets: &Assets) -> IdentityWebState {
    IdentityWebState {
        service: None,
        flags: None,
        renderer: Arc::new(LocalBrandRenderer(assets.clone())),
        secure_cookies: config.server.environment.is_production_like(),
        rate: None,
        risk_signals: None,
    }
}
pub async fn connect(state: &mut AppState) -> Result<(), String> {
    let pool = platform_db::Pool::connect_existing(&state.config.database)
        .await
        .map_err(|e| e.to_string())?;
    let renderer = platform::identifiers::SchemaRenderer::new(&state.config.database.schema_prefix)
        .map_err(|e| e.to_string())?;
    // Identity writes require the complete local delivery/risk assembly. PublicRead
    // may serve other domains; it must not make existing accounts writable without it.
    state.identity.service = if state.config.identity.active_token_version.is_some() {
        Some(Arc::new(service(
            &state.config,
            pool.clone(),
            renderer.clone(),
        )?))
    } else {
        None
    };
    state.identity.flags = Some(Arc::new(platform_db::flags_repo::DbFlagReader::new(
        pool, renderer,
    )));
    if let Some(runtime) = state.runtime.as_ref() {
        state.identity.rate = Some(RateProtection::approved_v1(
            state.config.features.redis,
            runtime.rate_limiter.clone(),
        ));
    }
    state.identity.risk_signals = state
        .config
        .identity
        .risk_key
        .as_ref()
        .map(|key| {
            RiskSignalKey::new(Secret::from(key.to_string()))
                .map(Arc::new)
                .map_err(|_| "identity risk key unavailable".to_string())
        })
        .transpose()?;
    Ok(())
}

/// Same ordinary runtime construction used by local route integration tests.
fn service(
    config: &platform::config::Config,
    pool: platform_db::Pool,
    renderer: platform::identifiers::SchemaRenderer,
) -> Result<IdentityService, String> {
    let material = crate::identity_services::tokens(config)?;
    if material.active.is_none() {
        return Err("identity local delivery and risk configuration required".into());
    }
    let screen: Arc<dyn PasswordScreen> = match (
        &config.identity.password_corpus_path,
        &config.identity.password_corpus_sha256,
    ) {
        (Some(path), Some(digest)) => Arc::new(
            OfflinePasswordScreen::load(std::path::Path::new(path), digest)
                .map_err(|_| "identity offline password corpus unavailable".to_string())?,
        ),
        (None, None) => Arc::new(CommonPasswordScreen::new()),
        _ => return Err("identity offline corpus configuration incomplete".into()),
    };
    let mut service = IdentityService::new(
        pool,
        renderer,
        Arc::new(ApprovedIdentityPolicy),
        Arc::new(moderation::core::CurrentPolicyReader::new(
            config.server.environment,
        )),
        Arc::new(
            PasswordEngine::new(config.identity.password_minimum, screen)
                .map_err(|_| "identity password engine unavailable".to_string())?,
        ),
    )
    .with_risk_policy(Arc::new(trust::risk::ApprovedIdentityRiskPolicy));
    if let Some(issuer) = material.active {
        let root = platform::integrations::local::LocalRoot::new(
            std::path::Path::new(
                config
                    .adapters
                    .local_root
                    .as_deref()
                    .ok_or("missing local adapter root")?,
            ),
            config.server.environment,
        )
        .map_err(|_| "identity local challenge root unavailable")?;
        service = service
            .with_token_issuer(issuer.clone())
            .with_challenge_provider(Arc::new(
                identity::challenge::LocalChallengeProvider::new(root, issuer)
                    .with_retained_issuers(material.retained)
                    .map_err(|_| "identity retained challenge keys unavailable")?,
            ));
    }
    Ok(service)
}

#[cfg(test)]
mod tests {
    use super::*;
    use identity::web_view::{Field, Row};
    #[test]
    fn identity_template_escapes_user_data_and_reuses_frozen_components() {
        let missing = std::env::temp_dir().join(format!(
            "bbbcm-assets-{}",
            platform::transaction::new_operation_id()
        ));
        let assets =
            Assets::load_for(platform::config::Environment::Test, &missing, false).unwrap();
        let renderer = LocalBrandRenderer(assets);
        let mut page = Page::new("个人资料", "/settings/profile");
        page.preset_avatar = true;
        page.csrf = "synthetic_csrf".into();
        let sentinel = "<script>alert('synthetic')</script>";
        let mut bio = Field::input("bio", "简介", "textarea");
        bio.value = sentinel.into();
        page.fields.push(bio);
        page.rows.push(Row {
            label: "合成邀请".into(),
            value: sentinel.into(),
            action: Some("/settings/invitations/revoke"),
            action_label: "撤销邀请码",
            reference: "synthetic_reference".into(),
        });
        let html = renderer.render(&page).unwrap();
        assert!(!html.contains(sentinel));
        assert!(html.contains("&lt;script&gt;") || html.contains("&#60;script&#62;"));
        assert!(
            html.contains("预设头像")
                && html.contains("撤销邀请码")
                && html.contains("/me/invites")
        );
        assert!(html.contains("/__fixtures/a00/assets/profile.css"));
        assert!(!html.contains("https://sb.sb"));
        assert!(!html.contains("<script"));
        page = Page::new("登录", "/login");
        page.fields
            .push(Field::input("password", "密码", "password"));
        let html = renderer.render(&page).unwrap();
        assert!(html.contains("autocomplete=\"current-password\""));
        page = Page::new("重置密码", "/reset-password");
        page.fields.extend([
            Field::input("token", "一次性凭据", "password"),
            Field::input("password", "新密码", "password"),
        ]);
        let html = renderer.render(&page).unwrap();
        assert!(
            html.contains("autocomplete=\"off\"") && html.contains("autocomplete=\"new-password\"")
        );
    }
}

#[cfg(test)]
#[path = "identity_web/runtime_tests.rs"]
pub(crate) mod runtime_tests;
