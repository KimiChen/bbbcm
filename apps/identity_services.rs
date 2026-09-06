//! Versioned token material shared by Web and both Worker hosts. No implicit secrets.
use identity::{contract::Secret, core::TokenIssuer, mail_source::IdentityMailSource};
use platform::{
    config::Config,
    identifiers::SchemaRenderer,
    mail::{MailMessageSource, UnwiredMailSource},
};
use platform_db::Pool;
use std::sync::Arc;

pub struct TokenMaterial {
    pub active: Option<Arc<TokenIssuer>>,
    pub retained: Vec<Arc<TokenIssuer>>,
}
pub fn tokens(config: &Config) -> Result<TokenMaterial, String> {
    config
        .identity
        .validate(config.server.environment, config.adapters.mode)
        .map_err(|_| "invalid identity assembly configuration".to_string())?;
    let mut active = None;
    let mut retained = Vec::new();
    for (version, key) in config
        .identity
        .token_keyring()
        .map_err(|_| "invalid identity keyring")?
    {
        let issuer = Arc::new(
            TokenIssuer::new(Secret::from(key.to_string()), version.clone())
                .map_err(|_| "invalid identity token key")?,
        );
        if config.identity.active_token_version.as_ref() == Some(&version) {
            active = Some(issuer.clone());
        }
        retained.push(issuer);
    }
    Ok(TokenMaterial { active, retained })
}
pub fn mail_source(
    config: &Config,
    pool: Pool,
    renderer: SchemaRenderer,
) -> Result<Arc<dyn MailMessageSource>, String> {
    let material = tokens(config)?;
    if material.active.is_none() {
        return Ok(Arc::new(UnwiredMailSource));
    }
    Ok(Arc::new(
        IdentityMailSource::new_local(
            pool,
            renderer,
            material.retained,
            config
                .identity
                .mail_from
                .clone()
                .ok_or("missing identity local sender")?,
            config
                .identity
                .mail_base_url
                .clone()
                .ok_or("missing identity local origin")?,
        )
        .map_err(|_| "identity mail assembly unavailable")?,
    ))
}
