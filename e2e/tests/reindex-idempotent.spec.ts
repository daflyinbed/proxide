import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { describe, expect, it } from "vitest";
import { PROXIDE_BINARY, PROXIDE_RUN_DIR } from "../globalSetup.js";
import { getMeiliSettings } from "../helpers.js";

const execFileAsync = promisify(execFile);

async function runReindexSearch(): Promise<{ exitCode: number; stderr: string }> {
  try {
    const { stderr } = await execFileAsync(PROXIDE_BINARY, ["reindex-search"], {
      cwd: PROXIDE_RUN_DIR,
      timeout: 60_000,
    });
    return { exitCode: 0, stderr: stderr ?? "" };
  } catch (err: any) {
    return {
      exitCode: typeof err.status === "number" ? err.status : 1,
      stderr: (err.stderr ?? "").toString(),
    };
  }
}

describe("reindex-search idempotency", () => {
  it("succeeds when the meilisearch index already exists (server created it on startup)", async () => {
    const first = await runReindexSearch();
    expect(first.exitCode).toBe(0);

    const second = await runReindexSearch();
    expect(second.exitCode).toBe(0);

    const settings = await getMeiliSettings();
    expect(settings.rankingRules).toContain("downloads.upstream:desc");
    expect(settings.filterableAttributes).toContain("package.scope");
  });
});
