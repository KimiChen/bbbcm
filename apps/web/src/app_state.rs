//! 进程状态：连接池、缓存与 Flag 集合随后续工作包装配进来。

use platform::config::Config;

pub struct AppState {
    pub content: content::http::ContentWebState,
    pub identity: identity::http::IdentityWebState,
    pub config: Config,
    pub assets: crate::assets::Assets,
    pub status: std::sync::Arc<dyn platform::status::StatusSource>,
    pub runtime: Option<platform::cache::RedisRuntime>,
    /// 治理命令的管理授权实现。
    ///
    /// 在装配根构造一次并共享：`/manage` 的治理路由（后续波次）与任何调用 moderation
    /// 命令的路径都必须经它，不允许各自另建一份判定。`None` 表示尚未装配，
    /// 此时治理命令整体失败关闭。
    pub moderation_authority:
        Option<std::sync::Arc<crate::moderation_authority::ApplicationModerationAuthority>>,
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
            // 装配在 content_authority::connect 中完成；未连接数据库时保持未装配。
            moderation_authority: None,
        }
    }
}
