import { describe, expect, it } from "vitest";
import { join } from "node:path";
import { readFile, writeFile } from "node:fs/promises";
import {
  execNpm,
  npmAdduser,
  createAuthDir,
  createTempPackageDir,
  createTempDir,
  npmrcEnv,
  setupCliAuth,
  makeUnscopedPkg,
  NPM_REGISTRY,
  fileExists,
  readJsonFile,
} from "../npm-cli-helpers.js";
import { uniqueName, login, apiJson, bearer } from "../helpers.js";

describe("npm adduser (legacy)", () => {
  it("writes token to .npmrc after adduser", async () => {
    const dir = await createAuthDir();
    const user = uniqueName("e2e-adduser");
    const res = await npmAdduser(dir, user, "pass1234");
    expect(res.exitCode).toBe(0);

    const npmrc = await readFile(join(dir, ".npmrc"), "utf-8");
    expect(npmrc).toContain("_authToken=");
  });

  it("token works with npm whoami", async () => {
    const dir = await createAuthDir();
    const user = uniqueName("e2e-whoami-cli");
    await npmAdduser(dir, user, "pass1234");

    const res = await execNpm(["whoami"], { cwd: dir });
    expect(res.exitCode).toBe(0);
    expect(res.stdout.trim()).toBe(user);
  });

  it("re-login overwrites the token", async () => {
    const dir = await createAuthDir();
    const user = uniqueName("e2e-relogin");
    await npmAdduser(dir, user, "pass1234");
    const before = await readFile(join(dir, ".npmrc"), "utf-8");

    await npmAdduser(dir, user, "pass1234");
    const after = await readFile(join(dir, ".npmrc"), "utf-8");

    expect(after).not.toEqual(before);
    const res = await execNpm(["whoami"], { cwd: dir });
    expect(res.stdout.trim()).toBe(user);
  });
});

describe("npm publish + install (CLI auth)", () => {
  it("authenticates then publishes and installs", async () => {
    const { username, authDir } = await setupCliAuth();
    const { name, version } = makeUnscopedPkg();
    const pkgDir = await createTempPackageDir(name, version);

    const pubRes = await execNpm(["publish"], {
      cwd: pkgDir,
      env: npmrcEnv(authDir),
    });
    expect(pubRes.exitCode).toBe(0);
    expect(pubRes.stdout).toContain(name);

    const installDir = await createTempDir("install-cli");
    const instRes = await execNpm(["install", name], {
      cwd: installDir,
      env: npmrcEnv(authDir),
    });
    expect(instRes.exitCode).toBe(0);

    const pkgJsonPath = join(installDir, "node_modules", name, "package.json");
    expect(await fileExists(pkgJsonPath)).toBe(true);
    const pkgJson = await readJsonFile(pkgJsonPath);
    expect(pkgJson.name).toBe(name);
    expect(pkgJson.version).toBe(version);
  });
});

describe("npm token list (CLI)", () => {
  it("lists tokens after login", async () => {
    const { authDir } = await setupCliAuth();

    const res = await execNpm(["token", "list", "--json"], { cwd: authDir });
    expect(res.exitCode).toBe(0);

    const parsed = JSON.parse(res.stdout);
    const objects = Array.isArray(parsed) ? parsed : parsed.objects;
    expect(objects.length).toBeGreaterThanOrEqual(1);
    expect(objects[0].readonly).toBe(false);
  });
});

describe("read-only token restriction", () => {
  it("read-only token cannot publish via npm CLI", async () => {
    const user = uniqueName("e2e-readonly");
    const password = "pass1234";
    const token = await login(user, password);

    const { body } = await apiJson<{ token: string; readonly: boolean }>(
      "/npm/-/npm/v1/tokens",
      {
        method: "POST",
        headers: { "content-type": "application/json", ...bearer(token) },
        body: JSON.stringify({ password, readonly: true }),
      },
    );
    expect(body.readonly).toBe(true);

    const roAuthDir = await createAuthDir();
    await writeFile(
      join(roAuthDir, ".npmrc"),
      [
        `registry=${NPM_REGISTRY}`,
        `//localhost:14873/npm/:_authToken=${body.token}`,
        `//localhost:14873/npm/:always-auth=true`,
      ].join("\n") + "\n",
    );

    const { name, version } = makeUnscopedPkg();
    const pkgDir = await createTempPackageDir(name, version);
    const pubRes = await execNpm(["publish"], {
      cwd: pkgDir,
      env: npmrcEnv(roAuthDir),
    });
    expect(pubRes.exitCode).not.toBe(0);
  });
});

describe("npm logout (CLI)", () => {
  it("revokes token and subsequent whoami fails", async () => {
    const { username, authDir } = await setupCliAuth();

    const out = await execNpm(["logout"], { cwd: authDir });
    expect(out.exitCode).toBe(0);

    const who = await execNpm(["whoami"], { cwd: authDir });
    expect(who.exitCode).not.toBe(0);
  });
});

describe("auth failure scenarios", () => {
  it("rejects publish without authentication", async () => {
    const noAuthDir = await createAuthDir();
    await writeFile(
      join(noAuthDir, ".npmrc"),
      `registry=${NPM_REGISTRY}\n`,
    );

    const { name, version } = makeUnscopedPkg();
    const pkgDir = await createTempPackageDir(name, version);
    const res = await execNpm(["publish"], {
      cwd: pkgDir,
      env: npmrcEnv(noAuthDir),
    });
    expect(res.exitCode).not.toBe(0);
    expect(res.stderr).toContain("ENEEDAUTH");
  });
});
