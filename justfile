_default:
  @just --list -u

init:
    cargo binstall cargo-release git-cliff
    cargo install sqlx-cli

up:
    sqlx migrate run

pre-release version:
    git cliff -o CHANGELOG.md --tag {{version}} && git add CHANGELOG.md