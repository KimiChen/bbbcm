//! 进程状态：连接池、缓存与 Flag 集合随后续工作包装配进来。

use platform::config::Config;

#[derive(Debug)]
pub struct AppState {
    pub config: Config,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}
