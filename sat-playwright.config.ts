import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "evals/sat-foundation",
  testMatch: "browser.spec.ts",
  workers: 1,
  outputDir: ".artifacts/sat-foundation/browser",
  use: { browserName: "chromium" },
});
