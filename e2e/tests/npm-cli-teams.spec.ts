import { describe, expect, it } from "vitest";
import {
  execNpm,
  createAuthDir,
  createTempPackageDir,
  writeNpmrc,
  npmrcEnv,
} from "../npm-cli-helpers.js";
import {
  apiJson,
  bearer,
  login,
  publishPackage,
  runOrgAddMember,
  runOrgCreate,
  uniqueName,
  uniqueScopedName,
} from "../helpers.js";

function rosterPath(org: string): string {
  return "/npm/-/org/" + encodeURIComponent(org) + "/user";
}

function teamsInOrgPath(scope: string): string {
  return "/npm/-/org/" + encodeURIComponent(scope) + "/team";
}

function teamUserPath(scope: string, team: string): string {
  return "/npm/-/team/" + encodeURIComponent(scope) + "/" + encodeURIComponent(team) + "/user";
}

function teamPackagePath(scope: string, team: string): string {
  return "/npm/-/team/" + encodeURIComponent(scope) + "/" + encodeURIComponent(team) + "/package";
}

function collaboratorsPath(name: string): string {
  return "/npm/-/package/" + encodeURIComponent(name) + "/collaborators";
}

async function setupOrgOwner(
  ownerPrefix: string,
  scopePrefix: string,
): Promise<{ token: string; authDir: string; scope: string; owner: string }> {
  const owner = uniqueName(ownerPrefix);
  const token = await login(owner, "pass1234");
  const scope = uniqueName(scopePrefix);
  await runOrgCreate(scope, owner);
  const authDir = await createAuthDir();
  await writeNpmrc(authDir, token);
  return { token, authDir, scope, owner };
}

async function registerMember(
  memberPrefix: string,
  scope: string,
  role?: string,
): Promise<{ token: string; authDir: string; member: string }> {
  const member = uniqueName(memberPrefix);
  const token = await login(member, "pass1234");
  await runOrgAddMember(scope, member, role);
  const authDir = await createAuthDir();
  await writeNpmrc(authDir, token);
  return { token, authDir, member };
}

describe("npm team create (CLI)", () => {
  it("creates a custom team via npm CLI", async () => {
    const { token, authDir, scope } = await setupOrgOwner(
      "e2e-cli-team-create-owner",
      "e2e-cli-team-create",
    );

    const res = await execNpm(["team", "create", `${scope}:core`], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });
    expect(res.exitCode).toBe(0);

    const { res: apiRes, body } = await apiJson<Record<string, string>>(
      `${teamsInOrgPath(scope)}?format=cli`,
      { headers: bearer(token) },
    );
    expect(apiRes.status).toBe(200);
    expect(body).toContain(`${scope}:core`);
    expect(body).toContain(`${scope}:developers`);
  });

  it("fails when creating the reserved 'developers' team", async () => {
    const { authDir, scope } = await setupOrgOwner(
      "e2e-cli-team-rs-owner",
      "e2e-cli-team-rs",
    );

    const res = await execNpm(["team", "create", `${scope}:developers`], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });
    expect(res.exitCode).not.toBe(0);
  });

  it("fails without auth", async () => {
    const dir = await createAuthDir();
    const res = await execNpm(
      ["team", "create", `${uniqueName("e2e-noauth")}:core`],
      { cwd: dir },
    );
    expect(res.exitCode).not.toBe(0);
  });
});

describe("npm team ls (CLI)", () => {
  it("lists all teams in an org", async () => {
    const { token, authDir, scope } = await setupOrgOwner(
      "e2e-cli-lsteams-owner",
      "e2e-cli-lsteams",
    );

    await execNpm(["team", "create", `${scope}:alpha`], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });
    await execNpm(["team", "create", `${scope}:beta`], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });

    const res = await execNpm(["team", "ls", scope], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });
    expect(res.exitCode).toBe(0);
    expect(res.stdout).toContain(`${scope}:developers`);
    expect(res.stdout).toContain(`${scope}:alpha`);
    expect(res.stdout).toContain(`${scope}:beta`);
  });

  it("lists members of a specific team", async () => {
    const { token, authDir, scope } = await setupOrgOwner(
      "e2e-cli-lsmem-owner",
      "e2e-cli-lsmem",
    );

    const { member } = await registerMember(
      "e2e-cli-lsmem-m1",
      scope,
      "developer",
    );

    await execNpm(["team", "create", `${scope}:core`], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });
    await execNpm(["team", "add", `${scope}:core`, member], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });

    const res = await execNpm(["team", "ls", `${scope}:core`], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });
    expect(res.exitCode).toBe(0);
    expect(res.stdout).toContain(member);

    const { body: devUsers } = await apiJson<string[]>(
      `${teamUserPath(scope, "core")}?format=cli`,
      { headers: bearer(token) },
    );
    expect(devUsers).toContain(member);
  });
});

describe("npm team add (CLI)", () => {
  it("adds an org member to a team via npm CLI", async () => {
    const { token, authDir, scope } = await setupOrgOwner(
      "e2e-cli-add-owner",
      "e2e-cli-add",
    );

    const { member } = await registerMember(
      "e2e-cli-add-member",
      scope,
      "developer",
    );

    await execNpm(["team", "create", `${scope}:core`], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });

    const res = await execNpm(["team", "add", `${scope}:core`, member], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });
    expect(res.exitCode).toBe(0);

    const { body: users } = await apiJson<string[]>(
      `${teamUserPath(scope, "core")}?format=cli`,
      { headers: bearer(token) },
    );
    expect(users).toContain(member);
  });

  it("fails when adding a non-org-member", async () => {
    const { authDir, scope } = await setupOrgOwner(
      "e2e-cli-add-nm-owner",
      "e2e-cli-add-nm",
    );

    const outsider = uniqueName("e2e-cli-outsider");
    await login(outsider, "pass1234");

    await execNpm(["team", "create", `${scope}:core`], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });

    const res = await execNpm(["team", "add", `${scope}:core`, outsider], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });
    expect(res.exitCode).not.toBe(0);
  });
});

describe("npm team rm (CLI)", () => {
  it("removes a member from a team via npm CLI", async () => {
    const { token, authDir, scope } = await setupOrgOwner(
      "e2e-cli-rm-owner",
      "e2e-cli-rm",
    );

    const { member } = await registerMember(
      "e2e-cli-rm-member",
      scope,
      "developer",
    );

    await execNpm(["team", "create", `${scope}:core`], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });
    await execNpm(["team", "add", `${scope}:core`, member], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });

    const res = await execNpm(["team", "rm", `${scope}:core`, member], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });
    expect(res.exitCode).toBe(0);

    const { body: users } = await apiJson<string[]>(
      `${teamUserPath(scope, "core")}?format=cli`,
      { headers: bearer(token) },
    );
    expect(users).not.toContain(member);
  });
});

describe("npm team destroy (CLI)", () => {
  it("destroys a custom team via npm CLI", async () => {
    const { token, authDir, scope } = await setupOrgOwner(
      "e2e-cli-destroy-owner",
      "e2e-cli-destroy",
    );

    await execNpm(["team", "create", `${scope}:tmp`], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });

    const res = await execNpm(["team", "destroy", `${scope}:tmp`], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });
    expect(res.exitCode).toBe(0);

    const { body: teams } = await apiJson<string[]>(
      `${teamsInOrgPath(scope)}?format=cli`,
      { headers: bearer(token) },
    );
    expect(teams).not.toContain(`${scope}:tmp`);
    expect(teams).toContain(`${scope}:developers`);
  });

  it("fails when destroying the developers team", async () => {
    const { authDir, scope } = await setupOrgOwner(
      "e2e-cli-destroy-dev-owner",
      "e2e-cli-destroy-dev",
    );

    const res = await execNpm(["team", "destroy", `${scope}:developers`], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });
    expect(res.exitCode).not.toBe(0);
  });
});

describe("npm access grant (CLI)", () => {
  it("grants read-write permission to a team", async () => {
    const { token, authDir, scope } = await setupOrgOwner(
      "e2e-cli-grant-rw-owner",
      "e2e-cli-grant-rw",
    );

    await execNpm(["team", "create", `${scope}:core`], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });

    const pkgName = uniqueScopedName(scope, "grant-rw");
    await publishPackage(token, pkgName, "1.0.0", { access: "public" });

    const res = await execNpm(
      ["access", "grant", "read-write", `${scope}:core`, pkgName],
      { cwd: authDir, env: npmrcEnv(authDir) },
    );
    expect(res.exitCode).toBe(0);

    const { body: teamPkgs } = await apiJson<Record<string, string>>(
      teamPackagePath(scope, "core"),
      { headers: bearer(token) },
    );
    expect(teamPkgs[pkgName]).toBe("write");
  });

  it("grants read-only permission to a team", async () => {
    const { token, authDir, scope } = await setupOrgOwner(
      "e2e-cli-grant-ro-owner",
      "e2e-cli-grant-ro",
    );

    await execNpm(["team", "create", `${scope}:ro`], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });

    const pkgName = uniqueScopedName(scope, "grant-ro");
    await publishPackage(token, pkgName, "1.0.0", { access: "public" });

    const res = await execNpm(
      ["access", "grant", "read-only", `${scope}:ro`, pkgName],
      { cwd: authDir, env: npmrcEnv(authDir) },
    );
    expect(res.exitCode).toBe(0);

    const { body: teamPkgs } = await apiJson<Record<string, string>>(
      teamPackagePath(scope, "ro"),
      { headers: bearer(token) },
    );
    expect(teamPkgs[pkgName]).toBe("read");
  });
});

describe("npm access revoke (CLI)", () => {
  it("revokes team package permission", async () => {
    const { token, authDir, scope } = await setupOrgOwner(
      "e2e-cli-revoke-owner",
      "e2e-cli-revoke",
    );

    await execNpm(["team", "create", `${scope}:core`], {
      cwd: authDir,
      env: npmrcEnv(authDir),
    });

    const pkgName = uniqueScopedName(scope, "revoke-pkg");
    await publishPackage(token, pkgName, "1.0.0", { access: "public" });

    await execNpm(
      ["access", "grant", "read-write", `${scope}:core`, pkgName],
      { cwd: authDir, env: npmrcEnv(authDir) },
    );

    const res = await execNpm(
      ["access", "revoke", `${scope}:core`, pkgName],
      { cwd: authDir, env: npmrcEnv(authDir) },
    );
    expect(res.exitCode).toBe(0);

    const { body: teamPkgs } = await apiJson<Record<string, string>>(
      teamPackagePath(scope, "core"),
      { headers: bearer(token) },
    );
    expect(teamPkgs[pkgName]).toBeUndefined();
  });
});

describe("npm access list collaborators (CLI)", () => {
  it("lists collaborators for a package", async () => {
    const { token, authDir, scope, owner } = await setupOrgOwner(
      "e2e-cli-collab-owner",
      "e2e-cli-collab",
    );

    const pkgName = uniqueScopedName(scope, "collab-pkg");
    await publishPackage(token, pkgName, "1.0.0", { access: "public" });

    const res = await execNpm(
      ["access", "list", "collaborators", pkgName, "--json"],
      { cwd: authDir, env: npmrcEnv(authDir) },
    );
    expect(res.exitCode).toBe(0);

    const parsed = JSON.parse(res.stdout);
    expect(parsed[owner]).toBeDefined();

    const { body: collab } = await apiJson<Record<string, string>>(
      collaboratorsPath(pkgName),
      { headers: bearer(token) },
    );
    expect(collab[owner]).toBe("write");
  });
});

describe("npm CLI end-to-end team workflow", () => {
  it("creates org → creates team → adds member → grants → publishes → member installs", async () => {
    const { token: ownerToken, authDir: ownerAuthDir, scope } = await setupOrgOwner(
      "e2e-cli-e2e-owner",
      "e2e-cli-e2e",
    );

    const { token: devToken, authDir: devAuthDir, member } = await registerMember(
      "e2e-cli-e2e-dev",
      scope,
      "developer",
    );

    await execNpm(["team", "create", `${scope}:release`], {
      cwd: ownerAuthDir,
      env: npmrcEnv(ownerAuthDir),
    });
    await execNpm(["team", "add", `${scope}:release`, member], {
      cwd: ownerAuthDir,
      env: npmrcEnv(ownerAuthDir),
    });

    const pkgName = uniqueScopedName(scope, "e2e-workflow");
    await publishPackage(ownerToken, pkgName, "1.0.0", { access: "restricted" });

    await execNpm(
      ["access", "grant", "read-write", `${scope}:release`, pkgName],
      { cwd: ownerAuthDir, env: npmrcEnv(ownerAuthDir) },
    );

    const pkgDir = await createTempPackageDir(pkgName, "2.0.0");
    await writeNpmrc(pkgDir, devToken);
    const pubRes = await execNpm(["publish", "--access", "restricted"], {
      cwd: pkgDir,
      env: npmrcEnv(devAuthDir),
    });
    expect(pubRes.exitCode).toBe(0);

    const installDir = await createTempPackageDir(pkgName, "2.0.0");
    await writeNpmrc(installDir, devToken);
    const instRes = await execNpm(["install", `${pkgName}@2.0.0`], {
      cwd: installDir,
      env: npmrcEnv(devAuthDir),
    });
    expect(instRes.exitCode).toBe(0);
  });
});
