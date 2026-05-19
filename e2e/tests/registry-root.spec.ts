import { describe, expect, it } from "vitest";
import { apiJson } from "../helpers.js";

describe("GET /npm/", () => {
  it("returns registry info", async () => {
    const { res, body } = await apiJson("/npm");
    expect(res.status).toBe(200);
    expect(body).toMatchObject({
      db_name: "registry",
    });
    expect(typeof body.doc_count).toBe("number");
  });
});
