import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { existsSync, openSync } from "node:fs";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { CreateBucketCommand, S3Client } from "@aws-sdk/client-s3";
import { api, apiJson, login, publishPackage, uniqueName, uniqueScopedName } from "../helpers.js";

const PROJECT_ROOT = resolve(import.meta.dirname, "../..");
const BINARY = join(PROJECT_ROOT, "target", "debug", "proxide");

const RANGE_DB_NAME = "proxide_e2e_downloads_range";
const RANGE_BUCKET = "proxide-e2e-downloads-range";
const RANGE_RUN_DIR = join(PROJECT_ROOT, "target", "e2e-run-downloads-range");
const RANGE_LOG_PATH = join(RANGE_RUN_DIR, "proxide.log");
const RANGE_PACKAGE = "e2e-upstream-downloads-pkg";

const SHUTDOWN_DB_NAME = "proxide_e2e_downloads_shutdown";
const SHUTDOWN_BUCKET = "proxide-e2e-downloads-shutdown";
const SHUTDOWN_RUN_DIR = join(PROJECT_ROOT, "target", "e2e-run-downloads-shutdown");
const SHUTDOWN_CACHE_DIR = join(SHUTDOWN_RUN_DIR, "tarball-cache");
const SHUTDOWN_LOG_PATH = join(SHUTDOWN_RUN_DIR, "proxide.log");
const SHUTDOWN_PACKAGE = "e2e-download-flush-pkg";
const SHUTDOWN_VERSION = "1.0.0";
const SHUTDOWN_FILENAME = `${SHUTDOWN_PACKAGE}-${SHUTDOWN_VERSION}.tgz`;
const SHUTDOWN_TARBALL = Buffer.from("download-flush-e2e-tarball");

const s3 = new S3Client({
  endpoint: "http://127.0.0.1:9000",
  region: "us-east-1",
  credentials: { accessKeyId: "proxide", secretAccessKey: "proxide123" },
  forcePathStyle: true,
});

function runMysql(dbName: string, sql: string): string {
  return execFileSync(
    "docker",
    [
      "exec",
      "proxide-mariadb",
      "mysql",
      "-uroot",
      "-proot",
      dbName,
      "-Nse",
      sql,
    ],
    { encoding: "utf8" },
  ).trim();
}

function runMysqlRoot(sql: string): void {
  execFileSync("docker", ["exec", "proxide-mariadb", "mysql", "-uroot", "-proot", "-e", sql], {
    stdio: "pipe",
  });
}

async function ensureBucket(bucket: string) {
  try {
    await s3.send(new CreateBucketCommand({ Bucket: bucket }));
  } catch (err: any) {
    if (err.Name !== "BucketAlreadyOwnedByYou" && err.Name !== "BucketAlreadyExists") {
      throw err;
    }
  }
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
      if (res.ok) {
        return;
      }
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 200));
  }

  throw new Error(`timed out waiting for ${url}`);
}

async function waitForExit(child: ChildProcess, timeoutMs = 30_000) {
  if (child.exitCode !== null) {
    return;
  }

  await new Promise<void>((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error("timed out waiting for child exit")), timeoutMs);
    child.once("exit", () => {
      clearTimeout(timeout);
      resolve();
    });
  });
}

function isoDate(date: Date): string {
  return date.toISOString().slice(0, 10);
}

function previousMonthWindow() {
  const today = new Date();
  const year = today.getUTCFullYear();
  const month = today.getUTCMonth();
  const monthStart = new Date(Date.UTC(year, month - 1, 1));
  const monthEnd = new Date(Date.UTC(year, month, 0));
  const partialStart = new Date(Date.UTC(year, month - 1, 10));
  const partialEnd = new Date(Date.UTC(year, month - 1, 12));

  return {
    partialStart: isoDate(partialStart),
    partialEnd: isoDate(partialEnd),
    monthStart: isoDate(monthStart),
    monthEnd: isoDate(monthEnd),
    monthLength: monthEnd.getUTCDate(),
  };
}

function buildServerConfig(options: {
  dbName: string;
  bucket: string;
  port: number;
  upstreamPort: number;
  tarballCacheDir?: string;
}) {
  const cacheDirLine = options.tarballCacheDir
    ? `tarballCacheDir = "${options.tarballCacheDir}"\n`
    : "";

  return `[database]\nuri = "mysql://root:root@127.0.0.1:3306/${options.dbName}"\n\n[server]\nbinding = "0.0.0.0"\nport = ${options.port}\nrootUrl = "http://localhost:${options.port}"\n${cacheDirLine}\n[storage.S3]\nendpoint = "http://127.0.0.1:9000"\naccessKeyId = "proxide"\nsecretAccessKey = "proxide123"\nbucketName = "${options.bucket}"\n\n[log]\nlevel = "warn"\n\n[worker]\nupstreamRegistry = "http://127.0.0.1:${options.upstreamPort}"\nconsumerCount = 1\ncronIntervalSecs = 3600\n\n[auth]\nallowPublishNonScopePackage = true\n`;
}

describe("downloads endpoints", () => {
  it("returns seven days for last-week", async () => {
    const name = uniqueName("e2e-downloads-week");
    const token = await login(uniqueName("e2e-downloads-week-user"), "pass1234");

    await publishPackage(token, name, "1.0.0");

    const { res, body } = await apiJson(`/api/downloads/range/last-week/${name}`);
    expect(res.status).toBe(200);
    expect(body.package).toBe(name);
    expect(body.downloads).toHaveLength(7);

    const start = new Date(`${body.start}T00:00:00Z`);
    const end = new Date(`${body.end}T00:00:00Z`);
    const days = (end.getTime() - start.getTime()) / 86_400_000;
    expect(days).toBe(6);
  });

  it("supports scoped packages on downloads routes", async () => {
    const name = uniqueScopedName("e2e-scope", "e2e-downloads-scoped");
    const token = await login(uniqueName("e2e-downloads-scoped-user"), "pass1234");

    await publishPackage(token, name, "1.0.0");

    const { res, body } = await apiJson(`/api/downloads/point/last-week/${name}`);
    expect(res.status).toBe(200);
    expect(body.package).toBe(name);
    expect(typeof body.downloads).toBe("number");
  });
});

describe("downloads upstream cache coverage", () => {
  let upstreamServer: Server;
  let proxideChild: ChildProcess | undefined;
  let proxidePort = 0;
  let upstreamPort = 0;
  let upstreamRangeRequestCount = 0;

  function proxideUrl() {
    return `http://localhost:${proxidePort}`;
  }

  function handleRangeRequest(req: IncomingMessage, res: ServerResponse) {
    const url = new URL(req.url ?? "/", `http://127.0.0.1:${upstreamPort}`);
    const parts = url.pathname.split("/").filter(Boolean);
    if (req.method !== "GET" || parts.length !== 4 || parts[0] !== "downloads" || parts[1] !== "range") {
      res.statusCode = 404;
      res.end("not found");
      return;
    }

    const [startText, endText] = parts[2].split(":");
    const fullname = decodeURIComponent(parts[3]);
    if (fullname !== RANGE_PACKAGE) {
      res.statusCode = 404;
      res.end("not found");
      return;
    }

    upstreamRangeRequestCount += 1;

    const start = new Date(`${startText}T00:00:00Z`);
    const end = new Date(`${endText}T00:00:00Z`);
    const downloads: Array<{ day: string; downloads: number }> = [];
    for (let time = start.getTime(); time <= end.getTime(); time += 86_400_000) {
      downloads.push({
        day: new Date(time).toISOString().slice(0, 10),
        downloads: 1,
      });
    }

    res.statusCode = 200;
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify({ downloads }));
  }

  beforeAll(async () => {
    if (!existsSync(BINARY)) {
      execFileSync("cargo", ["build"], { cwd: PROJECT_ROOT, stdio: "inherit" });
    }

    runMysqlRoot(`DROP DATABASE IF EXISTS ${RANGE_DB_NAME}`);
    runMysqlRoot(`CREATE DATABASE ${RANGE_DB_NAME}`);
    await ensureBucket(RANGE_BUCKET);

    proxidePort = await getFreePort();
    upstreamPort = await getFreePort();

    upstreamServer = createServer((req, res) => handleRangeRequest(req, res));
    await new Promise<void>((resolve, reject) => {
      upstreamServer.once("error", reject);
      upstreamServer.listen(upstreamPort, "127.0.0.1", () => resolve());
    });

    await rm(RANGE_RUN_DIR, { recursive: true, force: true });
    await mkdir(RANGE_RUN_DIR, { recursive: true });
    await writeFile(
      join(RANGE_RUN_DIR, "proxide.toml"),
      buildServerConfig({
        dbName: RANGE_DB_NAME,
        bucket: RANGE_BUCKET,
        port: proxidePort,
        upstreamPort,
      }),
    );

    const logFd = openSync(RANGE_LOG_PATH, "w");
    proxideChild = spawn(BINARY, ["server"], {
      cwd: RANGE_RUN_DIR,
      stdio: ["ignore", logFd, logFd],
      env: { ...process.env, RUST_LOG: "warn" },
    });

    await waitFor(`${proxideUrl()}/-/ping`);
  });

  afterAll(async () => {
    if (proxideChild && proxideChild.exitCode === null && !proxideChild.killed) {
      proxideChild.kill("SIGTERM");
      await waitForExit(proxideChild);
    }

    await new Promise<void>((resolve) => {
      if (!upstreamServer.listening) {
        resolve();
        return;
      }
      upstreamServer.close(() => resolve());
    });
  });

  it("refetches wider explicit ranges after a partial month cache", async () => {
    const dates = previousMonthWindow();

    upstreamRangeRequestCount = 0;
    runMysql(
      RANGE_DB_NAME,
      `
        DELETE FROM upstream_package_downloads;
        DELETE FROM packages WHERE name = '${RANGE_PACKAGE}';
        INSERT INTO packages (name, scope, description, source)
        VALUES ('${RANGE_PACKAGE}', NULL, 'e2e downloads cache range test', 'npmjs');
      `,
    );

    const first = await fetch(
      `${proxideUrl()}/api/downloads/range/${dates.partialStart}:${dates.partialEnd}/${RANGE_PACKAGE}`,
    );
    expect(first.status).toBe(200);
    const firstBody = await first.json();
    expect(firstBody.downloads).toHaveLength(3);
    expect(firstBody.downloads.every((entry: { downloads: number }) => entry.downloads === 1)).toBe(true);
    expect(upstreamRangeRequestCount).toBe(1);

    const second = await fetch(
      `${proxideUrl()}/api/downloads/range/${dates.monthStart}:${dates.monthEnd}/${RANGE_PACKAGE}`,
    );
    expect(second.status).toBe(200);
    const secondBody = await second.json();
    expect(secondBody.downloads).toHaveLength(dates.monthLength);
    expect(secondBody.downloads.every((entry: { downloads: number }) => entry.downloads === 1)).toBe(true);
    expect(upstreamRangeRequestCount).toBe(2);
  });
});

describe("downloads shutdown flush", () => {
  let upstreamServer: Server;
  let proxideChild: ChildProcess | undefined;
  let proxidePort = 0;
  let upstreamPort = 0;

  function proxideUrl() {
    return `http://localhost:${proxidePort}`;
  }

  beforeAll(async () => {
    if (!existsSync(BINARY)) {
      execFileSync("cargo", ["build"], { cwd: PROJECT_ROOT, stdio: "inherit" });
    }

    runMysqlRoot(`DROP DATABASE IF EXISTS ${SHUTDOWN_DB_NAME}`);
    runMysqlRoot(`CREATE DATABASE ${SHUTDOWN_DB_NAME}`);
    await ensureBucket(SHUTDOWN_BUCKET);

    proxidePort = await getFreePort();
    upstreamPort = await getFreePort();

    upstreamServer = createServer((req, res) => {
      if (req.method === "GET" && req.url === `/${SHUTDOWN_PACKAGE}/-/${SHUTDOWN_FILENAME}`) {
        res.statusCode = 200;
        res.setHeader("content-type", "application/octet-stream");
        res.setHeader("content-length", String(SHUTDOWN_TARBALL.length));
        res.end(SHUTDOWN_TARBALL);
        return;
      }

      res.statusCode = 404;
      res.end("not found");
    });

    await new Promise<void>((resolve, reject) => {
      upstreamServer.once("error", reject);
      upstreamServer.listen(upstreamPort, "127.0.0.1", () => resolve());
    });

    await rm(SHUTDOWN_RUN_DIR, { recursive: true, force: true });
    await mkdir(SHUTDOWN_RUN_DIR, { recursive: true });
    await mkdir(SHUTDOWN_CACHE_DIR, { recursive: true });
    await writeFile(
      join(SHUTDOWN_RUN_DIR, "proxide.toml"),
      buildServerConfig({
        dbName: SHUTDOWN_DB_NAME,
        bucket: SHUTDOWN_BUCKET,
        port: proxidePort,
        upstreamPort,
        tarballCacheDir: SHUTDOWN_CACHE_DIR,
      }),
    );

    const logFd = openSync(SHUTDOWN_LOG_PATH, "w");
    proxideChild = spawn(BINARY, ["server"], {
      cwd: SHUTDOWN_RUN_DIR,
      stdio: ["ignore", logFd, logFd],
      env: { ...process.env, RUST_LOG: "warn" },
    });

    await waitFor(`${proxideUrl()}/-/ping`);
  });

  afterAll(async () => {
    if (proxideChild && proxideChild.exitCode === null && !proxideChild.killed) {
      proxideChild.kill("SIGTERM");
      await waitForExit(proxideChild);
    }

    await new Promise<void>((resolve) => {
      if (!upstreamServer.listening) {
        resolve();
        return;
      }
      upstreamServer.close(() => resolve());
    });
  });

  it("flushes in-memory download counters on graceful shutdown", async () => {
    runMysql(
      SHUTDOWN_DB_NAME,
      `
        DELETE FROM package_downloads;
        DELETE FROM package_versions WHERE package_id IN (
          SELECT id FROM packages WHERE name = '${SHUTDOWN_PACKAGE}'
        );
        DELETE FROM packages WHERE name = '${SHUTDOWN_PACKAGE}';
        INSERT INTO packages (name, scope, description, source)
        VALUES ('${SHUTDOWN_PACKAGE}', NULL, 'e2e shutdown flush test', 'npmjs');
        INSERT INTO package_versions (package_id, version, publish_time, is_pre_release, padding_version)
        VALUES (
          (SELECT id FROM packages WHERE name = '${SHUTDOWN_PACKAGE}'),
          '${SHUTDOWN_VERSION}',
          NOW(),
          0,
          '${SHUTDOWN_VERSION}'
        );
      `,
    );

    const tarballRes = await fetch(`${proxideUrl()}/npm/${SHUTDOWN_PACKAGE}/-/${SHUTDOWN_FILENAME}`);
    expect(tarballRes.status).toBe(200);
    expect(Buffer.from(await tarballRes.arrayBuffer()).equals(SHUTDOWN_TARBALL)).toBe(true);

    proxideChild!.kill("SIGTERM");
    await waitForExit(proxideChild!);

    const now = new Date();
    const year = now.getUTCFullYear();
    const month = now.getUTCMonth() + 1;
    const dayColumn = `d${String(now.getUTCDate()).padStart(2, "0")}`;
    const countText = runMysql(
      SHUTDOWN_DB_NAME,
      `
        SELECT COALESCE(pd.${dayColumn}, 0)
        FROM package_downloads pd
        JOIN package_versions pv ON pv.id = pd.package_version_id
        JOIN packages p ON p.id = pv.package_id
        WHERE p.name = '${SHUTDOWN_PACKAGE}'
          AND pv.version = '${SHUTDOWN_VERSION}'
          AND pd.year = ${year}
          AND pd.month = ${month}
        LIMIT 1;
      `,
    );

    expect(Number(countText)).toBe(1);
  });
});
