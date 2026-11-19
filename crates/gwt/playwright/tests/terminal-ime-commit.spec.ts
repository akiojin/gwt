import { expect, test } from "@playwright/test";
import { APP_URL, APP_PROJECT_KEY, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { liveGwtProjectUrl } from "./_helpers/live-gwt";

const liveBase = process.env.GWT_PLAYWRIGHT_BASE_URL;
const appUrl = liveBase
  ? new URL(liveBase).pathname === "/" ? liveGwtProjectUrl(liveBase, APP_PROJECT_KEY) : liveBase
  : APP_URL;

const phrase = "症状は同じです。";

test("an IME composition commit reaches the PTY once", async ({ page }, info) => {
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
          id: "tab-1", title: "IME input", project_key: location.pathname.split("/").filter(Boolean).at(-1), project_root: "/fixture", kind: "git",
          workspace: {
            viewport: { x: 0, y: 0, zoom: 1 },
            windows: [{
              id: "agent-1", title: "Codex", preset: "agent", agent_id: "codex",
              agent_name: "Codex", session_id: "agent-1-session",
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
    const sent: { kind: string, id?: string, data?: string }[] = [];
    const echoes: string[] = [];
    const originalDebug = console.debug.bind(console);
    console.debug = (...args: unknown[]) => {
      const detail = args[1] as { reason?: string } | undefined;
      if (detail?.reason === "ime-echo") echoes.push(detail.reason);
      originalDebug(...args);
    };
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      readyState = FixtureWebSocket.CONNECTING;
      constructor(public url: string) {
        super();
        setTimeout(() => this.open(), 0);
      }
      open() {
        this.readyState = FixtureWebSocket.OPEN;
        this.dispatchEvent(new Event("open"));
        this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(workspace) }));
        this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify({ kind: "active_work_projection", projection: { id: "fixture-local-live", active_works: [], local_agent_activity: { project_key: location.pathname.split("/").filter(Boolean).at(-1), host_id: "ime-fixture", observed_at: new Date().toISOString(), agents: [
          { window_id: "agent-1", session_id: "agent-1-session", runtime_incarnation: 1, is_pm: false, issue_number: null }
        ] } } }) }));
      }
      send(raw: string) {
        if (this.readyState !== FixtureWebSocket.OPEN) throw new Error("send on a closed socket");
        sent.push(JSON.parse(raw));
      }
      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
    window.__imeFixture = { sent, echoes };
  });
  await page.goto(appUrl);
  await page.getByRole("button", { name: "Works", exact: true }).click();
  await page.locator('.work-row[data-window-id="agent-1"] .work-open').click();
  const tile = page.locator(".works-surface:not(.works-surface--preview) .works-terminal[data-window-id='agent-1']");
  await expect(tile).toBeVisible();
  const terminal = tile.locator(".xterm-helper-textarea");
  await expect(terminal).toBeAttached();
  await terminal.focus();
  await page.keyboard.type("a");
  const chunks = () => page.evaluate(() => window.__imeFixture.sent
    .filter(message => message.kind === "terminal_input" && message.id === "agent-1")
    .map(message => message.data ?? ""));
  await expect.poll(async () => (await chunks()).join("")).toBe("a");

  // xterm 6 sends the confirmed composition from a non-229 keydown and again
  // from compositionend's timeout. keyCode 0 still finalizes and does not
  // clear the textarea, so the timeout would repeat the phrase.
  const keyCode = await page.evaluate(async (text) => {
    const textarea = document.querySelector(".xterm-helper-textarea") as HTMLTextAreaElement;
    const flush = () => new Promise(resolve => setTimeout(resolve, 0));
    textarea.value = "";
    textarea.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    textarea.value = text;
    textarea.dispatchEvent(new CompositionEvent("compositionupdate", { bubbles: true, data: text }));
    await flush();
    await flush();
    await flush();
    await flush();
    const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
    const observed = event.keyCode;
    textarea.dispatchEvent(event);
    textarea.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: text }));
    await flush();
    await flush();
    await flush();
    await flush();
    return observed;
  }, phrase);

  expect(keyCode, "synthetic keydown must not be Enter, which would clear the textarea").toBe(0);
  await expect.poll(async () => (await chunks()).join("")).toBe(`a${phrase}`);
  const sent = await chunks();
  expect(sent.filter(chunk => chunk.includes(phrase))).toEqual([phrase]);
  await expect.poll(() => page.evaluate(() => window.__imeFixture.echoes.length)).toBeGreaterThan(0);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  const screenshot = info.outputPath("ime-commit-once.png");
  await page.screenshot({ path: screenshot });
  await info.attach("ime-commit-once", { path: screenshot, contentType: "image/png" });
  expect(errors, "zero console/page errors").toEqual([]);
});
