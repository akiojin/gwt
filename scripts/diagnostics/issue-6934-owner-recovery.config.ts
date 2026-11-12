import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  testMatch: "issue-6934-owner-recovery.spec.ts",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: "list",
  use: { ...devices["Desktop Chrome"], headless: false, actionTimeout: 30_000, navigationTimeout: 60_000 },
  projects: [
    { name: "chromium-dark", use: { colorScheme: "dark" } },
    { name: "chromium-light", use: { colorScheme: "light" } },
  ],
});
