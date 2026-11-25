import { chromium, defineConfig, devices } from "@playwright/test";

const chromiumChannel = process.env.GWT_PLAYWRIGHT_CHROMIUM_CHANNEL;
const desktopChrome = chromiumChannel
  ? { ...devices["Desktop Chrome"], channel: chromiumChannel }
  : devices["Desktop Chrome"];

// macOS clamps --window-position back onto the display, so headed windows
// stack under the real cursor. Each window another worker opens or closes there
// then delivers a native mouse leave/enter to the pages beneath it, which
// discards the hover a test just produced (#7755). CDP may move a window until
// only a corner sliver stays on the display; that keeps it visible but out of
// the cursor's way. Other platforms honor the launch position as is.
if (process.platform === "darwin") {
  const launch = chromium.launch.bind(chromium);
  chromium.launch = async options => {
    const browser = await launch(options);
    if (options?.headless !== false) return browser;
    const session = await browser.newBrowserCDPSession();
    session.on("Target.targetCreated", ({ targetInfo }) => {
      if (targetInfo.type !== "page") return;
      // A page closed before it is moved, or a headless shell, has no window.
      session.send("Browser.getWindowForTarget", { targetId: targetInfo.targetId })
        .then(({ windowId }) => session.send("Browser.setWindowBounds", { windowId, bounds: { left: 32000, top: 32000 } }))
        .catch(() => {});
    });
    await session.send("Target.setDiscoverTargets", { discover: true });
    return browser;
  };
}

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
    screenshot: process.env.GWT_HEADED_E2E_SCREENSHOTS === "1" ? "on" : "off",
    colorScheme: "dark",
    // For explicit headed runs, open the window off screen where supported.
    // Do not start it
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
