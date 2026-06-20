import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, openSync } from "node:fs";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import {
  CreateBucketCommand,
  S3Client,
} from "@aws-sdk/client-s3";

const PROJECT_ROOT = resolve(import.meta.dirname, "../..");
const BINARY = join(PROJECT_ROOT, "target", "debug", "proxide");
const RUN_DIR = join(PROJECT_ROOT, "target", "e2e-run-search-disabled");
const LOG_PATH = join(RUN_DIR, "proxide.log");

const DB_NAME = "proxide_e2e_search_disabled";
const BUCKET = "proxide-e2e-search-disabled";

let proxideChild: ChildProcess | undefined;
let proxidePort = 0;

const s3 = new S3Client({
  endpoint: "http://127.0.0.1:9000",
  region: "us-east-1",
  credentials: { accessKeyId: "proxide", secretAccessKey: "proxide123" },
  forcePathStyle: true,
});

function runMysqlRoot(sql: string): void {
  execFileSync("docker", ["exec", "proxide-mariadb", "mysql", "-uroot", "-proot", "-e", sql], {
    stdio: "pipe",
  });
}

async function getFreePort(): Promise<number> {
  return await new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        server.close(() => reject(new Error("failed to allocate port")));
        return;
      }
      const port = address.port;
      server.close((err) => {
        if (err) {
          reject(err);
          return;
        }
        resolve(port);
      });
    });
  });
}

async function ensureBucket(): Promise<void> {
  try {
    await s3.send(new CreateBucketCommand({ Bucket: BUCKET }));
  } catch (err: any) {
    if (err.Name !== "BucketAlreadyOwnedByYou" && err.Name !== "BucketAlreadyExists") {
      throw err;
    }
  }
}

async function waitFor(url: string, timeoutMs = 60_000): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(url);
      if (res.ok) return;
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`timed out waiting for ${url}`);
}

function proxideUrl(): string {
  return `http://localhost:${proxidePort}`;
}

beforeAll(async () => {
  if (!existsSync(BINARY)) {
    execFileSync("cargo", ["build"], { cwd: PROJECT_ROOT, stdio: "inherit" });
  }

  runMysqlRoot(`DROP DATABASE IF EXISTS ${DB_NAME}`);
  runMysqlRoot(`CREATE DATABASE ${DB_NAME}`);
  await ensureBucket();
  proxidePort = await getFreePort();

  await rm(RUN_DIR, { recursive: true, force: true });
  await mkdir(RUN_DIR, { recursive: true });

  const config = [
    `[database]`,
    `uri = "mysql://root:root@127.0.0.1:3306/${DB_NAME}"`,
    ``,
    `[server]`,
    `binding = "0.0.0.0"`,
    `port = ${proxidePort}`,
    `rootUrl = "${proxideUrl()}"`,
    ``,
    `[storage.S3]`,
    `endpoint = "http://127.0.0.1:9000"`,
    `accessKeyId = "proxide"`,
    `secretAccessKey = "proxide123"`,
    `bucketName = "${BUCKET}"`,
    ``,
    `[log]`,
    `level = "warn"`,
    ``,
    `[worker]`,
    `upstreamRegistry = "https://registry.npmjs.org"`,
    `consumerCount = 1`,
    `cronIntervalSecs = 3600`,
    ``,
    `[auth]`,
    `allowPublishNonScopePackage = true`,
    ``,
  ].join("\n");
  await writeFile(join(RUN_DIR, "proxide.toml"), config);

  const logFd = openSync(LOG_PATH, "w");
  proxideChild = spawn(BINARY, ["server"], {
    cwd: RUN_DIR,
    stdio: ["ignore", logFd, logFd],
    env: { ...process.env, RUST_LOG: "warn" },
  });

  await waitFor(`${proxideUrl()}/-/ping`);
}, 120_000);

afterAll(async () => {
  if (proxideChild && proxideChild.exitCode === null && !proxideChild.killed) {
    proxideChild.kill("SIGTERM");
    await new Promise<void>((resolve) => {
      proxideChild?.once("exit", () => resolve());
    });
  }
});

describe("search disabled (no [search] config)", () => {
  it("returns 501 when search is not configured", async () => {
    const res = await fetch(`${proxideUrl()}/npm/-/v1/search?text=foo`);
    expect(res.status).toBe(501);
    const body = await res.json();
    expect(body.error).toContain("search is not enabled");
  });
});
