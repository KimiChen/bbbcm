//! bbbcm-worker：后台任务宿主（Outbox 消费、邮件、媒体、清理、聚合）。
//! Outbox 循环与各 sink 随 platform-outbox 工作包接入；本进程不写业务规则。

use std::process::ExitCode;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

/// 轮询间隔。`features.redis = true` 时由 Wakeup 唤醒，间隔只作兜底。
const TICK: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let config = platform::config::Config::from_environment().map_err(|error| error.to_string())?;
    config
        .validate_for(platform::config::ProcessKind::Worker)
        .map_err(|error| error.to_string())?;
    platform_db::connection::check_startup(
        &config.database,
        platform_db::connection::Purpose::Worker,
    )
    .await
    .map_err(|err| err.to_string())?;

    tracing::info!("bbbcm-worker 骨架启动：Outbox 循环随 platform-outbox 接入");

    let mut ticker = tokio::time::interval(TICK);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // 轮询 Outbox 与定时作业随 platform-outbox 接入。
            }
            signal = shutdown_signal() => {
                tracing::info!("收到 {signal}，bbbcm-worker 退出");
                break;
            }
        }
    }
    Ok(())
}

/// 同时监听 Ctrl-C 与 SIGTERM：容器与 systemd 发的是 SIGTERM。
async fn shutdown_signal() -> &'static str {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
        "SIGINT"
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
                "SIGTERM"
            }
            Err(err) => {
                tracing::warn!(%err, "无法监听 SIGTERM");
                std::future::pending().await
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<&str>();

    tokio::select! {
        s = ctrl_c => s,
        s = terminate => s,
    }
}
