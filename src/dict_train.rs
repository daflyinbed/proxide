use crate::config::Config;
use crate::npm::types::Packument;
use crate::npm::{build_abbreviated_manifest, build_abbreviated_version};
use anyhow::{Context, Result, bail};
use futures::StreamExt;
use std::io::Write;

const DEFAULT_DICT_SIZE: usize = 110 * 1024;
const MAX_VERSIONS_PER_PACKAGE: usize = 30;
const SAMPLE_CHUNK_SIZE: usize = 16 * 1024;
const FETCH_CONCURRENCY: usize = 8;
const MIN_FETCHED_PACKAGES: usize = 20;
pub fn default_dict_size() -> usize {
    DEFAULT_DICT_SIZE
}

const TRAINING_PACKAGES: &[&str] = &[
    "lodash",
    "express",
    "react",
    "react-dom",
    "vue",
    "svelte",
    "angular",
    "jquery",
    "axios",
    "got",
    "node-fetch",
    "superagent",
    "ws",
    "typescript",
    "rxjs",
    "moment",
    "dayjs",
    "date-fns",
    "uuid",
    "chalk",
    "ansi-styles",
    "color-convert",
    "commander",
    "yargs",
    "yargs-parser",
    "minimist",
    "inquirer",
    "ora",
    "debug",
    "ms",
    "semver",
    "eslint",
    "prettier",
    "stylelint",
    "mocha",
    "jest",
    "vitest",
    "ava",
    "tap",
    "jasmine",
    "cypress",
    "nodemon",
    "pm2",
    "npm",
    "cross-env",
    "dotenv",
    "webpack",
    "webpack-cli",
    "rollup",
    "vite",
    "esbuild",
    "gulp",
    "browserify",
    "parcel",
    "babel-loader",
    "css-loader",
    "style-loader",
    "file-loader",
    "terser",
    "cssnano",
    "autoprefixer",
    "postcss",
    "sass",
    "less",
    "next",
    "nuxt",
    "gatsby",
    "astro",
    "remix",
    "react-router",
    "redux",
    "mobx",
    "zustand",
    "immer",
    "ramda",
    "lodash-es",
    "left-pad",
    "is-promise",
    "deep-equal",
    "mkdirp",
    "rimraf",
    "glob",
    "minimatch",
    "body-parser",
    "cookie-parser",
    "morgan",
    "cors",
    "helmet",
    "multer",
    "fastify",
    "koa",
    "hapi",
    "socket.io",
    "knex",
    "mongoose",
    "sequelize",
    "typeorm",
    "ioredis",
    "redis",
    "mongodb",
    "mysql",
    "mysql2",
    "pg",
    "sqlite3",
    "d3",
    "three",
    "chart.js",
    "highcharts",
    "echarts",
    "leaflet",
    "gsap",
    "winston",
    "pino",
    "sharp",
    "svgo",
    "imagemin",
    "jsonwebtoken",
    "bcrypt",
    "crypto-js",
    "passport",
    "styled-components",
    "@angular/core",
    "@angular/cli",
    "@angular/material",
    "@babel/core",
    "@babel/preset-env",
    "@babel/plugin-transform-runtime",
    "@types/node",
    "@types/react",
    "@types/lodash",
    "@types/express",
    "@vue/cli-service",
    "@vue/compiler-sfc",
    "@eslint/js",
    "@typescript-eslint/parser",
    "@typescript-eslint/eslint-plugin",
    "@testing-library/react",
    "@testing-library/jest-dom",
    "@storybook/react",
    "@storybook/addon-essentials",
    "@nestjs/core",
    "@nestjs/common",
    "@nestjs/platform-express",
    "@grpc/grpc-js",
    "@grpc/proto-loader",
    "@opentelemetry/api",
    "@opentelemetry/sdk-node",
    "@aws-sdk/client-s3",
    "@aws-sdk/client-ec2",
    "@azure/storage-blob",
    "@google-cloud/storage",
    "@mui/material",
    "@ant-design/icons",
    "@docusaurus/core",
    "@vitejs/plugin-react",
    "@sveltejs/kit",
    "@rollup/plugin-commonjs",
    "@rollup/plugin-node-resolve",
    "@swc/core",
    "@reduxjs/toolkit",
    "@commitlint/cli",
    "@commitlint/config-conventional",
    "@jest/globals",
    "@octokit/rest",
    "@octokit/auth-token",
    "@emotion/react",
    "@tailwindcss/postcss",
    "@tailwindcss/vite",
    "@biomejs/biome",
    "@playwright/test",
    "@prisma/client",
    "@tanstack/react-query",
    "@trpc/server",
    "@hono/node-server",
    "@elastic/elasticsearch",
    "@sendgrid/mail",
    "@sentry/node",
    "@sentry/react",
    "@apollo/server",
    "@graphql-tools/schema",
    "graphql",
    "hono",
];

pub async fn train_zstd_dict(
    config: &Config,
    client: &reqwest::Client,
    output: &str,
    max_dict_size: usize,
) -> Result<()> {
    log::info!(
        action = "dict_train_start";
        "fetching {} packuments from {}",
        TRAINING_PACKAGES.len(),
        config.worker.upstream_registry,
    );

    let packuments = futures::stream::iter(TRAINING_PACKAGES.iter())
        .map(|name| fetch_packument(config, client, name))
        .buffer_unordered(FETCH_CONCURRENCY)
        .filter_map(|res| async {
            match res {
                Ok(p) => {
                    log::info!(action = "dict_train_fetched"; "name={}", p.name);
                    Some(p)
                }
                Err(e) => {
                    log::warn!(action = "dict_train_fetch_error"; "{e:#}");
                    None
                }
            }
        })
        .collect::<Vec<_>>()
        .await;

    if packuments.len() < MIN_FETCHED_PACKAGES {
        bail!(
            "only {} packuments fetched, need at least {MIN_FETCHED_PACKAGES} to train",
            packuments.len()
        );
    }

    let mut samples: Vec<Vec<u8>> = Vec::new();
    for mut packument in packuments {
        collect_samples(&mut packument, &mut samples);
    }

    log::info!(
        action = "dict_train_samples";
        "collected {} samples ({} bytes)",
        samples.len(),
        samples.iter().map(|s| s.len()).sum::<usize>(),
    );

    let dict = zstd::dict::from_samples(&samples, max_dict_size)
        .context("zstd dictionary training failed")?;

    write_dictionary(output, &dict)?;

    log::info!(
        action = "dict_train_done";
        "trained {} byte dictionary from {} samples, written to {output}",
        dict.len(),
        samples.len(),
    );

    Ok(())
}

fn write_dictionary(output: &str, dict: &[u8]) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .with_context(|| format!("failed to create new dictionary at {output}"))?;
    file.write_all(dict)
        .with_context(|| format!("failed to write dictionary to {output}"))?;
    Ok(())
}

fn collect_samples(packument: &mut Packument, samples: &mut Vec<Vec<u8>>) {
    for ver_data in packument.versions.values().take(MAX_VERSIONS_PER_PACKAGE) {
        push_sample(samples, build_abbreviated_version(&ver_data.name, ver_data));
        push_sample(samples, serde_json::to_vec(ver_data).unwrap_or_default());
    }
    let abbreviated = build_abbreviated_manifest(packument);
    push_sample(
        samples,
        serde_json::to_vec(&abbreviated).unwrap_or_default(),
    );
    packument.readme = Some(String::new());
    push_sample(samples, serde_json::to_vec(packument).unwrap_or_default());
}

fn push_sample(samples: &mut Vec<Vec<u8>>, bytes: Vec<u8>) {
    if bytes.len() > SAMPLE_CHUNK_SIZE {
        for chunk in bytes.chunks(SAMPLE_CHUNK_SIZE) {
            samples.push(chunk.to_vec());
        }
    } else {
        samples.push(bytes);
    }
}

async fn fetch_packument(
    config: &Config,
    client: &reqwest::Client,
    fullname: &str,
) -> Result<Packument> {
    let url = format!("{}/{fullname}", config.worker.upstream_registry);

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

    resp.json()
        .await
        .with_context(|| format!("failed to parse packument from {url}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_dictionary_refuses_to_overwrite() {
        let path =
            std::env::temp_dir().join(format!("proxide-dict-write-{}.bin", std::process::id()));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"original").unwrap();

        let err = write_dictionary(path.to_str().unwrap(), b"replacement").unwrap_err();
        assert!(format!("{err:#}").contains("failed to create new dictionary"));
        assert_eq!(std::fs::read(&path).unwrap(), b"original");

        let _ = std::fs::remove_file(path);
    }
}
