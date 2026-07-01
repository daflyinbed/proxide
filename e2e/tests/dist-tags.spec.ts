import { describe, expect, it } from "vitest";
import { api, apiJson, login, packagePath, publishPackage, uniqueName } from "../helpers.js";

const PASSWORD = "pass1234";

function bearer(token: string): Record<string, string> {
  return { authorization: `Bearer ${token}` };
}

function jsonHeaders(token?: string): Record<string, string> {
  return { "content-type": "application/json", ...(token ? bearer(token) : {}) };
}

function distTagsPath(name: string): string {
  return "/npm/-/package/" + encodeURIComponent(name) + "/dist-tags";
}

function distTagPath(name: string, tag: string): string {
  return distTagsPath(name) + "/" + encodeURIComponent(tag);
}

async function createReadonlyToken(token: string): Promise<string> {
  const { body } = await apiJson<{ token: string }>(
    "/npm/-/npm/v1/tokens",
    {
      method: "POST",
      headers: jsonHeaders(token),
      body: JSON.stringify({ password: PASSWORD, readonly: true }),
    },
  );
  return body.token;
}

describe("GET /npm/-/package/{fullname}/dist-tags", () => {
  it("lists dist-tags for a published package", async () => {
    const name = uniqueName("e2e-dt-list");
    const token = await login(uniqueName("e2e-dt-pub"), PASSWORD);
    await publishPackage(token, name, "1.0.0");

    const { res, body } = await apiJson<Record<string, string>>(distTagsPath(name));
    expect(res.status).toBe(200);
    expect(body.latest).toBe("1.0.0");
  });

  it("returns 404 for a non-existent package", async () => {
    const { res } = await apiJson(distTagsPath(uniqueName("e2e-dt-none")));
    expect(res.status).toBe(404);
  });
});

describe("PUT /npm/-/package/{fullname}/dist-tags/{tag}", () => {
  it("adds a new tag (bare JSON string body)", async () => {
    const name = uniqueName("e2e-dt-add");
    const token = await login(uniqueName("e2e-dt-add-pub"), PASSWORD);
    await publishPackage(token, name, "1.0.0");
    await publishPackage(token, name, "2.0.0");

    const { res, body } = await apiJson<{ ok: boolean }>(distTagPath(name, "beta"), {
      method: "PUT",
      headers: jsonHeaders(token),
      body: JSON.stringify("1.0.0"),
    });
    expect(res.status).toBe(200);
    expect(body.ok).toBe(true);

    const { body: tags } = await apiJson<Record<string, string>>(distTagsPath(name));
    expect(tags.beta).toBe("1.0.0");
    expect(tags.latest).toBe("2.0.0");
  });

  it("reflects the new tag in the packument dist-tags", async () => {
    const name = uniqueName("e2e-dt-packument");
    const token = await login(uniqueName("e2e-dt-packument-pub"), PASSWORD);
    await publishPackage(token, name, "1.0.0");
    await publishPackage(token, name, "2.0.0");

    await apiJson(distTagPath(name, "next"), {
      method: "PUT",
      headers: jsonHeaders(token),
      body: JSON.stringify("1.0.0"),
    });

    const { body: packument } = await apiJson<any>(packagePath(name));
    expect(packument["dist-tags"].next).toBe("1.0.0");
    expect(packument["dist-tags"].latest).toBe("2.0.0");
  });

  it("updates an existing tag to a new version", async () => {
    const name = uniqueName("e2e-dt-update");
    const token = await login(uniqueName("e2e-dt-update-pub"), PASSWORD);
    await publishPackage(token, name, "1.0.0");
    await publishPackage(token, name, "2.0.0");

    await apiJson(distTagPath(name, "beta"), {
      method: "PUT",
      headers: jsonHeaders(token),
      body: JSON.stringify("1.0.0"),
    });

    const { res, body } = await apiJson<{ ok: boolean }>(distTagPath(name, "beta"), {
      method: "PUT",
      headers: jsonHeaders(token),
      body: JSON.stringify("2.0.0"),
    });
    expect(res.status).toBe(200);
    expect(body.ok).toBe(true);

    const { body: tags } = await apiJson<Record<string, string>>(distTagsPath(name));
    expect(tags.beta).toBe("2.0.0");
  });

  it("is idempotent when re-setting the same tag+version", async () => {
    const name = uniqueName("e2e-dt-idem");
    const token = await login(uniqueName("e2e-dt-idem-pub"), PASSWORD);
    await publishPackage(token, name, "1.0.0");

    const { res, body } = await apiJson<{ ok: boolean }>(distTagPath(name, "stable"), {
      method: "PUT",
      headers: jsonHeaders(token),
      body: JSON.stringify("1.0.0"),
    });
    expect(res.status).toBe(200);
    expect(body.ok).toBe(true);

    const { res: res2, body: body2 } = await apiJson<{ ok: boolean }>(distTagPath(name, "stable"), {
      method: "PUT",
      headers: jsonHeaders(token),
      body: JSON.stringify("1.0.0"),
    });
    expect(res2.status).toBe(200);
    expect(body2.ok).toBe(true);
  });

  it("returns 404 for a non-existent version", async () => {
    const name = uniqueName("e2e-dt-badver");
    const token = await login(uniqueName("e2e-dt-badver-pub"), PASSWORD);
    await publishPackage(token, name, "1.0.0");

    const { res } = await apiJson(distTagPath(name, "beta"), {
      method: "PUT",
      headers: jsonHeaders(token),
      body: JSON.stringify("9.9.9"),
    });
    expect(res.status).toBe(404);
  });

  it("returns 400 for an invalid version format", async () => {
    const name = uniqueName("e2e-dt-invalid");
    const token = await login(uniqueName("e2e-dt-invalid-pub"), PASSWORD);
    await publishPackage(token, name, "1.0.0");

    const { res } = await apiJson(distTagPath(name, "beta"), {
      method: "PUT",
      headers: jsonHeaders(token),
      body: JSON.stringify("not-a-version"),
    });
    expect(res.status).toBe(400);
  });

  it("returns 401 without auth", async () => {
    const name = uniqueName("e2e-dt-noauth");
    const token = await login(uniqueName("e2e-dt-noauth-pub"), PASSWORD);
    await publishPackage(token, name, "1.0.0");

    const { res } = await apiJson(distTagPath(name, "beta"), {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify("1.0.0"),
    });
    expect(res.status).toBe(401);
  });

  it("returns 403 with a read-only token", async () => {
    const name = uniqueName("e2e-dt-ro");
    const token = await login(uniqueName("e2e-dt-ro-pub"), PASSWORD);
    const readonlyToken = await createReadonlyToken(token);
    await publishPackage(token, name, "1.0.0");

    const { res } = await apiJson(distTagPath(name, "beta"), {
      method: "PUT",
      headers: jsonHeaders(readonlyToken),
      body: JSON.stringify("1.0.0"),
    });
    expect(res.status).toBe(403);
  });

  it("returns 403 for a non-maintainer", async () => {
    const name = uniqueName("e2e-dt-nomaint");
    const token = await login(uniqueName("e2e-dt-owner"), PASSWORD);
    await publishPackage(token, name, "1.0.0");

    const otherToken = await login(uniqueName("e2e-dt-stranger"), PASSWORD);
    const { res } = await apiJson(distTagPath(name, "beta"), {
      method: "PUT",
      headers: jsonHeaders(otherToken),
      body: JSON.stringify("1.0.0"),
    });
    expect(res.status).toBe(403);
  });

  it("returns 404 for a non-existent package", async () => {
    const token = await login(uniqueName("e2e-dt-ghost-pub"), PASSWORD);
    const { res } = await apiJson(distTagPath(uniqueName("e2e-dt-ghost"), "beta"), {
      method: "PUT",
      headers: jsonHeaders(token),
      body: JSON.stringify("1.0.0"),
    });
    expect(res.status).toBe(404);
  });
});

describe("DELETE /npm/-/package/{fullname}/dist-tags/{tag}", () => {
  it("removes an existing tag", async () => {
    const name = uniqueName("e2e-dt-rm");
    const token = await login(uniqueName("e2e-dt-rm-pub"), PASSWORD);
    await publishPackage(token, name, "1.0.0");
    await publishPackage(token, name, "2.0.0");

    await apiJson(distTagPath(name, "beta"), {
      method: "PUT",
      headers: jsonHeaders(token),
      body: JSON.stringify("1.0.0"),
    });

    const { res, body } = await apiJson<{ ok: boolean }>(distTagPath(name, "beta"), {
      method: "DELETE",
      headers: bearer(token),
    });
    expect(res.status).toBe(200);
    expect(body.ok).toBe(true);

    const { body: tags } = await apiJson<Record<string, string>>(distTagsPath(name));
    expect(tags.beta).toBeUndefined();
    expect(tags.latest).toBe("2.0.0");
  });

  it("reflects the removal in the packument dist-tags", async () => {
    const name = uniqueName("e2e-dt-rm-pack");
    const token = await login(uniqueName("e2e-dt-rm-pack-pub"), PASSWORD);
    await publishPackage(token, name, "1.0.0");
    await publishPackage(token, name, "2.0.0");

    await apiJson(distTagPath(name, "next"), {
      method: "PUT",
      headers: jsonHeaders(token),
      body: JSON.stringify("1.0.0"),
    });

    await apiJson(distTagPath(name, "next"), {
      method: "DELETE",
      headers: bearer(token),
    });

    const { body: packument } = await apiJson<any>(packagePath(name));
    expect(packument["dist-tags"].next).toBeUndefined();
    expect(packument["dist-tags"].latest).toBe("2.0.0");
  });

  it("is idempotent when removing a non-existent tag", async () => {
    const name = uniqueName("e2e-dt-rm-idem");
    const token = await login(uniqueName("e2e-dt-rm-idem-pub"), PASSWORD);
    await publishPackage(token, name, "1.0.0");

    const { res, body } = await apiJson<{ ok: boolean }>(distTagPath(name, "ghost"), {
      method: "DELETE",
      headers: bearer(token),
    });
    expect(res.status).toBe(200);
    expect(body.ok).toBe(true);
  });

  it("returns 403 when removing the latest tag", async () => {
    const name = uniqueName("e2e-dt-rm-latest");
    const token = await login(uniqueName("e2e-dt-rm-latest-pub"), PASSWORD);
    await publishPackage(token, name, "1.0.0");

    const { res, body } = await apiJson<{ error: string }>(distTagPath(name, "latest"), {
      method: "DELETE",
      headers: bearer(token),
    });
    expect(res.status).toBe(403);
    expect(body.error).toContain("latest");
  });

  it("returns 401 without auth", async () => {
    const name = uniqueName("e2e-dt-rm-noauth");
    const token = await login(uniqueName("e2e-dt-rm-noauth-pub"), PASSWORD);
    await publishPackage(token, name, "1.0.0");

    const res = await api(distTagPath(name, "beta"), { method: "DELETE" });
    expect(res.status).toBe(401);
  });

  it("returns 403 with a read-only token", async () => {
    const name = uniqueName("e2e-dt-rm-ro");
    const token = await login(uniqueName("e2e-dt-rm-ro-pub"), PASSWORD);
    const readonlyToken = await createReadonlyToken(token);
    await publishPackage(token, name, "1.0.0");

    const { res } = await apiJson(distTagPath(name, "beta"), {
      method: "DELETE",
      headers: bearer(readonlyToken),
    });
    expect(res.status).toBe(403);
  });

  it("returns 403 for a non-maintainer", async () => {
    const name = uniqueName("e2e-dt-rm-nomaint");
    const token = await login(uniqueName("e2e-dt-rm-owner"), PASSWORD);
    await publishPackage(token, name, "1.0.0");
    await publishPackage(token, name, "2.0.0");
    await apiJson(distTagPath(name, "beta"), {
      method: "PUT",
      headers: jsonHeaders(token),
      body: JSON.stringify("2.0.0"),
    });

    const otherToken = await login(uniqueName("e2e-dt-rm-stranger"), PASSWORD);
    const { res } = await apiJson(distTagPath(name, "beta"), {
      method: "DELETE",
      headers: bearer(otherToken),
    });
    expect(res.status).toBe(403);
  });

  it("returns 404 for a non-existent package", async () => {
    const token = await login(uniqueName("e2e-dt-rm-ghost-pub"), PASSWORD);
    const { res } = await apiJson(distTagPath(uniqueName("e2e-dt-rm-ghost"), "beta"), {
      method: "DELETE",
      headers: bearer(token),
    });
    expect(res.status).toBe(404);
  });
});
