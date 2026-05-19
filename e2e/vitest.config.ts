import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    globalSetup: ["./e2e/globalSetup.ts"],
    testTimeout: 30_000,
    hookTimeout: 120_000,
    include: ["e2e/tests/**/*.spec.ts"],
  },
});
