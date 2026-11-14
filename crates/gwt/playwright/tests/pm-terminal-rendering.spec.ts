import { expect, test, type Page } from "@playwright/test";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { withNativeHiddenPage } from "./_helpers/native-chromium";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const liveBase = process.env.GWT_PLAYWRIGHT_BASE_URL;
const appUrl = liveBase ? new URL(new URL(APP_URL).pathname, liveBase).href : APP_URL;
const PM = "pm-terminal";

// Issue #7030 AC-5: Codex can downgrade an unadvertised tier without exiting.
// Preserve its actual explanation through the same terminal protocol as PTY output.
test("Issue #7030 Ultrafast downgrade warning remains visible", async ({ page }, info) => {
  const errors: string[] = [];
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
  await pm.getByRole("button", { name: "Execution log", exact: true }).click();
  await expectGridConverged(page);
  const warning = "Configured service tier `ultrafast` is not advertised as supported for model `gpt-6-astra` and will be omitted from requests.";
  await emit(page, "terminal_output", warning + "\r\n");
  await expect.poll(async () => (await text(page)).replace(/\n/g, "")).toContain(warning);
  await expect(pm.locator(".xterm-screen")).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await info.attach(`ultrafast-warning-${theme}`, { body: await pm.screenshot(), contentType: "image/png" });
  expect(errors, "console and page errors").toEqual([]);
});

test("PM spinner redraw queued before resize keeps its original wrap grid", async ({ page }, info) => {
  const errors: string[] = [];
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
  await pm.getByRole("button", { name: "Execution log", exact: true }).click();
  await expectGridConverged(page);
  await emit(page, "terminal_output", Array.from({ length: 400 }, (_, i) => `Tool output ${i}\r\n`).join(""));
  await expect.poll(() => text(page)).toContain("Tool output 399");
  const cols = (await metrics(page)).cols;
  // The old-width repaint arrives in the same turn as the host resize. It
  // wraps once, then erases both physical spinner rows with relative CSI.
  await page.evaluate(({ cols }) => {
    const fixture = (window as any).__pmTerminalFixture;
    const bytes = "\x1b[2J\x1b[3J\x1b[HHEADER\r\n" + "* Spinning... ".padEnd(cols, "S")
      + "97\r\x1b[2K\x1b[1A\x1b[2KCURRENT";
    fixture.emit({ kind: "terminal_output", id: "pm-terminal", data_base64: btoa(bytes) });
    const host = document.querySelector('.workspace-window[data-id="pm-terminal"] .terminal-root') as HTMLElement;
    host.style.width = "480px";
    window.dispatchEvent(new Event("resize"));
  }, { cols });
  await expect.poll(async () => (await metrics(page)).cols).toBeLessThan(cols);
  try {
    await expect.poll(async () => (await text(page)).trim()).toBe("HEADER\nCURRENT");
  } finally {
    const screenshot = info.outputPath(`pm-spinner-resize-${theme}.png`);
    await pm.screenshot({ path: screenshot });
    await info.attach("spinner after resize", { path: screenshot, contentType: "image/png" });
    await info.attach("terminal buffer", { body: await text(page), contentType: "text/plain" });
  }
  expect(errors).toEqual([]);
});

// SPEC #5016 AC-HS1 / AC-HS2: the waiting-decision count is derived once.
// After a decision is answered in Decide, Home asks for the remaining one only.
test("Home and Decide read one waiting-decision count after an answer", async ({ page }, info) => {
  const errors: string[] = [];
  const theme = info.project.name.includes("light") ? "light" : "dark";
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
  await page.addInitScript(() => {
    const decision = (id: string, question: string) => ({ id, kind: "decision", created_at: "2026-11-12T00:00:00Z",
      structured: { summary: { result: question, impact: null, owner: "PM",
        decision: { question, recommendation: "Retry", reason: "fixture", options: ["Retry", "Wait"] } }, details: "", refs: [] } });
    (window as any).__pmFixtureReports = [decision("hs1-a", "Retry the launch?"), decision("hs1-b", "Ship the fix now?")];
    (window as any).__pmFixtureCurrent = { request_tasks: { state: "ready", revision: 1, total: 0, pending_intake: 0, tasks: [] } };
  });
  if (!liveBase) await installEmbeddedRoutes(page);
  await installPmBackend(page);
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto(appUrl);
  await page.locator("#op-pm-entry").click();
  const pm = page.locator(`.workspace-window[data-id="${PM}"]`);
  await pm.getByRole("button", { name: "Home", exact: true }).click();
  await expect(pm.locator(".task-home")).toContainText("2 decisions need your attention");
  // SPEC #5016 HSK: the status strip reads the same waiting set from the shared model.
  const strip = page.locator("#op-strip-issue-monitor");
  await expect(strip).toHaveAttribute("title", /Awaiting attention 2 \| PM decisions 2/);
  await pm.getByRole("button", { name: "Decide", exact: true }).click();
  await pm.locator(".task-decisions__card").first().getByRole("button", { name: "1. Retry" }).click();
  const send = await page.evaluate(() => (window as any).__pmTerminalFixture.sent.find((packet: any) => packet.kind === "pane_send_input"));
  expect(send, "the answer travels the chat reply path").toBeTruthy();
  await page.evaluate(requestId => (window as any).__pmTerminalFixture.emit({ kind: "pane_send_result", id: "pm-terminal",
    window_id: "pm-terminal", request_id: requestId, ok: true, error: null }), send.request_id);
  await expect(pm.locator(".task-decisions__card").first()).toContainText("Answer sent: Retry");
  await info.attach(`hs1-decide-${theme}`, { body: await pm.screenshot(), contentType: "image/png" });
  await pm.getByRole("button", { name: "Home", exact: true }).click();
  await expect(pm.locator(".task-home")).toContainText("1 decision needs your attention");
  await expect(strip).toHaveAttribute("title", /Awaiting attention 1 \| PM decisions 1/);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await info.attach(`hs1-home-${theme}`, { body: await pm.screenshot(), contentType: "image/png" });
  expect(errors, "console and page errors").toEqual([]);
});

// SPEC #5016 HSV: unresolved reviews from the PM current facts join the
// same awaiting-attention value the status strip reads.
test("status strip counts unresolved reviews from the PM current facts", async ({ page }, info) => {
  const errors: string[] = [];
  const theme = info.project.name.includes("light") ? "light" : "dark";
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
  await page.addInitScript(() => {
    (window as any).__pmFixtureCurrent = {
      facts: { source: "daemon", active_windows: [], review_attention: [
        { pr_number: 801, issue_numbers: [71], reason: "Changes requested" },
        { pr_number: 803, issue_numbers: [], reason: "2 unresolved review threads" },
      ] },
      request_tasks: { state: "ready", revision: 1, total: 0, pending_intake: 0, tasks: [] },
    };
  });
  if (!liveBase) await installEmbeddedRoutes(page);
  await installPmBackend(page);
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto(appUrl);
  await page.locator("#op-pm-entry").click();
  const pm = page.locator(`.workspace-window[data-id="${PM}"]`);
  await expect(pm).toBeVisible();
  const strip = page.locator("#op-strip-issue-monitor");
  await expect(strip).toHaveAttribute("title", /Awaiting attention 2 \| Unresolved reviews 2/);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await info.attach(`hsv-strip-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
  expect(errors, "console and page errors").toEqual([]);
});

// SPEC #5016 HSF: the PM Live facts needs_human line shows the shared value
// the status strip reads. A stopped (Held) Issue is not a pending decision,
// even though the raw facts list still names it.
test("PM Live facts and the status strip read one needs_human value", async ({ page }, info) => {
  const errors: string[] = [];
  const theme = info.project.name.includes("light") ? "light" : "dark";
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
  await page.addInitScript(() => {
    (window as any).__pmFixtureCurrent = {
      facts: { source: "daemon", active_windows: [], needs_human_count: 2 },
      request_tasks: { state: "ready", revision: 1, total: 0, pending_intake: 0, tasks: [] },
    };
  });
  if (!liveBase) await installEmbeddedRoutes(page);
  await installPmBackend(page);
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto(appUrl);
  await page.locator("#op-pm-entry").click();
  const pm = page.locator(`.workspace-window[data-id="${PM}"]`);
  await expect(pm).toBeVisible();
  await page.evaluate(() => (window as any).__pmTerminalFixture.emit({ kind: "issue_monitor_inbox", items: [
    { issue: { number: 91, title: "Decide the scope", state: "open", labels: [] }, state: "needs_human", error_message: "Scope decision" },
    { issue: { number: 92, title: "Held work", state: "open", labels: [] }, state: "needs_human", error_message: "stopped: held by PM" },
  ] }));
  await expect(page.locator("#op-strip-issue-monitor")).toHaveAttribute("title", /Needs human 1/);
  await expect(pm.locator('[aria-label="Live facts"]')).toContainText("needs_human: 1");
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await info.attach(`hsf-live-facts-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
  expect(errors, "console and page errors").toEqual([]);
});

// SPEC #6476 T-005pm (AC-1): on a phone the PM view toolbar stayed one 625px
// row in a 380px window, so Log / Reports / Execution log sat off screen, and
// the PM buttons, the report filter and the expanders were 18-31px tall.
test.describe("PM touch targets", () => {
  test.use({ hasTouch: true });
  test("PM views and their controls are reachable 44px tap targets on a phone", async ({ page }, info) => {
    const errors: string[] = [];
    const theme = info.project.name.includes("light") ? "light" : "dark";
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
    await page.addInitScript(() => {
      (window as any).__pmFixtureReports = [{ id: "t005pm-a", kind: "decision", created_at: "2026-11-14T00:00:00Z",
        structured: { summary: { result: "Retry the launch?", impact: null, owner: "PM",
          decision: { question: "Retry the launch?", recommendation: "Retry", reason: "fixture", options: ["Retry", "Wait"] } },
          details: "The last launch timed out.", refs: [] } }];
      (window as any).__pmFixtureCurrent = { request_tasks: { state: "ready", revision: 1, total: 1, pending_intake: 0, tasks: [{
        key: "t005pm-task", title: "Ship the fix", status: "in_progress", assignee: "agent-one", next_action: "Review the PR",
        criteria_preview: { purpose: "Fix the launch", completion_conditions: ["Tests pass"], completion_condition_total: 1 } }] } };
    });
    if (!liveBase) await installEmbeddedRoutes(page);
    await installPmBackend(page);
    await page.setViewportSize({ width: 1920, height: 1080 });
    await page.goto(appUrl);
    await page.locator("#op-pm-entry").click();
    const pm = page.locator(`.workspace-window[data-id="${PM}"] .pm-chat`);
    const views = pm.locator(".pm-chat__toolbar[role='group'] button");
    await expect(views).toHaveCount(8);
    const names = await views.allTextContents();
    const view = (name: string) => views.filter({ hasText: new RegExp(`^${name}$`) });
    const controls = () => pm.evaluate(root => [...root.querySelectorAll("button, select, summary")]
      .filter(node => (node as HTMLElement).offsetParent !== null)
      .map(node => {
        const box = node.getBoundingClientRect();
        return { name: node.getAttribute("aria-label") || node.textContent!.trim().slice(0, 30), left: box.left, right: box.right,
          width: box.width, height: box.height, minHeight: getComputedStyle(node).minHeight };
      }));
    // The fixture data renders the controls that only exist with PM data.
    await view("Decide").click();
    await expect(pm.getByRole("button", { name: "1. Retry" })).toBeVisible();
    await view("Tasks").click();
    await expect(pm.locator(".pm-chat__task-criteria > summary")).toBeVisible();
    await expect(pm.getByRole("button", { name: "View task details" })).toBeVisible();
    await view("Reports").click();
    await expect(pm.locator(".pm-chat__transcript summary").first()).toBeVisible();
    const minHeights = async () => {
      const seen: string[] = [];
      for (const name of names) {
        await view(name).click();
        seen.push(...(await controls()).map(control => `${name}:${control.name}:${control.minHeight}`));
      }
      return seen;
    };
    const desktop = await minHeights();
    expect(desktop.join("\n")).not.toContain(":44px");
    await page.setViewportSize({ width: 601, height: 900 });
    await expect.poll(minHeights, { message: "601px keeps the desktop control sizes" }).toEqual(desktop);
    for (const width of [320, 390]) {
      await page.setViewportSize({ width, height: 844 });
      const viewport = await page.evaluate(() => document.documentElement.clientWidth);
      for (const name of names) {
        await view(name).tap();
        await expect(view(name), `${name} opens at ${width}px`).toHaveAttribute("aria-pressed", "true");
        for (const control of await controls()) {
          expect(control.height, `${name} ${control.name} height at ${width}px`).toBeGreaterThanOrEqual(44);
          expect(control.width, `${name} ${control.name} width at ${width}px`).toBeGreaterThanOrEqual(44);
          expect(control.left, `${name} ${control.name} is on screen at ${width}px`).toBeGreaterThanOrEqual(0);
          expect(control.right, `${name} ${control.name} is on screen at ${width}px`).toBeLessThanOrEqual(viewport);
        }
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth),
          `${name} does not scroll sideways at ${width}px`).toBe(true);
      }
      await view("Tasks").tap();
      const criteria = pm.locator(".pm-chat__task-criteria");
      const before = await criteria.evaluate(node => (node as HTMLDetailsElement).open);
      const summary = (await criteria.locator("> summary").boundingBox())!;
      await criteria.locator("> summary").tap({ position: { x: summary.width / 2, y: summary.height - 4 } });
      await expect.poll(() => criteria.evaluate(node => (node as HTMLDetailsElement).open), "the enlarged summary area toggles").toBe(!before);
      await view("Home").tap();
      await pm.locator(".task-home__primary", { hasText: "Create task" }).tap();
      await expect(view("Chat"), "Create task opens the PM chat").toHaveAttribute("aria-pressed", "true");
      await expect(pm.locator(".pm-chat__draft")).toBeFocused();
      await info.attach(`t005pm-${width}-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
    }
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    expect(errors, "console and page errors").toEqual([]);
  });
});

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
  // Fail once inside the synchronous model subscriber, through the real
  // fixture WebSocket/dispatcher path. No fault hook ships in the product.
  await page.route("**/app.js", async route => {
    const response = liveBase ? await route.fetch() : undefined;
    let source = response ? await response.text() : readFileSync("crates/gwt/web/app.js", "utf8");
    for (const [stage, signature, bytes] of [
      ["merge", "mergeChunks: (chunks, windowId) => {", 'chunks.map(chunk => atob(chunk)).join("")'],
      ["write", "write: (windowId, text, onWritten) => {", "text"],
    ]) {
      expect(source.split(signature)).toHaveLength(2);
      source = source.replace(signature, `${signature}
        const fault = window.__terminalBatchFault;
        if (fault && windowId === "pm-terminal") {
          const bytes = ${bytes};
          fault.seen.push({ stage: "${stage}", bytes });
          if (fault.armed && fault.stage === "${stage}" && bytes.includes("FAILED BATCH")) {
            fault.armed = false;
            throw new Error("one-shot terminal ${stage} failure");
          }
        }`);
    }
    const snapshotDecode = "decodeTerminalSnapshotBoundary(decodeBase64(snapshot))";
    expect(source.split(snapshotDecode)).toHaveLength(2);
    source = source.replace(snapshotDecode, `(() => {
      const fault = window.__terminalBatchFault;
      if (fault?.snapshotFailures > 0) {
        fault.snapshotFailures--;
        throw new Error("one-shot replacement snapshot decode failure");
      }
      return ${snapshotDecode};
    })()`);
    const needle = "const packet = state.terminal;";
    expect(source.split(needle)).toHaveLength(2);
    await route.fulfill({ response, contentType: "text/javascript", body: source.replace(needle, `${needle}
        const fault = window.__terminalReceiveFault;
        if (fault && packet) {
          fault.seen.push(packet);
          if (fault.armed && packet.dataBase64 === fault.marker) {
            fault.armed = false;
            throw new Error('one-shot terminal subscriber failure');
          }
        }`) });
  });
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

  for (const kind of ["terminal_output", "terminal_snapshot"]) {
    const before = await page.evaluate(kind => {
      (window as any).__terminalReceiveFault = { armed: true, marker: btoa("FAILED PACKET"), seen: [] };
      const fixture = (window as any).__pmTerminalFixture;
      const connections = fixture.connections();
      fixture.emit({ kind, id: "pm-terminal", data_base64: btoa("FAILED PACKET") });
      fixture.emit({ kind: "terminal_output", id: "pm-terminal", data_base64: btoa("TORN SUFFIX") });
      return connections;
    }, kind);
    await expect.poll(() => page.evaluate(() => (window as any).__pmTerminalFixture.connections())).toBe(before + 1);
    await expect.poll(() => page.evaluate(() => (window as any).__terminalReceiveFault.seen
      .some((packet: any) => packet.kind === "terminal_snapshot" && atob(packet.dataBase64).includes("Reconnected status")))).toBe(true);
    expect(await page.evaluate(() => (window as any).__terminalReceiveFault.seen
      .some((packet: any) => atob(packet.dataBase64).includes("TORN SUFFIX")))).toBe(false);
    await emit(page, "terminal_output", `\r\nRecovered ${kind}`);
    await expect.poll(() => text(page)).toContain(`Recovered ${kind}`);
  }
  for (const scenario of ["merge", "write", "replacement-snapshot"]) {
    const stage = scenario === "replacement-snapshot" ? "write" : scenario;
    const recoveries = scenario === "replacement-snapshot" ? 2 : 1;
    const before = await page.evaluate(({ stage, recoveries }) => {
      (window as any).__terminalBatchFault = { stage, armed: true, seen: [], snapshotFailures: recoveries - 1 };
      const fixture = (window as any).__pmTerminalFixture;
      const connections = fixture.connections();
      fixture.emit({ kind: "terminal_output", id: "pm-terminal", data_base64: btoa("FAILED BATCH") });
      fixture.emit({ kind: "terminal_output", id: "pm-terminal", data_base64: btoa("TORN BATCH SUFFIX") });
      return connections;
    }, { stage, recoveries });
    await expect.poll(() => page.evaluate(() => (window as any).__pmTerminalFixture.connections())).toBe(before + recoveries);
    await expect.poll(async () => (await text(page)).trim()).toBe("Reconnected status");
    const seen = await page.evaluate(() => (window as any).__terminalBatchFault.seen);
    const failed = seen.findIndex((entry: any) => entry.stage === stage && entry.bytes.includes("FAILED BATCH"));
    expect(failed).toBeGreaterThanOrEqual(0);
    expect(seen.slice(failed + 1).some((entry: any) => entry.bytes.includes("TORN BATCH SUFFIX"))).toBe(false);
    await emit(page, "terminal_output", `\r\nRecovered batch ${scenario}`);
    await expect.poll(() => text(page)).toContain(`Recovered batch ${scenario}`);
    expect(await page.evaluate(() => (window as any).__pmTerminalFixture.connections())).toBe(before + recoveries);
  }
  const overflowConnections = await page.evaluate(() => {
    (window as any).__terminalBatchFault = { armed: false, seen: [] };
    const fixture = (window as any).__pmTerminalFixture;
    fixture.recoveryTail = "\r\nPOST SNAPSHOT SUFFIX";
    const before = fixture.connections();
    fixture.emit({ kind: "terminal_output", id: "pm-terminal", data_base64: btoa("X".repeat(512 * 1024 + 1)) });
    fixture.emit({ kind: "terminal_output", id: "pm-terminal", data_base64: btoa("DISCARDED OVERFLOW SUFFIX") });
    return before;
  });
  await expect.poll(() => page.evaluate(() => (window as any).__pmTerminalFixture.connections())).toBe(overflowConnections + 1);
  await expect.poll(async () => (await text(page)).trim()).toBe("Reconnected status\nPOST SNAPSHOT SUFFIX");
  expect(await page.evaluate(() => (window as any).__terminalBatchFault.seen
    .some((entry: any) => entry.bytes.includes("DISCARDED OVERFLOW SUFFIX")))).toBe(false);
  await emit(page, "terminal_output", "\r\nLive after overflow");
  await expect.poll(() => text(page)).toContain("Live after overflow");
  expect(await page.evaluate(() => (window as any).__pmTerminalFixture.connections())).toBe(overflowConnections + 1);
  const catalogConnections = await page.evaluate(() => {
    const fixture = (window as any).__pmTerminalFixture;
    fixture.recoveryTail = "\r\nCATALOG RECOVERY";
    const before = fixture.connections();
    fixture.emit({ kind: "terminal_output", id: "unmounted-retention-orphan", data_base64: btoa("ORPHAN PREFIX") });
    fixture.emitWorkspace(); // Same catalog: keyed DOM sync may be skipped.
    return before;
  });
  await expect.poll(() => page.evaluate(() => (window as any).__pmTerminalFixture.connections())).toBe(catalogConnections + 1);
  await expect.poll(async () => (await text(page)).trim()).toBe("Reconnected status\nCATALOG RECOVERY");
  await expect(page.locator('[data-id="unmounted-retention-orphan"]')).toHaveCount(0);
  await emit(page, "terminal_output", "\r\nLive after catalog recovery");
  await expect.poll(() => text(page)).toContain("Live after catalog recovery");
  expect(await page.evaluate(() => (window as any).__pmTerminalFixture.connections())).toBe(catalogConnections + 1);
  const coldConnections = await page.evaluate(() => {
    const fixture = (window as any).__pmTerminalFixture;
    fixture.addColdWindow(); // The browser has not received this catalog yet.
    fixture.recoveryTail = "\r\nCOLD RECOVERY";
    const before = fixture.connections();
    fixture.emit({ kind: "terminal_output", id: "cold-retention", data_base64: btoa("X".repeat(512 * 1024 + 1)) });
    return before;
  });
  await expect.poll(() => page.evaluate(() => (window as any).__pmTerminalFixture.connections())).toBe(coldConnections + 1);
  await expect.poll(async () => (await text(page)).trim()).toBe("Reconnected status\nCOLD RECOVERY");
  await page.getByRole("button", { name: "Works", exact: true }).click();
  await page.locator('.work-row[data-window-id="cold-retention"] .work-open').click();
  await expect(page.locator('.works-surface:not(.works-surface--preview) .works-terminal[data-window-id="cold-retention"]')).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).__gwtTerminalTestApi.metrics("cold-retention").isReady)).toBe(true);
  await expect.poll(async () => (await text(page, "cold-retention")).trim()).toBe("Cold snapshot");
  await page.evaluate(() => (window as any).__pmTerminalFixture.emit({ kind: "terminal_output", id: "cold-retention", data_base64: btoa("\r\nCOLD LIVE") }));
  await expect.poll(async () => (await text(page, "cold-retention")).trim()).toBe("Cold snapshot\nCOLD LIVE");
  await page.locator("#op-pm-entry").click();
  await pm.getByRole("button", { name: "Execution log", exact: true }).click();
  await emit(page, "terminal_output", "\r\nLive after cold recovery");
  await expect.poll(() => text(page)).toContain("Live after cold recovery");
  expect(await page.evaluate(() => (window as any).__pmTerminalFixture.connections())).toBe(coldConnections + 1);
  await expect(page.locator(".render-degradation-banner")).toBeVisible();

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
  const recovered = info.outputPath(`pm-receive-recovered-${theme}.png`);
  await pm.screenshot({ path: recovered });
  await info.attach("terminal receive recovery", { path: recovered, contentType: "image/png" });
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
async function text(page: Page, windowId = PM): Promise<string> {
  return page.evaluate(id => (window as any).__gwtTerminalTestApi.bufferText(id), windowId);
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

async function installPmBackend(page: Pick<Page, "addInitScript">): Promise<void> {
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
      recoveryTail: "",
      connections: () => connections,
      emit: (packet: unknown) => socket.emit(packet),
      emitWorkspace: () => socket.emit(workspace()),
      updateTitle: (id: string, title: string) => {
        const entry = windows.find(window => window.id === id);
        if (!entry) throw new Error(`Missing fixture window ${id}`);
        Object.assign(entry, { dynamic_title: title }); socket.emit(workspace());
      },
      addColdWindow: () => windows.push({ ...windows[0], id: "cold-retention", title: "Cold agent", preset: "agent", is_pm: false, session_id: "cold-retention-session" }),
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
          if (connections > 1) {
            this.emit({ kind: "terminal_snapshot", id: "pm-terminal", data_base64: btoa("\x1b[H\x1b[2JReconnected status") });
            if (fixture.recoveryTail) this.emit({ kind: "terminal_output", id: "pm-terminal", data_base64: btoa(fixture.recoveryTail) });
            if (windows.some(entry => entry.id === "cold-retention")) this.emit({ kind: "terminal_snapshot", id: "cold-retention", data_base64: btoa("Cold snapshot") });
            if (windows.some(entry => entry.id === "cold-retention")) this.emit({ kind: "active_work_projection", projection: {
              id: "cold-retention-live", active_works: [], local_agent_activity: {
                project_key: location.pathname.split("/").at(-1), host_id: "raw-cap-fixture", observed_at: new Date().toISOString(),
                agents: [{ window_id: "cold-retention", session_id: "cold-retention-session", runtime_incarnation: 1, is_pm: false, issue_number: null }],
              },
            } });
          }
        }, 0);
      }
      send(raw: string) {
        const packet = JSON.parse(raw);
        fixture.sent.push(packet);
        if (packet.kind === "load_pm_reports") this.emit({ kind: "pm_reports", id: packet.id, session_id: "pm-terminal-session", request_id: packet.request_id, snapshot: packet.version === null, version: { epoch: "fixture", revision: 1 }, reports: (window as any).__pmFixtureReports ?? [], current: (window as any).__pmFixtureCurrent ?? null });
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
      await expect.poll(async () => {
        await sendLiveGwtEvent(page, { kind: "get_project_execution_status" });
        return page.evaluate(() => (window as any).__gwtPlaywrightMessages
          .findLast((entry: any) => entry.payload.kind === "project_execution_status")?.payload.status);
      }, { timeout: 60_000 }).toMatchObject({ view_state: "open", degraded: false });
      const windows = () => page.evaluate(() => {
        const state = (window as any).__gwtPlaywrightMessages.findLast((entry: any) => entry.payload.kind === "workspace_state");
        return state?.payload.workspace.tabs.flatMap((tab: any) => tab.workspace.windows) || [];
      });
      const before = new Set((await windows()).map((entry: any) => entry.id));
      await sendLiveGwtEvent(page, { kind: "create_window", preset: "shell" });
      await expect.poll(async () => (await windows()).filter((entry: any) => entry.preset === "shell" && !before.has(entry.id))).toHaveLength(1);
      id = (await windows()).find((entry: any) => entry.preset === "shell" && !before.has(entry.id)).id;
      await expect.poll(() => page.evaluate(id => (window as any).__gwtPlaywrightMessages
        .findLast((entry: any) => entry.payload.kind === "terminal_status" && entry.payload.id === id)?.payload, id),
      { timeout: 60_000 }).toMatchObject({ status: "running" });
      const showShell = async () => {
        await sendLiveGwtEvent(page, { kind: "focus_window", id });
        await expect.poll(async () => (await windows()).reduce((top: any, entry: any) =>
          !top || (entry.z_index || 0) > (top.z_index || 0) ? entry : top, null)?.id).toBe(id);
        await page.reload();
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
      await grid("hidden-resize-reconnect");
      await page.locator("#split-view-button").click();
      await grid("split");
      await page.locator("#split-view-button").click();
      await grid("single");
      await shell.locator(".xterm-helper-textarea").focus();
      await page.keyboard.type("/model");
      await sendLiveGwtEvent(page, { kind: "terminal_input", id, data: "\x04" });
      // Unix PTYs can apply ONLCR to the probe's CRLF; ConPTY preserves it.
      await expect.poll(() => outputs.get(id!) || "").toMatch(/PTY_INPUT_1:2f6d6f64656c\r+\n/);
      await shell.locator(".xterm-helper-textarea").evaluate(node => {
        const clipboard = new DataTransfer();
        clipboard.setData("text/plain", "/model");
        node.dispatchEvent(new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: clipboard }));
      });
      await sendLiveGwtEvent(page, { kind: "terminal_input", id, data: "\x04" });
      await expect.poll(() => outputs.get(id!) || "").toMatch(/PTY_INPUT_2:2f6d6f64656c\r+\n/);
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

test("ready Works terminal follows model header layout while native hidden", async ({ page, playwright }, info) => {
  test.setTimeout(60_000);
  test.skip(!liveBase, "Requires the isolated browser-check checkout server");
  await page.goto("about:blank");
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const title = "Model task update " + "implement shared rendering and recover terminal state ".repeat(6).trim();
  const result = await withNativeHiddenPage(playwright.chromium, info, async nativePage => {
    await nativePage.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
    await installPmBackend(nativePage);
    await nativePage.goto(appUrl);
    await expect.poll(() => nativePage.evaluate(() => document.documentElement.dataset.theme)).toBe(theme);
    await expect.poll(() => nativePage.evaluate(() => !!(window as any).__pmTerminalFixture)).toBe(true);
    await nativePage.evaluate(() => { const fixture = (window as any).__pmTerminalFixture; fixture.addColdWindow(); fixture.reconnect(); });
    await nativePage.evaluate(() => (document.querySelector('[data-surface="works"]') as HTMLElement).click());
    await expect.poll(() => nativePage.evaluate(() => !!document.querySelector('.work-row[data-window-id="cold-retention"] .work-open'))).toBe(true);
    await nativePage.evaluate(() => (document.querySelector('.work-row[data-window-id="cold-retention"] .work-open') as HTMLElement).click());
    await expect.poll(() => nativePage.evaluate(() => (window as any).__gwtTerminalTestApi.metrics("cold-retention").isReady)).toBe(true);
    await nativePage.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  }, async title => {
    const selector = '.works-surface:not(.works-surface--preview) .works-terminal[data-window-id="cold-retention"]';
    const host = document.querySelector(selector)!, xterm = host.querySelector(".xterm");
    const capture = () => ({ ...(window as any).__gwtTerminalTestApi.metrics("cold-retention"),
      hostHeight: host.getBoundingClientRect().height, screenHeight: host.querySelector(".xterm-screen")!.getBoundingClientRect().height,
      header: document.querySelector('.works-surface:not(.works-surface--preview) .works-full-meta')!.textContent,
      sameHost: document.querySelector(selector) === host, sameXterm: host.querySelector(".xterm") === xterm,
      hidden: document.hidden, focused: document.hasFocus(),
      published: (window as any).__pmTerminalFixture.sent.filter((event: any) => event.kind === "update_terminal_grid" && event.id === "cold-retention").at(-1) });
    const before = capture(), started = performance.now();
    let focusEvents = 0, visibilityEvents = 0;
    const focus = () => focusEvents++, visibility = () => visibilityEvents++;
    window.addEventListener("focus", focus); host.addEventListener("focusin", focus); document.addEventListener("visibilitychange", visibility);
    return new Promise<any>(resolve => {
      const finish = (timedOut: boolean) => {
        observer.disconnect(); clearTimeout(timer);
        window.removeEventListener("focus", focus); host.removeEventListener("focusin", focus); document.removeEventListener("visibilitychange", visibility);
        resolve({ before, after: capture(), elapsedMs: performance.now() - started, timedOut, focusEvents, visibilityEvents });
      };
      const observer = new MutationObserver(() => {
        const after = capture(), cellHeight = before.screenHeight / before.rows;
        if (after.header?.includes(title) && after.hostHeight < before.hostHeight && after.rows < before.rows
          && after.screenHeight <= after.hostHeight && after.hostHeight - after.screenHeight < cellHeight + 1
          && after.published?.rows === after.rows && after.published?.cols === after.cols) finish(false);
      });
      const timer = setTimeout(() => finish(true), 1000);
      observer.observe(host.closest(".works-full")!, { childList: true, subtree: true, attributes: true, characterData: true });
      (window as any).__pmTerminalFixture.updateTitle("cold-retention", title);
    });
  }, title);
  await info.attach("native-ready-model-reflow.json", { contentType: "application/json", body: JSON.stringify(result) });
  expect(result.before.isReady).toBe(true); expect(result.after.isReady).toBe(true);
  expect(result.before.published).toMatchObject({ rows: result.before.rows, cols: result.before.cols });
  expect(result.before.screenHeight).toBeLessThanOrEqual(result.before.hostHeight);
  expect(result.before.hostHeight - result.before.screenHeight).toBeLessThan(result.before.screenHeight / result.before.rows + 1);
  expect(result.after.sameHost).toBe(true); expect(result.after.sameXterm).toBe(true);
  expect(result.after.header).toContain(title);
  expect(result.after.hostHeight).toBeGreaterThan(30); expect(result.after.hostHeight).toBeLessThan(result.before.hostHeight);
  expect(result.after.rows).toBeLessThan(result.before.rows);
  expect(result.after.published).toMatchObject({ rows: result.after.rows, cols: result.after.cols });
  expect(result.timedOut).toBe(false); expect(result.elapsedMs).toBeLessThanOrEqual(1000);
  expect([result.before.hidden, result.after.hidden]).toEqual([true, true]);
  expect([result.before.focused, result.after.focused]).toEqual([false, false]);
  expect(result.focusEvents).toBe(0); expect(result.visibilityEvents).toBe(0);
});
