/* SPEC #7007 T-036, Issue #7654: reproduction procedures for the terminal
 * requirements of design items 58 and 185 that no other spec reproduces.
 *
 * (2) Scrolling does not get heavier as output accumulates.
 * (3) Wide characters leave no ghost and never overlap.
 *
 * gwt's xterm renders rows into the DOM (no canvas or WebGL addon), so what
 * the rows show is what the user sees. The Playwright README maps all six
 * requirements to the spec or test that reproduces each one.
 */
import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { openWorks } from "./_helpers/works-entry";

async function openTerminal(page: Page, theme: string) {
  await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
  await installEmbeddedRoutes(page);
  await page.addInitScript(() => {
    (window as any).__gwtPlaywrightTestBridge = true;
    const projectKey = location.pathname.split("/").filter(Boolean).at(-1);
    const workspace = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [{
          id: "tab-1", title: "Terminal requirements", project_key: projectKey, project_root: "/fixture", kind: "git",
          workspace: {
            viewport: { x: 0, y: 0, zoom: 1 },
            windows: [{
              id: "agent-1", title: "Codex", preset: "agent", agent_id: "codex",
              agent_name: "Codex", session_id: "agent-1-session",
              agent_color: "cyan", status: "running", z_index: 1,
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
    const sockets: any[] = [];
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      readyState = 0;
      constructor(public url: string) {
        super();
        sockets.push(this);
        setTimeout(() => {
          this.readyState = 1;
          this.dispatchEvent(new Event("open"));
          this.receive(workspace);
          this.receive({ kind: "active_work_projection", projection: { id: "fixture-local-live", active_works: [],
            local_agent_activity: { project_key: projectKey, host_id: "terminal-fixture", observed_at: new Date().toISOString(),
              agents: [{ window_id: "agent-1", session_id: "agent-1-session", runtime_incarnation: 1, is_pm: false, issue_number: null }] } } });
        }, 0);
      }
      receive(message: unknown) {
        this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(message) }));
      }
      send() {}
      close() {
        this.readyState = 3;
        this.dispatchEvent(new CloseEvent("close"));
      }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
    // The PTY's output, delivered the way the host sends it.
    (window as any).__terminalOutput = (text: string) => {
      const bytes = new TextEncoder().encode(text);
      let binary = "";
      for (let index = 0; index < bytes.length; index += 0x8000) {
        binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
      }
      sockets.filter(socket => socket.readyState === 1)
        .forEach(socket => socket.receive({ kind: "terminal_output", id: "agent-1", data_base64: btoa(binary) }));
    };
  });
  await page.goto(APP_URL);
  await openWorks(page);
  await page.locator('.work-row[data-window-id="agent-1"] .work-open').click();
  const tile = page.locator(".works-surface:not(.works-surface--preview) .works-terminal[data-window-id='agent-1']");
  await expect(tile.locator(".xterm-rows")).toBeAttached();
  return tile;
}

const write = (page: Page, text: string) => page.evaluate(value => (window as any).__terminalOutput(value), text);

function watchErrors(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  return errors;
}

test("(3) wide characters leave no ghost and never overlap", async ({ page }, info) => {
  const errors = watchErrors(page);
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const tile = await openTerminal(page, theme);
  const rows = tile.locator(".xterm-rows > div");
  // The bt procedure: draw wide text, then overwrite it the way a TUI
  // redraws - narrower text on the same row, half of a wide cell, and a
  // cleared line - and look for anything left behind.
  await write(page, "\x1b[2J\x1b[H日本語の残像テスト\r\n全角ＡＢＣＤ\r\nあいうえお\r\n");
  await expect(rows.nth(0)).toContainText("日本語の残像テスト");
  await write(page, "\x1b[1;1H\x1b[2Kok");
  // Column 3 is the left half of "い": writing there erases the whole cell,
  // so its right half (column 4) becomes a blank instead of half a glyph.
  await write(page, "\x1b[2;1Hab\x1b[3;3HX");
  await write(page, "\x1b[4;1H😀x\x1b[4;1H\x1b[K");
  await expect(rows.nth(0)).toHaveText(/^ok\s*$/);
  await expect(rows.nth(1)).toHaveText(/^ab角ＡＢＣＤ\s*$/);
  await expect(rows.nth(2)).toHaveText(/^あX うえお\s*$/);
  await expect(rows.nth(3)).toHaveText(/^\s*$/);
  // No glyph is drawn wider than its cells: every wide glyph sits inside
  // a two-cell span and no span overlaps the next one.
  const overlaps = await tile.locator(".xterm-rows").evaluate(root => {
    const found: string[] = [];
    for (const row of Array.from(root.children)) {
      const spans = Array.from(row.querySelectorAll("span")).filter(span => span.textContent);
      for (let index = 1; index < spans.length; index += 1) {
        const previous = spans[index - 1].getBoundingClientRect();
        const current = spans[index].getBoundingClientRect();
        if (previous.right - current.left > 0.5) found.push(`${spans[index - 1].textContent}|${spans[index].textContent}`);
      }
    }
    return found;
  });
  expect(overlaps).toEqual([]);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  expect(errors, "zero console/page errors").toEqual([]);
});

test("(2) scrolling does not get heavier as output accumulates", async ({ page }, info) => {
  test.setTimeout(120_000);
  const errors = watchErrors(page);
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const tile = await openTerminal(page, theme);
  const rowsRoot = tile.locator(".xterm-rows");
  const lines = (from: number, count: number) => Array.from({ length: count },
    (_, index) => `line ${from + index} 幅のある文字と ascii の混在した出力`).join("\r\n") + "\r\n";
  // One measurement: scroll up through the history a fixed number of
  // steps and back to the bottom, and time it end to end in the page.
  const scrollCost = () => page.evaluate(async () => {
      const target = document.querySelector(".works-surface:not(.works-surface--preview) .works-terminal[data-window-id='agent-1'] .xterm-screen")!;
      const frame = () => new Promise(resolve => requestAnimationFrame(resolve));
      const started = performance.now();
      for (const delta of [...Array(40).fill(-120), ...Array(40).fill(120)]) {
        target.dispatchEvent(new WheelEvent("wheel", { deltaY: delta, bubbles: true, cancelable: true }));
        await frame();
      }
      const cost = performance.now() - started;
      // Back to the live bottom, outside the measured window, so new output
      // is followed again.
      (window as any).__gwtTerminalTestApi.scrollToBottom("agent-1");
      await frame();
      return cost;
    });
  const tail = () => page.evaluate(() => (window as any).__gwtTerminalTestApi.bufferText("agent-1", 4));
  await write(page, lines(0, 2_000));
  await expect.poll(tail).toContain("line 1999 ");
  const rowCount = await rowsRoot.locator(":scope > div").count();
  const early = await scrollCost();
  // Twenty-five times the output. Each chunk is drawn before the next one
  // arrives, as a live PTY paces it: an unpaced flood past the page's
  // pending-output limit waits for a host snapshot instead (requirement 1).
  for (let chunk = 1; chunk < 25; chunk += 1) {
    await write(page, lines(chunk * 2_000, 2_000));
    await expect.poll(tail).toContain(`line ${chunk * 2_000 + 1_999} `);
  }
  await expect(rowsRoot).toContainText("line 49999 ");
  const late = await scrollCost();
  info.annotations.push({ type: "scroll-cost", description: `early ${early.toFixed(0)}ms, late ${late.toFixed(0)}ms` });
  // The rendered rows stay the screen's rows, however long the history is.
  expect(await rowsRoot.locator(":scope > div").count()).toBe(rowCount);
  // The same scroll costs about the same. The bound leaves room for a busy
  // runner; an accumulating cost grows with the output and breaks it.
  expect(late, `early ${early.toFixed(0)}ms, late ${late.toFixed(0)}ms`).toBeLessThan(early * 3 + 500);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  expect(errors, "zero console/page errors").toEqual([]);
});
