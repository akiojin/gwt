import { defineConfig, devices } from "@playwright/test";

const isCI = !!process.env.CI;
const isE2ECoverage = process.env.E2E_COVERAGE === "1";
const devPort = isE2ECoverage ? 4274 : 4173;
const baseURL = `http://127.0.0.1:${devPort}`;
const webServerCommand = `pnpm run dev --host 127.0.0.1 --port ${devPort}`;

export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  expect: {
    timeout: 5_000,
  },
  fullyParallel: true,
  forbidOnly: isCI,
  retries: isCI ? 1 : 0,
  workers: isCI ? 1 : undefined,
  reporter: [["list"], ["html", { open: "never" }]],
  use: {
    baseURL,
    headless: isCI,
    trace: isCI ? "on-first-retry" : "retain-on-failure",
    screenshot: "only-on-failure",
    video: "retain-on-failure",
  },
  webServer: {
    command: webServerCommand,
    url: baseURL,
    reuseExistingServer: !isCI && !isE2ECoverage,
    timeout: 120_000,
  },
  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
      },
    },
  ],
});
