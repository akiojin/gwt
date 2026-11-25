/* Issue #7730 (SPEC #7007 item 210) — the user's own hooks in the Hook Center.
 *
 * AC-1: the hooks registered for the project are listed.
 * AC-2: a hook is registered from a timing and a script location, and the
 *       list is the audit's: a registered hook shows the audit's finding.
 *
 * Runs against the embedded frontend with a stub backend, in both themes,
 * with zero console / page errors.
 */
import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

type Hook = { timing: string; script: string; state: string };

async function installHookBackend(page: Page, hooks: Hook[]) {
  await page.addInitScript(({ hooks, appOrigin, theme }) => {
    // Init scripts can also run in the opaque document before app navigation.
    if (location.origin === appOrigin) localStorage.setItem("gwt:ui:theme", theme);
    try { window.sessionStorage.setItem("gwt:ui:briefing", "1"); } catch { /* no-op */ }
    (window as any).__gwtPlaywrightTestBridge = true;
    let current = hooks as any[];
    const sent: unknown[] = [];
    (window as any).__hooksSent = sent;
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [{
          id: "tab-1", title: "Hooks Fixture", project_root: "/fixture",
          project_key: location.pathname.split("/")[2], kind: "git",
          workspace: {
            viewport: { x: 0, y: 0, zoom: 1 },
            windows: [{
              id: "settings-fixture", title: "Settings", preset: "settings", status: "running",
              persist: true, z_index: 1, geometry: { x: 40, y: 40, width: 900, height: 760 },
            }],
          },
        }],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      constructor(url: string) {
        super();
        (this as any).url = url;
        (this as any).readyState = FixtureWebSocket.CONNECTING;
        setTimeout(() => {
          (this as any).readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }
      send(raw: string) {
        const message = JSON.parse(raw);
        sent.push(message);
        if (message.kind === "frontend_ready") this.emit(workspaceState);
        if (message.kind !== "get_hook_doctor" && message.kind !== "update_user_hooks") return;
        let changeError: string | null = null;
        const change = message.change;
        if (change?.action === "register") {
          const script = `/fixture/${change.script}`;
          if (change.script === "missing.sh") changeError = `No file exists at ${script}.`;
          else current = current.concat([{ timing: change.timing, script, state: "ready" }]);
        }
        if (change?.action === "remove") {
          current = current.filter(hook => !(hook.timing === change.timing && hook.script === change.script));
        }
        this.emit({
          kind: "hook_doctor", project_root: "/fixture", request_id: message.request_id, error: null,
          report: {
            health: { status: "ready", issues: [], slow_handlers: [] }, repair: null,
            user_hooks: { hooks: current, error: null, change_error: changeError },
          },
        });
      }
      close() {
        (this as any).readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }
      emit(payload: unknown) {
        setTimeout(() => this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) })), 0);
      }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
  }, {
    hooks, appOrigin: new URL(APP_URL).origin,
    theme: test.info().project.name.includes("light") ? "light" : "dark",
  });
}

test.describe("Hook Center — your hooks", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("registered hooks are listed, and a hook is registered from a timing and a script location (AC-1, AC-2)", async ({ page }, info) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    await installEmbeddedRoutes(page);
    await installHookBackend(page, [{ timing: "session_start", script: "/gone/setup.sh", state: "missing" }]);
    await page.goto(APP_URL);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const settings = page.locator('.workspace-window[data-preset="settings"]');
    await expect(settings).toBeVisible();
    await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "hooks" } })));
    const panel = settings.locator('[data-settings-panel="hooks"]');
    await expect(panel).toHaveAttribute("data-hook-audit-status", "loaded");
    await expect(panel.getByRole("heading", { name: "Your hooks", exact: true })).toBeVisible();
    await expect(panel).toContainText("Registered hooks are included in this audit.");

    // AC-1: what is registered is listed with what the audit found.
    const rows = panel.locator('[data-role="user-hook"]');
    await expect(rows).toHaveCount(1);
    await expect(rows.nth(0)).toContainText("setup.sh");
    await expect(rows.nth(0)).toContainText("Timing: Session start · Location: /gone/setup.sh");
    await expect(rows.nth(0)).toContainText("Audit: Script missing");

    // AC-2: a timing and a script location register a hook.
    const timing = panel.getByLabel("Timing", { exact: true });
    const script = panel.getByLabel("Script location", { exact: true });
    await expect(timing.locator("option")).toHaveText(["Session start", "Stop"]);
    await timing.selectOption("stop");
    await script.fill(".gwt/hooks/notify.sh");
    await panel.getByRole("button", { name: "Register", exact: true }).click();
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(1)).toContainText("notify.sh");
    await expect(rows.nth(1)).toContainText("Timing: Stop · Location: /fixture/.gwt/hooks/notify.sh");
    await expect(rows.nth(1)).toContainText("Audit: Registered");
    await expect(script).toHaveValue("");
    expect(await page.evaluate(() => (window as any).__hooksSent.filter((message: any) => message.kind === "update_user_hooks")
      .map((message: any) => message.change))).toEqual([{ action: "register", timing: "stop", script: ".gwt/hooks/notify.sh" }]);

    // A refused registration says why and keeps what was typed.
    await script.fill("missing.sh");
    await panel.getByRole("button", { name: "Register", exact: true }).click();
    await expect(panel.getByRole("alert")).toHaveText("No file exists at /fixture/missing.sh.");
    await expect(script).toHaveValue("missing.sh");
    await expect(timing).toHaveValue("stop");
    await expect(rows).toHaveCount(2);
    await panel.getByRole("heading", { name: "Your hooks", exact: true }).scrollIntoViewIfNeeded();
    const screenshot = info.outputPath(`user-hooks-${theme}.png`);
    await page.screenshot({ path: screenshot });
    await info.attach(`user-hooks-${theme}`, { path: screenshot, contentType: "image/png" });

    await rows.nth(0).getByRole("button", { name: "Remove /gone/setup.sh at Session start", exact: true }).click();
    await expect(rows).toHaveCount(1);
    await expect(rows.nth(0)).toContainText("notify.sh");
    expect(errors, "zero console and page errors").toEqual([]);
  });

  test("a project with no registered hooks says so and still offers registration (AC-1)", async ({ page }, info) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await installEmbeddedRoutes(page);
    await installHookBackend(page, []);
    await page.goto(APP_URL);
    await expect(page.locator("html")).toHaveAttribute("data-theme", info.project.name.includes("light") ? "light" : "dark");
    await expect(page.locator('.workspace-window[data-preset="settings"]')).toBeVisible();
    await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "hooks" } })));
    const panel = page.locator('[data-settings-panel="hooks"]');
    await expect(panel).toHaveAttribute("data-hook-audit-status", "loaded");
    await expect(panel.locator('[data-role="user-hook"]')).toHaveCount(0);
    await expect(panel).toContainText("No hooks are registered.");
    await expect(panel.getByRole("button", { name: "Register", exact: true })).toBeEnabled();
    expect(errors, "zero console and page errors").toEqual([]);
  });
});
