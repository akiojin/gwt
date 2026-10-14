import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// SPEC #3340 T-014/T-015/T-018: `interrupted` is a first-class runtime state.
// It must render as Interrupted (not fall back to Running), never project as
// done, and leave no stale chip class once the window recovers.
const browserErrors = new WeakMap<Page, string[]>();

test.beforeEach(async ({ page }, testInfo) => {
  const errors: string[] = [];
  browserErrors.set(page, errors);
  page.on("pageerror", (error) => errors.push(String(error)));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript((theme) => {
    localStorage.setItem("gwt:ui:theme", theme);
  }, theme);
  await page.addInitScript(() => {
    class FixtureSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      readyState = 0;
      constructor(public readonly url: string) {
        super();
        setTimeout(() => {
          this.readyState = FixtureSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }
      send(raw: string) {
        if (JSON.parse(raw).kind !== "frontend_ready") return;
        this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify({
          kind: "workspace_state",
          workspace: {
            app_version: "playwright",
            tabs: [{
              id: "tab-1", title: "Interrupted fixture",
              project_root: "/fixture", kind: "git",
              workspace: {
                viewport: { x: 0, y: 0, zoom: 1 },
                windows: [{
                  id: "agent-1", title: "Codex", preset: "agent",
                  geometry: { x: 40, y: 80, width: 520, height: 300 },
                  geometry_revision: 0, z_index: 1, status: "interrupted",
                  minimized: false, maximized: false, pre_maximize_geometry: null,
                  persist: true, purpose_title: null, dynamic_title: null,
                  dynamic_title_detail: null, agent_id: "codex", agent_color: "cyan",
                  lane_kind: "execution", tab_group_id: null, tab_group_active: false,
                }],
              },
            }],
            active_tab_id: "tab-1", recent_projects: [],
          },
        }) }));
      }
      close() { this.readyState = FixtureSocket.CLOSED; }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureSocket });
  });
  await installEmbeddedRoutes(page);
  await page.goto(APP_URL);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
});

test.afterEach(({ page }) => {
  expect(browserErrors.get(page), "console and page errors").toEqual([]);
});

test("interrupted window renders Interrupted, never done, and recovers without a stale class", async ({ page }, testInfo) => {
  const agentWindow = page.locator(".workspace-window[data-id='agent-1']");
  await expect(agentWindow).toBeVisible({ timeout: 10_000 });
  const chip = agentWindow.locator(".status-chip");
  await expect(agentWindow.locator(".status-label")).toHaveText("Interrupted");
  await expect(chip).toHaveClass(/\binterrupted\b/);
  await expect(agentWindow).not.toHaveAttribute("data-agent-state", "done");
  await agentWindow.screenshot({ path: testInfo.outputPath("interrupted-window.png") });

  await page.evaluate(() => {
    window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
      detail: { kind: "window_state", window_id: "agent-1", state: "running" },
    }));
  });
  await expect(agentWindow.locator(".status-label")).toHaveText("Running");
  await expect(chip).toHaveClass(/\brunning\b/);
  await expect(chip).not.toHaveClass(/\binterrupted\b/);
});
