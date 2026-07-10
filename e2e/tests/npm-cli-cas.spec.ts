import { describe, expect, it, beforeAll, afterAll } from "vitest";
import { spawn, type ChildProcess } from "node:child_process";
import { openSync, readFileSync, writeFileSync } from "node:fs";
import { copyFile, readFile } from "node:fs/promises";
import { resolve, join } from "node:path";
import {
  PROXIDE_BINARY,
  PROXIDE_RUN_DIR,
  BASE_URL,
  PROXIDE_PID_FILE,
} from "../globalSetup.js";
import { startMockCas, MOCK_CAS_PORT } from "../mock-cas-server.js";
import {
  execNpm,
  npmLoginWeb,
  npmAdduser,
  createAuthDir,
  npmrcEnv,
  createTempPackageDir,
  makeUnscopedPkg,
} from "../npm-cli-helpers.js";
import { uniqueName, apiJson } from "../helpers.js";

const CAS_TOML_SRC = resolve(import.meta.dirname, "..", "proxide.e2e.cas.toml");
const RUN_TOML = join(PROXIDE_RUN_DIR, "proxide.toml");
const CAS_REGISTRY = `${BASE_URL}/npm/`;

let mockCas: ReturnType<typeof startMockCas>;
let casChild: ChildProcess;
let legacyConfig: string;

async function waitForPing(timeoutMs = 60_000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(`${BASE_URL}/-/ping`);
      if (res.ok) return;
    } catch {}
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error("proxide /-/ping timed out");
}

beforeAll(async () => {
  legacyConfig = readFileSync(RUN_TOML, "utf-8");

  const pid = parseInt(readFileSync(PROXIDE_PID_FILE, "utf-8").trim());
  try {
    process.kill(pid, "SIGTERM");
  } catch {}
  await new Promise((r) => setTimeout(r, 3000));

  mockCas = startMockCas(MOCK_CAS_PORT);

  await copyFile(CAS_TOML_SRC, RUN_TOML);
  const logFd = openSync(join(PROXIDE_RUN_DIR, "proxide-cas.log"), "w");
  casChild = spawn(PROXIDE_BINARY, ["server"], {
    cwd: PROXIDE_RUN_DIR,
    stdio: ["ignore", logFd, logFd],
    env: { ...process.env, RUST_LOG: "info" },
  });

  await waitForPing();
}, 120_000);

afterAll(async () => {
  casChild.kill("SIGTERM");
  await new Promise((r) => casChild.once("exit", r));
  mockCas.close();

  writeFileSync(RUN_TOML, legacyConfig);
  const logFd = openSync(join(PROXIDE_RUN_DIR, "proxide.log"), "w");
  const legacyChild = spawn(PROXIDE_BINARY, ["server"], {
    cwd: PROXIDE_RUN_DIR,
    stdio: ["ignore", logFd, logFd],
    env: { ...process.env, RUST_LOG: "warn" },
  });
  await waitForPing();
  writeFileSync(PROXIDE_PID_FILE, String(legacyChild.pid));
}, 120_000);

describe("npm login (CAS web auth)", () => {
  it("completes full CAS login via npm CLI", async () => {
    const dir = await createAuthDir();
    const user = "casuser1";

    const res = await npmLoginWeb(dir, CAS_REGISTRY, async (sessionId) => {
      const cb = await fetch(
        `${BASE_URL}/api/auth/cas/callback/session/${sessionId}?ticket=ticket-${user}`,
      );
      expect(cb.status).toBe(200);
    });
    expect(res.exitCode).toBe(0);

    const who = await execNpm(["whoami"], { cwd: dir });
    expect(who.exitCode).toBe(0);
    expect(who.stdout.trim()).toBe(user);
  });

  it("publishes after CAS login", async () => {
    const dir = await createAuthDir();
    const user = "caspub1";

    await npmLoginWeb(dir, CAS_REGISTRY, async (sessionId) => {
      await fetch(
        `${BASE_URL}/api/auth/cas/callback/session/${sessionId}?ticket=ticket-${user}`,
      );
    });

    const { name, version } = makeUnscopedPkg();
    const pkgDir = await createTempPackageDir(name, version);
    const res = await execNpm(["publish"], {
      cwd: pkgDir,
      env: npmrcEnv(dir),
    });
    expect(res.exitCode).toBe(0);
    expect(res.stdout).toContain(name);
  });
});

describe("CAS restrictions", () => {
  it("rejects legacy adduser when CAS is enabled", async () => {
    const dir = await createAuthDir();
    const res = await npmAdduser(dir, uniqueName("e2e-cas-legacy"), "pass1234");
    expect(res.exitCode).not.toBe(0);
  });

  it("CAS user cannot create token (no password)", async () => {
    const dir = await createAuthDir();
    const user = "casnotoken1";

    await npmLoginWeb(dir, CAS_REGISTRY, async (sessionId) => {
      await fetch(
        `${BASE_URL}/api/auth/cas/callback/session/${sessionId}?ticket=ticket-${user}`,
      );
    });

    const npmrc = await readFile(join(dir, ".npmrc"), "utf-8");
    const token = npmrc.match(/_authToken=(.+)/)?.[1]?.trim();
    expect(token).toBeTruthy();

    const { res } = await apiJson("/npm/-/npm/v1/tokens", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ password: "anything" }),
    });
    expect(res.status).toBe(403);
  });
});

describe("CAS login pending state", () => {
  it("returns 202 while processing then completes", async () => {
    const dir = await createAuthDir();
    const user = "caspending1";

    const res = await npmLoginWeb(dir, CAS_REGISTRY, async (sessionId) => {
      const pending = await fetch(
        `${BASE_URL}/npm/-/v1/login/done/session/${sessionId}`,
      );
      expect(pending.status).toBe(202);

      await fetch(
        `${BASE_URL}/api/auth/cas/callback/session/${sessionId}?ticket=ticket-${user}`,
      );
    });
    expect(res.exitCode).toBe(0);
  });
});
