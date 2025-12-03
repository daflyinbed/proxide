# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Proxide is a Rust web service for managing binaries. It's built with Axum framework, uses MySQL for persistence, and provides a RESTful API with OpenAPI documentation.

## Architecture

The codebase follows a modular architecture with clear separation of concerns:

- **`src/main.rs`** - Application entry point that:
  - Loads configuration from `proxide.toml`
  - Sets up tracing (fastrace) and logging (logforth)
  - Binds to TCP listener and starts the Axum server
  - Handles graceful shutdown (Ctrl+C and SIGTERM)

- **`src/routes.rs`** - Router configuration:
  - Uses `utoipa-axum` for OpenAPI integration
  - Mounts API documentation at `/-/binary/scalar`
  - Registers routes and splits into parts for serving

- **`src/handlers/`** - HTTP request handlers:
  - Currently contains `home.rs` with `ping` endpoint
  - Handlers extract state and return JSON responses
  - Uses `#[debug_handler]` and `#[utoipa::path]` attributes

- **`src/state.rs`** - Application state management:
  - Holds `AppState` with database repository
  - Initializes repository on startup

- **`src/repository/`** - Data access layer:
  - Trait-based design (`Repository` trait)
  - `MysqlRepository` implements the trait
  - Uses sqlx with connection pooling

- **`src/config.rs`** - Configuration management:
  - TOML-based configuration via `proxide.toml`
  - Includes database, server, storage, and logging config
  - Supports Local and S3 storage backends

## Database

**Technology:** MySQL 8.4.7

**Schema:** The database includes four main tables:
- `binaries` - Stores binary metadata (category, parent, name, size, date)
- `tasks` - Task queue with state management
- `history_tasks` - Historical task records with attempt tracking
- `caches` - Cache information linking to binaries

**Migrations:** Located in `/migrations` folder, managed by sqlx CLI

## Common Development Commands

```bash
# List available commands
just

# Install development tools (cargo-release, git-cliff, sqlx-cli)
just init

# Run database migrations
just up

# Generate changelog for release
just pre-release version=<version>

# Start the application
cargo run

# Build the application
cargo build

# Run with release profile
cargo run --release

# Run database migrations manually
sqlx migrate run

# Revert last migration
sqlx migrate revert

# Check database connection
sqlx database check
```

**Note:** A `proxide.toml` configuration file must exist before running the application. The config structure is defined in `src/config.rs`.

## API Documentation

OpenAPI/Swagger documentation is available at: `/-/binary/scalar`

## Environment

**Database Configuration:** See `.env` file for MySQL connection details

**Required Configuration File:** `proxide.toml` (not included in repo)

## Code Style

- Editor config: See `.editorconfig`
- Rust formatting: 4 spaces indentation for `.rs` files
- Git ignore: See `.gitignore` (excludes `/target`)

## Dependencies

Key dependencies include:
- `axum` - Web framework
- `sqlx` - Async SQL toolkit with MySQL support
- `utoipa` & `utoipa-axum` - OpenAPI generation
- `fastrace` & `fastrace-tracing` - Distributed tracing
- `logforth` - Structured logging
- `tower-http` - HTTP middleware (cors, compression, timeout, trace)
