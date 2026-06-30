import { describe, expect, it } from "vitest";
import { api, apiJson, login, uniqueName } from "../helpers.js";

const PASSWORD = "pass1234";

function bearer(token: string): Record<string, string> {
  return { authorization: `Bearer ${token}` };
}

function jsonHeaders(token?: string): Record<string, string> {
  return { "content-type": "application/json", ...(token ? bearer(token) : {}) };
}

describe("GET /npm/-/npm/v1/user", () => {
  it("returns the profile of the authenticated user", async () => {
    const name = uniqueName("e2e-profile");
    const token = await login(name, PASSWORD);

    const { res, body } = await apiJson<{
      name: string;
      email?: string;
      email_verified: boolean;
      created: string;
      updated: string;
    }>("/npm/-/npm/v1/user", { headers: bearer(token) });

    expect(res.status).toBe(200);
    expect(body.name).toBe(name);
    expect(body.email_verified).toBe(false);
    expect(typeof body.created).toBe("string");
    expect(typeof body.updated).toBe("string");
  });

  it("returns 401 without a token", async () => {
    const res = await api("/npm/-/npm/v1/user");
    expect(res.status).toBe(401);
  });

  it("returns 401 with an invalid token", async () => {
    const res = await api("/npm/-/npm/v1/user", { headers: bearer("not-a-real-token") });
    expect(res.status).toBe(401);
  });
});

describe("POST /npm/-/npm/v1/user", () => {
  it("returns 403 for an authenticated user (profile updates not allowed)", async () => {
    const name = uniqueName("e2e-profile-update");
    const token = await login(name, PASSWORD);

    const res = await api("/npm/-/npm/v1/user", {
      method: "POST",
      headers: jsonHeaders(token),
      body: JSON.stringify({ name, password: PASSWORD }),
    });
    expect(res.status).toBe(403);
  });
});
