# AGENTS.md

## Project Overview

NPM registry mirror written in Rust. Three clap subcommands: `proxide server` (HTTP API), `proxide worker` (sync engine), `proxide cleanup-s3` (orphan S3 object remover).

## Commands

```bash
cargo run -- server             # HTTP server (reads proxide.toml from CWD)
cargo run -- worker             # Sync worker
cargo run -- cleanup-s3         # Remove orphan S3 objects
sqlx migrate run                # Apply migrations (needs DATABASE_URL in .env)
sqlx migrate revert             # Revert last migration
cargo sqlx prepare              # Refresh .sqlx/ offline cache (required after schema changes)
just init                       # Install cargo-release, git-cliff, sqlx-cli
just up                         # Alias for sqlx migrate run
just pre-release <version>      # Generate CHANGELOG + stage it
```

## Build & sqlx

- Compile-time checked queries only: `sqlx::query!` / `sqlx::query_as!` — never `sqlx::query()`
- Macro expansion needs `DATABASE_URL` in `.env` (pointing at a running MySQL)
- After any schema change, run `cargo sqlx prepare` to regenerate `.sqlx/` JSON files
- **Never manually edit or delete files in `.sqlx/`** — always use `cargo sqlx prepare` to regenerate

## Database

- MySQL 8.4 (local, credentials from `.env`)
- Tables: `dists`, `packages`, `package_versions`, `package_tags`, `change_stream_cursors`, `sync_tasks`, `users`, `tokens`, `maintainers`
- Migrations in `/migrations`, managed by sqlx-cli

## Configuration

`proxide.toml` loaded from CWD. Keys are **camelCase** (serde `rename_all = "camelCase"`), not snake_case. Required sections: `database`, `server`, `storage` (Local or S3), `log`, `worker`. Optional section: `auth`. See `src/config.rs` for full schema and defaults.

## Architecture

```
src/
  main.rs              # clap → server | worker | cleanup-s3
  lib.rs               # Module registration
  config.rs            # TOML config (camelCase keys)
  error.rs             # WebError → JSON responses
  state.rs             # AppState { repo, config, http, package_lock }

  npm/types.rs         # Packument, AbbreviatedPackument, FastMeta* types
  npm/mod.rs           # split_scope_name, decode_fullname

  storage/s3.rs        # object_store crate

  repository/mod.rs    # Row types + Repository trait (async_trait)
  repository/mysql.rs  # MysqlRepository — all DB operations

  middleware/
    mod.rs             # Middleware module
    auth.rs            # Auth middleware (token validation)

  handlers/
    registry.rs        # /npm/* package routes
    fast_meta.rs       # /fast/* fast-npm-meta routes
    tarball.rs         # /npm/{fullname}/-/{filename} (on-demand proxy)
    sync.rs            # PUT /npm/-/package/{fullname}/syncs
    auth.rs            # PUT /npm/-/user/{name} (legacy login, disabled when CAS enabled)
    cas.rs             # CAS 2.0 + npm web v1 login flow
    publish.rs         # PUT /npm/{fullname} (auth-protected)
    home.rs            # GET /-/ping

  worker/
    changes_poller.rs  # Poll upstream _changes → enqueue sync_tasks
    task_consumer.rs   # Claim & execute tasks
    sync_package.rs    # Fetch upstream packument → diff → write DB + S3
    cleanup.rs         # Requeue stale tasks, purge old tasks
    cleanup_s3.rs      # Remove orphan dists/S3 objects
```

## Worker

Three concurrent loops in `run_worker`:
1. **changes_poller** — polls upstream `_changes` feed, enqueues `sync_tasks` rows
2. **task_consumer** (N = `worker.consumer_count`) — claims pending tasks, runs `sync_package`, marks done/failed
3. **cleanup_scheduler** — periodically requeues stale tasks (`task_timeout_secs`) and purges old tasks (`task_retention_days`)

## Routes

- **NPM** (`/npm/`): `GET /`, `GET /{fullname}`, `GET /{fullname}/{version}`, `GET /{fullname}/-/{filename}`, `PUT /{fullname}`, `PUT /-/package/{fullname}/syncs`, `PUT /-/user/{name}`, `POST /-/v1/login`, `GET /-/v1/login/request/session/{sessionId}`, `GET /-/v1/login/done/session/{sessionId}`
- **fast-npm-meta** (`/fast/`): `GET /resolve/{pkg}`, `GET /versions/{pkg}`, `GET /full/{pkg}`
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

## Code Style

- No comments unless requested
- `.rs`: 4-space indent; everything else: 2-space (`.editorconfig`)
- `sqlx::query!` macros only
- Rust edition 2024, toolchain 1.95.0
- `async_fn_in_trait` lint explicitly allowed (`Cargo.toml` `[lints.rust]`)

## Auth & CAS Login

When `auth.casUrl` is set, CAS 2.0 SSO login is activated. Auth is implicitly enabled when `casUrl` or `allowScopes` is configured (no explicit `enabled` flag exists):

1. `POST /npm/-/v1/login` — creates a `login_sessions` row, returns `{ loginUrl, doneUrl }` pointing to CAS
2. npm CLI opens `loginUrl` in browser → user authenticates at CAS → CAS redirects back to `GET /npm/-/v1/login/request/session/{sessionId}?ticket=...`
3. Server validates ticket via CAS `/cas/serviceValidate`, parses XML (`<cas:loginid>` or `<cas:user>`), upserts user + creates token
4. `GET /npm/-/v1/login/done/session/{sessionId}` — npm polls: `202` with `retry-after: 5` while pending, `200 { token }` when done (session then deleted)

Legacy `PUT /-/user/{name}` is disabled when CAS is active. Sessions expire after 5 minutes.

Additional auth config: `casUrl` (CAS SSO endpoint, enables CAS login when non-empty), `allowScopes` (scopes allowed to publish, enables publish auth when non-empty), `allowPublishNonScopePackage` (allow unscoped package publish), `admins` (admin user list).

## Build Troubleshooting

- If `cargo check` / `cargo sqlx prepare` fails with database connection errors, **stop and ask the user to fix** — do not attempt workarounds
- Use `cargo check` to verify compilation
