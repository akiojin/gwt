import { expect, test } from "@playwright/test";
import { APP_URL, APP_PROJECT_KEY, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { liveGwtProjectUrl } from "./_helpers/live-gwt";

const liveBase = process.env.GWT_PLAYWRIGHT_BASE_URL;
const appUrl = liveBase
  ? new URL(liveBase).pathname === "/" ? liveGwtProjectUrl(liveBase, APP_PROJECT_KEY) : liveBase
  : APP_URL;

test("disconnected terminal typing is discarded with a persistent retype notice", async ({ page }, info) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = info.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
  if (!liveBase) await installEmbeddedRoutes(page);
  await page.addInitScript(() => {
    const workspace = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [{
          id: "tab-1", title: "Disconnected input", project_root: "/fixture", kind: "git",
          workspace: {
            viewport: { x: 0, y: 0, zoom: 1 },
            windows: [{
              id: "agent-1", title: "Codex", preset: "agent", agent_id: "codex",
              agent_color: "cyan", status: "idle", z_index: 1,
              geometry: { x: 180, y: 120, width: 720, height: 360 }, geometry_revision: 0,
              minimized: false, maximized: false, pre_maximize_geometry: null, persist: true,
              purpose_title: null, dynamic_title: null, dynamic_title_detail: null,
              tab_group_id: null, tab_group_active: false,
            }],
          },
        }],
        active_tab_id: "tab-1", recent_projects: [],
      },
    };
    const sent = [];
    let online = true;
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
        setTimeout(() => { if (online) this.open(); }, 0);
      }
      open() {
        this.readyState = FixtureWebSocket.OPEN;
        this.dispatchEvent(new Event("open"));
        this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(workspace) }));
      }
      send(raw) {
        if (this.readyState !== FixtureWebSocket.OPEN) throw new Error("send on a closed socket");
        sent.push(JSON.parse(raw));
      }
      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
    window.__disconnectFixture = {
      sent,
      disconnect() {
        online = false;
        FixtureWebSocket.instances.filter(socket => socket.readyState === FixtureWebSocket.OPEN)
          .forEach(socket => socket.close());
      },
      reconnect() {
        online = true;
        FixtureWebSocket.instances.filter(socket => socket.readyState === FixtureWebSocket.CONNECTING)
          .forEach(socket => socket.open());
      },
    };
  });
  await page.goto(appUrl);
  const terminal = page.locator(".workspace-window[data-id='agent-1'] .xterm-helper-textarea");
  await expect(terminal).toBeAttached();
  await terminal.focus();
  await page.keyboard.type("before");
  const received = () => page.evaluate(() => window.__disconnectFixture.sent
    .filter(message => message.kind === "terminal_input" && message.id === "agent-1")
    .map(message => message.data).join(""));
  await expect.poll(received).toBe("before");

  // Freeze timers so keyboard input and the notice are checked before the
  // overlay's grace period, without racing a loaded headed browser.
  await page.clock.install();
  await page.clock.pauseAt(new Date(Date.now() + 1000));
  await page.evaluate(() => window.__disconnectFixture.disconnect());
  await expect(terminal).toBeFocused();
  await page.keyboard.type("discard-me");
  const notice = page.locator("[data-toast-id='terminal-input-dropped']");
  await expect(notice).toBeVisible();
  await expect(notice).toContainText("Input not sent");
  await expect(notice).toContainText("Input typed while disconnected was discarded. Retype it after reconnecting.");
  await expect(page.locator(".connection-overlay")).toHaveCount(0);
  expect(await received()).toBe("before");

  await page.clock.runFor(1600);
  await expect(page.locator(".connection-overlay")).toBeVisible();
  await page.evaluate(() => window.__disconnectFixture.reconnect());
  await page.clock.runFor(100);
  await expect(page.locator(".connection-overlay")).toHaveCount(0);
  await expect(notice).toBeVisible();
  await expect(notice).toContainText("Connection restored");
  await expect(notice).toContainText("Input typed while disconnected was discarded and was not resent. Retype it when ready.");
  expect(await received()).toBe("before");
  await terminal.focus();
  await page.keyboard.type("after");
  await expect.poll(received).toBe("beforeafter");
  await page.clock.runFor(10000);
  await expect(notice).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  const screenshot = info.outputPath("reconnected-input-notice.png");
  await page.screenshot({ path: screenshot });
  await info.attach("reconnected-input-notice", { path: screenshot, contentType: "image/png" });
  await notice.getByRole("button", { name: "Dismiss notification", exact: true }).click();
  await page.clock.runFor(500);
  await expect(notice).toHaveCount(0);
  expect(errors, "zero console/page errors").toEqual([]);
});
