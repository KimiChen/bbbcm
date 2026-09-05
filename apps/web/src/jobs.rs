//! 内嵌任务循环（`worker.embedded = true`）：轮询 Outbox 与定时作业。
//! 实际作业随 platform-outbox 与各 sink 接入。

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::app_state::AppState;

/// 轮询间隔。`features.redis = true` 时由 Wakeup 唤醒，间隔只作兜底。
const TICK: Duration = Duration::from_secs(30);

pub async fn run(state: Arc<AppState>, cancel: CancellationToken) {
    tracing::info!(backend = %state.config.database.backend, "内嵌任务循环启动");
    let mut ticker = tokio::time::interval(TICK);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // Outbox 轮询与定时作业随 platform-outbox 接入。
            }
            () = cancel.cancelled() => {
                tracing::info!("内嵌任务循环收到取消信号，退出");
                return;
            }
        }
    }
}
