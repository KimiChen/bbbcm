//! bbbcm-web：Axum + Askama SSR 入口。
//! 体验模式（`worker.embedded = true`）下同时承担任务循环宿主。

mod app_state;
mod assets;
mod contract;
mod error;
mod fixtures;
mod jobs;
mod middleware;
mod router;

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // 启动失败是可预期的运行错误（配置缺失、端口占用），
            // 用退出码报告，不用 panic 打一屏 backtrace。
            tracing::error!("{err}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let config_path =
        std::env::var("BBBCM_CONFIG").unwrap_or_else(|_| "config/default.toml".to_string());
    let mut config = platform::config::Config::load(Path::new(&config_path))
        .map_err(|err| format!("配置加载失败（{config_path}）: {err}"))?;
    config.apply_env();
    config
        .validate()
        .map_err(|err| format!("配置校验失败（{config_path}）: {err}"))?;

    let state = Arc::new(app_state::AppState::new(config.clone()));

    // 内嵌任务循环与 HTTP 服务共享同一取消令牌：优雅退出时先停任务循环再落地。
    let cancel = CancellationToken::new();
    let jobs = config
        .worker
        .embedded
        .then(|| tokio::spawn(jobs::run(state.clone(), cancel.clone())));

    let listener = tokio::net::TcpListener::bind(&config.server.bind)
        .await
        .map_err(|err| format!("监听地址绑定失败（{}）: {err}", config.server.bind))?;
    tracing::info!("bbbcm-web 监听 http://{}", config.server.bind);

    let serve = axum::serve(listener, router::router(state))
        .with_graceful_shutdown(shutdown_signal(cancel.clone()))
        .await;

    cancel.cancel();
    if let Some(handle) = jobs {
        // 等任务循环真正退出，而不是随进程一起被砍掉。
        if let Err(err) = handle.await {
            tracing::warn!(%err, "内嵌任务循环未正常结束");
        }
    }
    serve.map_err(|err| format!("服务异常退出: {err}"))
}

/// 同时监听 Ctrl-C 与 SIGTERM：容器与 systemd 发的是 SIGTERM，
/// 只监听 Ctrl-C 会让优雅退出在生产完全失效。
async fn shutdown_signal(cancel: CancellationToken) {
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

    let signal = tokio::select! {
        s = ctrl_c => s,
        s = terminate => s,
        () = cancel.cancelled() => "内部取消",
    };
    tracing::info!("收到 {signal}，开始优雅退出");
    cancel.cancel();
}
