import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { createServer, type Server } from "node:http";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, openSync } from "node:fs";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import {
  CreateBucketCommand,
  DeleteObjectCommand,
  HeadObjectCommand,
  ListObjectsV2Command,
  S3Client,
} from "@aws-sdk/client-s3";
import { TARBALL_BYTES } from "../fixtures/tarball.js";
import { uniqueName } from "../helpers.js";


const PROJECT_ROOT = resolve(import.meta.dirname, "../..");
const BINARY = join(PROJECT_ROOT, "target", "debug", "proxide");
const RUN_DIR = join(PROJECT_ROOT, "target", "e2e-run-cleanup-storage");
const DB_NAME = "proxide_e2e_cleanup_storage";
const BUCKET = "proxide-e2e-cleanup-storage";
const BASE_URL = "http://localhost:14874";

const PACKAGE_NAME = uniqueName("e2e-cleanup");
const VERSION_1 = "1.0.0";
const VERSION_2 = "2.0.0";
const FILENAME_1 = `${PACKAGE_NAME}-${VERSION_1}.tgz`;
const FILENAME_2 = `${PACKAGE_NAME}-${VERSION_2}.tgz`;

let upstreamServer: Server;
let workerChild: ChildProcess | undefined;
let serverChild: ChildProcess | undefined;
let upstreamPort = 0;

const s3 = new S3Client({
  endpoint: "http://127.0.0.1:9000",
  region: "us-east-1",
  credentials: { accessKeyId: "proxide", secretAccessKey: "proxide123" },
  forcePathStyle: true,
});

async function apiJsonLocal<T = any>(
  path: string,
  opts: RequestInit = {},
): Promise<{ res: Response; body: T }> {
  const url = path.startsWith("http://") || path.startsWith("https://")
    ? path
    : `${BASE_URL}${path}`;
  const res = await fetch(url, opts);
  const body = (await res.json()) as T;
  return { res, body };
}

function runMysql(sql: string): string {
  return execFileSync(
    "docker",
    ["exec", "proxide-mysql", "mysql", "-uroot", "-proot", DB_NAME, "-Nse", sql],
    { encoding: "utf8" },
  ).trim();
}

function runMysqlRoot(sql: string): void {
  execFileSync("docker", ["exec", "proxide-mysql", "mysql", "-uroot", "-proot", "-e", sql], {
    stdio: "pipe",
  });
}

async function waitForWorkerReady(): Promise<void> {
  const namespace = createHash("sha256").update(DB_NAME).digest("hex").slice(0, 16);
  await waitForCondition(async () => {
    try {
      return runMysql(`SELECT IS_USED_LOCK('proxide:${namespace}:worker-writer') IS NOT NULL`) === "1";
    } catch {
      return false;
    }
  }, 30_000);
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
      const { port } = address;
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

async function waitFor(url: string, timeoutMs = 60_000) {
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

async function waitForCondition(check: () => Promise<boolean>, timeoutMs = 30_000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await check()) return;
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error("timed out waiting for condition");
}

async function ensureBucket() {
  try {
    await s3.send(new CreateBucketCommand({ Bucket: BUCKET }));
  } catch (err: any) {
    if (err.Name !== "BucketAlreadyOwnedByYou" && err.Name !== "BucketAlreadyExists") throw err;
  }
}

async function resetS3(prefix: string) {
  let continuationToken: string | undefined;
  do {
    const list = await s3.send(
      new ListObjectsV2Command({
        Bucket: BUCKET,
        Prefix: prefix,
        ContinuationToken: continuationToken,
      }),
    );
    for (const obj of list.Contents ?? []) {
      if (obj.Key) {
        await s3.send(new DeleteObjectCommand({ Bucket: BUCKET, Key: obj.Key }));
      }
    }
    continuationToken = list.NextContinuationToken;
  } while (continuationToken);
}

function buildPackument(versions: string[]) {
  const versionsObj: Record<string, any> = {};
  const timeObj: Record<string, string> = {
    modified: "2024-01-01T00:00:00.000Z",
  };
  for (const version of versions) {
    const filename = `${PACKAGE_NAME}-${version}.tgz`;
    versionsObj[version] = {
      name: PACKAGE_NAME,
      version,
      dist: {
        tarball: `http://127.0.0.1:${upstreamPort}/${PACKAGE_NAME}/-/${filename}`,
      },
    };
    timeObj[version] = "2024-01-01T00:00:00.000Z";
  }
  return JSON.stringify({
    name: PACKAGE_NAME,
    "dist-tags": { latest: versions[versions.length - 1] },
    versions: versionsObj,
    time: timeObj,
  });
}

function createUpstreamServer(versions: string[]): Server {
  const packument = buildPackument(versions);
  return createServer((req, res) => {
    if (req.method === "GET" && req.url?.startsWith("/_changes")) {
      res.statusCode = 200;
      res.setHeader("content-type", "application/json");
      res.end(JSON.stringify({ results: [], last_seq: 0 }));
      return;
    }
    if (req.method === "GET" && req.url === `/${PACKAGE_NAME}`) {
      res.statusCode = 200;
      res.setHeader("content-type", "application/json");
      res.end(packument);
      return;
    }
    if (req.method === "GET" && req.url?.startsWith(`/${PACKAGE_NAME}/-/`)) {
      res.statusCode = 200;
      res.setHeader("content-type", "application/octet-stream");
      res.setHeader("content-length", String(TARBALL_BYTES.length));
      res.end(TARBALL_BYTES);
      return;
    }
    res.statusCode = 404;
    res.end("not found");
  });
}

beforeAll(async () => {
  upstreamPort = await getFreePort();

  if (!existsSync(BINARY)) {
    execFileSync("cargo", ["build"], { cwd: PROJECT_ROOT, stdio: "inherit" });
  }

  runMysqlRoot(`DROP DATABASE IF EXISTS ${DB_NAME}`);
  runMysqlRoot(`CREATE DATABASE ${DB_NAME}`);
  await ensureBucket();
  await resetS3("");

  upstreamServer = createUpstreamServer([VERSION_1, VERSION_2]);
  await new Promise<void>((resolve, reject) => {
    upstreamServer.once("error", reject);
    upstreamServer.listen(upstreamPort, "127.0.0.1", () => resolve());
  });

  await rm(RUN_DIR, { recursive: true, force: true });
  await mkdir(RUN_DIR, { recursive: true });

  const workerConfig =
    `[database]\nuri = "mysql://root:root@127.0.0.1:3306/${DB_NAME}"\n\n` +
    `[server]\nport = 14874\nrootUrl = "${BASE_URL}"\n\n` +
    `[storage.S3]\nendpoint = "http://127.0.0.1:9000"\naccessKeyId = "proxide"\nsecretAccessKey = "proxide123"\nbucketName = "${BUCKET}"\ncompressJson = true\nzstdLevel = 3\n\n` +
    `[log]\nlevel = "info"\n\n` +
    `[worker]\nupstreamRegistry = "http://127.0.0.1:${upstreamPort}"\nchangesStreamUrl = "http://127.0.0.1:${upstreamPort}/_changes"\npollerEnabled = false\nconsumerCount = 1\nconsumerPollIntervalMs = 100\ncronIntervalSecs = 3600\n\n` +
    `[storageGc]\nstartupEnabled = false\nmaxDurationSecs = 0\nminAgeSecs = 0\n`;

  await writeFile(join(RUN_DIR, "proxide.toml"), workerConfig);

  const logFd = openSync(join(RUN_DIR, "proxide-server.log"), "w");
  serverChild = spawn(BINARY, ["server"], {
    cwd: RUN_DIR,
    stdio: ["ignore", logFd, logFd],
    env: { ...process.env, RUST_LOG: "info" },
  });

  await waitFor(`${BASE_URL}/-/ping`);

  workerChild = spawn(BINARY, ["worker"], {
    cwd: RUN_DIR,
    stdio: ["ignore", "inherit", "inherit"],
  });

  await waitForWorkerReady();
});

async function stopWriters(): Promise<void> {
  if (workerChild && workerChild.exitCode === null && !workerChild.killed) {
    workerChild.kill("SIGTERM");
    await new Promise<void>((resolve) => {
      workerChild!.once("exit", () => resolve());
    });
  }
  if (serverChild && serverChild.exitCode === null && !serverChild.killed) {
    serverChild.kill("SIGTERM");
    await new Promise<void>((resolve) => {
      serverChild!.once("exit", () => resolve());
    });
  }
}

afterAll(async () => {
  await stopWriters();
  await new Promise<void>((resolve) => {
    if (!upstreamServer.listening) {
      resolve();
      return;
    }
    upstreamServer.close(() => resolve());
  });
});

async function runSyncTask(): Promise<void> {
  const { res, body } = await apiJsonLocal(
    "/npm/-/package/" + encodeURIComponent(PACKAGE_NAME) + "/syncs",
    { method: "PUT" },
  );
  expect(res.status).toBe(200);
  expect(body.ok).toBe(true);

  await waitForCondition(async () => {
    const status = runMysql(
      `SELECT status FROM sync_tasks WHERE name = '${PACKAGE_NAME}' ORDER BY id DESC LIMIT 1`,
    );
    return status === "done";
  }, 30_000);
}

async function assertS3ObjectExists(path: string, shouldExist: boolean): Promise<void> {
  let exists = true;
  try {
    await s3.send(new HeadObjectCommand({ Bucket: BUCKET, Key: path }));
  } catch (error: any) {
    if (error?.$metadata?.httpStatusCode === 404 || error?.name === "NotFound") {
      exists = false;
    } else {
      throw error;
    }
  }
  expect(exists).toBe(shouldExist);
}

function tarDistPath(version: string): string {
  return runMysql(
    `SELECT d.path FROM package_versions pv JOIN packages p ON p.id = pv.package_id JOIN dists d ON d.id = pv.tar_dist_id WHERE p.name = '${PACKAGE_NAME}' AND pv.version = '${version}'`,
  );
}

async function touchJsdelivrFiles(version: string): Promise<void> {
  const res = await fetch(`${BASE_URL}/jsdelivr/npm/${PACKAGE_NAME}@${version}/package.json`);
  if (!res.ok) {
    throw new Error(`touchJsdelivrFiles failed for ${version}: ${res.status}`);
  }
}

describe.sequential("cleanup-storage reclaims orphan CAS objects", () => {
  it(
    "keeps an unreferenced object while writers are running",
    async () => {
      await runSyncTask();

      await touchJsdelivrFiles(VERSION_1);
      await touchJsdelivrFiles(VERSION_2);

      const version1Path = tarDistPath(VERSION_1);
      const version2Path = tarDistPath(VERSION_2);
      expect(version1Path).toMatch(/^objects\/raw\/sha256\//);
      expect(version2Path).toMatch(/^objects\/raw\/sha256\//);
      await assertS3ObjectExists(version1Path, true);
      await assertS3ObjectExists(version2Path, true);

      upstreamServer.close();
      await new Promise<void>((resolve) => upstreamServer.once("close", resolve));
      upstreamServer = createUpstreamServer([VERSION_2]);
      await new Promise<void>((resolve) => {
        upstreamServer.listen(upstreamPort, "127.0.0.1", () => resolve());
      });

      await runSyncTask();

      await waitForCondition(async () => {
        const count = Number(
          runMysql(
            `SELECT COUNT(*) FROM package_versions WHERE package_id = (SELECT id FROM packages WHERE name = '${PACKAGE_NAME}')`,
          ),
        );
        return count === 1;
      }, 30_000);

      await assertS3ObjectExists(version1Path, true);
      await assertS3ObjectExists(version2Path, true);

      const res = await fetch(`${BASE_URL}/jsdelivr/npm/${PACKAGE_NAME}@${VERSION_2}/package.json`);
      expect(res.status).toBe(200);

      const orphanFileCount = Number(
        runMysql(
          `SELECT COUNT(*) FROM dists d WHERE NOT EXISTS (SELECT 1 FROM packages p WHERE p.abbreviated_dist_id = d.id) AND NOT EXISTS (SELECT 1 FROM packages p WHERE p.full_dist_id = d.id) AND NOT EXISTS (SELECT 1 FROM package_versions pv WHERE pv.tar_dist_id = d.id) AND NOT EXISTS (SELECT 1 FROM package_versions pv WHERE pv.readme_dist_id = d.id)`,
        ),
      );
      expect(orphanFileCount).toBeGreaterThan(0);

      const manifestPath = runMysql(`
        SELECT d.path FROM packages p
        JOIN dists d ON d.id = p.full_dist_id
        WHERE p.name = '${PACKAGE_NAME}'
      `);
      await s3.send(new DeleteObjectCommand({ Bucket: BUCKET, Key: manifestPath }));
      const missingManifest = await fetch(`${BASE_URL}/npm/${PACKAGE_NAME}`);
      expect(missingManifest.status).toBe(503);
    },
    120_000,
  );

  it(
    "cleanup-storage command deletes orphan dists",
    async () => {
      await runSyncTask();

      await touchJsdelivrFiles(VERSION_2);
      const paths = runMysql("SELECT path FROM dists ORDER BY id").split("\n").filter(Boolean);
      expect(paths.length).toBeGreaterThan(0);

      await stopWriters();
      runMysql(`DELETE FROM package_versions WHERE package_id = (SELECT id FROM packages WHERE name = '${PACKAGE_NAME}');`);
      runMysql(`DELETE FROM packages WHERE name = '${PACKAGE_NAME}';`);
      runMysql("UPDATE dists SET created_at = DATE_SUB(NOW(), INTERVAL 1 SECOND)");

      const cleanupChild = spawn(BINARY, ["cleanup-storage"], {
        cwd: RUN_DIR,
        stdio: ["ignore", "pipe", "inherit"],
      });

      let cleanupOutput = "";
      cleanupChild.stdout?.on("data", (data: Buffer) => {
        cleanupOutput += data.toString();
      });

      await new Promise<void>((resolve, reject) => {
        cleanupChild.once("exit", (code) => {
          if (code === 0) resolve();
          else reject(new Error(`cleanup-storage exited with code ${code}`));
        });
      });

      expect(cleanupOutput).toContain("Deleted");

      for (const path of paths) {
        await assertS3ObjectExists(path, false);
      }
      expect(Number(runMysql("SELECT COUNT(*) FROM dists"))).toBe(0);
    },
    120_000,
  );
});
