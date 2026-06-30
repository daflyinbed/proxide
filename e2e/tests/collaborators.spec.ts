import { describe, expect, it } from "vitest";
import { apiJson, login, publishPackage, uniqueName, uniqueScopedName } from "../helpers.js";

describe("GET /npm/-/package/{fullname}/collaborators", () => {
  it("lists collaborators for an unscoped package", async () => {
    const name = uniqueName("e2e-collab");
    const publisher = uniqueName("e2e-collab-pub");
    const token = await login(publisher, "pass1234");

    await publishPackage(token, name, "1.0.0");

    const { res, body } = await apiJson(
      "/npm/-/package/" + encodeURIComponent(name) + "/collaborators",
    );
    expect(res.status).toBe(200);
    expect(body[publisher]).toBe("write");
  });

  it("lists collaborators for a scoped package via URL-encoded name", async () => {
    const name = uniqueScopedName("e2e-scope", "e2e-collab");
    const publisher = uniqueName("e2e-scoped-collab-pub");
    const token = await login(publisher, "pass1234");

    await publishPackage(token, name, "1.0.0");

    const { res, body } = await apiJson(
      "/npm/-/package/" + encodeURIComponent(name) + "/collaborators",
    );
    expect(res.status).toBe(200);
    expect(body[publisher]).toBe("write");
  });

  it("returns 403 for non-existent package", async () => {
    const { res } = await apiJson(
      "/npm/-/package/" + encodeURIComponent(uniqueName("e2e-nocollab")) + "/collaborators",
    );
    expect(res.status).toBe(403);
  });
});

describe("GET /npm/-/org/{username}/package", () => {
  it("lists packages for a user", async () => {
    const publisher = uniqueName("e2e-org-pub");
    const token = await login(publisher, "pass1234");
    const name = uniqueName("e2e-org-pkg");

    await publishPackage(token, name, "1.0.0");

    const { res, body } = await apiJson(
      "/npm/-/org/" + encodeURIComponent(publisher) + "/package",
    );
    expect(res.status).toBe(200);
    expect(body[name]).toBe("write");
  });

  it("returns 404 for non-existent user", async () => {
    const { res } = await apiJson(
      "/npm/-/org/" + encodeURIComponent(uniqueName("e2e-nouser")) + "/package",
    );
    expect(res.status).toBe(404);
  });
});
