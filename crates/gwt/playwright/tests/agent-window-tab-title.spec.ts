import { expect, test, type Page } from "@playwright/test";
import { APP_URL, APP_PROJECT_KEY, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { liveGwtProjectUrl } from "./_helpers/live-gwt";

const liveBase = process.env.GWT_PLAYWRIGHT_BASE_URL;
const appUrl = liveBase
  ? new URL(liveBase).pathname === "/" ? liveGwtProjectUrl(liveBase, APP_PROJECT_KEY) : liveBase
  : APP_URL;
const errors = new WeakMap<Page, string[]>();
const rowFor = (page: Page, title: string) => page.locator(".window-list-row")
  .filter({ has: page.locator(".window-list-title", { hasText: title }) });

// #4777 T-3: grouped agents remain reachable through Windows after removing
// the window tab strip. The existing titlebar/list retain title and telemetry.
test.describe("Agent window navigation without tabs", () => {
  test.use({ deviceScaleFactor: 1, viewport: { width: 1440, height: 900 } });

  test.beforeEach(async ({ page }, info) => {
    const captured: string[] = [];
    errors.set(page, captured);
    page.on("pageerror", error => captured.push(error.message));
    page.on("console", message => { if (message.type() === "error") captured.push(message.text()); });
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme),
      info.project.name.includes("light") ? "light" : "dark");
    if (!liveBase) await installEmbeddedRoutes(page);
    await installTabbedAgentsBackend(page);
    await page.goto(appUrl);
    await expect(page.locator(".workspace-window[data-id='agent-1']")).toBeVisible();
  });

  test.afterEach(async ({ page }, info) => {
    await expect(page.locator("html")).toHaveAttribute("data-theme",
      info.project.name.includes("light") ? "light" : "dark");
    expect(errors.get(page), "zero console/page errors").toEqual([]);
  });

  test("full titles remain in the titlebar and Windows list without tab DOM", async ({ page }) => {
    await expect(page.locator(".window-tab-strip, .window-tab")).toHaveCount(0);
    await expect(page.locator(".workspace-window[data-id='agent-1'] .title-text"))
      .toHaveAttribute("title", "Codex · implementing complete maximize");
    await page.locator("#window-list-button").click();
    await expect(rowFor(page, "Codex").locator(".window-list-title"))
      .toHaveAttribute("title", "Codex · implementing complete maximize");
    await expect(rowFor(page, "Claude").locator(".window-list-title"))
      .toHaveAttribute("title", "Claude");
  });

  test("Windows opens an inactive grouped agent for input and preserves Close Guard", async ({ page }) => {
    const active = page.locator(".workspace-window[data-id='agent-1']");
    const target = page.locator(".workspace-window[data-id='agent-2']");
    await expect(target).toBeHidden();
    await page.locator("#window-list-button").click();
    await rowFor(page, "Claude").click();
    await expect(target).toBeVisible();
    await expect(active).toBeHidden();
    await expect(target.locator(".title-text")).toHaveAttribute("title", "Claude");
    await target.locator(".xterm-helper-textarea").focus();
    await page.keyboard.type("hello");
    await expect.poll(() => page.evaluate(() => window.__tabbedAgentsFixture.sent
      .filter(message => message.kind === "terminal_input" && message.id === "agent-2")
      .map(message => message.data).join(""))).toBe("hello");
    const closedIds = () => page.evaluate(() => window.__tabbedAgentsFixture.sent
      .filter(message => message.kind === "close_window").map(message => message.id));
    const modal = page.locator("#window-close-confirm-modal");
    await target.getByRole("button", { name: "Close window", exact: true }).click();
    await expect(modal).toBeVisible();
    expect(await closedIds()).toEqual([]);
    await modal.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(modal).toBeHidden();
    expect(await closedIds()).toEqual([]);
    await target.getByRole("button", { name: "Close window", exact: true }).click();
    await modal.getByRole("button", { name: "Close window", exact: true }).click();
    await expect.poll(closedIds).toEqual(["agent-2"]);
  });

  test("titlebar docking can be undone with Ungroup window without changing geometry", async ({ page }, info) => {
    await page.evaluate(() => window.__tabbedAgentsFixture.resetUngrouped());
    const source = page.locator(".workspace-window[data-id='agent-1']");
    const target = page.locator(".workspace-window[data-id='agent-2']");
    await expect(source).toBeVisible();
    await expect(target).toBeVisible();
    const from = await source.locator(".titlebar").boundingBox();
    const to = await target.locator(".titlebar").boundingBox();
    await page.mouse.move(from!.x + 40, from!.y + from!.height / 2);
    await page.mouse.down();
    await page.mouse.move(to!.x + 40, to!.y + to!.height / 2, { steps: 8 });
    await page.mouse.up();
    await expect.poll(() => page.evaluate(() => window.__tabbedAgentsFixture.sent
      .filter(message => message.kind === "dock_window_tab")
      .map(message => [message.id, message.target_id]))).toEqual([["agent-1", "agent-2"]]);
    await expect(source).toBeVisible();
    await expect(target).toBeHidden();
    await expect(page.locator(".window-tab-strip, .window-tab")).toHaveCount(0);
    const geometry = () => source.evaluate(element => {
      const style = (element as HTMLElement).style;
      return { x: parseFloat(style.left), y: parseFloat(style.top),
        width: parseFloat(style.width), height: parseFloat(style.height) };
    });
    const before = await geometry();
    const ungroup = source.getByRole("button", { name: "Ungroup window", exact: true });
    await expect(ungroup).toBeVisible();
    const screenshot = info.outputPath("grouped-agent-before-ungroup.png");
    await page.screenshot({ path: screenshot });
    await info.attach("grouped-agent-before-ungroup", { path: screenshot, contentType: "image/png" });
    await ungroup.click();
    await expect.poll(() => page.evaluate(() => window.__tabbedAgentsFixture.sent
      .filter(message => message.kind === "detach_window_tab")
      .map(message => ({ id: message.id, geometry: message.geometry }))))
      .toEqual([{ id: "agent-1", geometry: before }]);
    await expect(source).toBeVisible();
    await expect(target).toBeVisible();
    expect(await geometry()).toEqual(before);
    await expect(source.getByRole("button", { name: "Ungroup window", exact: true })).toBeHidden();
  });

  test("Windows list updates inactive agent telemetry without activating it", async ({ page }) => {
    await page.locator("#window-list-button").click();
    const chip = rowFor(page, "Claude").locator(".status-chip");
    await expect(chip).toHaveClass(/running/);
    await expect(chip.locator(".status-label")).toHaveText("Running");
    await page.evaluate(() => window.__tabbedAgentsFixture.emitTerminalStatus("agent-2", "idle"));
    await expect(chip).toHaveClass(/idle/);
    await expect(chip.locator(".status-label")).toHaveText("Idle");
    await expect(page.locator(".workspace-window[data-id='agent-2']")).toBeHidden();
  });
});

async function installTabbedAgentsBackend(page) {
  await page.addInitScript(() => {
    const baseWindow = {
      preset: "agent",
      geometry: { x: 180, y: 120, width: 720, height: 360 },
      geometry_revision: 0,
      status: "idle",
      minimized: false,
      maximized: false,
      pre_maximize_geometry: null,
      persist: true,
      purpose_title: null,
      dynamic_title: null,
      dynamic_title_detail: null,
      tab_group_id: "grp-1",
    };

    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Tab Title Fixture",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                {
                  ...baseWindow,
                  id: "agent-1",
                  title: "Codex",
                  z_index: 2,
                  agent_id: "codex",
                  agent_color: "cyan",
                  dynamic_title_detail: "Codex · implementing complete maximize",
                  tab_group_active: true,
                },
                {
                  ...baseWindow,
                  id: "agent-2",
                  title: "Claude",
                  status: "running",
                  z_index: 1,
                  agent_id: "claude",
                  agent_color: "violet",
                  tab_group_active: false,
                },
              ],
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };

    const sent = [];

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      static instances = [];

      constructor(url) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        FixtureWebSocket.instances.push(this);
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(workspaceState);
        }, 0);
      }

      send(raw) {
        const message = JSON.parse(raw);
        sent.push(message);
        const windows = workspaceState.workspace.tabs[0].workspace.windows;
        if (message.kind === "activate_window_tab") {
          for (const data of windows) data.tab_group_active = data.id === message.id;
          this.emit(workspaceState);
        }
        if (message.kind === "dock_window_tab") {
          const target = windows.find(data => data.id === message.target_id);
          for (const data of windows) {
            data.tab_group_id = "grp-1";
            data.tab_group_active = data.id === message.id;
            data.geometry = { ...target.geometry };
          }
          this.emit(workspaceState);
        }
        if (message.kind === "detach_window_tab") {
          for (const data of windows) {
            data.tab_group_id = null;
            data.tab_group_active = false;
            if (data.id === message.id) data.geometry = { ...message.geometry };
          }
          this.emit(workspaceState);
        }
      }

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload) {
        setTimeout(() => {
          this.dispatchEvent(
            new MessageEvent("message", { data: JSON.stringify(payload) }),
          );
        }, 0);
      }
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
    window.__tabbedAgentsFixture = {
      sent,
      resetUngrouped() {
        workspaceState.workspace.tabs[0].workspace.windows.forEach((data, index) => {
          data.tab_group_id = null;
          data.tab_group_active = false;
          data.geometry = { x: 120 + index * 650, y: 120, width: 500, height: 360 };
        });
        FixtureWebSocket.instances.forEach(socket => socket.emit(workspaceState));
      },
      emitTerminalStatus(windowId, status) {
        FixtureWebSocket.instances?.forEach((socket) =>
          socket.emit({ kind: "terminal_status", id: windowId, status }),
        );
      },
    };
  });
}
