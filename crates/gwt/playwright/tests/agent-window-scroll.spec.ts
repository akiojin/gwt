import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { APP_URL, APP_PROJECT_KEY, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { liveGwtProjectUrl } from "./_helpers/live-gwt";

const liveBase = process.env.GWT_PLAYWRIGHT_BASE_URL;
const appUrl = liveBase
  ? new URL(liveBase).pathname === "/" ? liveGwtProjectUrl(liveBase, APP_PROJECT_KEY) : liveBase
  : APP_URL;

test.describe("Agent window scrollback lifecycle", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("mouse motion cannot return history review to the live screen", async ({ page }, info) => {
    const errors = await openAgentsFixture(page, info.project.name);
    const id = "agent-1";
    const history = numberedLines("tracking", 1200).replace(/tracking \d+/g,
      line => `\x1b]8;;https://example.com\x1b\\${line}\x1b]8;;\x1b\\`);
    await emitTerminalText(page, id, history + "\x1b[?1003h\x1b[?1006h");
    await waitForScrollableTerminal(page, id);
    await expectWheelMovesScrollback(page, id, true, 1000);
    const before = await terminalScrollMetrics(page, id);
    const inputs = await terminalInputs(page, id);
    const box = await agentTile(page, id).locator(".xterm-screen").boundingBox();
    await page.mouse.move(box!.x + 40, box!.y + 40);
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    expect((await terminalScrollMetrics(page, id)).viewportY).toBe(before.viewportY);
    expect(await terminalInputs(page, id), "history coordinates stay local").toEqual(inputs);
    await expect(agentTile(page, id).locator(".xterm-screen")).toHaveClass(/xterm-cursor-pointer/);
    await scrollTerminalToBottom(page, id);
    await page.mouse.move(box!.x + 80, box!.y + 80);
    await expect.poll(async () => (await terminalInputs(page, id)).length).toBeGreaterThan(inputs.length);
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("Codex inline redraw without mouse tracking retains history", async ({ page }, info) => {
    const errors = await openAgentsFixture(page, info.project.name);
    const id = "agent-1";
    // Codex 0.167.0 on Windows ConPTY emits ED2 without mouse tracking.
    await emitTerminalText(page, id, numberedLines("Codex", 1200));
    await emitTerminalText(page, id, "\x1b[H\x1b[2J" + numberedLines("Codex redraw", 40));
    await waitForScrollableTerminal(page, id);
    const metrics = await terminalScrollMetrics(page, id);
    expect(metrics.bufferLength - metrics.rows).toBeGreaterThanOrEqual(1000);
    // Windows overrides wheel deltas; native xterm normalizes a large delta
    // to a notch elsewhere. Tracking history below still verifies 1,000 lines.
    const windows = await page.evaluate(() => /\bWin/i.test(navigator.platform));
    await expectWheelMovesScrollback(page, id, true, windows ? 1000 : 1);
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("keeps wheel scrollback alive after hidden-visible cycles and Plan-mode idle", async ({ page }, info) => {
    const errors = await openAgentsFixture(page, info.project.name);

    await expect(agentTile(page, "agent-1")).toBeVisible();
    await emitTerminalText(page, "agent-1", numberedLines("plan-cycle", 180));
    await emitTerminalStatus(page, "agent-1", "ready", "Plan mode input wait");
    await waitForScrollableTerminal(page, "agent-1");

    for (let index = 0; index < 5; index += 1) {
      await setActiveAgentTab(page, "agent-2");
      await expect(agentTile(page, "agent-1")).toBeHidden();
      await emitTerminalText(page, "agent-1", numberedLines(`hidden-${index}`, 20));
      // Inert native caches retain a measured grid while another work is full.
      // Output must parse immediately so visible Issue mirrors remain live.
      await expect.poll(() => page.evaluate(() => window.__gwtTerminalTestApi!.bufferText("agent-1")))
        .toContain(`hidden-${index} 020`);
      await expectPendingRefresh(page, "agent-1", false);

      await setActiveAgentTab(page, "agent-1");
      await expect(agentTile(page, "agent-1")).toBeVisible();
      await expectPendingRefresh(page, "agent-1", false);
      await waitForScrollableTerminal(page, "agent-1");
      await expectWheelMovesScrollback(page, "agent-1");
    }
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("keeps wheel scrollback alive after a terminal_snapshot reset while hidden", async ({ page }, info) => {
    const errors = await openAgentsFixture(page, info.project.name);

    await expect(agentTile(page, "agent-1")).toBeVisible();
    await emitTerminalText(page, "agent-1", numberedLines("before-snapshot", 120));
    await waitForScrollableTerminal(page, "agent-1");

    await setActiveAgentTab(page, "agent-2");
    await expect(agentTile(page, "agent-1")).toBeHidden();
    await emitTerminalSnapshot(page, "agent-1", numberedLines("snapshot-replay", 220));
    await expect.poll(() => page.evaluate(() => window.__gwtTerminalTestApi!.bufferText("agent-1")))
      .toContain("snapshot-replay 220");
    await expectPendingRefresh(page, "agent-1", false);

    await setActiveAgentTab(page, "agent-1");
    await expect(agentTile(page, "agent-1")).toBeVisible();
    await expectPendingRefresh(page, "agent-1", false);
    await waitForScrollableTerminal(page, "agent-1");
    await expectWheelMovesScrollback(page, "agent-1");
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("stopped and interrupted scrolling stays local while keyboard input remains enabled", async ({ page }, info) => {
    const errors = await openAgentsFixture(page, info.project.name);
    const id = "agent-1";
    const box = await agentTile(page, id).boundingBox();
    expect(box).not.toBeNull();
    // Position before enabling mouse tracking: pointer motion remains input.
    await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
    const wheel = async () => {
      await page.mouse.wheel(0, -96);
      await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    };

    for (const state of ["stopped", "interrupted"]) {
      await emitTerminalText(page, id, "\x1b[?1049h\x1b[?1003l\x1b[?1006l\x1b[HRetained terminal\r\n");
      await emitBackendEvent(page, { kind: "window_state", window_id: id, state });
      await expect(page.locator(`.workspace-window[data-id="${id}"]`)).toHaveAttribute("data-agent-state", state);
      await expect.poll(() => terminalScrollMetrics(page, id)).toMatchObject({ baseY: 0, hidden: false });
      const before = await terminalInputs(page, id);
      await wheel();
      expect(await terminalInputs(page, id), `${state} wheel must not send fallback keys`).toEqual(before);
      await emitTerminalText(page, id, `\x1b[?1003h\x1b[?1006h${state} tracking ready\r\n`);
      await expect.poll(() => page.evaluate(() => window.__gwtTerminalTestApi!.bufferText("agent-1")))
        .toContain(`${state} tracking ready`);
      await wheel();
      expect(await terminalInputs(page, id), `${state} wheel must not send mouse reports`).toEqual(before);

      await emitTerminalText(page, id, "\x1b[?1003l\x1b[?1006l\x1b[?1049l" + numberedLines(state, 150));
      await waitForScrollableTerminal(page, id);
      await expectWheelMovesScrollback(page, id, true);
      const input = agentTile(page, id).locator(".xterm-helper-textarea");
      await input.focus();
      const beforeTyping = (await terminalInputs(page, id)).length;
      await page.keyboard.type("x");
      await expect.poll(async () => (await terminalInputs(page, id)).slice(beforeTyping)).toContain("x");
      await page.screenshot({ path: info.outputPath(`scroll-${state}.png`) });
    }

    await emitTerminalText(page, id, "\x1b[?1049h\x1b[HRunning terminal\r\n");
    await emitBackendEvent(page, { kind: "window_state", window_id: id, state: "running" });
    await expect(page.locator(`.workspace-window[data-id="${id}"]`)).toHaveAttribute("data-agent-state", "running");
    await expect.poll(() => terminalScrollMetrics(page, id)).toMatchObject({ baseY: 0 });
    const before = (await terminalInputs(page, id)).length;
    await wheel();
    await expect.poll(async () => (await terminalInputs(page, id)).slice(before)).toContain("\x1b[5~");
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("surfaces runtime error detail without covering the terminal with an overlay", async ({ page }, info) => {
    const errors = await openAgentsFixture(page, info.project.name);

    await expect(agentTile(page, "agent-1")).toBeVisible();
    await emitTerminalStatus(page, "agent-1", "error", "Stop-block hit an error");
    // SPEC #5016 AC-HS1: the backend publishes the project aggregate the strip renders.
    await emitBackendEvent(page, { kind: "project_agent_aggregate", aggregate: {
      running_count: 0, block_count: 1, idle_count: 0, error_count: 1, unread: true, revision: 1 } });

    // SPEC-2014 US-36 (FR-120/121): the raw TTY must never be covered by a
    // foreground status/error overlay. Error detail surfaces in the window
    // status strip and the attention toast instead of an xterm overlay.
    const overlay = agentTile(page, "agent-1").locator(".terminal-overlay");
    await expect(overlay).toBeHidden();
    await expect(page.locator("#op-strip-error")).toHaveText("1");
    // SPEC #3206: attention renders in the shared alerts stack.
    await expect(page.locator(".toast-alerts__item").first()).toContainText("Stop-block hit an error");
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("drag selection follows rendered rows before and after scrollback and snapshot replay", async ({ page }, info) => {
    await prepareClipboard(page);
    const errors = await openAgentsFixture(page, info.project.name);
    await emitTerminalText(page, "agent-1", numberedLines("selection", 240));
    await waitForScrollableTerminal(page, "agent-1");
    await expectDragCopiesRenderedRows(page, info, "bottom");
    await expectWheelMovesScrollback(page, "agent-1");
    await expectDragCopiesRenderedRows(page, info, "scrollback");
    await setActiveAgentTab(page, "agent-2");
    await emitTerminalSnapshot(page, "agent-1", numberedLines("snapshot", 240));
    await setActiveAgentTab(page, "agent-1");
    await expect(agentTile(page, "agent-1").locator(".xterm-rows")).toContainText("snapshot");
    await waitForScrollableTerminal(page, "agent-1");
    await expectDragCopiesRenderedRows(page, info, "snapshot");
    expect(errors, "zero console/page errors").toEqual([]);
  });

  for (const { id, provider, tracking } of [
    { id: "agent-1", provider: "Codex", tracking: "\x1b[?1003h\x1b[?1006h" },
    { id: "agent-2", provider: "Claude Code", tracking: "\x1b[?1002h\x1b[?1006h" },
    { id: "shell-1", provider: "Shell", tracking: "" },
  ]) {
    test(`${provider} wheel and selection survive terminal host changes and restored history`, async ({ page }, info) => {
      test.setTimeout(45_000);
      await prepareClipboard(page);
      const errors = await openAgentsFixture(page, info.project.name);
      await setActiveAgentTab(page, id);
      await emitTerminalText(page, id, numberedLines(provider, 1200) + tracking);
      if (tracking) {
        const inputs = await terminalInputs(page, id);
        await agentTile(page, id).locator(".xterm-screen").click();
        await expect.poll(async () => (await terminalInputs(page, id)).length)
          .toBeGreaterThan(inputs.length);
      }

      const check = async (phase: string) => {
        await waitForScrollableTerminal(page, id);
        const metrics = await terminalScrollMetrics(page, id);
        expect(metrics.bufferLength - metrics.rows, `${provider} ${phase} retains 1,000 history rows`)
          .toBeGreaterThanOrEqual(1000);
        const container = id === "shell-1" ? agentTile(page, id).locator(".terminal-root") : agentTile(page, id);
        await expect(container, `${provider} ${phase} explains how to select text`).toHaveAttribute("title", /Shift\+drag.*Option\+drag/);
        // The 1,000-line wheel AC covers agent panes; Shell keeps native notch speed.
        await expectWheelMovesScrollback(page, id, true, id === "shell-1" ? 1 : 1000);
        await expectDragCopiesRenderedRows(page, info, `${id}-${phase}`, id, Boolean(tracking));
      };
      await check("initial");
      await setActiveAgentTab(page, id === "agent-1" ? "agent-2" : "agent-1");
      await setActiveAgentTab(page, id);
      await check("tab-switch");

      await page.setViewportSize({ width: 1280, height: 820 });
      await check("resize");
      await page.setViewportSize({ width: 1440, height: 900 });

      await page.getByRole("button", { name: "Split view", exact: true }).click();
      const right = page.getByRole("region", { name: "Right pane", exact: true });
      await right.getByRole("combobox", { name: "Right pane surface" }).selectOption(id === "shell-1" ? "settings" : "works");
      await right.getByRole("combobox", { name: "Right pane surface" }).focus();
      await setActiveAgentTab(page, id);
      await check("split");
      await page.getByRole("button", { name: "Close split", exact: true }).click();

      if (id !== "shell-1") {
        await page.evaluate(windowId => window.__agentScrollFixture!.remountInKanban(windowId), id);
        await expect(page.locator('.agent-kanban-root[data-board-id="scroll-kanban"]')).toBeAttached();
        await expect(agentTile(page, id)).toBeVisible();
      } else {
        await setActiveAgentTab(page, "agent-1");
        await setActiveAgentTab(page, id);
      }
      await check("remount");
      await emitTerminalSnapshot(page, id, numberedLines(`${provider} replay`, 1200) + tracking);
      await expect.poll(() => page.evaluate(windowId => window.__gwtTerminalTestApi!.bufferText(windowId), id))
        .toContain(`${provider} replay 1200`);
      await check("snapshot");

      // The fixture backend persists the latest raw screen and replays it on a
      // new connection, exercising frontend restoration without a live provider.
      await page.reload();
      await setActiveAgentTab(page, id);
      await expect.poll(() => page.evaluate(windowId => window.__gwtTerminalTestApi!.bufferText(windowId), id))
        .toContain(`${provider} replay 1200`);
      await check("reload");
      expect(errors, "zero console/page errors across host changes and reload").toEqual([]);
    });
  }
});

// SPEC #6476 T-005 (FR-002, AC-2 automated part): on a phone the terminal is
// scrolled with a finger, not a wheel. A fullscreen agent has no scrollback for
// xterm's native touch scrolling to move, so a vertical swipe must reach the
// application exactly as the wheel does — PageUp/PageDown for agents that do
// not track the mouse, wheel mouse reports for agents that do — while retained
// history keeps xterm's own touch scrolling with no application input.
test.describe("Agent window touch scroll on a phone viewport", () => {
  test.use({
    deviceScaleFactor: 1,
    hasTouch: true,
    viewport: { width: 390, height: 844 },
  });

  // SPEC #6476 T-005r (AC-1): the Command Rail groups wrapped separately into
  // three rows on a phone (213px at 390px) and left the agent terminal 312px.
  // Every entry must stay visible without scrolling (#4777) as a 44px tap
  // target, in at most two rows at 390px and three at 320px.
  test("command rail stays compact on a phone and leaves the screen to the terminal", async ({ page }, info) => {
    const errors = await openAgentsFixture(page, info.project.name);
    for (const [width, rows] of [[320, 3], [390, 2]] as const) {
      await page.setViewportSize({ width, height: 844 });
      const rail = (await page.locator(".op-rail").boundingBox())!;
      const bar = (await page.locator(".project-bar").boundingBox())!;
      expect(rail.height, `rail height at ${width}px`).toBeLessThanOrEqual(rows * 48);
      expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth),
        "the page itself must not scroll sideways").toBeLessThanOrEqual(0);
      expect(await page.locator(".op-rail").evaluate(element => element.scrollWidth - element.clientWidth),
        "the rail must not hide entries behind a sideways scroll").toBeLessThanOrEqual(0);
      const items = page.locator(".op-rail__item:visible");
      expect(await items.count()).toBeGreaterThanOrEqual(11);
      for (const item of await items.all()) {
        await expect(item).toBeInViewport({ ratio: 1 });
        const box = (await item.boundingBox())!;
        expect(box.width, await item.innerText()).toBeGreaterThanOrEqual(44);
        expect(box.height, await item.innerText()).toBeGreaterThanOrEqual(44);
        expect(box.y + box.height).toBeLessThanOrEqual(rail.y + rail.height + 1);
        expect(await item.evaluate(element => {
          const r = element.getBoundingClientRect();
          return element.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
        }), "rail entry must be tappable").toBe(true);
      }
      const surface = (await page.locator(".surface-area").boundingBox())!;
      expect(surface.y, "the surface starts right below the rail").toBeLessThanOrEqual(bar.y + bar.height + rows * 48 + 1);
      if (width === 390) {
        const screen = (await agentTile(page, "agent-1").locator(".xterm-screen").boundingBox())!;
        expect(screen.height, "the terminal gains the rows the wrapped rail took (312px before)").toBeGreaterThanOrEqual(412);
      }
      await page.screenshot({ path: info.outputPath(`mobile-rail-${width}.png`) });
    }
    await page.getByRole("button", { name: "Logs", exact: true }).tap();
    await page.getByRole("button", { name: "Works", exact: true }).tap();
    await expect(agentTile(page, "agent-1")).toBeVisible();
    await page.setViewportSize({ width: 1360, height: 900 });
    const desktop = (await page.locator(".op-rail").boundingBox())!;
    expect(desktop.height, "desktop rail remains a vertical column").toBeGreaterThan(desktop.width);
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("swipe sends PageUp/PageDown to a fullscreen agent that does not track the mouse", async ({ page }, info) => {
    const errors = await openAgentsFixture(page, info.project.name);
    await setActiveAgentTab(page, "agent-2");
    await emitTerminalText(page, "agent-2", "\x1b[?1049h\x1b[Hfullscreen claude\r\n");
    await expect.poll(() => terminalScrollMetrics(page, "agent-2")).toMatchObject({ hidden: false, baseY: 0 });
    // baseY is also zero before parsing. Settle output-driven activation
    // before blur so its queued focus is not attributed to the gesture.
    await expect.poll(() => page.evaluate(() => window.__gwtTerminalTestApi!.bufferText("agent-2")))
      .toContain("fullscreen claude");
    await expectPendingRefresh(page, "agent-2", false);

    await page.evaluate(() => (document.activeElement as HTMLElement)?.blur());
    const before = (await terminalInputs(page, "agent-2")).length;
    await swipeTerminal(page, "agent-2", "down");
    await expect.poll(async () => (await terminalInputs(page, "agent-2")).slice(before))
      .toContain("\x1b[5~");
    const afterUp = (await terminalInputs(page, "agent-2")).length;
    await swipeTerminal(page, "agent-2", "up");
    await expect.poll(async () => (await terminalInputs(page, "agent-2")).slice(afterUp))
      .toContain("\x1b[6~");
    await expect(page.locator(".xterm-helper-textarea:focus"),
      "a swipe must not focus terminal input and summon the software keyboard").toHaveCount(0);
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("swipe reaches a mouse-tracking fullscreen agent as wheel reports, never PageUp", async ({ page }, info) => {
    const errors = await openAgentsFixture(page, info.project.name);
    await emitTerminalText(page, "agent-1", "\x1b[?1049h\x1b[?1003h\x1b[?1006h\x1b[Hfullscreen codex\r\n");
    await expect.poll(() => terminalScrollMetrics(page, "agent-1")).toMatchObject({ hidden: false, baseY: 0 });

    const before = (await terminalInputs(page, "agent-1")).length;
    await swipeTerminal(page, "agent-1", "down");
    await expect.poll(async () => (await terminalInputs(page, "agent-1")).slice(before).join(""))
      .toMatch(/\x1b\[<64;\d+;\d+M/);
    expect((await terminalInputs(page, "agent-1")).slice(before), "mouse-tracking agents must not get PageUp")
      .not.toContain("\x1b[5~");
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("swipe over retained history scrolls the viewport without application input", async ({ page }, info) => {
    const errors = await openAgentsFixture(page, info.project.name);
    await setActiveAgentTab(page, "agent-2");
    await emitTerminalText(page, "agent-2", numberedLines("touch-history", 240));
    await waitForScrollableTerminal(page, "agent-2");
    await scrollTerminalToBottom(page, "agent-2");
    const before = await terminalScrollMetrics(page, "agent-2");
    const inputsBefore = await terminalInputs(page, "agent-2");

    await swipeTerminal(page, "agent-2", "down");
    await expect.poll(async () => (await terminalScrollMetrics(page, "agent-2")).viewportY)
      .toBeLessThan(before.viewportY);
    expect(await terminalInputs(page, "agent-2"), "history swipe must not send PageUp or mouse reports")
      .toEqual(inputsBefore);
    expect(errors, "zero console/page errors").toEqual([]);
  });
});

// A real (trusted) single-finger swipe through Chromium's input pipeline.
// "down" moves the finger toward the bottom of the screen, which reveals
// earlier output, like a wheel scrolled up.
async function swipeTerminal(page: Page, windowId: string, direction: "up" | "down"): Promise<void> {
  const box = await agentTile(page, windowId).locator(".xterm-screen").boundingBox();
  expect(box).not.toBeNull();
  const x = Math.round(box!.x + box!.width / 2);
  const top = Math.round(box!.y + box!.height * 0.25);
  const bottom = Math.round(box!.y + box!.height * 0.75);
  const [from, to] = direction === "down" ? [top, bottom] : [bottom, top];
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x, y: from }] });
  const steps = 10;
  for (let step = 1; step <= steps; step += 1) {
    const y = Math.round(from + ((to - from) * step) / steps);
    await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x, y }] });
  }
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await cdp.detach();
}

async function prepareClipboard(page: Page): Promise<void> {
  if (liveBase) await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  else await page.addInitScript(() => {
    (window as any).__selectionCopy = "";
    (window as any).__nativeCopyEvents = [];
    document.addEventListener("copy", event => {
      const text = event.clipboardData?.getData("text/plain") ?? "";
      (window as any).__nativeCopyEvents.push(text);
      (window as any).__selectionCopy = text;
    });
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: {
      writeText: async (text: string) => { (window as any).__selectionCopy = text; },
      readText: async () => (window as any).__selectionCopy,
    } });
  });
}

async function expectDragCopiesRenderedRows(page: Page, info: TestInfo, phase: string, windowId = "agent-1", shift = false): Promise<void> {
  const platform = await page.evaluate(() => (navigator as any).userAgentData?.platform || navigator.platform || "");
  const modifier = /mac|iphone|ipad|ipod/i.test(platform) ? "Alt" : "Shift";
  const selection = await agentTile(page, windowId).locator(".xterm-screen").evaluate((screen, id) => {
    const rows = [...screen.querySelectorAll<HTMLElement>(".xterm-rows > div")];
    const start = rows[2].getBoundingClientRect();
    const end = rows[7].getBoundingClientRect();
    const cols = (window.__gwtTerminalTestApi!.metrics(id) as any).cols;
    const cellWidth = screen.getBoundingClientRect().width / cols;
    return { x1: start.x + 1, y1: start.y + start.height / 2,
      x2: end.x + (rows[7].textContent!.trimEnd().length + 0.1) * cellWidth,
      y2: end.y + end.height / 2,
      top: start.top, bottom: end.bottom,
      text: rows.slice(2, 8).map(row => row.textContent!.trimEnd()).join("\n") };
  }, windowId);
  const inputsBefore = shift ? await terminalInputs(page, windowId) : [];
  if (shift) await page.keyboard.down(modifier);
  await page.mouse.move(selection.x1, selection.y1);
  await page.mouse.down();
  await page.mouse.move(selection.x2, selection.y2, { steps: 12 });
  await page.mouse.up();
  if (shift) {
    expect(await terminalInputs(page, windowId), "forced selection must not report application mouse input")
      .toEqual(inputsBefore);
  }
  const highlight = agentTile(page, windowId).locator(".xterm-selection div");
  await expect(highlight.first()).toBeVisible();
  const first = await highlight.first().boundingBox();
  const last = await highlight.last().boundingBox();
  expect(Math.abs(first!.y - selection.top), "selection starts on dragged row").toBeLessThan(1);
  expect(Math.abs(last!.y + last!.height - selection.bottom), "selection ends on dragged row").toBeLessThan(1);
  await agentTile(page, windowId).screenshot({ path: info.outputPath(`selection-${phase}.png`) });
  await expect.poll(() => page.evaluate(async () => (await navigator.clipboard.readText()).replace(/\r\n/g, "\n")))
    .toBe(selection.text);
  await page.evaluate(() => navigator.clipboard.writeText("clipboard reset"));
  // Keep the forced-selection modifier and the pointer at the drag endpoint.
  // Moving over an ANY-tracking terminal reports mouse input, which xterm
  // correctly treats as application input and clears the selection.
  await page.mouse.down({ button: "right" });
  await page.mouse.up({ button: "right" });
  const copy = page.getByRole("menuitem", { name: "Copy", exact: true });
  await expect(copy).toBeEnabled();
  await copy.click();
  if (shift) await page.keyboard.up(modifier);
  await expect.poll(() => page.evaluate(async () => (await navigator.clipboard.readText()).replace(/\r\n/g, "\n")))
    .toBe(selection.text);
  await page.evaluate(() => navigator.clipboard.writeText("clipboard reset"));
  // Match the browser platform used by classifyTerminalCopyKeyEvent. macOS
  // intentionally leaves Ctrl-based keys on the PTY path and copies with Cmd+C.
  const shortcut = /mac|iphone|ipad|ipod/i.test(platform) ? "Meta+c" : /win/i.test(platform) ? "Control+c" : "Control+Shift+c";
  await info.attach(`copy-shortcut-${phase}`, { body: JSON.stringify({ platform, shortcut }), contentType: "application/json" });
  const nativeCopiesBefore = await page.evaluate(() => (window as any).__nativeCopyEvents?.length || 0);
  await page.keyboard.press(shortcut);
  if (!liveBase && /mac|iphone|ipad|ipod/i.test(platform)) {
    await expect.poll(() => page.evaluate(() => (window as any).__nativeCopyEvents.length)).toBeGreaterThan(nativeCopiesBefore);
    expect(await page.evaluate(() => (window as any).__nativeCopyEvents.at(-1))).toBe(selection.text);
  }
  await expect.poll(() => page.evaluate(async () => (await navigator.clipboard.readText()).replace(/\r\n/g, "\n")))
    .toBe(selection.text);
}

function agentTile(page: Page, windowId: string) {
  if (windowId === "shell-1") return page.locator('.workspace-window[data-id="shell-1"]');
  return page.locator(`.works-surface:not(.works-surface--preview) .works-terminal[data-window-id='${windowId}']`);
}

async function openAgentsFixture(page: Page, projectName: string): Promise<string[]> {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = projectName.includes("light") ? "light" : "dark";
  await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
  if (!liveBase) await installEmbeddedRoutes(page);
  await installAgentScrollBackend(page);
  await page.goto(appUrl);
  await page.getByRole("button", { name: "Works", exact: true }).click();
  await setActiveAgentTab(page, "agent-1");
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  return errors;
}

async function waitForScrollableTerminal(page: Page, windowId: string): Promise<void> {
  await expect.poll(() => terminalScrollMetrics(page, windowId)).toMatchObject({
    hasScrollback: true,
    hidden: false,
  });
  await waitForTerminalBufferIdle(page, windowId);
}

async function expectPendingRefresh(
  page: Page,
  windowId: string,
  expected: boolean,
): Promise<void> {
  await expect
    .poll(async () => {
      const metrics = await terminalScrollMetrics(page, windowId);
      return metrics.viewportRefreshPending;
    })
    .toBe(expected);
}

async function expectWheelMovesScrollback(page: Page, windowId: string, noApplicationInput = false, minimumLines = 1): Promise<void> {
  await expect
    .poll(async () => {
      // A host move can finish fitting after the first reset. Establish the
      // wheel's starting position against the current grid, without a sleep.
      await scrollTerminalToBottom(page, windowId);
      const metrics = await terminalScrollMetrics(page, windowId);
      return metrics.viewportY === metrics.baseY;
    })
    .toBe(true);
  const before = await terminalScrollMetrics(page, windowId);
  expect(before.hasScrollback).toBe(true);

  const root = agentTile(page, windowId);
  const box = await root.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(Math.round(box!.x + box!.width / 2), Math.round(box!.y + box!.height / 2));
  const inputsBefore = noApplicationInput ? await terminalInputs(page, windowId) : [];
  await page.mouse.wheel(0, -Math.max(900, minimumLines * 32));

  await expect
    .poll(async () => {
      const after = await terminalScrollMetrics(page, windowId);
      return after.viewportY;
    })
    .toBeLessThanOrEqual(before.viewportY - minimumLines);
  if (noApplicationInput) {
    expect(await terminalInputs(page, windowId), "history wheel must not send mouse reports or PageUp/PageDown").toEqual(inputsBefore);
  }
}

async function terminalInputs(page: Page, windowId: string): Promise<string[]> {
  return page.evaluate(id => window.__agentScrollFixture!.instance!.recordedSends
    .filter(message => message.kind === "terminal_input" && message.id === id).map(message => message.data), windowId);
}

async function waitForTerminalBufferIdle(page: Page, windowId: string): Promise<void> {
  let previousKey = "";
  let stableSamples = 0;
  await expect
    .poll(async () => {
      const metrics = await terminalScrollMetrics(page, windowId);
      const key = [
        metrics.hidden,
        metrics.baseY,
        metrics.bufferLength,
        metrics.viewportRefreshPending,
      ].join(":");
      if (
        key === previousKey &&
        metrics.hidden === false &&
        metrics.hasScrollback === true &&
        metrics.viewportRefreshPending === false
      ) {
        stableSamples += 1;
      } else {
        stableSamples = 0;
      }
      previousKey = key;
      return stableSamples;
    })
    .toBeGreaterThanOrEqual(2);
}

async function terminalScrollMetrics(page: Page, windowId: string): Promise<{
  hidden: boolean;
  viewportY: number;
  baseY: number;
  rows: number;
  bufferLength: number;
  hasScrollback: boolean;
  viewportRefreshPending: boolean;
}> {
  return page.evaluate((id) => {
    const tile = document.querySelector<HTMLElement>(id === "shell-1" ? '.workspace-window[data-id="shell-1"]'
      : `.works-surface:not(.works-surface--preview) .works-terminal[data-window-id='${id}']`);
    const metrics = window.__gwtTerminalTestApi?.metrics(id);
    const baseY = metrics?.baseY ?? 0;
    const rows = metrics?.rows ?? 0;
    return {
      hidden: !tile || !tile.getClientRects().length || getComputedStyle(tile).visibility === "hidden",
      viewportY: metrics?.viewportY ?? 0,
      baseY,
      rows,
      bufferLength: metrics?.bufferLength ?? 0,
      hasScrollback: baseY > 0 && baseY + rows > rows,
      viewportRefreshPending: metrics?.viewportRefreshPending ?? false,
    };
  }, windowId);
}

async function scrollTerminalToBottom(page: Page, windowId: string): Promise<void> {
  await page.evaluate((id) => {
    window.__gwtTerminalTestApi?.scrollToBottom(id);
  }, windowId);
}

async function emitTerminalText(page: Page, windowId: string, text: string): Promise<void> {
  await emitBackendEvent(page, {
    kind: "terminal_output",
    id: windowId,
    data_base64: await base64(page, text),
  });
}

async function emitTerminalSnapshot(page: Page, windowId: string, text: string): Promise<void> {
  await emitBackendEvent(page, {
    kind: "terminal_snapshot",
    id: windowId,
    data_base64: await base64(page, text),
  });
}

async function emitTerminalStatus(
  page: Page,
  windowId: string,
  status: string,
  detail: string,
): Promise<void> {
  await emitBackendEvent(page, {
    kind: "terminal_status",
    id: windowId,
    status,
    detail,
  });
}

async function setActiveAgentTab(page: Page, windowId: string): Promise<void> {
  if (windowId === "shell-1") {
    await page.evaluate(() => (window as any).__operatorShell.applyRuntimeHealth({ state: "ok", processes: [
      { pid: 303, role: "shell", name: "shell", focus_window_id: "shell-1" },
    ] }));
    await page.locator("#op-strip-runtime-health").hover();
    await page.getByRole("button", { name: "Focus shell process 303", exact: true }).click();
    await expect(agentTile(page, windowId)).toBeVisible();
    return;
  }
  await page.getByRole("button", { name: "Works", exact: true }).click();
  const back = page.locator('.works-surface:not(.works-surface--preview) .works-full:not([hidden]) [data-action="back"]');
  if (await back.isVisible()) await back.click();
  await page.locator(`.works-surface:not(.works-surface--preview) .work-row[data-window-id="${windowId}"] .work-open`).click();
  await expect(agentTile(page, windowId)).toBeVisible();
}

async function emitBackendEvent(page: Page, payload: unknown): Promise<void> {
  await page.evaluate((event) => {
    window.__agentScrollFixture?.emit(event);
  }, payload);
}

async function base64(page: Page, text: string): Promise<string> {
  return page.evaluate((value) => btoa(value), text);
}

function numberedLines(prefix: string, count: number): string {
  return Array.from({ length: count }, (_, index) => `${prefix} ${String(index + 1).padStart(3, "0")}`)
    .join("\r\n")
    .concat("\r\n");
}

async function installAgentScrollBackend(page: Page): Promise<void> {
  await page.addInitScript(() => {
    window.__gwtPlaywrightTestBridge = true;
    const restoredScreens: Record<string, string> = JSON.parse(sessionStorage.getItem("scroll-fixture-screens") || "{}");
    const remounted = new Set<string>();

    function agentWindow(id, active, x, zIndex) {
      return {
        id,
        title: id === "agent-1" ? "Codex Plan" : id === "agent-2" ? "Claude Review" : "Shell",
        preset: id === "shell-1" ? "shell" : "agent",
        geometry: { x, y: 110, width: 680, height: 440 },
        geometry_revision: 0,
        z_index: zIndex,
        status: active ? "ready" : "running",
        minimized: false,
        maximized: false,
        pre_maximize_geometry: null,
        persist: true,
        purpose_title: null,
        dynamic_title: null,
        dynamic_title_detail: null,
        agent_id: id === "shell-1" ? null : id === "agent-1" ? "codex" : "claude",
        agent_name: id === "agent-1" ? "Codex Plan" : id === "agent-2" ? "Claude Review" : null,
        session_id: id === "shell-1" ? null : `${id}-session`,
        agent_color: null,
        tab_group_id: id === "shell-1" ? null : "agent-scroll-group",
        tab_group_active: active,
        placement: remounted.has(id) ? {
          kind: "agent_kanban", board_id: "scroll-kanban", lane_id: "active", order: 0, collapsed: false,
        } : undefined,
      };
    }

    function workspaceState(activeId) {
      return {
        kind: "workspace_state",
        workspace: {
          app_version: "playwright",
          tabs: [
            {
              id: "tab-1",
              title: "Scroll Fixture",
              project_key: location.pathname.split("/").filter(Boolean).at(-1),
              project_root: "/fixture",
              kind: "git",
              workspace: {
                viewport: { x: 0, y: 0, zoom: 1 },
                windows: [
                  agentWindow("agent-1", activeId === "agent-1", 120, activeId === "agent-1" ? 3 : 1),
                  agentWindow("agent-2", activeId === "agent-2", 120, activeId === "agent-2" ? 3 : 1),
                  agentWindow("shell-1", false, 120, 0),
                  ...(remounted.size ? [{ ...agentWindow("scroll-kanban", false, 120, 0),
                    title: "Scroll Kanban", preset: "agent_kanban", agent_id: null, agent_name: null, session_id: null }] : []),
                ],
              },
            },
          ],
          active_tab_id: "tab-1",
          recent_projects: [],
        },
      };
    }

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        this.recordedSends = [];
        window.__agentScrollFixture.instance = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(workspaceState("agent-1"));
          this.emit({ kind: "active_work_projection", projection: { id: "fixture-local-live", active_works: [], local_agent_activity: { project_key: location.pathname.split("/").filter(Boolean).at(-1), host_id: "scroll-fixture", observed_at: new Date().toISOString(), agents: [
          { window_id: 'agent-1', session_id: 'agent-1-session', runtime_incarnation: 1, is_pm: false, issue_number: null },
          { window_id: 'agent-2', session_id: 'agent-2-session', runtime_incarnation: 1, is_pm: false, issue_number: null }
        ] } } });
          for (const [id, data] of Object.entries(restoredScreens)) {
            this.emit({ kind: "terminal_snapshot", id, data_base64: btoa(data) });
          }
        }, 0);
      }

      send(data) {
        this.recordedSends.push(JSON.parse(data));
      }

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload) {
        if (payload.kind === "terminal_output" || payload.kind === "terminal_snapshot") {
          const data = atob(payload.data_base64);
          restoredScreens[payload.id] = payload.kind === "terminal_snapshot" ? data : (restoredScreens[payload.id] || "") + data;
          sessionStorage.setItem("scroll-fixture-screens", JSON.stringify(restoredScreens));
        }
        setTimeout(() => {
          this.dispatchEvent(
            new MessageEvent("message", { data: JSON.stringify(payload) }),
          );
        }, 0);
      }
    }

    window.__agentScrollFixture = {
      instance: null,
      emit(payload) {
        this.instance?.emit(payload);
      },
      remountInKanban(id) {
        remounted.add(id);
        this.emit(workspaceState(id));
      },
    };

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}

declare global {
  interface Window {
    __agentScrollFixture?: {
      instance: { recordedSends: Array<{ kind?: string; id?: string; data: string }> } | null;
      emit(payload: unknown): void;
      remountInKanban(id: string): void;
    };
    __gwtPlaywrightTestBridge?: boolean;
    __gwtTerminalTestApi?: {
      bufferText(windowId: string): string;
      metrics(windowId: string): {
        baseY: number;
        rows: number;
        viewportY: number;
        viewportRefreshPending: boolean;
      };
      scrollToBottom(windowId: string): void;
    };
  }
}
