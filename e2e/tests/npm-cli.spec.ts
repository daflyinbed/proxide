import { describe, expect, it } from "vitest";
import { join } from "node:path";
import {
  execNpm,
  createTempDir,
  createTempPackageDir,
  writeNpmrc,
  npmLogin,
  makeUnscopedPkg,
  makeScopedPkg,
  fileExists,
  readJsonFile,
} from "../npm-cli-helpers.js";
import { uniqueName } from "../helpers.js";

async function setupAuth(): Promise<{ token: string; username: string }> {
  const username = uniqueName("e2e-npm-user");
  const token = await npmLogin(username, "pass1234");
  return { token, username };
}

async function publishPackage(
  token: string,
  name: string,
  version: string,
): Promise<void> {
  const pkgDir = await createTempPackageDir(name, version);
  await writeNpmrc(pkgDir, token);
  const args = name.startsWith("@") ? ["publish", "--access", "public"] : ["publish"];
  const res = await execNpm(args, { cwd: pkgDir });
  expect(res.exitCode).toBe(0);
  expect(res.stdout).toContain(name);
}

describe("npm login", () => {
  it("authenticates via legacy login and token works with npm publish", async () => {
    const { token } = await setupAuth();
    const { name, version } = makeUnscopedPkg();
    const pkgDir = await createTempPackageDir(name, version);
    await writeNpmrc(pkgDir, token);
    const res = await execNpm(["publish"], { cwd: pkgDir });
    expect(res.exitCode).toBe(0);
    expect(res.stdout).toContain(name);
  });
});

describe("npm publish + install (unscoped)", () => {
  it("publishes an unscoped package and installs it", async () => {
    const { token } = await setupAuth();
    const { name, version } = makeUnscopedPkg();
    await publishPackage(token, name, version);

    const installDir = await createTempDir("install-unscoped");
    await writeNpmrc(installDir, token);
    const res = await execNpm(["install", name], { cwd: installDir });
    expect(res.exitCode).toBe(0);

    const pkgJsonPath = join(installDir, "node_modules", name, "package.json");
    expect(await fileExists(pkgJsonPath)).toBe(true);
    const pkgJson = await readJsonFile(pkgJsonPath);
    expect(pkgJson.name).toBe(name);
    expect(pkgJson.version).toBe(version);
  });
});

describe("npm publish + install (scoped)", () => {
  it("publishes a scoped package and installs it", async () => {
    const { token } = await setupAuth();
    const { name, version } = makeScopedPkg();
    await publishPackage(token, name, version);

    const installDir = await createTempDir("install-scoped");
    await writeNpmrc(installDir, token);
    const res = await execNpm(["install", name], { cwd: installDir });
    expect(res.exitCode).toBe(0);

    const [scopePart, localPart] = name.split("/");
    const pkgJsonPath = join(installDir, "node_modules", scopePart, localPart, "package.json");
    expect(await fileExists(pkgJsonPath)).toBe(true);
    const pkgJson = await readJsonFile(pkgJsonPath);
    expect(pkgJson.name).toBe(name);
    expect(pkgJson.version).toBe(version);
  });
});

describe("npm install (unscoped)", () => {
  it("installs a previously published unscoped package", async () => {
    const { token } = await setupAuth();
    const { name, version } = makeUnscopedPkg();
    await publishPackage(token, name, version);

    const installDir = await createTempDir("install-unscoped-2");
    await writeNpmrc(installDir, token);
    const res = await execNpm(["install", name], { cwd: installDir });
    expect(res.exitCode).toBe(0);

    const pkgJsonPath = join(installDir, "node_modules", name, "package.json");
    expect(await fileExists(pkgJsonPath)).toBe(true);
    const pkgJson = await readJsonFile(pkgJsonPath);
    expect(pkgJson.version).toBe(version);
  });
});

describe("npm install (scoped)", () => {
  it("installs a previously published scoped package", async () => {
    const { token } = await setupAuth();
    const { name, version } = makeScopedPkg();
    await publishPackage(token, name, version);

    const installDir = await createTempDir("install-scoped-2");
    await writeNpmrc(installDir, token);
    const res = await execNpm(["install", name], { cwd: installDir });
    expect(res.exitCode).toBe(0);

    const [scopePart, localPart] = name.split("/");
    const pkgJsonPath = join(installDir, "node_modules", scopePart, localPart, "package.json");
    expect(await fileExists(pkgJsonPath)).toBe(true);
    const pkgJson = await readJsonFile(pkgJsonPath);
    expect(pkgJson.name).toBe(name);
    expect(pkgJson.version).toBe(version);
  });
});

describe("npm view (unscoped)", () => {
  it("shows package metadata for an unscoped package", async () => {
    const { token } = await setupAuth();
    const { name, version } = makeUnscopedPkg();
    await publishPackage(token, name, version);

    const viewDir = await createTempDir("view-unscoped");
    await writeNpmrc(viewDir, token);
    const res = await execNpm(["view", name], { cwd: viewDir });
    expect(res.exitCode).toBe(0);
    expect(res.stdout).toContain(version);
  });
});

describe("npm view (scoped)", () => {
  it("shows package metadata for a scoped package", async () => {
    const { token } = await setupAuth();
    const { name, version } = makeScopedPkg();
    await publishPackage(token, name, version);

    const viewDir = await createTempDir("view-scoped");
    await writeNpmrc(viewDir, token);
    const res = await execNpm(["view", name], { cwd: viewDir });
    expect(res.exitCode).toBe(0);
    expect(res.stdout).toContain(version);
  });
});
