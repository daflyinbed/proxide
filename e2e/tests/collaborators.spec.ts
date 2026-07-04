import { describe, expect, it } from "vitest";
import {
  api,
  apiJson,
  bearer,
  login,
  publishPackage,
  uniqueName,
  uniqueScopedName,
  updateMaintainers,
} from "../helpers.js";

function collaboratorsPath(name: string): string {
  return "/npm/-/package/" + encodeURIComponent(name) + "/collaborators";
}

async function fetchCollaborators(name: string): Promise<Record<string, string>> {
  const { res, body } = await apiJson<Record<string, string>>(
    collaboratorsPath(name),
  );
  expect(res.status).toBe(200);
  return body;
}

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

    await publishPackage(token, name, "1.0.0", { access: "public" });

    const { res, body } = await apiJson(
      "/npm/-/package/" + encodeURIComponent(name) + "/collaborators",
    );
    expect(res.status).toBe(200);
    expect(body[publisher]).toBe("write");
  });

  it("returns 404 for non-existent package", async () => {
    const { res } = await apiJson(
      "/npm/-/package/" + encodeURIComponent(uniqueName("e2e-nocollab")) + "/collaborators",
    );
    expect(res.status).toBe(404);
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

describe("PUT /npm/{fullname}/-rev/{rev} (npm owner add/rm)", () => {
  it("adds a new maintainer to an unscoped package", async () => {
    const name = uniqueName("e2e-owner-add");
    const ownerName = uniqueName("e2e-owner-add-pub");
    const token = await login(ownerName, "pass1234");
    const newCollabName = uniqueName("e2e-owner-add-new");
    await login(newCollabName, "pass1234");
    await publishPackage(token, name, "1.0.0");

    const { res, body } = await updateMaintainers(token, name, [
      { name: ownerName, email: `${ownerName}@e2e.test` },
      { name: newCollabName, email: `${newCollabName}@e2e.test` },
    ]);
    expect(res.status).toBe(200);
    expect(body.ok).toBe(true);

    const collaborators = await fetchCollaborators(name);
    expect(collaborators[ownerName]).toBe("write");
    expect(collaborators[newCollabName]).toBe("write");
  });

  it("adds a new maintainer to a scoped package", async () => {
    const name = uniqueScopedName("e2e-scope", "e2e-owner-add");
    const ownerName = uniqueName("e2e-scoped-owner-add-pub");
    const token = await login(ownerName, "pass1234");
    const newCollabName = uniqueName("e2e-scoped-owner-add-new");
    await login(newCollabName, "pass1234");
    await publishPackage(token, name, "1.0.0", { access: "public" });

    const { res } = await updateMaintainers(token, name, [
      { name: ownerName },
      { name: newCollabName },
    ]);
    expect(res.status).toBe(200);

    const collaborators = await fetchCollaborators(name);
    expect(collaborators[newCollabName]).toBe("write");
  });

  it("removes a maintainer from a package", async () => {
    const name = uniqueName("e2e-owner-rm");
    const ownerName = uniqueName("e2e-owner-rm-pub");
    const token = await login(ownerName, "pass1234");
    const otherName = uniqueName("e2e-owner-rm-other");
    await login(otherName, "pass1234");
    await publishPackage(token, name, "1.0.0");
    await updateMaintainers(token, name, [
      { name: ownerName },
      { name: otherName },
    ]);

    const { res } = await updateMaintainers(token, name, [
      { name: ownerName },
    ]);
    expect(res.status).toBe(200);

    const collaborators = await fetchCollaborators(name);
    expect(collaborators[otherName]).toBeUndefined();
    expect(collaborators[ownerName]).toBe("write");
  });

  it("grants write access to an added maintainer", async () => {
    const name = uniqueName("e2e-owner-write");
    const ownerName = uniqueName("e2e-owner-write-pub");
    const ownerToken = await login(ownerName, "pass1234");
    const newUserName = uniqueName("e2e-owner-write-new");
    const newUserToken = await login(newUserName, "pass1234");
    await publishPackage(ownerToken, name, "1.0.0");
    await updateMaintainers(ownerToken, name, [
      { name: ownerName },
      { name: newUserName },
    ]);

    const { res } = await publishPackage(newUserToken, name, "2.0.0");
    expect(res.status).toBe(200);
  });

  it("returns 400 without npm-command: owner header", async () => {
    const name = uniqueName("e2e-owner-nocmd");
    const token = await login(uniqueName("e2e-owner-nocmd-pub"), "pass1234");
    await publishPackage(token, name, "1.0.0");

    const res = await api(`/npm/${name}/-rev/1`, {
      method: "PUT",
      headers: { "content-type": "application/json", ...bearer(token) },
      body: JSON.stringify({ maintainers: [{ name: uniqueName("e2e-x") }] }),
    });
    expect(res.status).toBe(400);
  });

  it("returns 400 when maintainers list is empty", async () => {
    const name = uniqueName("e2e-owner-empty");
    const token = await login(uniqueName("e2e-owner-empty-pub"), "pass1234");
    await publishPackage(token, name, "1.0.0");

    const { res } = await updateMaintainers(token, name, []);
    expect(res.status).toBe(400);
  });

  it("returns 400 when a referenced maintainer user does not exist", async () => {
    const name = uniqueName("e2e-owner-ghost");
    const token = await login(uniqueName("e2e-owner-ghost-pub"), "pass1234");
    await publishPackage(token, name, "1.0.0");

    const { res } = await updateMaintainers(token, name, [
      { name: uniqueName("e2e-nonexistent-user") },
    ]);
    expect(res.status).toBe(400);
  });

  it("returns 403 when caller is not a maintainer", async () => {
    const name = uniqueName("e2e-owner-forbidden");
    const ownerToken = await login(
      uniqueName("e2e-owner-forbidden-pub"),
      "pass1234",
    );
    const otherToken = await login(
      uniqueName("e2e-owner-forbidden-other"),
      "pass1234",
    );
    await publishPackage(ownerToken, name, "1.0.0");

    const { res } = await updateMaintainers(otherToken, name, [
      { name: uniqueName("e2e-owner-forbidden-other") },
    ]);
    expect(res.status).toBe(403);
  });

  it("returns 404 for non-existent package", async () => {
    const token = await login(uniqueName("e2e-owner-missing-pub"), "pass1234");
    const { res } = await updateMaintainers(token, uniqueName("e2e-no-pkg"), [
      { name: uniqueName("e2e-owner-missing-pub") },
    ]);
    expect(res.status).toBe(404);
  });
});
