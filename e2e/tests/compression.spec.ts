import { describe, expect, it } from "vitest";
import { execFileSync } from "node:child_process";
import { api, apiJson, login, packagePath, publishPackage, uniqueName } from "../helpers.js";

const DB_NAME = "proxide_e2e";

function runMysql(sql: string): string {
  return execFileSync(
    "docker",
    ["exec", "proxide-mysql", "mysql", "-uroot", "-proot", DB_NAME, "-Nse", sql],
    { encoding: "utf8" },
  ).trim();
}

describe("zstd storage compression", () => {
  it("compresses JSON manifests with .zst suffix and reads them back transparently", async () => {
    const name = uniqueName("e2e-zstd");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-zstd-user"), "pass1234");

    const { res: pubRes } = await publishPackage(token, name, version, {
      description: "zstd compression test",
    });
    expect(pubRes.status).toBe(200);

    const { res: fullRes, body: packument } = await apiJson(packagePath(name));
    expect(fullRes.status).toBe(200);
    expect(packument.name).toBe(name);
    expect(packument.versions[version]).toBeDefined();

    const abbrevRes = await api(packagePath(name), {
      headers: { accept: "application/vnd.npm.install-v1+json" },
    });
    expect(abbrevRes.status).toBe(200);
    const abbrevBody = await abbrevRes.json();
    expect(abbrevBody.name).toBe(name);
    expect(abbrevBody.versions[version]).toBeDefined();

    const { res: verRes, body: verBody } = await apiJson(
      `${packagePath(name)}/${version}`,
    );
    expect(verRes.status).toBe(200);
    expect(verBody.name).toBe(name);
    expect(verBody.version).toBe(version);

    const paths = runMysql(
      `SELECT path FROM dists WHERE name LIKE '%${name}%'`,
    );
    const allPaths = paths.split("\n").map((l) => l.trim()).filter(Boolean);
    expect(allPaths.length).toBeGreaterThan(0);

    const jsonPaths = allPaths.filter((p) => p.endsWith(".json.zst") || p.endsWith(".md.zst"));
    const tgzPaths = allPaths.filter((p) => p.endsWith(".tgz"));
    expect(jsonPaths.length).toBeGreaterThan(0);
    expect(tgzPaths.length).toBe(1);
    expect(
      allPaths.some((p) => p.includes(`/${version}/package.json`)),
    ).toBe(false);
    expect(
      allPaths.some((p) => p.includes(`/${version}/abbreviated.json`)),
    ).toBe(false);
    for (const p of allPaths) {
      if (p.endsWith(".tgz")) {
        expect(p.endsWith(".zst")).toBe(false);
      } else {
        expect(p.endsWith(".zst")).toBe(true);
      }
    }
  });

  it("serves tarballs uncompressed", async () => {
    const name = uniqueName("e2e-zstd-tar");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-zstd-tar-user"), "pass1234");

    await publishPackage(token, name, version);

    const { body: packument } = await apiJson(packagePath(name));
    const tarballUrl = packument.versions[version].dist.tarball as string;
    const tarballPath = tarballUrl.replace("http://localhost:14873", "");

    const res = await api(tarballPath);
    expect(res.status).toBe(200);
    const buf = Buffer.from(await res.arrayBuffer());
    expect(buf.length).toBeGreaterThan(0);
  });
});
