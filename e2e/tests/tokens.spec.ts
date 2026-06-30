import { describe, expect, it } from "vitest";
import { api, apiJson, login, uniqueName } from "../helpers.js";

const PASSWORD = "pass1234";

function bearer(token: string): Record<string, string> {
  return { authorization: `Bearer ${token}` };
}

function jsonHeaders(token?: string): Record<string, string> {
  return { "content-type": "application/json", ...(token ? bearer(token) : {}) };
}

describe("GET /npm/-/whoami", () => {
  it("returns the authenticated username", async () => {
    const name = uniqueName("e2e-whoami");
    const token = await login(name, PASSWORD);

    const { res, body } = await apiJson<{ username: string }>(
      "/npm/-/whoami",
      { headers: bearer(token) },
    );
    expect(res.status).toBe(200);
    expect(body.username).toBe(name);
  });

  it("returns 401 without a token", async () => {
    const res = await api("/npm/-/whoami");
    expect(res.status).toBe(401);
  });

  it("returns 401 with an invalid token", async () => {
    const res = await api("/npm/-/whoami", { headers: bearer("not-a-real-token") });
    expect(res.status).toBe(401);
  });
});

describe("GET /npm/-/npm/v1/tokens", () => {
  it("lists tokens with the npm envelope shape", async () => {
    const name = uniqueName("e2e-token-list");
    const token = await login(name, PASSWORD);

    const { res, body } = await apiJson<{
      objects: Array<{ token: string; key: string; readonly: boolean; cidr_whitelist: string[]; created: string; updated: string }>;
      total: number;
      urls: object;
    }>("/npm/-/npm/v1/tokens", { headers: bearer(token) });

    expect(res.status).toBe(200);
    expect(body.total).toBeGreaterThanOrEqual(1);
    expect(body.urls).toEqual({});
    expect(Array.isArray(body.objects)).toBe(true);
    expect(body.objects.length).toBe(body.total);

    const obj = body.objects[0];
    expect(typeof obj.key).toBe("string");
    expect(obj.key).toHaveLength(64);
    expect(obj.token.endsWith("...")).toBe(true);
    expect(obj.readonly).toBe(false);
    expect(Array.isArray(obj.cidr_whitelist)).toBe(true);
    expect(typeof obj.created).toBe("string");
    expect(typeof obj.updated).toBe("string");
  });

  it("returns 401 without a token", async () => {
    const res = await api("/npm/-/npm/v1/tokens");
    expect(res.status).toBe(401);
  });
});

describe("POST /npm/-/npm/v1/tokens (create)", () => {
  it("creates a token and returns the plaintext secret", async () => {
    const name = uniqueName("e2e-token-create");
    const token = await login(name, PASSWORD);

    const { res, body } = await apiJson<{
      token: string;
      key: string;
      readonly: boolean;
      cidr_whitelist: string[];
      created: string;
      updated: string;
    }>("/npm/-/npm/v1/tokens", {
      method: "POST",
      headers: jsonHeaders(token),
      body: JSON.stringify({
        password: PASSWORD,
        readonly: true,
        cidr_whitelist: ["10.0.0.0/8"],
      }),
    });

    expect(res.status).toBe(200);
    expect(typeof body.token).toBe("string");
    expect(body.token.length).toBeGreaterThan(0);
    expect(body.key).toHaveLength(64);
    expect(body.readonly).toBe(true);
    expect(body.cidr_whitelist).toEqual(["10.0.0.0/8"]);
    expect(typeof body.created).toBe("string");

    const { body: list } = await apiJson<{ objects: Array<{ key: string }> }>(
      "/npm/-/npm/v1/tokens",
      { headers: bearer(token) },
    );
    expect(list.objects.map((o) => o.key)).toContain(body.key);
  });

  it("creates a token without cidr_whitelist (defaults to empty)", async () => {
    const name = uniqueName("e2e-token-create-nocidr");
    const token = await login(name, PASSWORD);

    const { res, body } = await apiJson<{ cidr_whitelist: string[] }>(
      "/npm/-/npm/v1/tokens",
      {
        method: "POST",
        headers: jsonHeaders(token),
        body: JSON.stringify({ password: PASSWORD }),
      },
    );
    expect(res.status).toBe(200);
    expect(body.cidr_whitelist).toEqual([]);
  });

  it("rejects an invalid password with 401", async () => {
    const name = uniqueName("e2e-token-create-badpw");
    const token = await login(name, PASSWORD);

    const res = await api("/npm/-/npm/v1/tokens", {
      method: "POST",
      headers: jsonHeaders(token),
      body: JSON.stringify({ password: "totally-wrong" }),
    });
    expect(res.status).toBe(401);
  });
});

describe("DELETE /npm/-/npm/v1/tokens/token/{key} (revoke)", () => {
  it("revokes a token by its key", async () => {
    const name = uniqueName("e2e-token-revoke");
    const token = await login(name, PASSWORD);

    const { body: created } = await apiJson<{ key: string }>(
      "/npm/-/npm/v1/tokens",
      {
        method: "POST",
        headers: jsonHeaders(token),
        body: JSON.stringify({ password: PASSWORD }),
      },
    );

    const res = await api(
      `/npm/-/npm/v1/tokens/token/${created.key}`,
      { method: "DELETE", headers: bearer(token) },
    );
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true });

    const { body: list } = await apiJson<{ objects: Array<{ key: string }> }>(
      "/npm/-/npm/v1/tokens",
      { headers: bearer(token) },
    );
    expect(list.objects.map((o) => o.key)).not.toContain(created.key);
  });

  it("returns 404 for an unknown key", async () => {
    const token = await login(uniqueName("e2e-token-revoke-unknown"), PASSWORD);
    const res = await api(
      `/npm/-/npm/v1/tokens/token/${"0".repeat(64)}`,
      { method: "DELETE", headers: bearer(token) },
    );
    expect(res.status).toBe(404);
  });

  it("forbids revoking a token owned by another user", async () => {
    const tokenA = await login(uniqueName("e2e-token-owner-a"), PASSWORD);
    const tokenB = await login(uniqueName("e2e-token-owner-b"), PASSWORD);

    const { body: created } = await apiJson<{ key: string }>(
      "/npm/-/npm/v1/tokens",
      {
        method: "POST",
        headers: jsonHeaders(tokenA),
        body: JSON.stringify({ password: PASSWORD }),
      },
    );

    const res = await api(
      `/npm/-/npm/v1/tokens/token/${created.key}`,
      { method: "DELETE", headers: bearer(tokenB) },
    );
    expect(res.status).toBe(403);
  });
});

describe("DELETE /npm/-/user/token/{token} (logout)", () => {
  it("logs out by revoking the bearer token", async () => {
    const name = uniqueName("e2e-logout");
    const token = await login(name, PASSWORD);

    const res = await api(`/npm/-/user/token/${token}`, {
      method: "DELETE",
      headers: bearer(token),
    });
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true });

    const after = await api("/npm/-/whoami", { headers: bearer(token) });
    expect(after.status).toBe(401);
  });

  it("rejects logout when the path token does not match the bearer", async () => {
    const token = await login(uniqueName("e2e-logout-mismatch"), PASSWORD);

    const res = await api(`/npm/-/user/token/some-other-token-value`, {
      method: "DELETE",
      headers: bearer(token),
    });
    expect(res.status).toBe(400);
  });
});
