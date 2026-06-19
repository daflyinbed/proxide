# AGENTS.md

## Project Overview

NPM registry mirror written in Rust. Four clap subcommands: `proxide server` (HTTP API), `proxide worker` (sync engine), `proxide cleanup-storage` (orphan storage object remover), `proxide reindex-search` (rebuild Meilisearch index from DB).

## Commands

```bash
cargo run -- server             # HTTP server (reads proxide.toml from CWD)
cargo run -- worker             # Sync worker
cargo run -- cleanup-storage    # Remove orphan storage objects
cargo run -- reindex-search     # Rebuild Meilisearch index (requires [search] config)
cargo check                     # Verify compilation
sqlx migrate run                # Apply migrations (needs DATABASE_URL in .env)
sqlx migrate revert             # Revert last migration
cargo sqlx prepare              # Refresh .sqlx/ offline cache (required after schema changes)
just init                       # Install cargo-release, git-cliff, sqlx-cli
just up                         # Alias for sqlx migrate run
just pre-release <version>      # Generate CHANGELOG + stage it
pnpm test:e2e                   # End-to-end tests (requires Docker + cargo build first)
```

## Build & sqlx

- Compile-time checked queries only: `sqlx::query!` / `sqlx::query_as!` — never `sqlx::query()`
- Macro expansion needs `DATABASE_URL` in `.env` (pointing at a running MySQL)
- After any schema change, run `cargo sqlx prepare` to regenerate `.sqlx/` JSON files
- **Never manually edit or delete files in `.sqlx/`** — always use `cargo sqlx prepare` to regenerate

## Database

- MariaDB 10.1 (local, credentials from `.env`)
- Tables: `dists`, `packages`, `package_versions`, `package_tags`, `change_stream_cursors`, `sync_tasks`, `users`, `tokens`, `maintainers`, `package_downloads` (local, per-version per-day counters `d01`..`d31`), `upstream_package_downloads` (upstream npm, per-package per-day counters)
- Migrations in `/migrations`, managed by sqlx-cli
- Login sessions are **in-memory** (`DashMap` in `AppState`), not a DB table

## Infrastructure

Docker Compose provides local dev dependencies:
- **MariaDB 10.1** on `localhost:3306` (root/root, database: `proxide`)
- **RustFS** (S3-compatible) on `localhost:9000` (access: `proxide`/`proxide123`)
- **Meilisearch v1.12** on `localhost:7700` (master key: `proxide`)

```bash
docker compose up -d --wait    # Start all services
```

## Configuration

`proxide.toml` loaded from CWD. Keys are **camelCase** (serde `rename_all = "camelCase"`), not snake_case. Required sections: `database`, `server`, `storage` (Local or S3), `log`, `worker`. Optional sections: `auth`, `search`. See `src/config.rs` for full schema and defaults.

## Architecture

```
src/
  main.rs              # clap → server | worker | cleanup-storage | reindex-search
  lib.rs               # Module registration
  config.rs            # TOML config (camelCase keys)
  error.rs             # WebError → JSON responses
  routes.rs            # Route definitions, nests /npm, /fast, /api
  state.rs             # AppState { repo, config, http, package_lock, login_sessions, tarball_downloads, download_counters, search }

  npm/types.rs         # Packument, AbbreviatedPackument, FastMeta* types
  npm/mod.rs           # split_scope_name, decode_fullname

  storage/backend.rs   # object_store crate (S3 + LocalFileSystem)

  repository/mod.rs    # Row types + Repository trait (async_trait)
  repository/mysql.rs  # MysqlRepository — all DB operations

  search/              # Meilisearch integration (optional, enabled by [search] config)
    mod.rs             # SearchIndex + reindex_all
    document.rs        # SearchDocument builder

  server/
    download_flush.rs  # Periodic flush of in-memory download counters → DB

  middleware/
    mod.rs             # Middleware module
    auth.rs            # Auth middleware (token validation)

  handlers/
    mod.rs
    registry.rs        # /npm/* package routes
    fast_meta.rs       # /fast/* fast-npm-meta routes
    tarball.rs         # Tarball download (on-demand proxy)
    package_dispatch.rs # Fallback handler — routes GET/PUT by path pattern
    publish.rs         # PUT publish (auth-protected)
    sync.rs            # PUT /-/package/{fullname}/syncs
    search.rs          # GET /npm/-/v1/search (Meilisearch)
    downloads.rs       # /api/downloads/{point,range}/* (npm download-counts API)
    auth.rs            # PUT /-/user/org.couchdb.user:{name} (legacy login)
    web_login.rs       # POST /-/v1/login + GET poll done
    home.rs            # GET /-/ping
    sso/
      cas.rs           # CAS 2.0 callback handler

  worker/
    changes_poller.rs  # Poll upstream _changes → enqueue sync_tasks
    task_consumer.rs   # Claim & execute tasks
    sync_package.rs    # Fetch upstream packument → diff → write DB + storage + search index
    cleanup.rs         # Requeue stale tasks, purge old tasks
    cleanup_storage.rs # Remove orphan dists/storage objects
```

## Worker

Three concurrent loops in `run_worker` (plus initial `cleanup_once`):
1. **changes_poller** — polls upstream `_changes` feed, enqueues `sync_tasks` rows
2. **task_consumer** (N = `worker.consumer_count`) — claims pending tasks, runs `sync_package`, marks done/failed
3. **cleanup_scheduler** — periodically requeues stale tasks (`task_timeout_secs`) and purges old tasks (`task_retention_days`)

## Routes

Defined in `src/routes.rs`. Package routes use a **fallback handler** (`package_dispatch`) that parses the URL path to dispatch to registry/tarball/publish:

- **NPM** (`/npm/`): `GET /`, `GET /{fullname}`, `GET /{fullname}/{version}`, `GET /{fullname}/-/{filename}`, `PUT /{fullname}`, `PUT /-/package/{fullname}/syncs`, `PUT /-/user/org.couchdb.user:{name}`, `POST /-/v1/login`, `GET /-/v1/login/done/session/{sessionId}`, `GET /-/v1/search`
- **fast-npm-meta** (`/fast/`): `GET /resolve/{pkg}`, `GET /versions/{pkg}`, `GET /full/{pkg}`
- **API** (`/api/`): `GET /auth/cas/callback/session/{sessionId}`, `GET /downloads/point/{*rest}`, `GET /downloads/range/{*rest}`
- **Misc**: `GET /-/ping`

## S3 Paths

```
packages/{fullname}/
  abbreviated_manifests.json
  full_manifests.json
  {version}/
    abbreviated.json
    package.json
    {name}-{version}.tgz
```

## Search (optional)

Meilisearch integration, enabled when `[search] meiliUrl` is non-empty. `SearchIndex` in `AppState.search` is `Option`; when `None`, search routes/index writes are skipped. Config (`src/config.rs`): `meiliUrl`, `meiliKey`, `indexName` (default `packages`). The worker upserts/deletes documents on every package sync; `proxide reindex-search` does a full rebuild from DB (batches of 500, pulls local + upstream download counts for ranking).

## E2E Tests

End-to-end tests in `e2e/` using Vitest. The global setup (`e2e/globalSetup.ts`):
1. Starts Docker Compose (MariaDB + RustFS)
2. Creates `proxide_e2e` database and S3 bucket
3. Builds and starts proxide server with `e2e/proxide.e2e.toml`
4. Waits for `/-/ping` to respond
5. Clears the Meilisearch index (`proxide-e2e`)

Run: `pnpm test:e2e`

## Code Style

- No comments unless requested
- `.rs`: 4-space indent; everything else: 2-space (`.editorconfig`)
- `sqlx::query!` macros only
- Rust edition 2024, toolchain 1.95.0
- `async_fn_in_trait` lint explicitly allowed (`Cargo.toml` `[lints.rust]`)
- TypeScript/JavaScript: use static `import … from …` at the top of the file, never dynamic `await import(...)`

## Auth & CAS Login

When `auth.casUrl` is set, CAS 2.0 SSO login is activated. Auth is implicitly enabled when `casUrl` or `allowScopes` is configured (no explicit `enabled` flag exists):

1. `POST /npm/-/v1/login` — creates an in-memory `LoginSession`, returns `{ loginUrl, doneUrl }` pointing to CAS
2. npm CLI opens `loginUrl` in browser → user authenticates at CAS → CAS redirects back to `GET /api/auth/cas/callback/session/{sessionId}?ticket=...`
3. Server validates ticket via CAS `/cas/serviceValidate`, parses XML (`<cas:loginid>` or `<cas:user>`), upserts user + creates token
4. `GET /npm/-/v1/login/done/session/{sessionId}` — npm polls: `202` with `retry-after: 5` while pending, `200 { token }` when done (session then deleted)

Legacy `PUT /-/user/org.couchdb.user:{name}` is disabled when CAS is active. Sessions expire after 5 minutes.

Additional auth config: `casUrl` (CAS SSO endpoint, enables CAS login when non-empty), `allowScopes` (scopes allowed to publish, enables publish auth when non-empty), `allowPublishNonScopePackage` (allow unscoped package publish), `admins` (admin user list).

## Build Troubleshooting

- If `cargo check` / `cargo sqlx prepare` fails with database connection errors, **stop and ask the user to fix** — do not attempt workarounds
- Use `cargo check` to verify compilation
