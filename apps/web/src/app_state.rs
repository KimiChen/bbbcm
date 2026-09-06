//! 进程状态：连接池、缓存与 Flag 集合随后续工作包装配进来。

use platform::config::Config;

pub struct AppState {
    pub content: content::http::ContentWebState,
    pub identity: identity::http::IdentityWebState,
    pub config: Config,
    pub assets: crate::assets::Assets,
    pub status: std::sync::Arc<dyn platform::status::StatusSource>,
    pub runtime: Option<platform::cache::RedisRuntime>,
}

impl AppState {
    pub fn new(config: Config, assets: crate::assets::Assets) -> Self {
        Self {
            content: crate::content_web::closed(&config, &assets),
            identity: crate::identity_web::closed(&config, &assets),
            status: std::sync::Arc::new(platform_db::status_repo::DbStatusSource::new(
                config.database.clone(),
            )),
            config,
            assets,
            runtime: None,
        }
    }
}
