import { describe, expect, it } from "vitest";
import {
  api,
  apiJson,
  bearer,
  login,
  packagePath,
  publishPackage,
  runOrgAddMember,
  runOrgCreate,
  uniqueName,
  uniqueScopedName,
} from "../helpers.js";

function rosterPath(org: string): string {
  return "/npm/-/org/" + encodeURIComponent(org) + "/user";
}

function orgPackagesPath(org: string): string {
  return "/npm/-/org/" + encodeURIComponent(org) + "/package";
}

function teamPath(scope: string, team: string): string {
  return "/npm/-/team/" + encodeURIComponent(scope) + "/" + encodeURIComponent(team);
}

function teamUserPath(scope: string, team: string): string {
  return teamPath(scope, team) + "/user";
}

function teamPackagePath(scope: string, team: string): string {
  return teamPath(scope, team) + "/package";
}

function teamsInOrgPath(scope: string): string {
  return "/npm/-/org/" + encodeURIComponent(scope) + "/team";
}

function collaboratorsPath(name: string): string {
  return "/npm/-/package/" + encodeURIComponent(name) + "/collaborators";
}

function putJson(path: string, token: string, body: unknown): Promise<Response> {
  return api(path, {
    method: "PUT",
    headers: { "content-type": "application/json", ...bearer(token) },
    body: JSON.stringify(body),
  });
}

function deleteJson(path: string, token: string, body: unknown): Promise<Response> {
  return api(path, {
    method: "DELETE",
    headers: { "content-type": "application/json", ...bearer(token) },
    body: JSON.stringify(body),
  });
}

describe("proxide org create (CLI bootstrap)", () => {
  it("creates an org with developers team and owner", async () => {
    const owner = uniqueName("e2e-org-cli-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-cli");
    await runOrgCreate(scope, owner);

    const { res, body } = await apiJson<Record<string, string>>(rosterPath(scope));
    expect(res.status).toBe(200);
    expect(body[owner]).toBe("owner");

    const teamsRes = await apiJson<string[]>(`${teamsInOrgPath(scope)}?format=cli`);
    expect(teamsRes.res.status).toBe(200);
    expect(teamsRes.body).toContain(`${scope}:developers`);

    const devUsersRes = await apiJson<string[]>(
      `${teamUserPath(scope, "developers")}?format=cli`,
    );
    expect(devUsersRes.res.status).toBe(200);
    expect(devUsersRes.body).toContain(owner);
  });

  it("rejects creating an org for a non-existent owner", async () => {
    const scope = uniqueName("e2e-org-noowner");
    await expect(runOrgCreate(scope, uniqueName("e2e-ghost-user"))).rejects.toThrow();
  });
});

describe("PUT /-/org/{org}/user (org set member)", () => {
  it("adds a member with default developer role and auto-adds to developers team", async () => {
    const owner = uniqueName("e2e-set-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-set");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const member = uniqueName("e2e-set-member");
    await login(member, "pass1234");

    const res = await putJson(rosterPath(scope), ownerToken, { user: member });
    expect(res.status).toBe(201);
    const body = await res.json();
    expect(body.org.name).toBe(scope);
    expect(body.org.size).toBeGreaterThanOrEqual(2);
    expect(body.user).toBe(member);
    expect(body.role).toBe("developer");

    const { body: roster } = await apiJson<Record<string, string>>(rosterPath(scope));
    expect(roster[member]).toBe("developer");

    const { body: devUsers } = await apiJson<string[]>(
      `${teamUserPath(scope, "developers")}?format=cli`,
    );
    expect(devUsers).toContain(member);
  });

  it("sets an explicit admin role", async () => {
    const owner = uniqueName("e2e-set-admin-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-admin");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const member = uniqueName("e2e-set-admin");
    await login(member, "pass1234");

    const res = await putJson(rosterPath(scope), ownerToken, {
      user: member,
      role: "admin",
    });
    expect(res.status).toBe(201);
    const body = await res.json();
    expect(body.role).toBe("admin");
  });

  it("rejects invalid roles", async () => {
    const owner = uniqueName("e2e-set-invalid-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-invalid-role");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;
    const member = uniqueName("e2e-set-invalid");
    await login(member, "pass1234");

    const res = await putJson(rosterPath(scope), ownerToken, {
      user: member,
      role: "overlord",
    });
    expect(res.status).toBe(400);
  });

  it("rejects adding a non-existent user", async () => {
    const owner = uniqueName("e2e-set-ghost-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-ghost");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const res = await putJson(rosterPath(scope), ownerToken, {
      user: uniqueName("e2e-ghost"),
    });
    expect(res.status).toBe(400);
  });

  it("rejects a developer (non-manager) adding members", async () => {
    const owner = uniqueName("e2e-set-dev-block-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-dev-block");
    await runOrgCreate(scope, owner);
    const member = uniqueName("e2e-set-dev-block");
    const memberToken = await login(member, "pass1234");
    await runOrgAddMember(scope, member, "developer");

    const outsider = uniqueName("e2e-set-dev-block-outsider");
    await login(outsider, "pass1234");
    const res = await putJson(rosterPath(scope), memberToken, { user: outsider });
    expect(res.status).toBe(403);
  });

  it("rejects without auth", async () => {
    const scope = uniqueName("e2e-org-noauth");
    const res = await putJson(rosterPath(scope), "fake", { user: "x" });
    expect(res.status).toBe(401);
  });

  it("returns 404 for a non-existent org", async () => {
    const token = await login(uniqueName("e2e-set-noorg-pub"), "pass1234");
    const res = await putJson(rosterPath(uniqueName("e2e-noorg")), token, {
      user: "x",
    });
    expect(res.status).toBe(404);
  });
});

describe("DELETE /-/org/{org}/user (org rm member)", () => {
  it("removes a member and cascades to all teams", async () => {
    const owner = uniqueName("e2e-rm-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-rm");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const member = uniqueName("e2e-rm-member");
    await login(member, "pass1234");
    await runOrgAddMember(scope, member, "developer");

    const { body: devUsersBefore } = await apiJson<string[]>(
      `${teamUserPath(scope, "developers")}?format=cli`,
    );
    expect(devUsersBefore).toContain(member);

    const res = await deleteJson(rosterPath(scope), ownerToken, { user: member });
    expect(res.status).toBe(204);

    const { body: roster } = await apiJson<Record<string, string>>(rosterPath(scope));
    expect(roster[member]).toBeUndefined();

    const { body: devUsersAfter } = await apiJson<string[]>(
      `${teamUserPath(scope, "developers")}?format=cli`,
    );
    expect(devUsersAfter).not.toContain(member);
  });

  it("rejects removing the last owner", async () => {
    const owner = uniqueName("e2e-last-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-last-owner");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const res = await deleteJson(rosterPath(scope), ownerToken, { user: owner });
    expect(res.status).toBe(409);
  });
});

describe("PUT /-/org/{scope}/team (team create)", () => {
  it("creates a custom team", async () => {
    const owner = uniqueName("e2e-team-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-team");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const res = await putJson(teamsInOrgPath(scope), ownerToken, {
      name: "release",
      description: "release team",
    });
    expect(res.status).toBe(201);
    const body = await res.json();
    expect(body.name).toBe("release");
  });

  it("rejects the reserved name 'developers'", async () => {
    const owner = uniqueName("e2e-team-reserved-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-reserved");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const res = await putJson(teamsInOrgPath(scope), ownerToken, { name: "developers" });
    expect(res.status).toBe(400);
  });

  it("rejects uppercase/punctuation names", async () => {
    const owner = uniqueName("e2e-team-case-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-case");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const badNames = ["TeamX", "team x", "team.x"];
    for (const name of badNames) {
      const res = await putJson(teamsInOrgPath(scope), ownerToken, { name });
      expect(res.status).toBe(400);
    }
  });

  it("rejects duplicate team names", async () => {
    const owner = uniqueName("e2e-team-dup-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-dup");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const first = await putJson(teamsInOrgPath(scope), ownerToken, { name: "core" });
    expect(first.status).toBe(201);

    const second = await putJson(teamsInOrgPath(scope), ownerToken, { name: "core" });
    expect(second.status).toBe(409);
  });
});

describe("DELETE /-/team/{scope}/{team} (team destroy)", () => {
  it("destroys a custom team", async () => {
    const owner = uniqueName("e2e-destroy-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-destroy");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    await putJson(teamsInOrgPath(scope), ownerToken, { name: "tmp" });

    const res = await api(teamPath(scope, "tmp"), {
      method: "DELETE",
      headers: bearer(ownerToken),
    });
    expect(res.status).toBe(204);
  });

  it("rejects destroying the developers team", async () => {
    const owner = uniqueName("e2e-destroy-dev-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-destroy-dev");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const res = await api(teamPath(scope, "developers"), {
      method: "DELETE",
      headers: bearer(ownerToken),
    });
    expect(res.status).toBe(400);
  });
});

describe("team members", () => {
  it("adds and removes a user from a custom team", async () => {
    const owner = uniqueName("e2e-tm-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-tm");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const member = uniqueName("e2e-tm-member");
    await login(member, "pass1234");
    await runOrgAddMember(scope, member, "developer");

    await putJson(teamsInOrgPath(scope), ownerToken, { name: "core" });

    const addRes = await putJson(teamUserPath(scope, "core"), ownerToken, { user: member });
    expect(addRes.status).toBe(201);

    const { body: users } = await apiJson<string[]>(
      `${teamUserPath(scope, "core")}?format=cli`,
    );
    expect(users).toContain(member);

    const rmRes = await deleteJson(teamUserPath(scope, "core"), ownerToken, {
      user: member,
    });
    expect(rmRes.status).toBe(204);

    const { body: usersAfter } = await apiJson<string[]>(
      `${teamUserPath(scope, "core")}?format=cli`,
    );
    expect(usersAfter).not.toContain(member);
  });

  it("rejects adding a user to a team when they are not an org member", async () => {
    const owner = uniqueName("e2e-tm-notmem-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-notmem");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    await putJson(teamsInOrgPath(scope), ownerToken, { name: "core" });

    const outsider = uniqueName("e2e-tm-outsider");
    await login(outsider, "pass1234");

    const res = await putJson(teamUserPath(scope, "core"), ownerToken, {
      user: outsider,
    });
    expect(res.status).toBe(403);
  });

  it("rejects adding a non-member to the developers team directly", async () => {
    const owner = uniqueName("e2e-tm-dev-notmem-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-dev-notmem");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const outsider = uniqueName("e2e-tm-dev-outsider");
    await login(outsider, "pass1234");

    const res = await putJson(teamUserPath(scope, "developers"), ownerToken, {
      user: outsider,
    });
    expect(res.status).toBe(403);
  });
});

describe("GET /-/org/{scope}/team?format=cli", () => {
  it("lists all teams as scope:team strings", async () => {
    const owner = uniqueName("e2e-lsteams-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-lsteams");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;
    await putJson(teamsInOrgPath(scope), ownerToken, { name: "alpha" });
    await putJson(teamsInOrgPath(scope), ownerToken, { name: "beta" });

    const { res, body } = await apiJson<string[]>(`${teamsInOrgPath(scope)}?format=cli`);
    expect(res.status).toBe(200);
    expect(body).toContain(`${scope}:developers`);
    expect(body).toContain(`${scope}:alpha`);
    expect(body).toContain(`${scope}:beta`);
  });
});

describe("developers team auto-grant on first publish", () => {
  it("grants developers team write access and overrides maintainers on first publish", async () => {
    const owner = uniqueName("e2e-dev-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-dev-grant");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const secondMember = uniqueName("e2e-dev-second");
    await login(secondMember, "pass1234");
    await runOrgAddMember(scope, secondMember, "developer");

    const pkgName = uniqueScopedName(scope, "auto-grant");
    const { res: pubRes } = await publishPackage(ownerToken, pkgName, "1.0.0", {
      access: "public",
    });
    expect(pubRes.status).toBe(200);

    const { body: collab } = await apiJson<Record<string, string>>(
      collaboratorsPath(pkgName),
      { headers: bearer(ownerToken) },
    );
    expect(collab[owner]).toBe("write");
    expect(collab[secondMember]).toBe("write");

    const { res: teamPkgRes, body: teamPkgBody } = await apiJson<Record<string, string>>(
      teamPackagePath(scope, "developers"),
      { headers: bearer(ownerToken) },
    );
    expect(teamPkgRes.status).toBe(200);
    expect(teamPkgBody[pkgName]).toBe("write");
  });

  it("does not apply developers team auto-grant on second publish (existing package)", async () => {
    const owner = uniqueName("e2e-dev-2nd-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-dev-2nd");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const member2 = uniqueName("e2e-dev-2nd-m2");
    await login(member2, "pass1234");
    await runOrgAddMember(scope, member2, "developer");

    const member3 = uniqueName("e2e-dev-2nd-m3");
    await login(member3, "pass1234");
    await runOrgAddMember(scope, member3, "developer");

    const pkgName = uniqueScopedName(scope, "second-publish");
    await publishPackage(ownerToken, pkgName, "1.0.0", { access: "public" });

    const { body: collabAfterV1 } = await apiJson<Record<string, string>>(
      collaboratorsPath(pkgName),
      { headers: bearer(ownerToken) },
    );
    expect(collabAfterV1[member3]).toBe("write");

    await publishPackage(ownerToken, pkgName, "2.0.0");

    const { body: collabAfterV2 } = await apiJson<Record<string, string>>(
      collaboratorsPath(pkgName),
      { headers: bearer(ownerToken) },
    );
    expect(collabAfterV2[member3]).toBe("write");
  });
});

describe("team-based package permissions", () => {
  it("grants and revokes read/write permission via /-/team/{scope}/{team}/package", async () => {
    const owner = uniqueName("e2e-perm-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-perm");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    await putJson(teamsInOrgPath(scope), ownerToken, { name: "core" });

    const pkgName = uniqueScopedName(scope, "perm-pkg");
    await publishPackage(ownerToken, pkgName, "1.0.0", { access: "public" });

    const grantRes = await putJson(teamPackagePath(scope, "core"), ownerToken, {
      package: pkgName,
      permissions: "read-write",
    });
    expect(grantRes.status).toBe(200);
    const grantBody = await grantRes.json();
    expect(grantBody.ok).toBe(true);

    const { body: teamPkgs } = await apiJson<Record<string, string>>(
      teamPackagePath(scope, "core"),
      { headers: bearer(ownerToken) },
    );
    expect(teamPkgs[pkgName]).toBe("write");

    const revokeRes = await deleteJson(teamPackagePath(scope, "core"), ownerToken, {
      package: pkgName,
    });
    expect(revokeRes.status).toBe(204);

    const { body: teamPkgsAfter } = await apiJson<Record<string, string>>(
      teamPackagePath(scope, "core"),
      { headers: bearer(ownerToken) },
    );
    expect(teamPkgsAfter[pkgName]).toBeUndefined();
  });

  it("accepts read-only permission level", async () => {
    const owner = uniqueName("e2e-perm-ro-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-ro");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;
    await putJson(teamsInOrgPath(scope), ownerToken, { name: "ro" });

    const pkgName = uniqueScopedName(scope, "ro-pkg");
    await publishPackage(ownerToken, pkgName, "1.0.0", { access: "public" });

    const res = await putJson(teamPackagePath(scope, "ro"), ownerToken, {
      package: pkgName,
      permissions: "read-only",
    });
    expect(res.status).toBe(200);

    const { body } = await apiJson<Record<string, string>>(teamPackagePath(scope, "ro"), {
      headers: bearer(ownerToken),
    });
    expect(body[pkgName]).toBe("read");
  });

  it("rejects cross-org grant (package scope != team scope)", async () => {
    const owner = uniqueName("e2e-cross-owner");
    await login(owner, "pass1234");
    const scopeA = uniqueName("e2e-org-cross-a");
    const scopeB = uniqueName("e2e-org-cross-b");
    await runOrgCreate(scopeA, owner);
    await runOrgCreate(scopeB, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;
    await putJson(teamsInOrgPath(scopeB), ownerToken, { name: "core" });

    const pkgName = uniqueScopedName(scopeA, "cross-pkg");
    await publishPackage(ownerToken, pkgName, "1.0.0", { access: "public" });

    const res = await putJson(teamPackagePath(scopeB, "core"), ownerToken, {
      package: pkgName,
      permissions: "read-write",
    });
    expect(res.status).toBe(403);
  });
});

describe("org-scoped private package visibility for team members", () => {
  it("lets a developer read a private org-scoped package via developers team", async () => {
    const owner = uniqueName("e2e-vis-org-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-vis");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const devMember = uniqueName("e2e-vis-dev");
    const devToken = await login(devMember, "pass1234");
    await runOrgAddMember(scope, devMember, "developer");

    const pkgName = uniqueScopedName(scope, "private-pkg");
    await publishPackage(ownerToken, pkgName, "1.0.0", { access: "restricted" });

    const anonRes = await api(packagePath(pkgName));
    expect(anonRes.status).toBe(404);

    const { res: memberRes, body: packument } = await apiJson(packagePath(pkgName), {
      headers: bearer(devToken),
    });
    expect(memberRes.status).toBe(200);
    expect(packument.name).toBe(pkgName);
  });
});

describe("GET /-/org/{org}/package (org packages list)", () => {
  it("returns 404 for a non-existent org (triggers CLI fallback)", async () => {
    const scope = uniqueName("e2e-org-missing");
    const { res } = await apiJson(orgPackagesPath(scope));
    expect(res.status).toBe(404);
  });

  it("lists packages in an org viewable by the viewer", async () => {
    const owner = uniqueName("e2e-pkgs-owner");
    await login(owner, "pass1234");
    const scope = uniqueName("e2e-org-pkgs");
    await runOrgCreate(scope, owner);
    const ownerToken = (await login(owner, "pass1234")) as string;

    const pkgName = uniqueScopedName(scope, "listable");
    await publishPackage(ownerToken, pkgName, "1.0.0", { access: "public" });

    const { res, body } = await apiJson<Record<string, string>>(orgPackagesPath(scope), {
      headers: bearer(ownerToken),
    });
    expect(res.status).toBe(200);
    expect(body[pkgName]).toBeDefined();
  });
});

describe("GET /-/user/{user}/package (user packages list)", () => {
  it("lists packages owned by a user via the user fallback path", async () => {
    const publisher = uniqueName("e2e-user-pkgs-pub");
    const token = await login(publisher, "pass1234");
    const pkgName = uniqueName("e2e-user-pkg");

    await publishPackage(token, pkgName, "1.0.0");

    const { res, body } = await apiJson<Record<string, string>>(
      "/npm/-/user/" + encodeURIComponent(publisher) + "/package",
    );
    expect(res.status).toBe(200);
    expect(body[pkgName]).toBe("write");
  });
});
