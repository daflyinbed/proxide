import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { createServer, type Server } from "node:http";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, openSync } from "node:fs";
import { mkdir, readdir, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import {
  CreateBucketCommand,
  HeadObjectCommand,
  PutObjectCommand,
  S3Client,
} from "@aws-sdk/client-s3";

const PROJECT_ROOT = resolve(import.meta.dirname, "../..");
const BINARY = join(PROJECT_ROOT, "target", "debug", "proxide");
const RUN_DIR = join(PROJECT_ROOT, "target", "e2e-run-tarball-cache");
const CACHE_DIR = join(RUN_DIR, "tarball-cache");
const LOG_PATH = join(RUN_DIR, "proxide.log");

const DB_NAME = "proxide_e2e_tarball_cache";
const BUCKET = "proxide-e2e-tarball-cache";
const PACKAGE_NAME = "e2e-upstream-cache-pkg";
const VERSION = "1.0.0";
const FILENAME = `${PACKAGE_NAME}-${VERSION}.tgz`;
const CHUNK_A = Buffer.alloc(256 * 1024, 0x61);
const CHUNK_B = Buffer.alloc(256 * 1024, 0x62);
const CHUNK_C = Buffer.alloc(256 * 1024, 0x63);
const TARBALL_BYTES = Buffer.concat([CHUNK_A, CHUNK_B, CHUNK_C]);

let upstreamServer: Server;
let proxideChild: ChildProcess | undefined;
let upstreamRequestCount = 0;
let proxidePort = 0;
let upstreamPort = 0;
let upstreamTailGate: Promise<void> | undefined;

function proxideUrl(): string {
  return `http://localhost:${proxidePort}`;
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

const s3 = new S3Client({
  endpoint: "http://127.0.0.1:9000",
  region: "us-east-1",
  credentials: { accessKeyId: "proxide", secretAccessKey: "proxide123" },
  forcePathStyle: true,
});

function runMysql(sql: string): string {
  return execFileSync(
    "docker",
    [
      "exec",
      "proxide-mysql",
      "mysql",
      "-uroot",
      "-proot",
      DB_NAME,
      "-Nse",
      sql,
    ],
    { encoding: "utf8" },
  ).trim();
}

function runMysqlRoot(sql: string): void {
  execFileSync("docker", ["exec", "proxide-mysql", "mysql", "-uroot", "-proot", "-e", sql], {
    stdio: "pipe",
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

async function waitForCondition(check: () => Promise<boolean>, timeoutMs = 30_000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await check()) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error("timed out waiting for condition");
}

function pauseNextUpstreamTail(): () => void {
  let release = () => {};
  upstreamTailGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  return release;
}

async function withTimeout<T>(promise: Promise<T>, timeoutMs: number): Promise<T> {
  let timeout: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<T>((_, reject) => {
        timeout = setTimeout(() => reject(new Error("operation timed out")), timeoutMs);
      }),
    ]);
  } finally {
    if (timeout) {
      clearTimeout(timeout);
    }
  }
}

async function ensureBucket() {
  try {
    await s3.send(new CreateBucketCommand({ Bucket: BUCKET }));
  } catch (err: any) {
    if (err.Name !== "BucketAlreadyOwnedByYou" && err.Name !== "BucketAlreadyExists") {
      throw err;
    }
  }
}

async function resetFixture(invalidChecksum = false) {
  upstreamRequestCount = 0;
  const checksumBytes = invalidChecksum ? Buffer.from("invalid tarball") : TARBALL_BYTES;
  const shasum = createHash("sha1").update(checksumBytes).digest("hex");
  const integrity = `sha512-${createHash("sha512").update(checksumBytes).digest("base64")}`;

  runMysql(`
    DELETE FROM package_versions WHERE package_id IN (
      SELECT id FROM packages WHERE name = '${PACKAGE_NAME}'
    );
    DELETE FROM packages WHERE name = '${PACKAGE_NAME}';
    DELETE FROM dists;
    INSERT INTO packages (name, scope, description, source)
    VALUES ('${PACKAGE_NAME}', NULL, 'e2e upstream tarball cache test', 'npmjs');
    INSERT INTO package_versions (
      package_id, version, publish_time, is_pre_release, padding_version,
      tar_shasum, tar_integrity
    )
    VALUES (
      (SELECT id FROM packages WHERE name = '${PACKAGE_NAME}'),
      '${VERSION}',
      NOW(),
      0,
      '${VERSION}',
      '${shasum}',
      '${integrity}'
    );
  `);
}

beforeAll(async () => {
  if (!existsSync(BINARY)) {
    execFileSync("cargo", ["build"], { cwd: PROJECT_ROOT, stdio: "inherit" });
  }

  runMysqlRoot(`DROP DATABASE IF EXISTS ${DB_NAME}`);
  runMysqlRoot(`CREATE DATABASE ${DB_NAME}`);
  await ensureBucket();
  proxidePort = await getFreePort();
  upstreamPort = await getFreePort();

  upstreamServer = createServer((req, res) => {
    if (req.method !== "GET" || req.url !== `/${PACKAGE_NAME}/-/${FILENAME}`) {
      res.statusCode = 404;
      res.end("not found");
      return;
    }

    upstreamRequestCount += 1;
    res.statusCode = 200;
    res.setHeader("content-type", "application/octet-stream");
    res.setHeader("content-length", String(TARBALL_BYTES.length));
    const tailGate = upstreamTailGate;
    upstreamTailGate = undefined;
    res.write(CHUNK_A);
    if (tailGate) {
      void tailGate.then(() => {
        res.write(CHUNK_B);
        res.end(CHUNK_C);
      });
      return;
    }
    setTimeout(() => {
      res.write(CHUNK_B);
      setTimeout(() => {
        res.end(CHUNK_C);
      }, 150);
    }, 150);
  });

  await new Promise<void>((resolve, reject) => {
    upstreamServer.once("error", reject);
    upstreamServer.listen(upstreamPort, "127.0.0.1", () => resolve());
  });

  await rm(RUN_DIR, { recursive: true, force: true });
  await mkdir(RUN_DIR, { recursive: true });
  await mkdir(CACHE_DIR, { recursive: true });

  const config = `[database]\nuri = "mysql://root:root@127.0.0.1:3306/${DB_NAME}"\n\n[server]\nbinding = "0.0.0.0"\nport = ${proxidePort}\nrootUrl = "${proxideUrl()}"\ntarballCacheDir = "${CACHE_DIR}"\n\n[storage.S3]\nendpoint = "http://127.0.0.1:9000"\naccessKeyId = "proxide"\nsecretAccessKey = "proxide123"\nbucketName = "${BUCKET}"\n\n[log]\nlevel = "warn"\n\n[worker]\nupstreamRegistry = "http://127.0.0.1:${upstreamPort}"\nconsumerCount = 1\ncronIntervalSecs = 3600\n\n[auth]\nallowPublishNonScopePackage = true\n`;
  await writeFile(join(RUN_DIR, "proxide.toml"), config);

  const logFd = openSync(LOG_PATH, "w");
  proxideChild = spawn(BINARY, ["server"], {
    cwd: RUN_DIR,
    stdio: ["ignore", logFd, logFd],
    env: { ...process.env, RUST_LOG: "warn" },
  });

  await waitFor(`${proxideUrl()}/-/ping`);
  await resetFixture();
});

afterAll(async () => {
  if (proxideChild && proxideChild.exitCode === null && !proxideChild.killed) {
    proxideChild.kill("SIGTERM");
    await new Promise<void>((resolve) => {
      proxideChild?.once("exit", () => resolve());
    });
  }
  await new Promise<void>((resolve) => {
    if (!upstreamServer.listening) {
      resolve();
      return;
    }
    upstreamServer.close(() => resolve());
  });
});

describe("tarball cache miss flow", () => {
  it(
    "coalesces concurrent upstream tarball downloads and backfills storage",
    async () => {
      await resetFixture();

      const tarballUrl = `${proxideUrl()}/npm/${PACKAGE_NAME}/-/${FILENAME}`;
      const releaseUpstreamTail = pauseNextUpstreamTail();
      let firstResponse: Response;
      let secondResponse: Response;
      let firstReader: ReadableStreamDefaultReader<Uint8Array>;
      let firstChunk: ReadableStreamReadResult<Uint8Array>;
      try {
        [firstResponse, secondResponse] = await withTimeout(
          Promise.all([fetch(tarballUrl), fetch(tarballUrl)]),
          5_000,
        );
        firstReader = firstResponse.body!.getReader();
        firstChunk = await withTimeout(firstReader.read(), 5_000);
        const cacheEntries = await readdir(CACHE_DIR, { withFileTypes: true });
        expect(cacheEntries).toHaveLength(1);
        expect(cacheEntries[0].isFile()).toBe(true);
        expect(cacheEntries[0].name).toMatch(/^[0-9a-f]{32}\.tmp$/);
      } finally {
        releaseUpstreamTail();
      }
      expect(firstResponse.status).toBe(200);
      expect(secondResponse.status).toBe(200);
      expect(firstChunk.done).toBe(false);
      const firstChunks = [Buffer.from(firstChunk.value!)];
      while (true) {
        const chunk = await firstReader.read();
        if (chunk.done) {
          break;
        }
        firstChunks.push(Buffer.from(chunk.value));
      }
      const firstBody = Buffer.concat(firstChunks);
      const secondBody = Buffer.from(await secondResponse.arrayBuffer());

      expect(firstBody.equals(TARBALL_BYTES)).toBe(true);
      expect(secondBody.equals(TARBALL_BYTES)).toBe(true);
      expect(upstreamRequestCount).toBe(1);
      await waitForCondition(async () => (await readdir(CACHE_DIR)).length === 0);

      await waitForCondition(async () => {
        const tarDistId = Number(
          runMysql(`
            SELECT COALESCE(pv.tar_dist_id, 0)
            FROM package_versions pv
            JOIN packages p ON p.id = pv.package_id
            WHERE p.name = '${PACKAGE_NAME}' AND pv.version = '${VERSION}'
            LIMIT 1;
          `),
        );
        return tarDistId > 0;
      });

      const tarDistId = Number(
        runMysql(`
          SELECT pv.tar_dist_id
          FROM package_versions pv
          JOIN packages p ON p.id = pv.package_id
          WHERE p.name = '${PACKAGE_NAME}' AND pv.version = '${VERSION}'
          LIMIT 1;
        `),
      );
      expect(tarDistId).toBeGreaterThan(0);

      const distPath = runMysql(`
        SELECT d.path
        FROM dists d
        JOIN package_versions pv ON pv.tar_dist_id = d.id
        JOIN packages p ON p.id = pv.package_id
        WHERE p.name = '${PACKAGE_NAME}' AND pv.version = '${VERSION}'
        LIMIT 1;
      `);
      expect(distPath).toMatch(/^objects\/raw\/sha256\/[0-9a-f]{2}\/[0-9a-f]{2}\/[0-9a-f]{64}$/);
      const s3Head = await s3.send(new HeadObjectCommand({ Bucket: BUCKET, Key: distPath }));
      expect(s3Head.ContentLength).toBe(TARBALL_BYTES.length);
    },
    120_000,
  );

  it(
    "aborts an optimistic response without caching a checksum mismatch",
    async () => {
      await resetFixture(true);

      const tarballUrl = `${proxideUrl()}/npm/${PACKAGE_NAME}/-/${FILENAME}`;
      const response = await fetch(tarballUrl);
      expect(response.status).toBe(200);
      await expect(response.arrayBuffer()).rejects.toThrow();

      const tarDistId = Number(
        runMysql(`
          SELECT COALESCE(pv.tar_dist_id, 0)
          FROM package_versions pv
          JOIN packages p ON p.id = pv.package_id
          WHERE p.name = '${PACKAGE_NAME}' AND pv.version = '${VERSION}'
          LIMIT 1;
        `),
      );
      expect(tarDistId).toBe(0);
      expect(Number(runMysql("SELECT COUNT(*) FROM dists;"))).toBe(0);
      await waitForCondition(async () => (await readdir(CACHE_DIR)).length === 0);
    },
    120_000,
  );

  it(
    "rejects a corrupted stored tarball",
    async () => {
      await resetFixture();

      const tarballUrl = `${proxideUrl()}/npm/${PACKAGE_NAME}/-/${FILENAME}`;
      const populateResponse = await fetch(tarballUrl);
      expect(populateResponse.status).toBe(200);
      expect(Buffer.from(await populateResponse.arrayBuffer()).equals(TARBALL_BYTES)).toBe(true);

      await waitForCondition(async () => {
        const tarDistId = Number(
          runMysql(`
            SELECT COALESCE(pv.tar_dist_id, 0)
            FROM package_versions pv
            JOIN packages p ON p.id = pv.package_id
            WHERE p.name = '${PACKAGE_NAME}' AND pv.version = '${VERSION}'
            LIMIT 1;
          `),
        );
        return tarDistId > 0;
      });

      const distPath = runMysql(`
        SELECT d.path
        FROM dists d
        JOIN package_versions pv ON pv.tar_dist_id = d.id
        JOIN packages p ON p.id = pv.package_id
        WHERE p.name = '${PACKAGE_NAME}' AND pv.version = '${VERSION}'
        LIMIT 1;
      `);
      await s3.send(
        new PutObjectCommand({
          Bucket: BUCKET,
          Key: distPath,
          Body: Buffer.alloc(TARBALL_BYTES.length, 0x78),
        }),
      );

      const corruptedResponse = await fetch(tarballUrl);
      expect(corruptedResponse.status).toBe(200);
      await expect(corruptedResponse.arrayBuffer()).rejects.toThrow();
      expect(upstreamRequestCount).toBe(1);
    },
    120_000,
  );
});
