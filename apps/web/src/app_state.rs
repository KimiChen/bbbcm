//! 进程状态：连接池、缓存与 Flag 集合随后续工作包装配进来。

use platform::config::Config;

#[derive(Debug)]
pub struct AppState {
    pub config: Config,
    pub assets: crate::assets::Assets,
}

impl AppState {
    pub fn new(config: Config, assets: crate::assets::Assets) -> Self {
        Self { config, assets }
    }
}
