import { execSync, spawn } from "node:child_process";
import { existsSync, openSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { copyFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { S3Client, CreateBucketCommand } from "@aws-sdk/client-s3";

const PROJECT_ROOT = resolve(import.meta.dirname, "..");
const E2E_DIR = import.meta.dirname;
const COMPOSE_FILE = join(PROJECT_ROOT, "docker-compose.yml");
const BINARY = join(PROJECT_ROOT, "target", "debug", "proxide");
const CONFIG_SRC = join(E2E_DIR, "proxide.e2e.toml");
const RUN_DIR = join(PROJECT_ROOT, "target", "e2e-run");

export const BASE_URL = "http://localhost:14873";

function log(msg: string) {
  console.log(`[e2e:setup] ${msg}`);
}

function run(cmd: string, opts?: { cwd?: string; env?: Record<string, string> }) {
  execSync(cmd, { stdio: "inherit", cwd: opts?.cwd, env: { ...process.env, ...opts?.env } });
}

async function waitFor(url: string, timeoutMs = 60_000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(url);
      if (res.ok) return;
    } catch {}
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`timed out waiting for ${url}`);
}

async function createBucket() {
  const client = new S3Client({
    endpoint: "http://127.0.0.1:9000",
    region: "us-east-1",
    credentials: { accessKeyId: "proxide", secretAccessKey: "proxide123" },
    forcePathStyle: true,
  });
  try {
    await client.send(new CreateBucketCommand({ Bucket: "proxide-e2e" }));
    log("S3 bucket created");
  } catch (err: any) {
    if (err.Name !== "BucketAlreadyOwnedByYou" && err.Name !== "BucketAlreadyExists") throw err;
    log("S3 bucket already exists");
  }
}

export default async function setup() {
  log("starting docker compose...");
  run(`docker compose -f ${COMPOSE_FILE} up -d --wait`);

  log("waiting for MariaDB...");
  for (let i = 0; i < 60; i++) {
    try {
      execSync(
        `docker exec proxide-mariadb mysql -uroot -proot -e "CREATE DATABASE IF NOT EXISTS proxide_e2e"`,
        { stdio: "pipe" },
      );
      break;
    } catch {
      await new Promise((r) => setTimeout(r, 1_000));
    }
  }

  log("creating S3 bucket...");
  await createBucket();

  log("building proxide...");
  if (!existsSync(BINARY)) {
    run("cargo build", { cwd: PROJECT_ROOT });
  }

  log("preparing run directory...");
  await mkdir(RUN_DIR, { recursive: true });

  const configDest = join(RUN_DIR, "proxide.toml");
  await copyFile(CONFIG_SRC, configDest);

  log("starting proxide server...");
  const logFd = openSync(join(RUN_DIR, "proxide.log"), "w");
  const child = spawn(BINARY, ["server"], {
    cwd: RUN_DIR,
    stdio: ["ignore", logFd, logFd],
    env: { ...process.env, RUST_LOG: "warn" },
  });

  log("waiting for proxide /-/ping...");
  await waitFor(`${BASE_URL}/-/ping`);
  log("proxide is ready");

  return async () => {
    log("stopping proxide...");
    if (child.exitCode === null && !child.killed) {
      child.kill("SIGTERM");
      await new Promise<void>((resolve) => {
        child.once("exit", () => resolve());
      });
    }
    log("done");
  };
}
