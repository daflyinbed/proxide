import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { createServer, type Server } from "node:http";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { apiJson, uniqueName, uniqueScopedName } from "../helpers.js";

describe("PUT /npm/-/package/{fullname}/syncs", () => {
  it("enqueues a sync task", async () => {
    const name = uniqueName("e2e-sync");
    const { res, body } = await apiJson("/npm/-/package/" + encodeURIComponent(name) + "/syncs", {
      method: "PUT",
    });
    expect(res.status).toBe(200);
    expect(body.ok).toBe(true);
    expect(body.log).toBe("queued");
  });

  it("returns already queued for duplicate sync", async () => {
    const name = uniqueName("e2e-sync-dup");
    await apiJson("/npm/-/package/" + encodeURIComponent(name) + "/syncs", {
      method: "PUT",
    });
    const { res, body } = await apiJson(
      "/npm/-/package/" + encodeURIComponent(name) + "/syncs",
      { method: "PUT" },
    );
    expect(res.status).toBe(200);
    expect(body.ok).toBe(true);
    expect(body.log).toBe("already queued");
  });

  it("enqueues a sync task for a scoped package", async () => {
    const name = uniqueScopedName("e2e-scope", "e2e-sync");
    const { res, body } = await apiJson("/npm/-/package/" + encodeURIComponent(name) + "/syncs", {
      method: "PUT",
    });
    expect(res.status).toBe(200);
    expect(body.ok).toBe(true);
    expect(body.log).toBe("queued");
  });

  it("returns already queued for duplicate scoped sync", async () => {
    const name = uniqueScopedName("e2e-scope", "e2e-sync-dup");
    await apiJson("/npm/-/package/" + encodeURIComponent(name) + "/syncs", {
      method: "PUT",
    });
    const { res, body } = await apiJson(
      "/npm/-/package/" + encodeURIComponent(name) + "/syncs",
      { method: "PUT" },
    );
    expect(res.status).toBe(200);
    expect(body.ok).toBe(true);
    expect(body.log).toBe("already queued");
  });
});

describe("worker sync flow", () => {
  const PROJECT_ROOT = resolve(import.meta.dirname, "../..");
  const BINARY = join(PROJECT_ROOT, "target", "debug", "proxide");
  const WORKER_DIR = join(PROJECT_ROOT, "target", "e2e-run-worker");
  const DB_NAME = "proxide_e2e";
  const BASE_URL = "http://localhost:14873";

  const SYNC_PKG = uniqueName("e2e-worker-sync");
  const SYNC_VERSION = "1.0.0";

  let upstreamServer: Server;
  let upstreamPort = 0;
  let workerChild: ChildProcess | undefined;
  let upstreamVersionTime = "2024-01-01T00:00:00.000Z";

  function packument(): string {
    return JSON.stringify({
      name: SYNC_PKG,
      "dist-tags": { latest: SYNC_VERSION },
      versions: {
        [SYNC_VERSION]: {
          name: SYNC_PKG,
          version: SYNC_VERSION,
          dist: {
            tarball: `http://127.0.0.1:0/${SYNC_PKG}/-/${SYNC_PKG}-${SYNC_VERSION}.tgz`,
          },
        },
      },
      time: {
        modified: upstreamVersionTime,
        [SYNC_VERSION]: upstreamVersionTime,
      },
    });
  }

  function runMysql(sql: string): string {
    return execFileSync(
      "docker",
      ["exec", "proxide-mysql", "mysql", "-uroot", "-proot", DB_NAME, "-Nse", sql],
      { encoding: "utf8" },
    ).trim();
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

  async function waitForLatestSyncTask(): Promise<void> {
    const start = Date.now();
    while (Date.now() - start < 15_000) {
      const status = runMysql(
        `SELECT status FROM sync_tasks WHERE name = '${SYNC_PKG}' ORDER BY id DESC LIMIT 1`,
      );
      if (status === "done") return;
      if (status === "failed") throw new Error(`sync task failed for ${SYNC_PKG}`);
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
    throw new Error(`timed out waiting for sync task for ${SYNC_PKG}`);
  }

  beforeAll(async () => {
    upstreamPort = await getFreePort();

    upstreamServer = createServer((req, res) => {
      if (req.method === "GET" && req.url?.startsWith("/_changes")) {
        res.statusCode = 200;
        res.setHeader("content-type", "application/json");
        res.end(JSON.stringify({ results: [], last_seq: 0 }));
        return;
      }
      if (req.method === "GET" && req.url === `/${SYNC_PKG}`) {
        res.statusCode = 200;
        res.setHeader("content-type", "application/json");
        res.end(packument());
        return;
      }
      res.statusCode = 404;
      res.end("not found");
    });

    await new Promise<void>((resolve, reject) => {
      upstreamServer.once("error", reject);
      upstreamServer.listen(upstreamPort, "127.0.0.1", () => resolve());
    });

    await rm(WORKER_DIR, { recursive: true, force: true });
    await mkdir(WORKER_DIR, { recursive: true });

    const workerConfig =
      `[database]\nuri = "mysql://root:root@127.0.0.1:3306/${DB_NAME}"\n\n` +
      `[server]\nport = 14873\nrootUrl = "${BASE_URL}"\n\n` +
      `[storage.S3]\nendpoint = "http://127.0.0.1:9000"\naccessKeyId = "proxide"\nsecretAccessKey = "proxide123"\nbucketName = "proxide-e2e"\ncompressJson = true\nzstdLevel = 3\n\n` +
      `[log]\nlevel = "info"\n\n` +
      `[worker]\nupstreamRegistry = "http://127.0.0.1:${upstreamPort}"\nchangesStreamUrl = "http://127.0.0.1:${upstreamPort}/_changes"\nconsumerCount = 1\nconsumerPollIntervalMs = 100\ncronIntervalSecs = 3600\n`;

    await writeFile(join(WORKER_DIR, "proxide.toml"), workerConfig);

    workerChild = spawn(BINARY, ["worker"], {
      cwd: WORKER_DIR,
      stdio: ["ignore", "inherit", "inherit"],
    });
  });

  afterAll(async () => {
    if (workerChild && workerChild.exitCode === null && !workerChild.killed) {
      workerChild.kill("SIGTERM");
      await new Promise<void>((resolve) => {
        workerChild!.once("exit", () => resolve());
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

  it(
    "worker claims and completes an enqueued sync task",
    async () => {
      const { res, body } = await apiJson(
        "/npm/-/package/" + encodeURIComponent(SYNC_PKG) + "/syncs",
        { method: "PUT" },
      );
      expect(res.status).toBe(200);
      expect(body.ok).toBe(true);
      expect(body.log).toBe("queued");

      await waitForLatestSyncTask();

      const pkgRes = await fetch(`${BASE_URL}/npm/${SYNC_PKG}`);
      expect(pkgRes.status).toBe(200);
      const pkgBody = await pkgRes.json();
      expect(pkgBody.versions[SYNC_VERSION]).toBeDefined();

      const publishTime = runMysql(`
        SELECT DATE_FORMAT(pv.publish_time, '%Y-%m-%dT%H:%i:%s.%fZ')
        FROM package_versions pv
        JOIN packages p ON p.id = pv.package_id
        WHERE p.name = '${SYNC_PKG}' AND pv.version = '${SYNC_VERSION}'
      `);
      expect(publishTime).toBe("2024-01-01T00:00:00.000000Z");

      upstreamVersionTime = "not-a-time";
      const { res: resyncRes, body: resyncBody } = await apiJson(
        "/npm/-/package/" + encodeURIComponent(SYNC_PKG) + "/syncs",
        { method: "PUT" },
      );
      expect(resyncRes.status).toBe(200);
      expect(resyncBody.log).toBe("queued");
      await waitForLatestSyncTask();

      const preservedPublishTime = runMysql(`
        SELECT DATE_FORMAT(pv.publish_time, '%Y-%m-%dT%H:%i:%s.%fZ')
        FROM package_versions pv
        JOIN packages p ON p.id = pv.package_id
        WHERE p.name = '${SYNC_PKG}' AND pv.version = '${SYNC_VERSION}'
      `);
      expect(preservedPublishTime).toBe("2024-01-01T00:00:00.000000Z");
    },
    30_000,
  );
});
