import { beforeAll, describe, expect, it } from "vitest";
import { spawnSync } from "node:child_process";
import { join, resolve } from "node:path";
import {
  api,
  apiJson,
  clearSearchIndex,
  deleteSearchDoc,
  login,
  publishPackage,
  searchPackages,
  uniqueName,
  uniqueScopedName,
  waitForSearch,
} from "../helpers.js";

const PROJECT_ROOT = resolve(import.meta.dirname, "../..");
const BINARY = join(PROJECT_ROOT, "target", "debug", "proxide");
const RUN_DIR = join(PROJECT_ROOT, "target", "e2e-run");

function findInObjects(body: any, name: string) {
  return body.objects.find((o: any) => o.package.name === name);
}

beforeAll(async () => {
  await clearSearchIndex();
}, 30_000);

describe("GET /npm/-/v1/search — indexing via publish", () => {
  it("indexes a published package and finds it by name", async () => {
    const name = uniqueName("e2e-search-basic");
    const token = await login(uniqueName("e2e-search-pub"), "pass1234");
    const { res } = await publishPackage(token, name, "1.2.3");
    expect(res.status).toBe(200);

    const body = await waitForSearch(
      name,
      (b) => !!findInObjects(b, name),
    );

    const hit = findInObjects(body, name);
    expect(hit).toBeDefined();
    expect(hit.package.version).toBe("1.2.3");
    expect(hit.package.name).toBe(name);
  });

  it("finds package by description text", async () => {
    const desc = "unique e2e database connector tool";
    const name = uniqueName("e2e-search-desc");
    const token = await login(uniqueName("e2e-search-desc-pub"), "pass1234");
    await publishPackage(token, name, "1.0.0", { description: desc });

    const body = await waitForSearch(
      "database connector",
      (b) => !!findInObjects(b, name),
    );

    const hit = findInObjects(body, name);
    expect(hit).toBeDefined();
    expect(hit.package.description).toBe(desc);
  });

  it("finds package by keyword", async () => {
    const keyword = uniqueName("e2e-kw");
    const name = uniqueName("e2e-search-kw");
    const token = await login(uniqueName("e2e-search-kw-pub"), "pass1234");
    await publishPackage(token, name, "1.0.0", {
      keywords: [keyword, "searchable"],
    });

    const body = await waitForSearch(
      keyword,
      (b) => !!findInObjects(b, name),
    );

    const hit = findInObjects(body, name);
    expect(hit).toBeDefined();
  });

  it("indexes scoped packages with correct scope", async () => {
    const name = uniqueScopedName("e2e-scope", "e2e-search-scoped");
    const token = await login(uniqueName("e2e-search-scoped-pub"), "pass1234");
    const { res } = await publishPackage(token, name, "1.0.0");
    expect(res.status).toBe(200);

    const unscoped = name.split("/")[1];
    const body = await waitForSearch(
      unscoped,
      (b) => !!findInObjects(b, name),
    );

    const hit = findInObjects(body, name);
    expect(hit).toBeDefined();
    expect(hit.package.scope).toBe("e2e-scope");
  });
});

describe("GET /npm/-/v1/search — response structure", () => {
  it("returns { objects, total } with package and downloads fields", async () => {
    const name = uniqueName("e2e-search-struct");
    const token = await login(uniqueName("e2e-search-struct-pub"), "pass1234");
    await publishPackage(token, name, "3.1.0");

    const body = await waitForSearch(
      name,
      (b) => !!findInObjects(b, name),
    );

    expect(body).toHaveProperty("objects");
    expect(body).toHaveProperty("total");
    expect(body.total).toBeGreaterThanOrEqual(1);

    const hit = findInObjects(body, name);
    expect(hit.id).toBe(name);
    expect(hit.package.name).toBe(name);
    expect(hit.package.version).toBe("3.1.0");
    expect(hit.package).toHaveProperty("scope");
    expect(hit.downloads).toEqual({ all: 0 });
  });

  it("includes dist-tags with latest", async () => {
    const name = uniqueName("e2e-search-tags");
    const token = await login(uniqueName("e2e-search-tags-pub"), "pass1234");
    await publishPackage(token, name, "2.0.0");

    const body = await waitForSearch(
      name,
      (b) => !!findInObjects(b, name),
    );

    const hit = findInObjects(body, name);
    expect(hit.package["dist-tags"]).toBeDefined();
    expect(hit.package["dist-tags"].latest).toBe("2.0.0");
  });
});

describe("GET /npm/-/v1/search — pagination", () => {
  it("respects size parameter", async () => {
    const prefix = uniqueName("e2e-search-page");
    const token = await login(uniqueName("e2e-search-page-pub"), "pass1234");

    const names: string[] = [];
    for (let i = 0; i < 3; i++) {
      const n = `${prefix}-${i}`;
      names.push(n);
      await publishPackage(token, n, "1.0.0");
    }

    for (const n of names) {
      await waitForSearch(n, (b) => !!findInObjects(b, n));
    }

    const { body } = await searchPackages(prefix, { size: 1 });
    expect(body.objects.length).toBe(1);
    expect(body.total).toBeGreaterThanOrEqual(3);
  });

  it("respects from parameter", async () => {
    const prefix = uniqueName("e2e-search-from");
    const token = await login(uniqueName("e2e-search-from-pub"), "pass1234");

    for (let i = 0; i < 3; i++) {
      await publishPackage(token, `${prefix}-${i}`, "1.0.0");
    }

    const { body: page0 } = await searchPackages(prefix, { from: 0, size: 1 });
    const { body: page1 } = await searchPackages(prefix, { from: 1, size: 1 });

    expect(page0.objects.length).toBe(1);
    expect(page1.objects.length).toBe(1);
    expect(page0.objects[0].package.name).not.toBe(page1.objects[0].package.name);
  });
});

describe("GET /npm/-/v1/search — ranking", () => {
  it("ranks exact name match first", async () => {
    const exact = uniqueName("e2e-search-rank-exact");
    const suffix = `${exact}-extra`;
    const token = await login(uniqueName("e2e-search-rank-pub"), "pass1234");
    await publishPackage(token, exact, "1.0.0");
    await publishPackage(token, suffix, "1.0.0");

    await waitForSearch(exact, (b) => !!findInObjects(b, exact) && !!findInObjects(b, suffix));

    const { body } = await searchPackages(exact);
    expect(body.objects.length).toBeGreaterThanOrEqual(1);
    expect(body.objects[0].package.name).toBe(exact);
  });
});

describe("GET /npm/-/v1/search — error handling", () => {
  it("returns 400 for empty text", async () => {
    const res = await api("/npm/-/v1/search?text=");
    expect(res.status).toBe(400);
  });

  it("returns empty results for no-match query", async () => {
    const { res, body } = await searchPackages("zzzznomatch999xyz");
    expect(res.status).toBe(200);
    expect(body.objects).toEqual([]);
    expect(body.total).toBe(0);
  });
});

describe("reindex-search subcommand", () => {
  it(
    "rebuilds index from DB after documents are removed",
    async () => {
      const nameA = uniqueName("e2e-reindex-a");
      const nameB = uniqueName("e2e-reindex-b");
      const token = await login(uniqueName("e2e-reindex-pub"), "pass1234");
      const { res: resA } = await publishPackage(token, nameA, "1.0.0");
      expect(resA.status).toBe(200);
      const { res: resB } = await publishPackage(token, nameB, "1.0.0");
      expect(resB.status).toBe(200);

      await waitForSearch(nameA, (b) => !!findInObjects(b, nameA), 20_000);
      await waitForSearch(nameB, (b) => !!findInObjects(b, nameB), 20_000);

      await deleteSearchDoc(nameA);
      await deleteSearchDoc(nameB);

      const start = Date.now();
      while (Date.now() - start < 10_000) {
        const { body: checkA } = await searchPackages(nameA);
        const { body: checkB } = await searchPackages(nameB);
        if (!findInObjects(checkA, nameA) && !findInObjects(checkB, nameB)) break;
        await new Promise((resolve) => setTimeout(resolve, 300));
      }

      const result = spawnSync(BINARY, ["reindex-search"], {
        cwd: RUN_DIR,
        encoding: "utf8",
        timeout: 60_000,
      });
      expect(result.status).toBe(0);

      const bodyA = await waitForSearch(nameA, (b) => !!findInObjects(b, nameA));
      const bodyB = await waitForSearch(nameB, (b) => !!findInObjects(b, nameB));
      expect(findInObjects(bodyA, nameA)).toBeDefined();
      expect(findInObjects(bodyB, nameB)).toBeDefined();
    },
    120_000,
  );
});
