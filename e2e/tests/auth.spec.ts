import { describe, expect, it } from "vitest";
import { api, apiJson, uniqueName } from "../helpers.js";

describe("PUT /npm/-/user/org.couchdb.user:{name} (legacy login)", () => {
  it("creates a new user and returns a token", async () => {
    const name = uniqueName("e2e-user");
    const { res, body } = await apiJson<{ ok: boolean; token: string }>(
      `/npm/-/user/org.couchdb.user:${encodeURIComponent(name)}`,
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          name,
          password: "test-password-123",
          email: `${name}@e2e.test`,
        }),
      },
    );
    expect(res.status).toBe(201);
    expect(body.ok).toBe(true);
    expect(typeof body.token).toBe("string");
    expect(body.token.length).toBeGreaterThan(0);
  });

  it("logs in existing user and returns a new token", async () => {
    const name = uniqueName("e2e-user-relogin");
    const password = "test-password-456";

    await apiJson(
      `/npm/-/user/org.couchdb.user:${encodeURIComponent(name)}`,
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ name, password, email: `${name}@e2e.test` }),
      },
    );

    const { res, body } = await apiJson<{ ok: boolean; token: string }>(
      `/npm/-/user/org.couchdb.user:${encodeURIComponent(name)}`,
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ name, password }),
      },
    );
    expect(res.status).toBe(201);
    expect(body.ok).toBe(true);
    expect(typeof body.token).toBe("string");
  });

  it("returns 400 when name in URL does not match body", async () => {
    const res = await api(
      `/npm/-/user/org.couchdb.user:${encodeURIComponent("user-a")}`,
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ name: "user-b", password: "pw" }),
      },
    );
    expect(res.status).toBe(400);
  });

  it("returns 400 when password is empty", async () => {
    const name = uniqueName("e2e-user-nopw");
    const res = await api(
      `/npm/-/user/org.couchdb.user:${encodeURIComponent(name)}`,
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ name, password: "" }),
      },
    );
    expect(res.status).toBe(400);
  });
});
