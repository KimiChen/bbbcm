//! bbbcm-web：Axum + Askama SSR 入口。
//! 体验模式（`worker.embedded = true`）下同时承担任务循环宿主。

mod app_state;
mod assets;
mod content_authority;
mod content_risk;
#[cfg(test)]
mod content_runtime_tests;
mod content_web;
mod contract;
mod error;
mod fixture_assets;
mod fixtures;
#[path = "../../identity_services.rs"]
mod identity_services;
mod identity_web;
mod jobs;
#[cfg(test)]
mod manage_runtime_tests;
mod manage_web;
mod management_gate;
mod middleware;
mod moderation_authority;
#[cfg(test)]
mod moderation_runtime_tests;
#[cfg(test)]
mod parameter_conformance_tests;
mod router;
#[cfg(test)]
mod security_tests;
mod status;
#[path = "../../worker_services.rs"]
mod worker_services;

use std::process::ExitCode;
use std::sync::Arc;
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
        Err(err) => {
            // 启动失败是可预期的运行错误（配置缺失、端口占用），
            // 用退出码报告，不用 panic 打一屏 backtrace。
            tracing::error!("{err}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let config = platform::config::Config::from_environment().map_err(|err| err.to_string())?;
    config
        .validate_for(platform::config::ProcessKind::Web)
        .map_err(|err| err.to_string())?;

    platform_db::connection::check_startup(&config.database, platform_db::connection::Purpose::App)
        .await
        .map_err(|err| err.to_string())?;

    let assets = assets::Assets::from_environment(
        config.server.environment,
        Arc::new(assets::NoBrandOverrides),
    )
    .map_err(|err| format!("静态资产加载失败: {err}"))?;
    let runtime = platform::cache::RedisRuntime::from_config(&config).map_err(|e| e.to_string())?;
    platform::commit_signal::install(runtime.wakeup.clone()).map_err(|e| e.to_string())?;
    let mut state = app_state::AppState::new(config.clone(), assets);
    state.runtime = Some(runtime.clone());
    identity_web::connect(&mut state).await?;
    content_authority::connect(&mut state).await?;
    // /manage 控制台在内容与治理授权之后装配：它依赖两者已就绪的服务句柄。
    manage_web::connect(&mut state).await;
    let state = Arc::new(state);

    // 内嵌任务循环与 HTTP 服务共享同一取消令牌：优雅退出时先停任务循环再落地。
    let cancel = CancellationToken::new();
    let prepared_jobs = if config.worker.embedded {
        Some(jobs::prepare(&config, &runtime).await?)
    } else {
        None
    };

    let listener = tokio::net::TcpListener::bind(&config.server.bind)
        .await
        .map_err(|err| format!("监听地址绑定失败（{}）: {err}", config.server.bind))?;
    tracing::info!("bbbcm-web 监听 http://{}", config.server.bind);

    let jobs = prepared_jobs.map(|prepared| tokio::spawn(prepared.run(cancel.clone())));
    let serve = axum::serve(
        listener,
        router::router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(cancel.clone()))
    .await;

    cancel.cancel();
    if let Some(handle) = jobs {
        // 等任务循环真正退出，而不是随进程一起被砍掉。
        handle
            .await
            .map_err(|_| "内嵌任务循环未正常结束".to_owned())??;
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
