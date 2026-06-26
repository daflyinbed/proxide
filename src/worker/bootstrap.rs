use crate::config::Config;
use crate::npm::types::Packument;
use crate::repository::Repository;
use anyhow::{Context, Result, bail};
use semver::Version;
use std::io::Read;
use std::sync::Arc;

const ALL_PACKAGE_NAMES: &str = "all-package-names";

pub async fn bootstrap_all(
    repo: Arc<dyn Repository>,
    config: &Config,
    client: &reqwest::Client,
) -> Result<()> {
    let count = repo.count_packages().await?;
    if count > 0 {
        bail!(
            "bootstrap aborted: {count} packages already exist in the database. \
             This command is only for seeding an empty registry."
        );
    }

    log::info!(action = "bootstrap_start"; "fetching packument for {ALL_PACKAGE_NAMES}");

    let packument = fetch_packument(config, client).await?;

    let latest_3x = find_latest_3x_version(&packument)?;
    log::info!(
        action = "bootstrap_version";
        "latest 3.x version of {ALL_PACKAGE_NAMES}: {latest_3x}"
    );

    let tarball_url = packument
        .versions
        .get(&latest_3x)
        .context("version not found in packument")?
        .dist
        .tarball
        .clone();

    log::info!(action = "bootstrap_download"; "downloading tarball from {tarball_url}");

    let tarball_bytes = client
        .get(&tarball_url)
        .send()
        .await
        .context("failed to download tarball")?
        .error_for_status()
        .context("tarball download returned error status")?
        .bytes()
        .await
        .context("failed to read tarball body")?;

    log::info!(
        action = "bootstrap_extract";
        "tarball downloaded {} bytes, extracting data/names.json",
        tarball_bytes.len()
    );

    let names = tokio::task::spawn_blocking(move || extract_names(&tarball_bytes))
        .await
        .context("extract task panicked")??;

    log::info!(
        action = "bootstrap_names";
        "extracted {} package names, enqueuing sync tasks",
        names.len()
    );

    let total = names.len();
    let mut enqueued = 0u64;
    const BATCH: usize = 5000;

    for (i, chunk) in names.chunks(BATCH).enumerate() {
        let batch_enqueued = repo.bulk_enqueue_sync_tasks(chunk, "bootstrap").await?;
        enqueued += batch_enqueued;
        let processed = (i + 1) * BATCH;
        log::info!(
            action = "bootstrap_progress";
            "enqueued {enqueued}/{total} ({}/{processed} processed)",
            enqueued.min(total as u64),
        );
    }

    log::info!(
        action = "bootstrap_done";
        "bootstrap complete: {enqueued} sync tasks enqueued out of {total} names"
    );

    Ok(())
}

async fn fetch_packument(config: &Config, client: &reqwest::Client) -> Result<Packument> {
    let url = format!("{}/{ALL_PACKAGE_NAMES}", config.worker.upstream_registry);

    let mut request = client.get(&url);
    if !config.worker.upstream_auth_token.is_empty() {
        request = request.bearer_auth(&config.worker.upstream_auth_token);
    }

    let resp = request
        .send()
        .await
        .with_context(|| format!("failed to fetch packument from {url}"))?;

    if !resp.status().is_success() {
        bail!("{url} returned status {}", resp.status());
    }

    let packument: Packument = resp
        .json()
        .await
        .with_context(|| format!("failed to parse packument from {url}"))?;

    Ok(packument)
}

fn find_latest_3x_version(packument: &Packument) -> Result<String> {
    let latest = packument
        .versions
        .keys()
        .filter_map(|v| Version::parse(v).ok())
        .filter(|v| v.major == 3)
        .max()
        .context("no 3.x version found for all-package-names")?;

    Ok(latest.to_string())
}

fn extract_names(tarball_bytes: &[u8]) -> Result<Vec<String>> {
    let gz = flate2::read::GzDecoder::new(tarball_bytes);
    let mut archive = tar::Archive::new(gz);

    for entry in archive.entries()? {
        let mut entry = entry.context("failed to read tar entry")?;
        let path = entry.path().context("failed to read entry path")?;

        if path.ends_with("data/names.json") {
            let mut json_str = String::new();
            entry
                .read_to_string(&mut json_str)
                .context("failed to read names.json")?;

            let names: Vec<String> =
                serde_json::from_str(&json_str).context("failed to parse names.json")?;

            return Ok(names);
        }
    }

    bail!("data/names.json not found in tarball");
}
