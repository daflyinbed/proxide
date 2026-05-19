import { describe, expect, it } from "vitest";
import { apiJson, uniqueName, uniqueScopedName } from "../helpers.js";

describe("PUT /npm/-/package/{fullname}/syncs", () => {
  it("enqueues a sync task", async () => {
    const name = uniqueName("e2e-sync");
    const { res, body } = await apiJson("/npm/-/package/" + encodeURIComponent(name) + "/syncs", {
      method: "PUT",
    });
    expect(res.status).toBe(200);
    expect(body.ok).toBe(true);
    expect(body.log).toBe("queued");
  });

  it("returns already queued for duplicate sync", async () => {
    const name = uniqueName("e2e-sync-dup");
    await apiJson("/npm/-/package/" + encodeURIComponent(name) + "/syncs", {
      method: "PUT",
    });
    const { res, body } = await apiJson(
      "/npm/-/package/" + encodeURIComponent(name) + "/syncs",
      { method: "PUT" },
    );
    expect(res.status).toBe(200);
    expect(body.ok).toBe(true);
    expect(body.log).toBe("already queued");
  });

  it("enqueues a sync task for a scoped package", async () => {
    const name = uniqueScopedName("e2e-scope", "e2e-sync");
    const { res, body } = await apiJson("/npm/-/package/" + encodeURIComponent(name) + "/syncs", {
      method: "PUT",
    });
    expect(res.status).toBe(200);
    expect(body.ok).toBe(true);
    expect(body.log).toBe("queued");
  });

  it("returns already queued for duplicate scoped sync", async () => {
    const name = uniqueScopedName("e2e-scope", "e2e-sync-dup");
    await apiJson("/npm/-/package/" + encodeURIComponent(name) + "/syncs", {
      method: "PUT",
    });
    const { res, body } = await apiJson(
      "/npm/-/package/" + encodeURIComponent(name) + "/syncs",
      { method: "PUT" },
    );
    expect(res.status).toBe(200);
    expect(body.ok).toBe(true);
    expect(body.log).toBe("already queued");
  });
});
