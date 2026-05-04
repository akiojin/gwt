import { defineConfig, devices } from "@playwright/test";

// SPEC-2356 Operator Design System — Visual regression baseline.
// Tests render the embedded gwt frontend and snapshot per surface × theme.
export default defineConfig({
  testDir: "./tests",
  snapshotDir: "./snapshots",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/",
    trace: "on-first-retry",
    colorScheme: "dark",
  },
  expect: {
    toHaveScreenshot: {
      maxDiffPixelRatio: 0.005,
      animations: "disabled",
    },
  },
  projects: [
    {
      name: "chromium-dark",
      use: { ...devices["Desktop Chrome"], colorScheme: "dark" },
    },
    {
      name: "chromium-light",
      use: { ...devices["Desktop Chrome"], colorScheme: "light" },
    },
  ],
});
