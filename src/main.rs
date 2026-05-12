use anyhow::Result;
use clap::{Parser, Subcommand};
use fastrace::collector;
use logforth::append;
use logforth::record::LevelFilter;
use proxide::config;
use proxide::routes::build_router;
use proxide::state::AppState;
use proxide::worker;
use tokio::net::TcpListener;
use tokio::signal;
use tracing_subscriber::layer::SubscriberExt;

#[derive(Parser)]
#[command(name = "proxide", version, about = "NPM registry mirror")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Server,
    Worker,
    CleanupS3,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = config::load_config("./proxide.toml")?;

    fastrace::set_reporter(collector::ConsoleReporter, collector::Config::default());

    let subscriber =
        tracing_subscriber::Registry::default().with(fastrace_tracing::FastraceCompatLayer::new());
    tracing::subscriber::set_global_default(subscriber).unwrap();

    logforth::starter_log::builder()
        .dispatch(|d| {
            d.filter(LevelFilter::MoreSevereEqual(cfg.log.level.into()))
                .append(append::Stderr::default())
        })
        .apply();

    match cli.command {
        Commands::Server => run_server(cfg).await,
        Commands::Worker => run_worker(cfg).await,
        Commands::CleanupS3 => run_cleanup_s3(cfg).await,
    }
}

async fn run_server(config: config::Config) -> Result<()> {
    log::info!("Proxide server starting up...");

    let state = AppState::new(config).await?;
    let listener = TcpListener::bind(state.config.server.full_url()).await?;
    let router = build_router(state);
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn run_worker(config: config::Config) -> Result<()> {
    log::info!("Proxide worker starting up...");

    let state = AppState::new(config).await?;
    worker::run_worker(state.repo, state.config, state.http).await
}

async fn run_cleanup_s3(config: config::Config) -> Result<()> {
    let state = AppState::new(config).await?;
    worker::cleanup_s3::cleanup_orphan_s3(&state.repo).await
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
