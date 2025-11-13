pub mod config;
pub mod handlers;
pub mod repository;
pub mod routes;
pub mod state;

use crate::routes::build_router;
use crate::state::AppState;
use anyhow::Result;
use fastrace::collector;
use logforth::append;
use logforth::record::LevelFilter;
use tokio::net::TcpListener;
use tokio::signal;
use tracing_subscriber::layer::SubscriberExt;

#[tokio::main]
async fn main() -> Result<()> {
    let config = config::load_config("./proxide.toml")?;
    fastrace::set_reporter(collector::ConsoleReporter, collector::Config::default());

    let subscriber =
        tracing_subscriber::Registry::default().with(fastrace_tracing::FastraceCompatLayer::new());
    tracing::subscriber::set_global_default(subscriber).unwrap();

    logforth::starter_log::builder()
        .dispatch(|d| {
            d.filter(LevelFilter::MoreSevereEqual(config.log.level.into()))
                .append(append::Stderr::default())
        })
        .apply();

    log::info!("Proxide starting up...");

    let listener = TcpListener::bind(config.server.full_url()).await?;
    let router = build_router(AppState::new(config).await?);
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
