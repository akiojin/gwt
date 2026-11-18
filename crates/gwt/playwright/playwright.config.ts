import { defineConfig, devices } from "@playwright/test";

const chromiumChannel = process.env.GWT_PLAYWRIGHT_CHROMIUM_CHANNEL;
const desktopChrome = chromiumChannel
  ? { ...devices["Desktop Chrome"], channel: chromiumChannel }
  : devices["Desktop Chrome"];

// WebView behavior coverage runs against both Operator themes.
export default defineConfig({
  testDir: "./tests",
  outputDir: "./test-results",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/",
    trace: "on-first-retry",
    colorScheme: "dark",
    // verify.run always adds --headed; open the window off screen so it does
    // not cover or steal focus from the developer's work (#6977). Do not start it
    // minimized: a minimized page reports visibilityState "hidden".
    launchOptions: { args: ["--window-position=-32000,-32000"] },
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
