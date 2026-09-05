//! Application composition: ordinary identity services and the existing asset resolver.
use crate::{
    app_state::AppState,
    assets::{Assets, filters},
};
use askama::Template;
use identity::{
    contract::{DisabledPasswordScreen, IdentityError, UnapprovedIdentityPolicy},
    core::{IdentityService, PasswordEngine},
    http::IdentityWebState,
    rate::RateProtection,
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
    let service = IdentityService::new(
        pool.clone(),
        renderer.clone(),
        Arc::new(UnapprovedIdentityPolicy),
        Arc::new(moderation::core::CurrentPolicyReader::new(
            state.config.server.environment,
        )),
        Arc::new(
            PasswordEngine::new(8, Arc::new(DisabledPasswordScreen)).map_err(|e| e.to_string())?,
        ),
    );
    state.identity.service = Some(Arc::new(service));
    state.identity.flags = Some(Arc::new(platform_db::flags_repo::DbFlagReader::new(
        pool, renderer,
    )));
    if let Some(runtime) = state.runtime.as_ref() {
        state.identity.rate = Some(RateProtection {
            redis_enabled: state.config.features.redis,
            limiter: runtime.rate_limiter.clone(),
            registration: None,
            login: None,
            recovery: None,
            invitation: None,
        });
    }
    Ok(())
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
