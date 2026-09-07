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
    /// `/manage` 控制台状态。后端为 `None` 时全部管理路由 404。
    pub console: manage::console::http::ConsoleState,
    /// 举报入口状态。未装配数据库时为 `None`，整组举报路由不注册（框架 404）。
    pub reports: Option<moderation::http::ReportState>,
    /// 在装配根构造一次并共享：`/manage` 的治理路由与任何调用 moderation 命令的
    /// 路径都必须经它，不允许各自另建一份判定。`None` 表示尚未装配，
    /// 此时治理命令整体失败关闭。
    pub moderation_authority:
        Option<std::sync::Arc<crate::moderation_authority::ApplicationModerationAuthority>>,
}

impl AppState {
    pub fn new(config: Config, assets: crate::assets::Assets) -> Self {
        Self {
            reports: None,
            content: crate::content_web::closed(&config, &assets),
            identity: crate::identity_web::closed(&config, &assets),
            status: std::sync::Arc::new(platform_db::status_repo::DbStatusSource::new(
                config.database.clone(),
            )),
            console: crate::manage_web::closed(&config, &assets),
            config,
            assets,
            runtime: None,
            // 装配在 content_authority::connect 中完成；未连接数据库时保持未装配。
            moderation_authority: None,
        }
    }
}
