//! bbbcm-worker：后台任务宿主（Outbox 消费、邮件、媒体、清理、聚合）。
//! 使用与内嵌 jobs 相同的调度器；本进程不写业务规则。

#[path = "../../worker_services.rs"]
mod worker_services;
use platform_outbox::HostLock;
use std::process::ExitCode;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .json()
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

    let runtime = platform::cache::RedisRuntime::from_config(&config).map_err(|e| e.to_string())?;
    platform::commit_signal::install(runtime.wakeup.clone()).map_err(|e| e.to_string())?;
    let pool =
        platform_db::Pool::connect_for(&config.database, platform_db::connection::Purpose::Worker)
            .await
            .map_err(|error| error.to_string())?;
    let _host_lock = HostLock::acquire(&config.database).map_err(|error| error.to_string())?;
    let scheduler = worker_services::scheduler(&config, pool.clone(), &runtime)?;
    let cancel = CancellationToken::new();
    let signal_cancel = cancel.clone();
    let signal = tokio::spawn(async move {
        let name = shutdown_signal().await;
        tracing::info!("收到 {name}，停止领取并释放未完成租约");
        signal_cancel.cancel();
    });
    tracing::info!(adapter_mode=?config.adapters.mode, "bbbcm-worker Outbox 调度启动");
    let observer = tokio::spawn(worker_services::observe_mail(
        config,
        pool.clone(),
        cancel.clone(),
    ));
    let result = scheduler
        .run(cancel.clone())
        .await
        .map_err(|error| error.to_string());
    cancel.cancel();
    let _ = observer.await;
    signal.abort();
    pool.close().await;
    result?;
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
