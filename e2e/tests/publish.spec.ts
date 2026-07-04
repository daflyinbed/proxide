import { describe, expect, it } from "vitest";
import {
  api,
  apiJson,
  bearer,
  login,
  packagePath,
  publishPackage,
  tarballFilename,
  uniqueName,
  uniqueScopedName,
  unpublishPackage,
  unpublishPath,
  unpublishVersion,
} from "../helpers.js";

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

    const { res: pubRes, body: pubBody } = await publishPackage(token, name, version, { access: "public" });
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

    await publishPackage(token, name, version, { access: "public" });

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

    await publishPackage(token, name, version, { access: "public" });

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

    await publishPackage(token, name, version, { access: "public" });

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

describe("unpublish whole package (DELETE /{fullname}/-rev/{rev})", () => {
  it("removes an unscoped package completely", async () => {
    const name = uniqueName("e2e-unpub");
    const token = await login(uniqueName("e2e-unpub-owner"), "pass1234");
    await publishPackage(token, name, "1.0.0");

    const res = await unpublishPackage(token, name);
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.ok).toBe(true);

    const { res: getRes } = await apiJson(packagePath(name));
    expect(getRes.status).toBe(404);
  });

  it("removes a scoped package completely", async () => {
    const name = uniqueScopedName("e2e-scope", "e2e-unpub");
    const token = await login(uniqueName("e2e-unpub-scoped-owner"), "pass1234");
    await publishPackage(token, name, "1.0.0", { access: "public" });

    const res = await unpublishPackage(token, name);
    expect(res.status).toBe(200);

    const { res: getRes } = await apiJson(packagePath(name));
    expect(getRes.status).toBe(404);
  });

  it("deletes the package tarball as well", async () => {
    const name = uniqueName("e2e-unpub-tar");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-unpub-tar-owner"), "pass1234");
    await publishPackage(token, name, version);

    await unpublishPackage(token, name);

    const tarballRes = await api(
      `${packagePath(name)}/-/${tarballFilename(name, version)}`,
    );
    expect(tarballRes.status).toBe(404);
  });

  it("returns 400 without npm-command: unpublish header", async () => {
    const name = uniqueName("e2e-unpub-nocmd");
    const token = await login(uniqueName("e2e-unpub-nocmd-owner"), "pass1234");
    await publishPackage(token, name, "1.0.0");

    const res = await api(unpublishPath(name), {
      method: "DELETE",
      headers: bearer(token),
    });
    expect(res.status).toBe(400);
  });

  it("returns 401 without auth token", async () => {
    const name = uniqueName("e2e-unpub-noauth");
    const res = await api(unpublishPath(name), {
      method: "DELETE",
      headers: { "npm-command": "unpublish" },
    });
    expect(res.status).toBe(401);
  });

  it("returns 403 when caller is not a maintainer", async () => {
    const name = uniqueName("e2e-unpub-forbidden");
    const owner = await login(uniqueName("e2e-unpub-owner-a"), "pass1234");
    const other = await login(uniqueName("e2e-unpub-other"), "pass1234");
    await publishPackage(owner, name, "1.0.0");

    const res = await unpublishPackage(other, name);
    expect(res.status).toBe(403);
  });

  it("returns 404 for non-existent package", async () => {
    const token = await login(uniqueName("e2e-unpub-missing-owner"), "pass1234");
    const res = await unpublishPackage(token, uniqueName("e2e-no-such-pkg"));
    expect(res.status).toBe(404);
  });
});

describe("unpublish single version (DELETE /{fullname}/-/{filename}/-rev/{rev})", () => {
  it("removes the package when the last version is unpublished", async () => {
    const name = uniqueName("e2e-unpub-ver-last");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-unpub-ver-last-owner"), "pass1234");
    await publishPackage(token, name, version);

    const res = await unpublishVersion(token, name, version);
    expect(res.status).toBe(200);

    const { res: getRes } = await apiJson(packagePath(name));
    expect(getRes.status).toBe(404);
  });

  it("removes a single version but keeps the others", async () => {
    const name = uniqueName("e2e-unpub-ver-one");
    const token = await login(uniqueName("e2e-unpub-ver-one-owner"), "pass1234");
    await publishPackage(token, name, "1.0.0");
    await publishPackage(token, name, "2.0.0");

    const res = await unpublishVersion(token, name, "1.0.0");
    expect(res.status).toBe(200);

    const { res: getRes, body: packument } = await apiJson(packagePath(name));
    expect(getRes.status).toBe(200);
    expect(packument.versions["1.0.0"]).toBeUndefined();
    expect(packument.versions["2.0.0"]).toBeDefined();
    expect(packument["dist-tags"].latest).toBe("2.0.0");
  });

  it("recomputes latest tag when latest version is unpublished", async () => {
    const name = uniqueName("e2e-unpub-ver-latest");
    const token = await login(
      uniqueName("e2e-unpub-ver-latest-owner"),
      "pass1234",
    );
    await publishPackage(token, name, "1.0.0");
    await publishPackage(token, name, "2.0.0");
    await publishPackage(token, name, "1.5.0");

    const res = await unpublishVersion(token, name, "2.0.0");
    expect(res.status).toBe(200);

    const { body: packument } = await apiJson(packagePath(name));
    expect(packument.versions["2.0.0"]).toBeUndefined();
    expect(packument["dist-tags"].latest).toBe("1.5.0");
  });

  it("removes the tarball of the unpublished version", async () => {
    const name = uniqueName("e2e-unpub-ver-tar");
    const token = await login(uniqueName("e2e-unpub-ver-tar-owner"), "pass1234");
    await publishPackage(token, name, "1.0.0");
    await publishPackage(token, name, "2.0.0");

    await unpublishVersion(token, name, "1.0.0");

    const removed = await api(
      `${packagePath(name)}/-/${tarballFilename(name, "1.0.0")}`,
    );
    expect(removed.status).toBe(404);
    const kept = await api(
      `${packagePath(name)}/-/${tarballFilename(name, "2.0.0")}`,
    );
    expect(kept.status).toBe(200);
  });

  it("returns 400 without npm-command: unpublish header", async () => {
    const name = uniqueName("e2e-unpub-ver-nocmd");
    const token = await login(
      uniqueName("e2e-unpub-ver-nocmd-owner"),
      "pass1234",
    );
    await publishPackage(token, name, "1.0.0");

    const res = await api(
      `/npm/${name}/-/${tarballFilename(name, "1.0.0")}/-rev/1`,
      { method: "DELETE", headers: bearer(token) },
    );
    expect(res.status).toBe(400);
  });

  it("returns 403 when caller is not a maintainer", async () => {
    const name = uniqueName("e2e-unpub-ver-forbidden");
    const owner = await login(
      uniqueName("e2e-unpub-ver-owner-b"),
      "pass1234",
    );
    const other = await login(
      uniqueName("e2e-unpub-ver-other"),
      "pass1234",
    );
    await publishPackage(owner, name, "1.0.0");

    const res = await unpublishVersion(other, name, "1.0.0");
    expect(res.status).toBe(403);
  });

  it("returns 404 for non-existent version", async () => {
    const name = uniqueName("e2e-unpub-ver-missing");
    const token = await login(
      uniqueName("e2e-unpub-ver-missing-owner"),
      "pass1234",
    );
    await publishPackage(token, name, "1.0.0");

    const res = await unpublishVersion(token, name, "9.9.9");
    expect(res.status).toBe(404);
  });
});
