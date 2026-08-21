import { describe, expect, it } from "vitest";
import { api, apiJson, login, publishPackage, uniqueName } from "../helpers.js";

const EVICTION_WINDOW_MS = 7_000;

describe("unpacked store eviction (cdn.unpackedMaxBytes = 1)", () => {
  it(
    "serves files and listings again after eviction re-extraction",
    async () => {
      const name = uniqueName("e2e-unpacked-evict");
      const version = "1.0.0";
      const token = await login(uniqueName("e2e-unpacked-evict-pub"), "pass1234");
      await publishPackage(token, name, version);

      const first = await api(`/jsdelivr/npm/${name}@${version}/package.json`);
      expect(first.status).toBe(200);

      await new Promise((r) => setTimeout(r, EVICTION_WINDOW_MS));

      const second = await api(`/jsdelivr/npm/${name}@${version}/package.json`);
      expect(second.status).toBe(200);
      const manifest = JSON.parse(await second.text());
      expect(manifest.main).toBe("index.js");

      const { res, body } = await apiJson<any>(
        `/jsdelivr/api/npm/${name}@${version}?structure=flat`,
      );
      expect(res.status).toBe(200);
      expect(body.files.some((f: any) => f.name === "/package.json")).toBe(true);
    },
    30_000,
  );

  it("rejects encoded path traversal in file path", async () => {
    const name = uniqueName("e2e-unpacked-traversal");
    const version = "1.0.0";
    const token = await login(uniqueName("e2e-unpacked-traversal-pub"), "pass1234");
    await publishPackage(token, name, version);

    const res = await api(
      `/jsdelivr/npm/${name}@${version}/%2e%2e%2f%2e%2e%2fetc%2fpasswd`,
    );
    expect(res.status).toBe(400);
  });
});
