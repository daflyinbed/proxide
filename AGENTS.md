# AGENTS.md

## Project Overview

Proxide is an NPM registry mirror written in Rust. Two binaries via clap subcommands: `proxide server` (HTTP API) and `proxide worker` (sync engine).

## Architecture

```
src/
  main.rs              # Entry: clap → server | worker
  lib.rs               # Module registration
  config.rs            # TOML config loading (proxide.toml)
  error.rs             # WebError (CustomApi / NotFound / BadRequest) → JSON responses
  state.rs             # AppState { repo: Arc<dyn Repository>, config, http }

  npm/
    mod.rs             # split_scope_name, decode_fullname
    types.rs           # Packument, AbbreviatedPackument, ChangesResult, FastMeta* types

  storage/
    mod.rs             # (exports s3 only)
    s3.rs              # S3 storage via object_store crate

  repository/
    mod.rs             # Row types + Repository trait (async_trait)
    mysql.rs           # MysqlRepository — all DB operations via sqlx

  handlers/
    home.rs            # GET /-/ping
    registry.rs        # /npm/* package routes
    fast_meta.rs       # /fast/* fast-npm-meta routes
    tarball.rs         # /npm/{fullname}/-/{filename} tarball download (proxy on demand)
    sync.rs            # PUT /npm/-/package/{fullname}/syncs — enqueue manual sync

  worker/
    changes_poller.rs  # Poll upstream _changes feed → enqueue sync_tasks
    task_consumer.rs   # Claim & execute sync tasks from DB queue
    sync_package.rs    # Core sync: fetch upstream packument → diff → write DB + S3
    cleanup.rs         # Periodic: requeue stale tasks, purge old tasks

  routes.rs            # Router: /npm and /fast nested under separate prefixes
```

## Routes

- **NPM Registry** (`/npm/`): `GET /` (root info), `GET /{fullname}`, `GET /{fullname}/{version}`, `GET /{fullname}/-/{filename}`, `PUT /-/package/{fullname}/syncs`
- **fast-npm-meta** (`/fast/`): `GET /resolve/{pkg}`, `GET /versions/{pkg}`, `GET /full/{pkg}`
- **Misc**: `GET /-/ping`

## Worker Architecture

The worker runs three concurrent loops:
1. **changes_poller** — polls upstream `_changes` feed, enqueues `sync_tasks` rows
2. **task_consumer** (N instances, `worker.consumer_count`) — claims pending tasks, runs `sync_package`, marks done/failed
3. **cleanup_scheduler** — periodically requeues stale tasks (timeout) and purges old tasks (retention_days)

## Common Commands

```bash
cargo run -- server          # Run HTTP server
cargo run -- worker          # Run sync worker
sqlx migrate run             # Apply migrations
sqlx migrate revert          # Revert last migration
cargo sqlx prepare           # Refresh offline query cache (run after schema changes)
```

## sqlx Compile-Time Macros

Use `sqlx::query!` / `sqlx::query_as!` (compile-time checked), NOT `sqlx::query()` (runtime). Requires `DATABASE_URL` in `.env` for macro expansion. For offline builds, run `cargo sqlx prepare` first — `.sqlx/` directory holds the cached query metadata.

## Database

- **MySQL 8.4** via `docker-compose up db`
- **Tables**: `dists`, `packages`, `package_versions`, `package_tags`, `change_stream_cursors`, `sync_tasks`
- **Migrations**: `/migrations`, managed by sqlx-cli

## Configuration

`proxide.toml` (loaded from CWD, see `src/config.rs` for full schema). Required sections: `database`, `server`, `storage` (Local or S3), `log`, `worker`.

## S3 Paths

```
packages/{fullname}/
  abbreviated_manifests.json    # All-versions abbreviated
  full_manifests.json           # All-versions full
  {version}/
    abbreviated.json            # Per-version abbreviated
    package.json                # Per-version full manifest
    {name}-{version}.tgz        # Tarball (on-demand proxy)
```

## Code Style

- No comments unless requested
- `.rs` files: 4-space indent (see `.editorconfig`)
- `sqlx::query!` macros, not `sqlx::query()`
- Rust edition 2024, toolchain 1.95.0
