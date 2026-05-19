import { describe, expect, it } from "vitest";
import { api } from "../helpers.js";

describe("GET /-/ping", () => {
  it("returns empty object", async () => {
    const res = await api("/-/ping");
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({});
  });
});
