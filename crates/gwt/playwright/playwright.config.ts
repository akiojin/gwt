import { defineConfig, devices } from "@playwright/test";

const chromiumChannel = process.env.GWT_PLAYWRIGHT_CHROMIUM_CHANNEL;
const desktopChrome = chromiumChannel
  ? { ...devices["Desktop Chrome"], channel: chromiumChannel }
  : devices["Desktop Chrome"];

// SPEC-2356 Operator Design System — Visual regression baseline.
// Tests render the embedded gwt frontend and snapshot per surface × theme.
export default defineConfig({
  testDir: "./tests",
  outputDir: "./test-results",
  snapshotDir: "./snapshots",
  snapshotPathTemplate: "{snapshotDir}/{testFilePath}/{projectName}/{platform}/{arg}{ext}",
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
      use: { ...desktopChrome, colorScheme: "dark" },
    },
    {
      name: "chromium-light",
      use: { ...desktopChrome, colorScheme: "light" },
    },
  ],
});
