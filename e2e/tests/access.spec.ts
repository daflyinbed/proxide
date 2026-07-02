import { describe, expect, it } from "vitest";
import { api, apiJson, login, publishPackage, uniqueName, uniqueScopedName } from "../helpers.js";

function visibilityPath(name: string): string {
  return "/npm/-/package/" + encodeURIComponent(name) + "/visibility";
}

function accessPath(name: string): string {
  return "/npm/-/package/" + encodeURIComponent(name) + "/access";
}

describe("GET /npm/-/package/{fullname}/visibility", () => {
  it("reports an unscoped package as public", async () => {
    const name = uniqueName("e2e-vis-unscoped");
    const token = await login(uniqueName("e2e-vis-unscoped-pub"), "pass1234");
    await publishPackage(token, name, "1.0.0");

    const { res, body } = await apiJson(visibilityPath(name));
    expect(res.status).toBe(200);
    expect(body.public).toBe(true);
  });

  it("defaults a scoped package to private when no access is given", async () => {
    const name = uniqueScopedName("e2e-vis", "scoped-private");
    const token = await login(uniqueName("e2e-vis-private-pub"), "pass1234");
    await publishPackage(token, name, "1.0.0");

    const anonymous = await apiJson(visibilityPath(name));
    expect(anonymous.res.status).toBe(404);

    const { res, body } = await apiJson(visibilityPath(name), {
      headers: { authorization: `Bearer ${token}` },
    });
    expect(res.status).toBe(200);
    expect(body.public).toBe(false);
  });

  it("reports a scoped package as public when published with access=public", async () => {
    const name = uniqueScopedName("e2e-vis", "scoped-public");
    const token = await login(uniqueName("e2e-vis-public-pub"), "pass1234");
    await publishPackage(token, name, "1.0.0", { access: "public" });

    const { res, body } = await apiJson(visibilityPath(name));
    expect(res.status).toBe(200);
    expect(body.public).toBe(true);
  });

  it("returns 404 for a non-existent package", async () => {
    const { res } = await apiJson(visibilityPath(uniqueName("e2e-vis-none")));
    expect(res.status).toBe(404);
  });
});

describe("GET /npm/-/package/{fullname}/visibility — access control for restricted packages", () => {
  it("returns 404 for a restricted package without auth", async () => {
    const name = uniqueScopedName("e2e-vis", "noauth-restricted");
    const token = await login(uniqueName("e2e-vis-noauth-pub"), "pass1234");
    await publishPackage(token, name, "1.0.0");

    const { res } = await apiJson(visibilityPath(name));
    expect(res.status).toBe(404);
  });

  it("returns 404 for a restricted package with a non-maintainer token", async () => {
    const name = uniqueScopedName("e2e-vis", "nonmaintainer-restricted");
    const owner = await login(uniqueName("e2e-vis-owner"), "pass1234");
    await publishPackage(owner, name, "1.0.0");

    const other = await login(uniqueName("e2e-vis-other"), "pass1234");
    const { res } = await apiJson(visibilityPath(name), {
      headers: { authorization: `Bearer ${other}` },
    });
    expect(res.status).toBe(404);
  });

  it("allows the publishing maintainer to read a restricted package's visibility", async () => {
    const name = uniqueScopedName("e2e-vis", "maintainer-restricted");
    const token = await login(uniqueName("e2e-vis-maintainer-pub"), "pass1234");
    await publishPackage(token, name, "1.0.0");

    const { res, body } = await apiJson(visibilityPath(name), {
      headers: { authorization: `Bearer ${token}` },
    });
    expect(res.status).toBe(200);
    expect(body.public).toBe(false);
  });
});

describe("POST /npm/-/package/{fullname}/access", () => {
  it("sets a scoped package public then private", async () => {
    const name = uniqueScopedName("e2e-access", "toggle");
    const token = await login(uniqueName("e2e-access-toggle-pub"), "pass1234");
    await publishPackage(token, name, "1.0.0");

    const setPublic = await api(accessPath(name), {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ access: "public" }),
    });
    expect(setPublic.status).toBe(200);
    expect((await setPublic.json()).ok).toBe(true);

    const visPub = await apiJson(visibilityPath(name));
    expect(visPub.body.public).toBe(true);

    const setPrivate = await api(accessPath(name), {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ access: "private" }),
    });
    expect(setPrivate.status).toBe(200);

    const visPriv = await apiJson(visibilityPath(name), {
      headers: { authorization: `Bearer ${token}` },
    });
    expect(visPriv.body.public).toBe(false);
  });

  it("accepts 'restricted' as an alias for 'private'", async () => {
    const name = uniqueScopedName("e2e-access", "restricted");
    const token = await login(uniqueName("e2e-access-restricted-pub"), "pass1234");
    await publishPackage(token, name, "1.0.0", { access: "public" });

    const res = await api(accessPath(name), {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ access: "restricted" }),
    });
    expect(res.status).toBe(200);

    const { body } = await apiJson(visibilityPath(name), {
      headers: { authorization: `Bearer ${token}` },
    });
    expect(body.public).toBe(false);
  });

  it("returns 400 for an invalid access value", async () => {
    const name = uniqueScopedName("e2e-access", "invalid");
    const token = await login(uniqueName("e2e-access-invalid-pub"), "pass1234");
    await publishPackage(token, name, "1.0.0");

    const res = await api(accessPath(name), {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ access: "bogus" }),
    });
    expect(res.status).toBe(400);
  });

  it("returns 401 without an auth token", async () => {
    const name = uniqueScopedName("e2e-access", "noauth");
    const token = await login(uniqueName("e2e-access-noauth-pub"), "pass1234");
    await publishPackage(token, name, "1.0.0");

    const res = await api(accessPath(name), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ access: "public" }),
    });
    expect(res.status).toBe(401);
  });

  it("returns 403 for a non-maintainer", async () => {
    const name = uniqueScopedName("e2e-access", "forbidden");
    const publisher = await login(uniqueName("e2e-access-forbidden-owner"), "pass1234");
    await publishPackage(publisher, name, "1.0.0");

    const other = await login(uniqueName("e2e-access-forbidden-other"), "pass1234");

    const res = await api(accessPath(name), {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${other}`,
      },
      body: JSON.stringify({ access: "public" }),
    });
    expect(res.status).toBe(403);
  });

  it("returns 404 for a non-existent package", async () => {
    const token = await login(uniqueName("e2e-access-notfound-pub"), "pass1234");

    const res = await api(accessPath(uniqueScopedName("e2e-access", "missing")), {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ access: "public" }),
    });
    expect(res.status).toBe(404);
  });
});
