use anyhow::Result;
use clap::{Parser, Subcommand};
use fastrace::collector;
use logforth::append;
use logforth::record::LevelFilter;
use proxide::config;
use proxide::org_cli::{OrgAction, run_org};
use proxide::repository::ProcessLock;
use proxide::routes::build_router;
use proxide::state::AppState;
use proxide::worker;
use std::future::{Future, IntoFuture};
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
    CleanupStorage {
        #[arg(long)]
        full_scan: bool,
    },
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
        Commands::CleanupStorage { full_scan } => run_cleanup_storage(cfg, full_scan).await,
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
    let mut server_lock = state
        .repo
        .try_acquire_process_lock("server-writer")
        .await?
        .ok_or_else(|| anyhow::anyhow!("another proxide server writer is active"))?;
    state.repo.migrate().await?;

    if state.config.storage_gc.startup_enabled {
        if let Some(mut worker_lock) = state.repo.try_acquire_process_lock("worker-writer").await? {
            let deleted = run_while_locks_held(
                worker::cleanup_storage::cleanup_orphan_storage(
                    &state.repo,
                    &state.config.storage_gc,
                ),
                &mut [&mut server_lock, &mut worker_lock],
            )
            .await?;
            worker_lock.release().await?;
            log::info!(action = "storage_gc_complete"; "deleted={deleted}");
        } else {
            log::warn!(action = "storage_gc_skipped"; "worker writer is active");
        }
    }

    if state.config.cdn.enabled {
        proxide::unpacked::startup_scan(&state).await?;
    }

    let flush_state = state.clone();
    let flush_handle = tokio::spawn(async move {
        proxide::server::run_download_flush(flush_state).await;
    });

    let eviction_handle = if state.config.cdn.enabled {
        let eviction_state = state.clone();
        Some(tokio::spawn(async move {
            proxide::unpacked::run_eviction(eviction_state).await;
        }))
    } else {
        None
    };

    let listener = TcpListener::bind(state.config.server.full_url()).await?;
    let shutdown_flush_state = state.clone();
    let router = build_router(state);
    let server = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .into_future();
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => result?,
        result = monitor_process_lock(server_lock) => {
            return Err(result.expect_err("process lock monitor returned unexpectedly"));
        }
    }

    flush_handle.abort();
    let _ = flush_handle.await;
    if let Some(handle) = eviction_handle {
        handle.abort();
        let _ = handle.await;
    }

    proxide::server::flush_download_counters(&shutdown_flush_state).await?;

    Ok(())
}

async fn run_worker(config: config::Config) -> Result<()> {
    log::info!("Proxide worker starting up...");

    let state = AppState::new(config).await?;
    let ping_url = format!(
        "{}/-/ping",
        state.config.server.root_url.trim_end_matches('/')
    );
    let ping = state
        .http
        .get(&ping_url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| anyhow::anyhow!("server is not healthy at {ping_url}: {error}"))?;
    if !ping.status().is_success() {
        anyhow::bail!(
            "server is not healthy at {ping_url}: status {}",
            ping.status()
        );
    }
    let worker_lock = state
        .repo
        .try_acquire_process_lock("worker-writer")
        .await?
        .ok_or_else(|| anyhow::anyhow!("another proxide worker writer is active"))?;
    let worker = worker::run_worker(
        state.repo,
        state.config,
        state.http,
        state.package_lock,
        state.search,
    );
    tokio::pin!(worker);
    tokio::select! {
        result = &mut worker => result,
        result = monitor_process_lock(worker_lock) => {
            Err(result.expect_err("process lock monitor returned unexpectedly"))
        }
    }
}

async fn run_cleanup_storage(config: config::Config, full_scan: bool) -> Result<()> {
    let state = AppState::new(config).await?;
    let mut server_lock = state
        .repo
        .try_acquire_process_lock("server-writer")
        .await?
        .ok_or_else(|| anyhow::anyhow!("server writer is active; refusing storage cleanup"))?;
    let mut worker_lock = state
        .repo
        .try_acquire_process_lock("worker-writer")
        .await?
        .ok_or_else(|| anyhow::anyhow!("worker writer is active; refusing storage cleanup"))?;
    state.repo.migrate().await?;
    let (deleted, untracked) = run_while_locks_held(
        async {
            let deleted = worker::cleanup_storage::cleanup_orphan_storage(
                &state.repo,
                &state.config.storage_gc,
            )
            .await?;
            let untracked = if full_scan {
                worker::cleanup_storage::cleanup_untracked_storage(
                    &state.repo,
                    &state.config.storage_gc,
                )
                .await?
            } else {
                0
            };
            Ok((deleted, untracked))
        },
        &mut [&mut server_lock, &mut worker_lock],
    )
    .await?;
    worker_lock.release().await?;
    server_lock.release().await?;
    println!("Deleted {deleted} orphan dist object(s) and {untracked} untracked object(s).");
    Ok(())
}

async fn run_reindex_search(config: config::Config) -> Result<()> {
    let state = AppState::new(config).await?;
    state.repo.migrate().await?;
    let search = state.search.as_ref().ok_or_else(|| {
        anyhow::anyhow!("search is not enabled (configure [search] in proxide.toml)")
    })?;
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

async fn monitor_process_lock(mut lock: ProcessLock) -> Result<()> {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        interval.tick().await;
        lock.check().await?;
    }
}

async fn run_while_locks_held<F, T>(future: F, locks: &mut [&mut ProcessLock]) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => return result,
            _ = interval.tick() => {
                for lock in locks.iter_mut() {
                    lock.check().await?;
                }
            }
        }
    }
}
