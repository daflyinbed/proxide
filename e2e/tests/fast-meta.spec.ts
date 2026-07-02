import { describe, expect, it } from "vitest";
import { api, apiJson, login, publishPackage, uniqueName, uniqueScopedName } from "../helpers.js";

async function seedPackage() {
  const name = uniqueName("e2e-fast");
  const version = "1.2.3";
  const token = await login(uniqueName("e2e-fast-pub"), "pass1234");
  await publishPackage(token, name, version);
  return { name, version };
}

async function seedScopedPackage() {
  const name = uniqueScopedName("e2e-scope", "e2e-fast");
  const version = "1.2.3";
  const token = await login(uniqueName("e2e-scoped-fast-pub"), "pass1234");
  await publishPackage(token, name, version, { access: "public" });
  return { name, version };
}

describe("GET /fast/resolve/{pkg}", () => {
  it("resolves latest to published version", async () => {
    const { name, version } = await seedPackage();
    const { res, body } = await apiJson(
      `/fast/resolve/${encodeURIComponent(name)}@latest`,
    );
    expect(res.status).toBe(200);
    expect(body.name).toBe(name);
    expect(body.version).toBe(version);
  });

  it("returns 404 for unknown package", async () => {
    const res = await api(
      `/fast/resolve/${encodeURIComponent("e2e-nonexist-" + Date.now())}@latest`,
    );
    expect(res.status).toBe(404);
  });
});

describe("GET /fast/versions/{pkg}", () => {
  it("lists published versions", async () => {
    const { name, version } = await seedPackage();
    const { res, body } = await apiJson(
      `/fast/versions/${encodeURIComponent(name)}`,
    );
    expect(res.status).toBe(200);
    expect(body.name).toBe(name);
    expect(body.versions).toContain(version);
  });
});

describe("GET /fast/full/{pkg}", () => {
  it("returns full metadata", async () => {
    const { name, version } = await seedPackage();
    const { res, body } = await apiJson(
      `/fast/full/${encodeURIComponent(name)}`,
    );
    expect(res.status).toBe(200);
    expect(body.name).toBe(name);
    expect(body.dist_tags.latest).toBe(version);
    expect(body.versions_meta).toBeDefined();
    expect(body.versions_meta[version]).toBeDefined();
  });
});

describe("scoped package fast-meta", () => {
  it("resolves latest for scoped package", async () => {
    const { name, version } = await seedScopedPackage();
    const { res, body } = await apiJson(
      `/fast/resolve/${encodeURIComponent(name)}@latest`,
    );
    expect(res.status).toBe(200);
    expect(body.name).toBe(name);
    expect(body.version).toBe(version);
  });

  it("lists versions for scoped package", async () => {
    const { name, version } = await seedScopedPackage();
    const { res, body } = await apiJson(
      `/fast/versions/${encodeURIComponent(name)}`,
    );
    expect(res.status).toBe(200);
    expect(body.name).toBe(name);
    expect(body.versions).toContain(version);
  });

  it("returns full metadata for scoped package", async () => {
    const { name, version } = await seedScopedPackage();
    const { res, body } = await apiJson(
      `/fast/full/${encodeURIComponent(name)}`,
    );
    expect(res.status).toBe(200);
    expect(body.name).toBe(name);
    expect(body.dist_tags.latest).toBe(version);
    expect(body.versions_meta).toBeDefined();
    expect(body.versions_meta[version]).toBeDefined();
  });
});
