//! Ternox 入口：配置加载、日志初始化、HTTP 服务与优雅关闭。

mod api;
mod app;
mod auth;
mod config;
mod terminal;

use std::sync::Arc;

#[cfg(unix)]
use tokio::signal::unix::{signal, SignalKind};

use app::{build_router, AppState};
use auth::Auth;
use config::Config;
use terminal::manager::TerminalManager;

fn main() {
    // 加载 backend/.env（若存在）；已显式导出的环境变量优先，不会被覆盖
    let _ = dotenvy::dotenv();

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(message) => {
            eprintln!("ternox: 配置错误: {message}");
            std::process::exit(1);
        }
    };
    init_logging();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("构建 tokio runtime 失败");
    runtime.block_on(run(config));
}

fn init_logging() {
    use tracing_subscriber::EnvFilter;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(config::LOG_LEVEL))
        .with_target(false)
        .init();
}

async fn run(config: Config) {
    let config = Arc::new(config);
    // 日志使用脱敏 Debug（密码 [REDACTED]），不输出敏感信息
    tracing::info!(config = ?config, "Ternox 启动");

    let state = AppState {
        config: config.clone(),
        auth: Arc::new(Auth::new(
            &config.auth_username,
            &config.auth_password,
            config::SESSION_TTL,
        )),
        // 工作目录为 $HOME（无路径白名单限制）
        terminals: TerminalManager::new(),
    };

    // 后台任务：终端空闲回收 + 过期认证会话清理
    tokio::spawn({
        let terminals = state.terminals.clone();
        async move {
            terminals.run_reaper().await;
        }
    });
    tokio::spawn({
        let auth = state.auth.clone();
        async move {
            let mut ticker = tokio::time::interval(auth::SESSION_PURGE_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                auth.purge_sessions();
            }
        }
    });

    let addr = format!("{}:{}", config.host, config.port);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(e) => {
            tracing::error!("监听 {addr} 失败: {e}");
            std::process::exit(1);
        }
    };
    tracing::info!("HTTP 服务已就绪: http://{addr}");

    let shutdown_state = state.clone();
    axum::serve(
        listener,
        build_router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(shutdown_state.clone()))
    .await
    .expect("HTTP 服务异常退出");

    // 优雅关闭：终止 Shell 进程组并释放 PTY，避免子进程遗留
    shutdown_state.terminals.shutdown_all().await;
    tracing::info!("Ternox 已退出");
}

/// 优雅关闭信号：SIGTERM（Docker stop）或 SIGINT（Ctrl+C）。
///
/// 触发后先踢掉活跃 WebSocket，使 axum 的优雅关闭尽快完成。
async fn shutdown_signal(state: AppState) {
    #[cfg(unix)]
    {
        let mut terminate = signal(SignalKind::terminate()).expect("注册 SIGTERM 处理失败");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }

    tracing::info!("收到关闭信号，停止接受新连接");
    state.terminals.kick_connections().await;
}
