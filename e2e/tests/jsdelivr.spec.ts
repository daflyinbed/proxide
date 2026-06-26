import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import {
  api,
  apiJson,
  login,
  publishPackage,
  uniqueName,
  uniqueScopedName,
} from "../helpers.js";

function decodeHashLen(hash: string): number {
  return Buffer.from(hash, "base64").length;
}

function findFlat(files: any[], name: string): any {
  return files.find((f) => f.name === name);
}

describe("jsdelivr CDN file serving (unscoped)", () => {
  it("serves an extracted file with correct content-type and body", async () => {
    const name = uniqueName("e2e-jsdelivr");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-jsdelivr-pub"), "pass1234");
    await publishPackage(token, name, version);

    const res = await api(`/jsdelivr/npm/${name}@${version}/index.js`);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toContain("javascript");
    expect(Number(res.headers.get("content-length"))).toBe(0);
    const buf = Buffer.from(await res.arrayBuffer());
    expect(buf.length).toBe(0);
  });

  it("serves package.json with json content-type", async () => {
    const name = uniqueName("e2e-jsdelivr-pj");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-jsdelivr-pj-pub"), "pass1234");
    await publishPackage(token, name, version);

    const res = await api(`/jsdelivr/npm/${name}@${version}/package.json`);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toContain("json");
    const body = await res.text();
    const parsed = JSON.parse(body);
    expect(parsed.main).toBe("index.js");
  });

  it("returns 404 for a missing file", async () => {
    const name = uniqueName("e2e-jsdelivr-404");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-jsdelivr-404-pub"), "pass1234");
    await publishPackage(token, name, version);

    const res = await api(`/jsdelivr/npm/${name}@${version}/does/not/exist.js`);
    expect(res.status).toBe(404);
  });

  it("redirects non-exact version spec to canonical version", async () => {
    const name = uniqueName("e2e-jsdelivr-redir");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-jsdelivr-redir-pub"), "pass1234");
    await publishPackage(token, name, version);

    const res = await api(`/jsdelivr/npm/${name}@latest/index.js`, {
      redirect: "manual",
    });
    expect(res.status).toBe(307);
    const location = res.headers.get("location") ?? "";
    expect(location).toBe(`/jsdelivr/npm/${name}@${version}/index.js`);
  });

  it("hash and size match between listing and served bytes", async () => {
    const name = uniqueName("e2e-jsdelivr-hash");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-jsdelivr-hash-pub"), "pass1234");
    await publishPackage(token, name, version);

    const { body } = await apiJson<any>(
      `/jsdelivr/api/npm/${name}@${version}?structure=flat`,
    );
    const pj = findFlat(body.files, "/package.json");
    expect(pj).toBeDefined();
    expect(decodeHashLen(pj.hash)).toBe(32);
    expect(pj.size).toBeGreaterThan(0);

    const res = await api(`/jsdelivr/npm/${name}@${version}/package.json`);
    const buf = Buffer.from(await res.arrayBuffer());
    expect(buf.length).toBe(pj.size);
    const recomputed = createHash("sha256").update(buf).digest("base64");
    expect(recomputed).toBe(pj.hash);
  });
});

describe("jsdelivr data API listing", () => {
  it("returns a nested file tree", async () => {
    const name = uniqueName("e2e-jsdelivr-tree");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-jsdelivr-tree-pub"), "pass1234");
    await publishPackage(token, name, version);

    const { res, body } = await apiJson<any>(
      `/jsdelivr/api/npm/${name}@${version}`,
    );
    expect(res.status).toBe(200);
    expect(body.type).toBe("npm");
    expect(body.name).toBe(name);
    expect(body.version).toBe(version);

    const names = collectFileNames(body.files);
    expect(names).toContain("package.json");
    expect(names).toContain("index.js");
    expect(names).toContain(".babelrc");
    expect(names).toContain("README.md");

    const indexJs = findInTree(body.files, "index.js");
    expect(indexJs).toBeDefined();
    expect(indexJs.type).toBe("file");
    expect(indexJs.size).toBe(0);
  });

  it("returns a flat list with ?structure=flat", async () => {
    const name = uniqueName("e2e-jsdelivr-flat");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-jsdelivr-flat-pub"), "pass1234");
    await publishPackage(token, name, version);

    const { res, body } = await apiJson<any>(
      `/jsdelivr/api/npm/${name}@${version}?structure=flat`,
    );
    expect(res.status).toBe(200);
    expect(Array.isArray(body.files)).toBe(true);
    const pj = findFlat(body.files, "/package.json");
    expect(pj).toBeDefined();
    expect(typeof pj.hash).toBe("string");
    expect(decodeHashLen(pj.hash)).toBe(32);
    expect(typeof pj.size).toBe("number");
  });

  it("redirects non-exact version spec to canonical version", async () => {
    const name = uniqueName("e2e-jsdelivr-api-redir");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-jsdelivr-api-redir-pub"), "pass1234");
    await publishPackage(token, name, version);

    const res = await api(`/jsdelivr/api/npm/${name}@latest`, {
      redirect: "manual",
    });
    expect(res.status).toBe(307);
    expect(res.headers.get("location")).toBe(
      `/jsdelivr/api/npm/${name}@${version}`,
    );
  });
});

describe("jsdelivr CDN (scoped package)", () => {
  it("serves a file and lists a scoped package", async () => {
    const name = uniqueScopedName("e2e-scope", "e2e-jsdelivr-scoped");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-jsdelivr-scoped-pub"), "pass1234");
    await publishPackage(token, name, version);

    const res = await api(
      `/jsdelivr/npm/${name}@${version}/package.json`,
    );
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toContain("json");

    const { body } = await apiJson<any>(
      `/jsdelivr/api/npm/${name}@${version}?structure=flat`,
    );
    expect(body.name).toBe(name);
    expect(findFlat(body.files, "/package.json")).toBeDefined();
  });
});

function collectFileNames(nodes: any[]): string[] {
  const out: string[] = [];
  const walk = (ns: any[], prefix: string) => {
    for (const n of ns) {
      const full = prefix ? `${prefix}/${n.name}` : n.name;
      if (n.type === "directory") {
        walk(n.files ?? [], full);
      } else {
        out.push(full);
      }
    }
  };
  walk(nodes, "");
  return out;
}

function findInTree(nodes: any[], target: string): any {
  for (const n of nodes) {
    if (n.type === "file" && n.name === target) return n;
    if (n.type === "directory") {
      const hit = findInTree(n.files ?? [], target);
      if (hit) return hit;
    }
  }
  return undefined;
}
