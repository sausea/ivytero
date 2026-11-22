use agentero_rss_server::api::{router, AppState};
use agentero_rss_server::config::Config;
use agentero_rss_server::ingest::scheduler::Scheduler;
use agentero_rss_server::storage::Archive;
use std::sync::Arc;
use std::time::Duration;

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config: {e}");
            std::process::exit(2);
        }
    };
    let archive = match Archive::open(&config.db_path) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("open {}: {e}", config.db_path.display());
            std::process::exit(1);
        }
    };
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let scheduler = Scheduler {
        archive: archive.clone(),
        policy: Arc::new(config.fetch.clone()),
        concurrency: config.concurrency,
        tick: Duration::from_secs(15),
    };
    let scheduler = tokio::spawn(scheduler.run(stop_rx));

    let app = router(AppState {
        archive,
        read_token: Arc::new(config.read_token.clone()),
        admin_token: config.admin_token.clone().map(Arc::new),
        default_refresh_secs: config.default_refresh_secs,
    });
    let listener = match tokio::net::TcpListener::bind(config.bind).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind {}: {e}", config.bind);
            std::process::exit(1);
        }
    };
    log::info!("listening on {}", config.bind);
    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        log::error!("server: {e}");
    }
    let _ = stop_tx.send(true);
    let _ = scheduler.await;
}
