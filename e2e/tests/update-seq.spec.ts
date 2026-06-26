import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { createServer, type Server } from "node:http";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

const PROJECT_ROOT = resolve(import.meta.dirname, "../..");
const BINARY = join(PROJECT_ROOT, "target", "debug", "proxide");
const WORKER_DIR = join(PROJECT_ROOT, "target", "e2e-run-update-seq");
const DB_NAME = "proxide_e2e";
const BASE_URL = "http://localhost:14873";

const UPDATE_SEQ = 1000;
const MARGIN = 10;

let upstreamServer: Server;
let upstreamPort = 0;
let workerChild: ChildProcess | undefined;

const updateSeqHits: { headers: Record<string, string | string[] | undefined> }[] = [];
const changesHits: { since: string }[] = [];

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

beforeAll(async () => {
  upstreamPort = await getFreePort();

  upstreamServer = createServer((req, res) => {
    const url = req.url ?? "";
    if (req.method === "GET" && (url === "/" || url === "")) {
      updateSeqHits.push({ headers: { ...req.headers } });
      res.statusCode = 200;
      res.setHeader("content-type", "application/json");
      res.end(JSON.stringify({ update_seq: UPDATE_SEQ }));
      return;
    }
    if (req.method === "GET" && url.startsWith("/_changes")) {
      const since = new URL(url, "http://x").searchParams.get("since") ?? "";
      changesHits.push({ since });
      res.statusCode = 200;
      res.setHeader("content-type", "application/json");
      res.end(JSON.stringify({ results: [], last_seq: UPDATE_SEQ }));
      return;
    }
    res.statusCode = 404;
    res.end("not found");
  });

  await new Promise<void>((resolve, reject) => {
    upstreamServer.once("error", reject);
    upstreamServer.listen(upstreamPort, "127.0.0.1", () => resolve());
  });

  runMysql("DELETE FROM change_stream_cursors");

  await rm(WORKER_DIR, { recursive: true, force: true });
  await mkdir(WORKER_DIR, { recursive: true });

  const upstream = `http://127.0.0.1:${upstreamPort}`;
  const workerConfig =
    `[database]\nuri = "mysql://root:root@127.0.0.1:3306/${DB_NAME}"\n\n` +
    `[server]\nport = 14873\nrootUrl = "${BASE_URL}"\n\n` +
    `[storage.S3]\nendpoint = "http://127.0.0.1:9000"\naccessKeyId = "proxide"\nsecretAccessKey = "proxide123"\nbucketName = "proxide-e2e"\n\n` +
    `[log]\nlevel = "info"\n\n` +
    `[worker]\nupstreamRegistry = "${upstream}"\nchangesStreamUrl = "${upstream}/_changes"\nupdateSeqUrl = "${upstream}/"\nconsumerCount = 1\nconsumerPollIntervalMs = 100\ncronIntervalSecs = 3600\n`;

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

async function waitFor<T>(
  fn: () => T | undefined | null,
  timeoutMs = 15_000,
): Promise<T> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const v = fn();
    if (v) return v;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error("timed out waiting for condition");
}

describe("worker update_seq request", () => {
  it(
    "fetches update_seq on first poll and uses it (minus margin) as the initial since",
    async () => {
      await waitFor(() => (updateSeqHits.length > 0 ? updateSeqHits[0] : null));
      await waitFor(() => (changesHits.length > 0 ? changesHits[0] : null));

      expect(updateSeqHits.length).toBeGreaterThanOrEqual(1);
      const updateSeqHeaders = updateSeqHits[0].headers;
      expect(updateSeqHeaders["npm-replication-opt-in"]).toBeUndefined();

      expect(changesHits[0].since).toBe(String(UPDATE_SEQ - MARGIN));
    },
    30_000,
  );
});
