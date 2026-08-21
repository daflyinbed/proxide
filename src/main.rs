use anyhow::Result;
use clap::{Parser, Subcommand};
use fastrace::collector;
use logforth::append;
use logforth::record::LevelFilter;
use proxide::config;
use proxide::org_cli::{OrgAction, run_org};
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
    CleanupStorage,
    ReindexSearch,
    Bootstrap,
    TrainZstdDict {
        #[arg(long, default_value = "zstd-dictionary.bin")]
        output: String,
        #[arg(long, default_value_t = proxide::dict_train::default_dict_size())]
        max_dict_size: usize,
    },
    Org {
        #[command(subcommand)]
        action: OrgAction,
    },
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
        Commands::CleanupStorage => run_cleanup_storage(cfg).await,
        Commands::ReindexSearch => run_reindex_search(cfg).await,
        Commands::Bootstrap => run_bootstrap(cfg).await,
        Commands::TrainZstdDict {
            output,
            max_dict_size,
        } => run_train_zstd_dict(cfg, output, max_dict_size).await,
        Commands::Org { action } => run_org(cfg, action).await,
    }
}

async fn run_server(config: config::Config) -> Result<()> {
    log::info!("Proxide server starting up...");

    let state = AppState::new(config).await?;
    state.repo.migrate().await?;

    let flush_state = state.clone();
    let flush_handle = tokio::spawn(async move {
        proxide::server::run_download_flush(flush_state).await;
    });

    let listener = TcpListener::bind(state.config.server.full_url()).await?;
    let shutdown_flush_state = state.clone();
    let router = build_router(state);
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    flush_handle.abort();
    let _ = flush_handle.await;

    proxide::server::flush_download_counters(&shutdown_flush_state).await?;

    Ok(())
}

async fn run_worker(config: config::Config) -> Result<()> {
    log::info!("Proxide worker starting up...");

    let state = AppState::new(config).await?;
    state.repo.migrate().await?;
    worker::run_worker(
        state.repo,
        state.config,
        state.http,
        state.package_lock,
        state.search,
    )
    .await
}

async fn run_cleanup_storage(config: config::Config) -> Result<()> {
    let state = AppState::new(config).await?;
    state.repo.migrate().await?;
    worker::cleanup_storage::cleanup_orphan_storage(&state.repo).await
}

async fn run_reindex_search(config: config::Config) -> Result<()> {
    let state = AppState::new(config).await?;
    state.repo.migrate().await?;
    let search = state
        .search
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("search is not enabled (configure [search] in proxide.toml)"))?;
    search.ensure_index().await?;
    proxide::search::reindex_all(&*state.repo, search).await
}

async fn run_bootstrap(config: config::Config) -> Result<()> {
    let state = AppState::new(config).await?;
    state.repo.migrate().await?;
    worker::bootstrap::bootstrap_all(state.repo, &state.config, &state.http).await
}

async fn run_train_zstd_dict(
    config: config::Config,
    output: String,
    max_dict_size: usize,
) -> Result<()> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(300))
        .build()?;
    proxide::dict_train::train_zstd_dict(&config, &client, &output, max_dict_size).await
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
