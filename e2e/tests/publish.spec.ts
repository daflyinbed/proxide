import { describe, expect, it } from "vitest";
import { api, apiJson, login, packagePath, publishPackage, uniqueName, uniqueScopedName } from "../helpers.js";

describe("publish flow", () => {
  it("publishes a package and retrieves it", async () => {
    const name = uniqueName("e2e-pkg");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-publisher"), "pass1234");

    const { res: pubRes, body: pubBody } = await publishPackage(token, name, version);
    expect(pubRes.status).toBe(200);
    expect(pubBody.ok).toBe(true);

    const { res: getRes, body: packument } = await apiJson(packagePath(name));
    expect(getRes.status).toBe(200);
    expect(packument.name).toBe(name);
    expect(packument.versions[version]).toBeDefined();
    expect(packument["dist-tags"].latest).toBe(version);
  });

  it("publishes and retrieves with abbreviated Accept header", async () => {
    const name = uniqueName("e2e-pkg-abbrev");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-publisher-abbrev"), "pass1234");

    await publishPackage(token, name, version);

    const res = await api(packagePath(name), {
      headers: { accept: "application/vnd.npm.install-v1+json" },
    });
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.name).toBe(name);
    expect(body.versions[version]).toBeDefined();
  });

  it("publishes and retrieves a specific version", async () => {
    const name = uniqueName("e2e-pkg-ver");
    const version = "2.0.0";
    const token = await login(uniqueName("e2e-publisher-ver"), "pass1234");

    await publishPackage(token, name, version);

    const { res, body } = await apiJson(
      `${packagePath(name)}/${version}`,
    );
    expect(res.status).toBe(200);
    expect(body.name).toBe(name);
    expect(body.version).toBe(version);
  });

  it("publishes and downloads the tarball", async () => {
    const name = uniqueName("e2e-pkg-tar");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-publisher-tar"), "pass1234");

    await publishPackage(token, name, version);

    const { body: packument } = await apiJson(packagePath(name));
    const tarballUrl = packument.versions[version].dist.tarball as string;
    const tarballPath = tarballUrl.replace("http://localhost:14873", "");

    const res = await api(tarballPath);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toContain("application/octet-stream");

    const buf = Buffer.from(await res.arrayBuffer());
    expect(buf.length).toBeGreaterThan(0);
  });

  it("returns 409 when publishing a duplicate version", async () => {
    const name = uniqueName("e2e-pkg-dup");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-publisher-dup"), "pass1234");

    await publishPackage(token, name, version);
    const { res } = await publishPackage(token, name, version);
    expect(res.status).toBe(409);
  });

  it("returns 401 without auth token", async () => {
    const name = uniqueName("e2e-pkg-noauth");
    const { buildPublishPayload } = await import("../fixtures/tarball.js");
    const payload = buildPublishPayload(name, "1.0.0");

    const res = await api(packagePath(name), {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
    });
    expect(res.status).toBe(401);
  });

  it("returns 404 for non-existent package", async () => {
    const res = await api("/npm/e2e-nonexistent-pkg-" + Date.now());
    expect(res.status).toBe(404);
  });
});

describe("scoped package publish flow", () => {
  it("publishes a scoped package and retrieves it", async () => {
    const name = uniqueScopedName("e2e-scope", "e2e-scoped-pkg");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-scoped-publisher"), "pass1234");

    const { res: pubRes, body: pubBody } = await publishPackage(token, name, version);
    expect(pubRes.status).toBe(200);
    expect(pubBody.ok).toBe(true);

    const { res: getRes, body: packument } = await apiJson(packagePath(name));
    expect(getRes.status).toBe(200);
    expect(packument.name).toBe(name);
    expect(packument.versions[version]).toBeDefined();
    expect(packument["dist-tags"].latest).toBe(version);
  });

  it("publishes a scoped package and retrieves with abbreviated Accept header", async () => {
    const name = uniqueScopedName("e2e-scope", "e2e-scoped-abbrev");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-scoped-pub-abbrev"), "pass1234");

    await publishPackage(token, name, version);

    const res = await api(packagePath(name), {
      headers: { accept: "application/vnd.npm.install-v1+json" },
    });
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.name).toBe(name);
    expect(body.versions[version]).toBeDefined();
  });

  it("publishes a scoped package and retrieves a specific version", async () => {
    const name = uniqueScopedName("e2e-scope", "e2e-scoped-ver");
    const version = "2.0.0";
    const token = await login(uniqueName("e2e-scoped-pub-ver"), "pass1234");

    await publishPackage(token, name, version);

    const { res, body } = await apiJson(
      `${packagePath(name)}/${version}`,
    );
    expect(res.status).toBe(200);
    expect(body.name).toBe(name);
    expect(body.version).toBe(version);
  });

  it("publishes a scoped package and downloads the tarball", async () => {
    const name = uniqueScopedName("e2e-scope", "e2e-scoped-tar");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-scoped-pub-tar"), "pass1234");

    await publishPackage(token, name, version);

    const { body: packument } = await apiJson(packagePath(name));
    const tarballUrl = packument.versions[version].dist.tarball as string;
    const tarballPath = tarballUrl.replace("http://localhost:14873", "");

    const res = await api(tarballPath);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toContain("application/octet-stream");

    const buf = Buffer.from(await res.arrayBuffer());
    expect(buf.length).toBeGreaterThan(0);
  });

  it("returns 409 when publishing a duplicate version of scoped package", async () => {
    const name = uniqueScopedName("e2e-scope", "e2e-scoped-dup");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-scoped-pub-dup"), "pass1234");

    await publishPackage(token, name, version);
    const { res } = await publishPackage(token, name, version);
    expect(res.status).toBe(409);
  });
});
