import { expect, test, type Page } from "@playwright/test";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const liveBase = process.env.GWT_PLAYWRIGHT_BASE_URL;
const appUrl = liveBase ? new URL(new URL(APP_URL).pathname, liveBase).href : APP_URL;
const PM = "pm-terminal";

// A deterministic PTY-protocol fixture isolates browser parsing, layout and input
// from a particular Codex version. browser-check still serves every UI asset.
test("PM terminal keeps redraw, grid and input consistent across view changes", async ({ page }, info) => {
  test.setTimeout(90_000);
  const errors: string[] = [];
  const grids: unknown[] = [];
  const theme = info.project.name.includes("light") ? "light" : "dark";
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
  if (!liveBase) await installEmbeddedRoutes(page);
  await installPmBackend(page);
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto(appUrl);
  await page.locator("#op-pm-entry").click();
  const pm = page.locator(`.workspace-window[data-id="${PM}"]`);
  await expect(pm.getByRole("button", { name: "Execution log", exact: true })).toBeVisible();
  await pm.getByRole("button", { name: "Execution log", exact: true }).click();
  await expect(pm.locator(".xterm-helper-textarea")).toBeVisible();
  await expect.poll(() => metrics(page)).toMatchObject({ isReady: true });

  const parserFixture = process.env.GWT_ISSUE6420_PARSER_FIXTURE;
  if (parserFixture) {
    const fixture = JSON.parse(readFileSync(parserFixture, "utf8"));
    const normalized = (value: string) => value.split("\n").map(line => line.trimEnd()).join("\n").trimEnd();
    // These bytes and expected screens come from the checkout backend parser;
    // replay each path in real xterm, including the cursor-relative live tail.
    await emit(page, "terminal_output", fixture.raw);
    await expect.poll(async () => normalized(await text(page))).toBe(normalized(fixture.expected_before.text));
    await emit(page, "terminal_output", fixture.continuation);
    await expect.poll(async () => normalized(await text(page))).toBe(normalized(fixture.expected_after.text));
    await emit(page, "terminal_snapshot", fixture.snapshot);
    await expect.poll(async () => normalized(await text(page))).toBe(normalized(fixture.expected_before.text));
    await emit(page, "terminal_output", fixture.continuation);
    await expect.poll(async () => normalized(await text(page))).toBe(normalized(fixture.expected_after.text));
    await info.attach("backend parser and xterm comparison", { body: JSON.stringify(fixture), contentType: "application/json" });
  }

  for (const width of [1920, 960]) {
    await page.setViewportSize({ width, height: 1080 });
    await expectGridConverged(page);
    grids.push({ phase: `visible-${width}`, ...await metrics(page) });

    const old = "\x1b[2J\x1b[HOLD STATUS\r\n> /modelmodel\r\n\r\n7\r\n97";
    await emit(page, "terminal_output", old);
    await expect.poll(() => text(page)).toContain("OLD STATUS");
    const before = info.outputPath(`pm-redraw-before-${width}-${theme}.png`);
    await pm.screenshot({ path: before });
    await info.attach(`before redraw ${width}`, { path: before, contentType: "image/png" });
    // A normal in-place TUI repaint clears the previous status and input rows.
    await emit(page, "terminal_output", "\x1b[1;1H\x1b[JGPT-6.1-Sol status\r\n> Ready");
    await expect.poll(async () => (await text(page)).trim()).toBe("GPT-6.1-Sol status\n> Ready");
    const after = info.outputPath(`pm-redraw-after-${width}-${theme}.png`);
    await pm.screenshot({ path: after });
    await info.attach(`after redraw ${width}`, { path: after, contentType: "image/png" });

    await pm.getByRole("button", { name: "Reports", exact: true }).click();
    await expect(pm.locator(".pm-conversation-log")).toBeHidden();
    await settle(page);
    const oldCols = (await metrics(page)).cols;
    // Codex repaints with wrap-relative cursor movements at the PTY's current
    // grid. Parse this while hidden, before a reveal measures a wider host.
    const hiddenRepaint = "\x1b[2J\x1b[HHEADER\r\n" + "X".repeat(oldCols)
      + "97\r\x1b[2K\x1b[1A\x1b[2KCURRENT";
    await emit(page, "terminal_output", hiddenRepaint);
    await expect.poll(async () => (await text(page)).trim(), {
      message: "hidden PM output parses at the grid which generated its cursor movements",
    }).toBe("HEADER\nCURRENT");
    await page.evaluate(() => { (window as any).__pmTerminalFixture.sent.length = 0; });
    await page.setViewportSize({ width: Math.floor(width / 2), height: 920 });
    await settle(page);
    expect(await page.evaluate(() => (window as any).__pmTerminalFixture.sent.filter((entry: any) => entry.kind === "update_terminal_grid" && entry.id === "pm-terminal")))
      .toEqual([]);
    await pm.getByRole("button", { name: "Execution log", exact: true }).click();
    await expectGridConverged(page);
    const reveal = info.outputPath(`pm-hidden-reveal-${width}-${theme}.png`);
    await pm.screenshot({ path: reveal });
    await info.attach(`hidden repaint reveal ${width}`, { path: reveal, contentType: "image/png" });
    await info.attach(`hidden repaint buffer ${width}`, { body: await text(page), contentType: "text/plain" });
    await expect.poll(async () => (await text(page)).trim()).toBe("HEADER\nCURRENT");
    grids.push({ phase: `revealed-${width}`, ...await metrics(page) });
  }

  // PM is a single-pane surface. Split a peer surface, then return through the
  // real rail entry and assert that the same terminal retains its state.
  await page.getByRole("button", { name: "Board", exact: true }).click();
  await page.locator("#split-view-button").click();
  await expect(page.locator("#split-surfaces")).toHaveAttribute("data-split", "true");
  await page.locator("#op-pm-entry").click();
  await expect(page.locator("#split-surfaces")).toHaveAttribute("data-split", "false");
  await expectGridConverged(page);
  await expect.poll(async () => (await text(page)).trim()).toBe("HEADER\nCURRENT");

  await emit(page, "terminal_snapshot", "\x1b[H\x1b[2JSnapshot status\r\n> model changed");
  await expect.poll(() => text(page)).toContain("Snapshot status");
  await emit(page, "terminal_output", "\r\nLive after snapshot");
  await expect.poll(() => text(page)).toContain("Live after snapshot");
  await page.evaluate(() => (window as any).__pmTerminalFixture.reconnect());
  await expect.poll(() => text(page)).toContain("Reconnected status");
  await emit(page, "terminal_output", "\r\nLive after reconnect");
  await expect.poll(() => text(page)).toContain("Live after reconnect");

  const input = pm.locator(".xterm-helper-textarea");
  await input.focus();
  await page.evaluate(() => { (window as any).__pmTerminalFixture.sent.length = 0; });
  await page.keyboard.type("/model");
  await expect.poll(() => sentInput(page)).toBe("/model");
  await settle(page);
  expect(await sentInput(page)).toBe("/model");
  await page.evaluate(() => { (window as any).__pmTerminalFixture.sent.length = 0; });
  await input.evaluate(node => {
    const clipboard = new DataTransfer();
    clipboard.setData("text/plain", "/model");
    node.dispatchEvent(new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: clipboard }));
  });
  await expect.poll(() => sentInput(page)).toBe("/model");
  expect(await sentInput(page)).not.toContain("/modelmodel");
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await info.attach("PM terminal grid and errors", { body: JSON.stringify({ theme, grids, errors }), contentType: "application/json" });
  expect(errors, "zero console/page errors").toEqual([]);
});

async function metrics(page: Page): Promise<any> {
  return page.evaluate(id => {
    const host = document.querySelector(`.workspace-window[data-id="${id}"] .terminal-root`)!;
    const screen = host.querySelector(".xterm-screen")!;
    const measured = (window as any).__gwtTerminalTestApi.metrics(id);
    const box = host.getBoundingClientRect();
    const cells = screen.getBoundingClientRect();
    return { ...measured, hostWidth: box.width, hostHeight: box.height,
      cellWidth: cells.width / measured.cols, cellHeight: cells.height / measured.rows };
  }, PM);
}
async function expectGridConverged(page: Page): Promise<void> {
  await expect.poll(async () => {
    const { cols, rows } = await metrics(page);
    return page.evaluate(({ cols, rows }) => {
      const last = (window as any).__pmTerminalFixture.sent.filter((entry: any) => entry.kind === "update_terminal_grid" && entry.id === "pm-terminal").at(-1);
      return last?.cols === cols && last?.rows === rows;
    }, { cols, rows });
  }, { message: "the visible PM grid is published to the PTY" }).toBe(true);
}
async function text(page: Page): Promise<string> {
  return page.evaluate(id => (window as any).__gwtTerminalTestApi.bufferText(id), PM);
}
async function sentInput(page: Page): Promise<string> {
  return page.evaluate(() => (window as any).__pmTerminalFixture.sent.filter((entry: any) => entry.kind === "terminal_input" && entry.id === "pm-terminal").map((entry: any) => entry.data).join(""));
}
async function emit(page: Page, kind: string, bytes: string): Promise<void> {
  await page.evaluate(({ kind, bytes }) => (window as any).__pmTerminalFixture.emit({ kind, id: "pm-terminal", data_base64: btoa(bytes) }), { kind, bytes });
}
async function settle(page: Page): Promise<void> {
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
}

async function installPmBackend(page: Page): Promise<void> {
  await page.addInitScript(() => {
    (window as any).__gwtPlaywrightTestBridge = true;
    sessionStorage.setItem("gwt:ui:briefing", "1");
    let socket: FixtureWebSocket;
    let connections = 0;
    const windows = [
      { id: "pm-terminal", title: "PM", preset: "agent", is_pm: true, session_id: "pm-terminal-session", agent_id: "codex" },
      { id: "board-window", title: "Board", preset: "board" },
      { id: "settings-window", title: "Settings", preset: "settings" },
    ].map((entry, index) => ({ ...entry, status: "running", geometry: { x: 40, y: 40, width: 1000, height: 720 }, z_index: index + 1, persist: true }));
    const workspace = () => ({ kind: "workspace_state", workspace: { app_version: "fixture", active_tab_id: "tab-1", recent_projects: [], tabs: [{
      id: "tab-1", title: "PM terminal fixture", project_key: location.pathname.split("/").at(-1), project_root: "/fixture", kind: "git",
      workspace: { viewport: { x: 0, y: 0, zoom: 1 }, windows },
    }] } });
    const fixture = (window as any).__pmTerminalFixture = {
      sent: [] as any[],
      emit: (packet: unknown) => socket.emit(packet),
      reconnect: () => socket.close(),
    };
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      readyState = 0;
      url: string;
      constructor(url: string) {
        super();
        this.url = url;
        socket = this;
        connections++;
        setTimeout(() => {
          this.readyState = 1;
          this.dispatchEvent(new Event("open"));
          this.emit(workspace());
          if (connections > 1) this.emit({ kind: "terminal_snapshot", id: "pm-terminal", data_base64: btoa("\x1b[H\x1b[2JReconnected status") });
        }, 0);
      }
      send(raw: string) {
        const packet = JSON.parse(raw);
        fixture.sent.push(packet);
        if (packet.kind === "load_pm_reports") this.emit({ kind: "pm_reports", id: packet.id, session_id: "pm-terminal-session", request_id: packet.request_id, snapshot: packet.version === null, version: { epoch: "fixture", revision: 1 }, reports: [], current: null });
      }
      emit(packet: unknown) { this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(packet) })); }
      close() { this.readyState = 3; this.dispatchEvent(new CloseEvent("close")); }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
  });
}

test("real PTY receives one input and converges with the visible terminal grid", async ({ page }, info) => {
  const checkHome = process.env.GWT_PLAYWRIGHT_CHECK_HOME;
  test.skip(!liveBase || !checkHome, "requires browser-check's isolated checkout and fresh HOME");
  test.setTimeout(120_000);
  await withLiveGwtBackendLock(liveBase!, info, async () => {
    const errors: string[] = [];
    const outputs = new Map<string, string>();
    const observations: unknown[] = [];
    const theme = info.project.name.includes("light") ? "light" : "dark";
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("websocket", socket => socket.on("framereceived", ({ payload }) => {
      try {
        const packet = JSON.parse(String(payload));
        if (packet.kind === "terminal_output") outputs.set(packet.id,
          (outputs.get(packet.id) || "") + Buffer.from(packet.data_base64, "base64").toString("utf8"));
      } catch { /* binary/non-terminal frames are outside this probe */ }
    }));
    await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
    const probe = join(checkHome!, `issue-6420-pty-${theme}.cjs`);
    // The child observes the actual ConPTY dimensions and bytes, independently
    // from the frontend's own WebSocket message inventory.
    writeFileSync(probe, `
      process.stdin.setRawMode(true);
      let bytes = [], grid = 0, input = 0;
      process.stdin.on('data', chunk => { for (const byte of chunk) {
        if (byte === 7) process.stdout.write('\\r\\nPTY_GRID_' + (++grid) + ':' + process.stdout.columns + 'x' + process.stdout.rows + '\\r\\n');
        else if (byte === 4) { process.stdout.write('\\r\\nPTY_INPUT_' + (++input) + ':' + Buffer.from(bytes).toString('hex') + '\\r\\n'); bytes = []; }
        else if (byte === 3) process.exit(0);
        else bytes.push(byte);
      }});
      process.stdout.write('\\r\\nPTY_PROBE_READY\\r\\n');
    `);
    let id: string | undefined;
    try {
      await page.setViewportSize({ width: 1920, height: 1080 });
      await gotoLiveGwt(page, liveBase!, { enableTestBridge: true });
      const windows = () => page.evaluate(() => {
        const state = (window as any).__gwtPlaywrightMessages.findLast((entry: any) => entry.payload.kind === "workspace_state");
        return state?.payload.workspace.tabs.flatMap((tab: any) => tab.workspace.windows) || [];
      });
      const before = new Set((await windows()).map((entry: any) => entry.id));
      await sendLiveGwtEvent(page, { kind: "create_window", preset: "shell" });
      await expect.poll(async () => (await windows()).filter((entry: any) => entry.preset === "shell" && !before.has(entry.id))).toHaveLength(1);
      id = (await windows()).find((entry: any) => entry.preset === "shell" && !before.has(entry.id)).id;
      const showShell = async () => {
        const index = (await windows()).findIndex((entry: any) => entry.id === id);
        await page.locator("#window-list-button").click();
        await page.locator("#window-list-panel .window-list-row").nth(index).click();
        await expect(page.locator(`.split-pane .workspace-window[data-id="${id}"]`)).toBeVisible();
      };
      await showShell();
      const shell = page.locator(`.workspace-window[data-id="${id}"]`);
      await expect.poll(() => page.evaluate(id => (window as any).__gwtTerminalTestApi.metrics(id).isReady, id)).toBe(true);
      await sendLiveGwtEvent(page, { kind: "terminal_input", id, data: `node "${probe.replace(/\\/g, "/")}"\r` });
      await expect.poll(() => outputs.get(id!) || "", { timeout: 30_000 }).toContain("PTY_PROBE_READY");
      const grid = async (phase: string) => {
        // A grid dispatch and fast-path input may cross; query fresh markers
        // until the actual PTY has caught up with the visible frontend grid.
        await expect(async () => {
          const count = [...(outputs.get(id!) || "").matchAll(/PTY_GRID_\d+:(\d+)x(\d+)/g)].length;
          await sendLiveGwtEvent(page, { kind: "terminal_input", id, data: "\x07" });
          await expect.poll(() => [...(outputs.get(id!) || "").matchAll(/PTY_GRID_\d+:(\d+)x(\d+)/g)].length).toBeGreaterThan(count);
          const actual = [...(outputs.get(id!) || "").matchAll(/PTY_GRID_\d+:(\d+)x(\d+)/g)].at(-1)!;
          const measured = await page.evaluate(id => (window as any).__gwtTerminalTestApi.metrics(id), id);
          expect({ cols: Number(actual[1]), rows: Number(actual[2]) }).toEqual({ cols: measured.cols, rows: measured.rows });
          observations.push({ phase, frontend: { cols: measured.cols, rows: measured.rows }, pty: actual[0] });
        }).toPass({ timeout: 15_000, intervals: [250, 500] });
      };
      await grid("initial-1920x1080");
      await page.setViewportSize({ width: 960, height: 800 });
      await grid("resized-960x800");
      await page.getByRole("button", { name: "Board", exact: true }).click();
      await page.setViewportSize({ width: 1280, height: 900 });
      await showShell();
      await grid("hidden-resize-reveal");
      await page.locator("#split-view-button").click();
      await grid("split");
      await page.locator("#split-view-button").click();
      await grid("single");
      await shell.locator(".xterm-helper-textarea").focus();
      await page.keyboard.type("/model");
      await sendLiveGwtEvent(page, { kind: "terminal_input", id, data: "\x04" });
      await expect.poll(() => outputs.get(id!) || "").toContain("PTY_INPUT_1:2f6d6f64656c\r\n");
      await shell.locator(".xterm-helper-textarea").evaluate(node => {
        const clipboard = new DataTransfer();
        clipboard.setData("text/plain", "/model");
        node.dispatchEvent(new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: clipboard }));
      });
      await sendLiveGwtEvent(page, { kind: "terminal_input", id, data: "\x04" });
      await expect.poll(() => outputs.get(id!) || "").toContain("PTY_INPUT_2:2f6d6f64656c\r\n");
      await info.attach("real PTY observations", { body: JSON.stringify({ theme, observations, inputHex: "2f6d6f64656c", errors }), contentType: "application/json" });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      expect(errors, "zero console/page errors").toEqual([]);
    } finally {
      if (id) {
        await sendLiveGwtEvent(page, { kind: "terminal_input", id, data: "\x03" });
        await page.getByRole("button", { name: "Works", exact: true }).click();
        if (await page.locator("#split-surfaces").getAttribute("data-split") === "true") await page.locator("#split-view-button").click();
        await sendLiveGwtEvent(page, { kind: "close_window", id });
      }
    }
  });
});

// Actual provider proof is opt-in: discovery never copies credentials or starts a provider.
test("owned actual Codex PM retains genuine history and composer across Works reentry", async ({ page }, info) => {
  test.skip(process.env.GWT_PLAYWRIGHT_REAL_CODEX_PM !== "1", "requires explicitly enabled isolated actual Codex PM");
  test.setTimeout(300_000);
  const { spawnSync } = await import("node:child_process");
  const { createHash, randomBytes } = await import("node:crypto");
  const { constants } = await import("node:fs");
  const { isAbsolute } = await import("node:path");
  const { access, chmod, copyFile, readFile, readdir, rm, writeFile } = await import("node:fs/promises");
  const { homedir } = await import("node:os");
  const { startExactRelaunchFixture, writeFixtureNodeCommand } = await import("./_helpers/exact-relaunch");
  const nativePath = process.env.GWT_PLAYWRIGHT_REAL_CODEX_PATH;
  expect(process.platform, "actual native proof requires Darwin").toBe("darwin");
  expect(nativePath, "GWT_PLAYWRIGHT_REAL_CODEX_PATH must identify the installed native executable").toBeTruthy();
  if (!nativePath || !isAbsolute(nativePath)) throw new Error("Expected an absolute native Codex path");
  await access(nativePath, constants.X_OK);
  await access(join(homedir(), ".codex", "auth.json"));
  const hash = createHash("sha256").update(await readFile(nativePath)).digest("hex");
  const version = spawnSync(nativePath, ["--version"], { encoding: "utf8" });
  if (version.error) throw version.error;
  expect(version.status).toBe(0);
  expect(version.stdout.trim()).toMatch(/^codex-cli\s+\S+/);
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = info.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
  let home = "", preferences = "", id = "", session = "";
  let fixture: Awaited<ReturnType<typeof startExactRelaunchFixture>> | undefined;
  let captured: { pid: number; birth: string; parent: string; cwd: string; session: string; ancestry: number[] } | undefined;
  let monitoring = false, unexpectedTool = false, monitorError: unknown;
  let ownedGuiPid = 0;
  let monitor: Promise<void> | undefined;
  const received: string[] = [];
  page.on("websocket", socket => socket.on("framereceived", ({ payload }) => {
    try {
      const packet = JSON.parse(String(payload));
      if (packet.kind === "terminal_output" && packet.id === id) received.push(Buffer.from(packet.data_base64, "base64").toString("utf8"));
    } catch { /* Ignore unrelated non-JSON frames. */ }
  }));
  const ps = (pid: number, columns: string) => {
    const result = spawnSync("ps", ["-p", String(pid), "-o", columns], { encoding: "utf8" });
    if (result.error) throw result.error;
    const value = result.stdout.trim();
    if (result.status === 1 && !value && !result.stderr.trim()) return "";
    if (result.status !== 0 || !value || (columns === "lstart=" && !Number.isFinite(Date.parse(value)))) {
      throw new Error(`Cannot inspect owned PID ${pid}: ps status ${result.status}, column ${columns}`);
    }
    return value;
  };
  const ownedNative = () => !!captured && captured.ancestry.includes(ownedGuiPid)
    && captured.cwd.startsWith(home + "/") && /^[0-9a-f-]{36}$/i.test(captured.session)
    && ps(captured.pid, "lstart=") === captured.birth
    && (ps(captured.pid, "command=") === nativePath || ps(captured.pid, "command=").startsWith(nativePath + " "));
  const nativeRows = async (directory: string): Promise<any[]> => {
    const entries = await readdir(directory, { withFileTypes: true }).catch(() => []);
    return (await Promise.all(entries.map(async entry => {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) return nativeRows(path);
      if (!entry.name.endsWith(".jsonl")) return [];
      const contents = await readFile(path, "utf8").catch(() => "");
      const lines = contents.split("\n");
      if (!contents.endsWith("\n")) lines.pop();
      return lines.filter(Boolean).map(line => JSON.parse(line));
    }))).flat();
  };
  const rows = () => nativeRows(join(home, ".codex", "sessions"));
  let primaryError: unknown;
  const cleanupErrors: unknown[] = [];
  try {
    fixture = await startExactRelaunchFixture(info, {
      async prepare(context) {
        home = context.home;
        await copyFile(join(homedir(), ".codex", "auth.json"), join(home, ".codex", "auth.json"));
        await chmod(join(home, ".codex", "auth.json"), 0o600);
        // Exec preserves PID and argv. Only the owned Session records its birth before exec.
        await writeFixtureNodeCommand(context.bin, "codex", `#!/bin/sh\nif [ -n "$GWT_SESSION_ID" ]; then\n  owner="$HOME/native-owner.$$"\n  printf '%s\\n%s\\n%s\\n' "$$" "$GWT_SESSION_ID" "$PWD" > "$owner" || exit 1\n  ps -p $$ -o lstart= -o ppid= >> "$owner" || exit 1\n  ancestor=$$\n  while [ "$ancestor" -gt 1 ]; do\n    ancestor=$(ps -p "$ancestor" -o ppid= | tr -d ' ') || exit 1\n    [ -n "$ancestor" ] || exit 1\n    printf '%s\\n' "$ancestor" >> "$owner" || exit 1\n  done\n  mv "$owner" "$HOME/native-owner" || exit 1\nfi\nexec '${nativePath.replace(/'/g, "'\\''")}' "$@"\n`);
      },
      async beforeStart(context) {
        preferences = context.preferences;
        await writeFile(join(preferences, "pm.json"), JSON.stringify({ settings: { auto_start: false, paused: true, launch_profile: { agent_id: "codex" } } }));
      },
    });
    ownedGuiPid = fixture.ownedPid!;
    expect(ownedGuiPid).toBeGreaterThan(0);
    await page.setViewportSize({ width: 400, height: 1000 });
    await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
    await page.locator('.op-rail__surface[data-surface="agents"]').click();
    const surface = page.locator(".agents-surface:not(.agents-surface--preview)");
    await surface.getByRole("textbox", { name: "Name for new member" }).fill("Owned Codex PM");
    await surface.getByRole("combobox", { name: "Tool for new member" }).selectOption("codex");
    await surface.getByRole("checkbox", { name: "PM eligible for new member" }).check();
    await surface.locator('.roster-capacity [data-action="add"]').click();
    const member = surface.locator(".roster-row").filter({ hasText: "Owned Codex PM" });
    await expect(member).toHaveCount(1);
    await member.click();
    const detail = surface.locator(".roster-detail");
    await expect(detail.locator('[data-action="select-pm"]')).toBeEnabled();
    await detail.locator('[data-action="select-pm"]').click();
    await expect(detail.locator('[data-action="select-pm"]')).toHaveText("Configured Project Manager");
    expect(JSON.parse(await readFile(join(preferences, "pm.json"), "utf8")).settings.launch_profile.agent_id).toBe("codex");
    await sendLiveGwtEvent(page, { kind: "open_pm_agent" });
    await expect.poll(async () => {
      const prefs = JSON.parse(await readFile(join(preferences, "pm.json"), "utf8"));
      session = prefs.registration?.session_id ?? "";
      return session;
    }, { timeout: 90_000 }).not.toBe("");
    id = await page.waitForFunction(sessionId => {
      const rows = (window as any).__gwtPlaywrightMessages ?? [];
      const state = rows.findLast((row: any) => row.payload.kind === "workspace_state")?.payload.workspace;
      const pane = state?.tabs.flatMap((tab: any) => tab.workspace.windows).find((row: any) => row.session_id === sessionId);
      return pane?.is_pm && pane.agent_id === "codex" ? pane.id : null;
    }, session, { timeout: 60_000 }).then(handle => handle.jsonValue());
    await expect.poll(async () => (await readFile(join(home, "native-owner"), "utf8").catch(() => "")).split("\n").length, { timeout: 30_000 }).toBeGreaterThanOrEqual(5);
    const [pidText, ownerSession, cwd, birthParent, ...ancestors] = (await readFile(join(home, "native-owner"), "utf8")).trim().split("\n");
    const match = birthParent.trim().match(/^(.*?)\s+(\d+)$/)!;
    captured = { pid: Number(pidText), session: ownerSession, cwd, birth: match[1], parent: match[2], ancestry: ancestors.map(Number) };
    expect(captured.session).toBe(session);
    expect(captured.cwd.startsWith(home + "/")).toBe(true);
    expect(ps(captured.pid, "lstart=")).toBe(captured.birth);
    expect(ownedNative(), "captured native PID and ancestry belong to this fixture").toBe(true);
    monitoring = true;
    monitor = (async () => {
      while (monitoring) {
        const events = await rows();
        if (events.some(row => row.type === "response_item" && typeof row.payload?.type === "string" && row.payload.type.endsWith("_call"))) {
          unexpectedTool = true;
          if (ownedNative()) process.kill(captured!.pid, "SIGTERM");
          return;
        }
        await new Promise(resolve => setTimeout(resolve, 100));
      }
    })().catch(error => { monitorError = error; });
    const latestOwnActivity = (snapshots: any[], projectKey: string) => snapshots.filter(value => value?.project_key === projectKey).at(-1);
    const matchesIdentity = (value: any, windowId: string, sessionId: string, incarnation?: number) =>
      !!value?.host_id && Array.isArray(value.agents) && value.agents.some((agent: any) => agent.window_id === windowId
        && agent.session_id === sessionId && agent.runtime_incarnation > 0
        && (incarnation === undefined || agent.runtime_incarnation === incarnation));
    // A newer full census must never fall back to the older matching census.
    const matching = { project_key: "probe", host_id: "host", agents: [{ window_id: "w", session_id: "s", runtime_incarnation: 1 }] };
    for (const agents of [[], null, [{ window_id: "w", session_id: "replacement", runtime_incarnation: 2 }],
      [{ window_id: "w", session_id: "s", runtime_incarnation: 2 }]]) {
      expect(matchesIdentity(latestOwnActivity([matching, { ...matching, agents }], "probe"), "w", "s", 1)).toBe(false);
    }
    expect(matchesIdentity(latestOwnActivity([matching, { ...matching }], "probe"), "w", "s", 1)).toBe(true);
    const projectKey = await page.evaluate(() => location.pathname.split("/")[2]);
    const activity = async () => latestOwnActivity(await page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .map((row: any) => row.payload.kind === "local_agent_activity" ? row.payload.activity : row.payload.projection?.local_agent_activity)), projectKey);
    await expect.poll(async () => matchesIdentity(await activity(), id, session), { timeout: 60_000 }).toBe(true);
    const initialActivity = await activity();
    expect(matchesIdentity(initialActivity, id, session)).toBe(true);
    const incarnation = initialActivity.agents.find((agent: any) => agent.window_id === id && agent.session_id === session).runtime_incarnation;
    const runtimePath = join(home, ".gwt", "sessions", "runtime", String(ownedGuiPid), `${session}.json`);
    const runtime = async () => JSON.parse(await readFile(runtimePath, "utf8"));
    await expect.poll(async () => (await runtime().catch(() => ({}))).child_pid, { timeout: 30_000 }).toBe(captured!.pid);
    const initialRuntime = await runtime();
    expect(initialRuntime.runtime_incarnation).toBe(incarnation);
    expect(initialRuntime.child_started_at).toBeGreaterThan(0);
    expect(initialRuntime.host_started_at).toBeGreaterThan(0);
    await page.locator("#op-pm-entry").click();
    const pm = page.locator(`.workspace-window[data-id="${id}"]`);
    await pm.getByRole("button", { name: "Execution log", exact: true }).click();
    const buffer = () => page.evaluate(id => (window as any).__gwtTerminalTestApi.bufferText(id), id);
    const metrics = () => page.evaluate(id => (window as any).__gwtTerminalTestApi.metrics(id), id);
    await expect.poll(buffer, { timeout: 60_000 }).toContain("Ask Codex");
    const input = pm.locator(".xterm-helper-textarea");
    await input.focus();
    const marker = `GWT4777${randomBytes(8).toString("hex").toUpperCase()}`;
    await page.keyboard.type(`Reply with exactly 64 lines numbered 01 through 64. Each line must contain ${marker} immediately followed by its two-digit number without spaces. Do not use tools.`);
    const beforeOutput = received.length;
    // Submit and leave in one browser task: no asynchronous provider response
    // can reach the visible PM before the Works surface is selected.
    await page.evaluate(id => {
      const textarea = document.querySelector(`.workspace-window[data-id="${id}"] .xterm-helper-textarea:focus`) as HTMLTextAreaElement;
      if (!textarea) throw new Error("owned PM composer is not focused");
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", keyCode: 13, which: 13, bubbles: true, cancelable: true }));
      const works = document.querySelector('[data-surface="works"].op-rail__surface') as HTMLElement;
      if (!works) throw new Error("Works entry missing");
      works.click();
    }, id);
    await expect(page.locator('.works-surface:not(.works-surface--preview)')).toBeVisible();
    await expect.poll(async () => {
      expect(unexpectedTool, "unexpected native tool call").toBe(false);
      if (monitorError) throw monitorError;
      const rolloutRows = await rows();
      return rolloutRows.filter(row => row.type === "response_item" && row.payload?.type === "message" && row.payload?.role === "assistant")
        .flatMap(row => row.payload.content ?? []).map((part: any) => part.text ?? "").join("\n");
    }, { timeout: 180_000 }).toContain(`${marker}64`);
    expect(received.slice(beforeOutput).join(""), "assistant-only final numbered line arrived while PM was hidden").toContain(`${marker}64`);
    const rolloutRows = await rows();
    const prompts = rolloutRows.filter(row => row.type === "response_item" && row.payload?.type === "message" && row.payload?.role === "user");
    expect(prompts).toHaveLength(1);
    expect(JSON.stringify(prompts[0].payload.content)).toContain(marker);
    const response = rolloutRows.filter(row => row.type === "response_item" && row.payload?.type === "message" && row.payload?.role === "assistant")
      .flatMap(row => row.payload.content ?? []).map((part: any) => part.text ?? "").join("\n");
    for (let n = 1; n <= 64; n++) expect(response).toContain(`${marker}${String(n).padStart(2, "0")}`);
    expect(rolloutRows.some(row => row.type === "response_item" && typeof row.payload?.type === "string" && row.payload.type.endsWith("_call"))).toBe(false);
    await expect.poll(async () => (await rows()).some(row => row.type === "event_msg" && row.payload?.type === "task_complete"), { timeout: 60_000 }).toBe(true);
    await page.locator("#op-pm-entry").click();
    await expect.poll(buffer, { timeout: 60_000 }).toContain("Ask Codex");
    await expect.poll(async () => (await metrics()).baseY).toBeGreaterThan(0);
    const box = await pm.locator(".terminal-root").boundingBox();
    expect(box).not.toBeNull();
    const bottom = await metrics();
    await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
    await page.mouse.wheel(0, -900);
    await expect.poll(async () => (await metrics()).viewportY).toBeLessThan(bottom.viewportY);
    const reading = await metrics(), history = await buffer();
    await pm.locator(".terminal-root .xterm").evaluate(node => { (window as any).__ownedPmNative = node; });
    await pm.getByRole("button", { name: "Reports", exact: true }).click();
    await pm.getByRole("button", { name: "Execution log", exact: true }).click();
    expect((await metrics()).viewportY).toBe(reading.viewportY);
    expect(await buffer()).toBe(history);
    await page.getByRole("button", { name: "Works", exact: true }).click();
    const row = page.locator('.works-surface:not(.works-surface--preview) .work-row').filter({ hasText: "Owned Codex PM" });
    await row.getByRole("button", { name: "Open terminal" }).click();
    await expect(page.locator(".works-full")).toBeVisible();
    expect((await metrics()).viewportY).toBe(reading.viewportY);
    await page.locator('.op-rail__surface[data-surface="agents"]').click();
    await page.getByRole("button", { name: "Works", exact: true }).click();
    await expect(page.locator(".works-full")).toBeVisible();
    expect((await metrics()).viewportY).toBe(reading.viewportY);
    expect(await buffer()).toBe(history);
    const sequence = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
    await sendLiveGwtEvent(page, { kind: "frontend_ready" });
    await page.waitForFunction(({ after, id }) => ((window as any).__gwtPlaywrightMessages ?? []).some((row: any) =>
      row.sequence > after && row.payload.kind === "terminal_snapshot" && row.payload.id === id), { after: sequence, id }, { timeout: 30_000 });
    await expect(page.locator(".works-full")).toBeVisible();
    expect((await metrics()).viewportY).toBe(reading.viewportY);
    expect(await buffer()).toContain(`${marker}64`);
    expect(await page.locator(`.works-terminal[data-window-id="${id}"] .xterm`).evaluate(node => node === (window as any).__ownedPmNative)).toBe(true);
    await page.locator("#op-pm-entry").click();
    await page.evaluate(id => (window as any).__gwtTerminalTestApi.scrollToBottom(id), id);
    await expect.poll(buffer).toContain("Ask Codex");
    const canary = `UNSENT${theme.toUpperCase()}4777`;
    const beforeComposer = received.length;
    await input.focus();
    await page.keyboard.type(canary);
    await expect.poll(buffer).toContain(canary);
    expect((await buffer()).split(canary)).toHaveLength(2);
    await expect.poll(() => received.slice(beforeComposer).join("")).toContain(canary);
    const beforeUndo = received.length;
    for (let i = 0; i < canary.length; i++) await page.keyboard.press("Backspace");
    await expect.poll(() => received.length).toBeGreaterThan(beforeUndo);
    await expect.poll(buffer).not.toContain(canary);
    expect(unexpectedTool, "no native tool call during the owned turn").toBe(false);
    if (monitorError) throw monitorError;
    const finalActivity = await activity();
    const finalRuntime = await runtime();
    expect(finalRuntime).toMatchObject({ child_pid: initialRuntime.child_pid, child_started_at: initialRuntime.child_started_at,
      host_started_at: initialRuntime.host_started_at, runtime_incarnation: incarnation });
    expect(matchesIdentity(finalActivity, id, session, incarnation), "latest own-project census retains the exact PM identity").toBe(true);
    expect(finalActivity.host_id).toBe(initialActivity.host_id);
    expect(ps(captured.pid, "lstart=")).toBe(captured.birth);
    expect(ps(captured.pid, "command=")).toContain(nativePath);
    expect(createHash("sha256").update(await readFile(nativePath)).digest("hex")).toBe(hash);
    const finalVersion = spawnSync(nativePath, ["--version"], { encoding: "utf8" });
    if (finalVersion.error) throw finalVersion.error;
    expect(finalVersion.status).toBe(0);
    expect(finalVersion.stdout.trim()).toBe(version.stdout.trim());
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    expect(errors).toEqual([]);
    await info.attach(`actual-pm-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
    await info.attach("actual-pm-identity", { body: JSON.stringify({ theme, session, id, host: initialActivity.host_id, incarnation, nativePath, hash, version: version.stdout.trim(), pid: captured.pid, birth: captured.birth, parent: captured.parent, ancestry: captured.ancestry, cwd: captured.cwd, childStartedAt: initialRuntime.child_started_at, hostStartedAt: initialRuntime.host_started_at }), contentType: "application/json" });
  } catch (error) {
    primaryError = error;
  } finally {
    await page.goto("about:blank").catch(() => {});
    try { await fixture?.stop(); } catch (error) { cleanupErrors.push(error); }
    try {
      // Never infer ownership from a name alone; an exact birth and executable
      // must still match the record made before exec before sending a signal.
      if (!captured && home) {
        const lines = (await readFile(join(home, "native-owner"), "utf8").catch(() => "")).trim().split("\n");
        const match = lines[3]?.trim().match(/^(.*?)\s+(\d+)$/);
        if (match && ownedGuiPid) captured = { pid: Number(lines[0]), session: lines[1], cwd: lines[2], birth: match[1], parent: match[2], ancestry: lines.slice(4).map(Number) };
      }
      if (captured && ownedNative()) {
        process.kill(captured.pid, "SIGTERM");
        await expect.poll(() => ps(captured!.pid, "lstart="), { timeout: 10_000 }).not.toBe(captured.birth);
      }
      if (captured) expect(ps(captured.pid, "lstart="), "owned native PID must exit").not.toBe(captured.birth);
    } catch (error) {
      cleanupErrors.push(error);
    } finally {
      monitoring = false;
      try { await monitor; } catch (error) { cleanupErrors.push(error); }
      try { if (home) await rm(join(home, ".codex", "auth.json"), { force: true }); }
      catch (error) { cleanupErrors.push(error); }
    }
    // The watcher remains active through fixture.stop and native cleanup. Check its final state
    // after joining and cleanup so a late tool call cannot become a pass.
    if (unexpectedTool) cleanupErrors.push(new Error("Unexpected native tool call observed"));
    if (monitorError) cleanupErrors.push(monitorError);
  }
  if (primaryError || cleanupErrors.length) throw new AggregateError(
    [...(primaryError ? [primaryError] : []), ...cleanupErrors], "Owned PM proof or cleanup failed");
});
