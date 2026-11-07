/* Issue #4777 T-19: approved Works list and reusable Agents roster.
 * Uses the surface-rail fixture's real assets + EventTarget WebSocket boundary.
 * No process liveness is inferred from this transport fixture; live provider
 * exit and owned GUI restart are covered by the separate live acceptance suite.
 */
import { expect, test, type Page } from "@playwright/test";
import { APP_PROJECT_KEY, APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { startWorksManagedSource } from "./_helpers/works-managed-source";

const appUrl = process.env.GWT_PLAYWRIGHT_BASE_URL
  ? new URL(new URL(APP_URL).pathname, process.env.GWT_PLAYWRIGHT_BASE_URL).href
  : APP_URL;
const capturedErrors = new WeakMap<Page, string[]>();
const main = (page: Page) => page.getByRole("region", { name: "Main pane", exact: true });
const works = (page: Page) => main(page).locator(".works-surface:not(.works-surface--preview)");
const roster = (page: Page) => main(page).locator(".agents-surface:not(.agents-surface--preview)");
const workRow = (page: Page, id: string) => works(page).locator(`.work-row[data-window-id="${id}"]`);
const full = (page: Page) => works(page).locator(".works-full");
const openRail = (page: Page, surface: string) => page.locator(`.op-rail__surface[data-surface="${surface}"]`).click();
const sent = (page: Page) => page.evaluate(() => [...(window as any).__worksAgentsFixture.sent]);

async function expectNoHorizontalOverflow(page: Page, selector: string) {
  await expect.poll(() => page.locator(selector).evaluate(node => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
}

test.describe("Works and Agents approved baseline", () => {
  test.setTimeout(60_000);
  test.use({ deviceScaleFactor: 1, viewport: { width: 1440, height: 1000 } });
  let managedSource: Awaited<ReturnType<typeof startWorksManagedSource>> | undefined;

  test.beforeAll(async ({ browser }, info) => {
    test.setTimeout(60_000);
    if (process.env.GWT_PLAYWRIGHT_WORKS_SOURCE === "1") {
      managedSource = await startWorksManagedSource(browser, info);
    }
  });

  test.afterAll(async () => {
    await managedSource?.stop();
    managedSource = undefined;
  });

  test.beforeEach(async ({ page }, info) => {
    await managedSource?.audit("before");
    const errors: string[] = []; capturedErrors.set(page, errors);
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await page.addInitScript(theme => {
      localStorage.setItem("gwt:ui:theme", theme);
      localStorage.setItem("gwt:ui:agent-columns", "3");
    }, info.project.name.includes("light") ? "light" : "dark");
    if (!managedSource && !process.env.GWT_PLAYWRIGHT_BASE_URL) await installEmbeddedRoutes(page);
    await installWorksAgentsBackend(page);
    await page.goto(managedSource
      ? new URL(new URL(APP_URL).pathname, managedSource.url).href
      : appUrl);
    await openRail(page, "works");
    await expect(main(page)).toHaveAttribute("data-surface", "works");
    await expect(workRow(page, "agent-one")).toBeVisible();
  });

  test.afterEach(async ({ page }, info) => {
    const failures: unknown[] = [];
    try {
      await expect(page.locator(".render-degradation-banner")).toHaveCount(0);
      await expect(page.locator("html")).toHaveAttribute("data-theme", info.project.name.includes("light") ? "light" : "dark");
      expect(capturedErrors.get(page), "zero console/page errors, including readback and navigation").toEqual([]);
      expect((await sent(page)).filter(message => ["close_window", "stop_agent", "stop_window", "restart_window"].includes(message.kind))).toEqual([]);
    } catch (error) {
      failures.push(error);
    }
    try {
      await managedSource?.audit("post");
    } catch (error) {
      failures.push(error);
    }
    if (failures.length === 1) throw failures[0];
    if (failures.length > 1) throw new AggregateError(failures, "Works assertions and source post proof failed");
  });

  test("live Works never disappear while activity and catalogs converge", async ({ page }, info) => {
    await page.evaluate(() => {
      const surface = document.querySelector(".works-surface:not(.works-surface--preview)")!;
      (window as any).__worksUnderTest = surface;
      const list = surface.querySelector(".works-list")!;
      (window as any).__emptyWorks = [];
      (window as any).__worksObserver = new MutationObserver(() => {
        if (!list.querySelector(".work-row")) (window as any).__emptyWorks.push("empty live list");
      });
      (window as any).__worksObserver.observe(list, { childList: true });
    });
    const row = workRow(page, "agent-one");
    const activity = async (session: string, incarnation: number) => {
      await page.evaluate(({ session, incarnation }) => (window as any).__worksAgentsFixture.activityForWorks(session, incarnation), { session, incarnation });
      await expect(row.locator(".work-purpose")).toHaveText(`Live ${session}`);
      await expect(works(page).locator(".work-row")).toHaveCount(1);
    };
    const catalog = async (session: string, placement = "active") => {
      const version = () => page.evaluate(() => Number((window as any).__worksUnderTest.dataset.catalogVersion));
      const previous = await version();
      await page.evaluate(({ session, placement }) => (window as any).__worksAgentsFixture.catalogForWorks(session, placement), { session, placement });
      await expect.poll(version).toBeGreaterThan(previous);
      expect(await page.evaluate(() => (window as any).__worksUnderTest.querySelectorAll(".work-row").length)).toBe(1);
      // Tab activation resets rail selection; an empty catalog shows the
      // project picker. Inspect the retained Works view during that gap.
      if (placement !== "empty") {
        await openRail(page, "works");
        if (await full(page).isVisible()) await full(page).getByRole("button", { name: "Back to Works" }).click();
        await expect(works(page).locator(".work-row")).toHaveCount(1);
      }
    };
    const rendered = () => page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => resolve())));

    // Each half is rendered separately; coalescing two same-frame updates
    // would hide the inconsistent membership this regression exercises.
    await activity("s-one", 1);
    await row.getByRole("button", { name: "Open terminal" }).click();
    await expect(full(page)).toBeVisible();
    const before = (await sent(page)).filter(message => message.kind === "terminal_input").length;
    await activity("activity-first", 4);
    await expect(full(page)).toBeHidden();
    await expect(row.locator(".work-open")).toBeDisabled();
    await rendered();
    await page.keyboard.press("x");
    expect((await sent(page)).filter(message => message.kind === "terminal_input")).toHaveLength(before);
    await catalog("activity-first");
    await expect(row.getByRole("button", { name: "Open terminal" })).toBeEnabled();
    await rendered();

    await catalog("catalog-first");
    await expect(row.locator(".work-open")).toBeDisabled();
    await rendered();
    await activity("catalog-first", 5);
    await expect(row.getByRole("button", { name: "Open terminal" })).toBeEnabled();
    await catalog("catalog-first", "background");
    await expect(row.getByRole("button", { name: "Open terminal" })).toBeEnabled();
    await rendered();
    await catalog("catalog-first", "empty");
    expect(await page.evaluate(() => (window as any).__worksUnderTest.querySelector(".work-open").disabled)).toBe(true);
    await rendered();
    await catalog("catalog-first");
    await expect(row.getByRole("button", { name: "Open terminal" })).toBeEnabled();
    await rendered();
    await expect(works(page).locator(".works-empty")).toBeHidden();
    expect(await page.evaluate(() => (window as any).__emptyWorks)).toEqual([]);
    await page.screenshot({ path: info.outputPath("works-converged.png") });
    await page.evaluate(() => {
      (window as any).__worksObserver.disconnect();
      (window as any).__worksAgentsFixture.retire("agent-one", false);
    });
    await expect(works(page).locator(".work-row")).toHaveCount(0);
    await expect(works(page).locator(".works-empty")).toHaveText("No local agents are active.");
  });

  test("same-identity Works keeps real xterm scrollback and input across unavailable activity, clear and catalog gaps at 400px", async ({ page }, info) => {
    await page.setViewportSize({ width: 400, height: 900 });
    await workRow(page, "agent-one").getByRole("button", { name: "Open terminal" }).click();
    await expect(full(page)).toBeVisible();
    const root = full(page).locator('.works-terminal[data-window-id="agent-one"]');
    const input = root.locator(".xterm-helper-textarea");
    await root.evaluate(node => {
      (window as any).__retainedWorks = { root: node, native: node.querySelector(".xterm"), input: node.querySelector("textarea") };
    });
    const metrics = () => page.evaluate(() => window.__gwtTerminalTestApi!.metrics("agent-one"));
    const received = () => page.evaluate(() => (window as any).__worksAgentsFixture.sent
      .filter((message: any) => message.kind === "terminal_input" && message.id === "agent-one").map((message: any) => message.data));
    const buffer = () => page.evaluate(() => window.__gwtTerminalTestApi!.bufferText("agent-one"));
    await page.evaluate(() => {
      (window as any).__worksAgentsFixture.echoInputForWorks();
      (window as any).__worksAgentsFixture.output("agent-one", Array.from({ length: 220 }, (_, index) => `retained ${String(index + 1).padStart(3, "0")}\r\n`).join(""));
    });
    await expect.poll(buffer).toContain("retained 220");
    await expect.poll(metrics).toMatchObject({ hasRuntime: true, isReady: true, readOnly: false, viewportRefreshPending: false });
    await input.focus(); await page.keyboard.type("draft-T790");
    await expect.poll(buffer).toContain("draft-T790");
    for (const phase of ["unknown", "clear", "catalog"] as const) {
      // Reuse the real-buffer/real-wheel method from agent-window-scroll.spec.ts;
      // these are xterm buffer coordinates, never synthetic DOM scroll markers.
      await page.evaluate(() => window.__gwtTerminalTestApi!.scrollToBottom("agent-one"));
      await expect.poll(async () => { const value = await metrics(); return value.viewportY === value.baseY; }).toBe(true);
      const bottom = await metrics(); expect(bottom.baseY).toBeGreaterThan(0);
      const box = await root.boundingBox(); expect(box).not.toBeNull();
      await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
      await page.mouse.wheel(0, -900);
      await expect.poll(async () => (await metrics()).viewportY).toBeLessThan(bottom.viewportY);
      const before = await metrics(), text = await buffer(), inputs = await received();
      if (phase === "catalog") await openRail(page, "agents");
      // As in terminal-input-disconnect.spec.ts, freeze the overlay grace timer
      // while checking real keyboard input during a disconnected transport.
      if (phase === "clear") { await page.clock.install(); await page.clock.pauseAt(new Date(Date.now() + 1000)); }
      await page.evaluate(phase => (window as any).__worksAgentsFixture.loseWorksProof(phase), phase);
      if (phase === "clear") await page.clock.runFor(100);
      await expect.poll(metrics).toMatchObject({ hasRuntime: true, readOnly: true });
      if (phase === "catalog") await openRail(page, "works");
      await expect(full(page)).toBeVisible();
      await expect(full(page).locator(".works-full-meta")).toContainText("Input unavailable");
      expect(await root.evaluate(node => {
        const saved = (window as any).__retainedWorks;
        return node === saved.root && node.querySelector(".xterm") === saved.native && node.querySelector("textarea") === saved.input;
      })).toBe(true);
      expect((await metrics()).viewportY, `${phase} preserves the actual scrolled xterm viewport`).toBe(before.viewportY);
      expect(await buffer()).toBe(text);
      await input.focus(); await page.keyboard.type("blocked");
      expect(await received(), `${phase} cannot forward stale keyboard input`).toEqual(inputs);
      if (phase === "clear") await page.clock.runFor(1000);
      await page.evaluate(phase => (window as any).__worksAgentsFixture.restoreWorksProof(phase), phase);
      if (phase === "clear") { await page.clock.runFor(100); await page.clock.resume(); }
      await expect.poll(metrics).toMatchObject({ hasRuntime: true, isReady: true, readOnly: false, viewportRefreshPending: false });
      await expect(full(page)).toBeVisible();
      expect(await root.evaluate(node => node.querySelector(".xterm") === (window as any).__retainedWorks.native)).toBe(true);
      expect((await metrics()).viewportY).toBe(before.viewportY);
      expect(await buffer()).toBe(text);
      await input.focus(); await page.keyboard.type(`fresh-${phase}`);
      await expect.poll(async () => (await received()).join("")).toContain(`fresh-${phase}`);
      await expect.poll(buffer).toContain(`fresh-${phase}`);
      await page.keyboard.press("Enter");
      await expectNoHorizontalOverflow(page, ".works-full");
      await info.attach(`works-retained-${phase}`, { body: JSON.stringify({ before, recovered: await metrics(), inputCount: (await received()).length }), contentType: "application/json" });
    }
    // Enable actual xterm alternate-buffer + SGR mouse tracking with terminal
    // output. Real wheel input must report mouse events, with no PageUp fallback.
    await page.evaluate(() => (window as any).__worksAgentsFixture.output("agent-one", "\u001b[?1049h\u001b[?1000h\u001b[?1006h"));
    await expect(root.locator(".xterm")).toHaveClass(/enable-mouse-events/);
    await expect.poll(async () => (await metrics()).baseY).toBe(0);
    const mouseStart = (await received()).length;
    const box = await root.boundingBox();
    await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
    await page.mouse.wheel(0, -300);
    await expect.poll(async () => (await received()).slice(mouseStart).some((data: string) => /\u001b\[<64;\d+;\d+M/.test(data))).toBe(true);
    expect((await received()).slice(mouseStart).some((data: string) => data.includes("\u001b[5~") || data.includes("\u001b[6~"))).toBe(false);
    await page.evaluate(() => (window as any).__worksAgentsFixture.output("agent-one", "\u001b[?1000l\u001b[?1006l\u001b[?1049l"));
    await expect(root.locator(".xterm")).not.toHaveClass(/enable-mouse-events/);
    await expect.poll(buffer).toContain("draft-T790");
    await page.screenshot({ path: info.outputPath("works-retained-real-xterm-400px.png") });
    const previousInputs = (await received()).length;
    await page.evaluate(() => (window as any).__worksAgentsFixture.rebindForWorks());
    await expect(full(page)).toBeHidden();
    await expect(workRow(page, "agent-one").locator(".work-open")).toBeEnabled();
    await workRow(page, "agent-one").locator(".work-open").click();
    await expect(full(page)).toBeVisible();
    expect(await full(page).locator(".xterm").evaluate(node => node !== (window as any).__retainedWorks.native)).toBe(true);
    await expect.poll(buffer).toContain("fresh rebound snapshot");
    expect(await buffer()).not.toContain("draft-T790");
    await expect.poll(metrics).toMatchObject({ readOnly: false, isReady: true });
    await full(page).locator(".xterm-helper-textarea").focus();
    await page.keyboard.type("new-session-input");
    await expect.poll(async () => (await received()).slice(previousInputs).join("")).toContain("new-session-input");
    await info.attach("works-rebound-native", { body: JSON.stringify(await metrics()), contentType: "application/json" });

  });

  test("workspace snapshots refresh Works and Agents while preserving native controls", async ({ page }, info) => {
    const terminal = works(page).locator('.work-runtime-cache[data-window-id="agent-one"] .xterm');
    await terminal.evaluate(node => { (window as any).__workspaceTerminal = node; });
    const rail = page.locator('.op-rail__surface[data-surface="works"]');
    await rail.focus();
    for (const title of ['First workspace snapshot', '<img src=x> latest workspace']) {
      await page.evaluate(title => (window as any).__worksAgentsFixture.workspaceTitle(title), title);
      await expect(workRow(page, "agent-one").locator(".work-purpose")).toHaveText(title);
      await expect(rail).toBeFocused();
      expect(await terminal.evaluate(node => node === (window as any).__workspaceTerminal)).toBe(true);
    }
    await page.screenshot({ path: info.outputPath("workspace-state-works.png") });
    await openRail(page, "agents");
    const current = roster(page).locator('.roster-row[data-member-id="m-one"] .roster-purpose');
    await expect(current).toHaveText('<img src=x> latest workspace');
    await expect(current.locator("img")).toHaveCount(0);
    for (const title of ['Shared workspace title', 'Shared workspace title']) {
      await page.evaluate(title => (window as any).__worksAgentsFixture.workspaceTitle(title), title);
      await expect(current).toHaveText(title);
    }
    await page.screenshot({ path: info.outputPath("workspace-state-agents.png") });
    await current.click();
    await expect(full(page)).toBeVisible();
    expect(await full(page).locator(".xterm").evaluate(node => node === (window as any).__workspaceTerminal)).toBe(true);
    await full(page).getByRole("button", { name: "Back to Works" }).click();
    await expect(workRow(page, "agent-one").locator(".work-purpose")).toHaveText("Shared workspace title");
  });

  test("Issue #6362 shared census preserves member, runtime and reservation counts", async ({ page }, info) => {
    await expect(page.locator("#op-strip-running")).toHaveText("2");
    await expect(page.locator("#op-strip-waiting")).toHaveText("1");
    await page.evaluate(() => (window as any).__worksAgentsFixture.output("agent-one", "Waiting for background terminal\r\n"));
    await openRail(page, "agents");
    await expect(roster(page).locator(".roster-capacity-note")).toContainText("Busy 2 / Team 3");
    await expect(roster(page).locator('.roster-row[data-member-id="m-one"] .roster-state')).toContainText("In Works");
    await page.screenshot({ path: info.outputPath("issue-6362-background-busy.png") });
    await page.evaluate(() => (window as any).__worksAgentsFixture.census(null));
    await expect(roster(page).locator(".roster-capacity-note")).toContainText("Busy — / Team 3");
    await expect(page.locator("#op-strip-running")).toHaveText("—");
    await page.evaluate(() => (window as any).__worksAgentsFixture.census(["pm-window", "agent-one"]));
    await expect(roster(page).locator(".roster-capacity-note")).toContainText("Busy 1 / Team 3");
    await expect(page.locator("#op-strip-running")).toHaveText("2");
    await expect(page.locator("#op-strip-waiting")).toHaveText("0");
    await openRail(page, "works");
    await expect(works(page).locator(".work-row")).toHaveCount(2);
    await expect(works(page).locator('[data-count="running"]')).toHaveText("2");
    await expect(workRow(page, "agent-one")).toBeVisible();
    await page.screenshot({ path: info.outputPath("issue-6362-shared-census.png") });

    // PM is live in Works and RUNNING, outside the implementation member pool.
    await page.evaluate(() => (window as any).__worksAgentsFixture.census(["pm-window"]));
    await expect(works(page).locator(".work-row")).toHaveCount(1);
    await expect(workRow(page, "pm-window").locator(".window-role-badge")).toHaveText("PM");
    await expect(page.locator("#op-strip-running")).toHaveText("1");
    await openRail(page, "agents");
    await expect(roster(page).locator(".roster-capacity-note")).toContainText("Busy 0 / Team 3");

    // An assigned idle child is still Busy while its runtime is counted as IDLE.
    await page.evaluate(() => {
      (window as any).__worksAgentsFixture.activityForWorks("s-one", 1);
      (window as any).__worksAgentsFixture.status("agent-one", "idle");
    });
    await expect(roster(page).locator(".roster-capacity-note")).toContainText("Busy 1 / Team 3");
    await expect(page.locator("#op-strip-running")).toHaveText("0");
    await expect(page.locator("#op-strip-idle")).toHaveText("1");
    await openRail(page, "works");
    await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => resolve())));
    await expect(works(page).locator(".work-row")).toHaveCount(1);
    await expect(workRow(page, "agent-one").locator(".work-state")).toHaveText("Idle");
    await page.evaluate(() => (window as any).__worksAgentsFixture.workspaceStatus("waiting"));
    await expect(workRow(page, "agent-one").locator(".work-state")).toHaveText("Waiting");
    await expect(page.locator("#op-strip-waiting")).toHaveText("1");
    await page.evaluate(() => (window as any).__worksAgentsFixture.status("agent-one", "running"));
    await expect(page.locator("#op-strip-running")).toHaveText("1");
    await expect(works(page).locator('[data-count="running"]')).toHaveText("1");
    await openRail(page, "agents");
    await expect(roster(page).locator(".roster-capacity-note")).toContainText("Busy 1 / Team 3");

    // A Monitor reservation retains its slot without inventing a live member.
    await page.evaluate(() => {
      (window as any).__worksAgentsFixture.census([]);
      (window as any).__worksAgentsFixture.monitorActive(1);
    });
    await expect(roster(page).locator(".roster-capacity-note")).toContainText("Busy 0 / Team 3");
    await expect(page.locator("#op-strip-running")).toHaveText("0");
    await expect(page.locator("#op-strip-issue-monitor-value")).toHaveText("Run Q0 A1/3");
    await openRail(page, "works");
    await expect(works(page).locator(".work-row")).toHaveCount(0);
    await expect(works(page).locator(".works-empty")).toBeVisible();
    await expect(works(page).locator(".works-empty")).toHaveText("No local agents are active.");
  });

  test("PM-first list opens the same native terminal, passes Esc and links exact current members", async ({ page }, info) => {
    await expect(works(page).locator(".work-row")).toHaveCount(3);
    expect(await works(page).locator(".work-row").evaluateAll(rows => rows.map(row => (row as HTMLElement).dataset.windowId))).toEqual(["pm-window", "agent-one", "agent-two"]);
    await expect(workRow(page, "pm-window").locator(".work-name")).toHaveText("Yuu");
    await expect(workRow(page, "pm-window").locator(".window-role-badge")).toHaveText("PM");
    await expect(workRow(page, "pm-window").locator(".work-member")).toHaveCount(0);
    for (const [state, count] of [["running", "2"], ["waiting", "1"], ["idle", "0"], ["error", "0"]]) await expect(works(page).locator(`[data-count="${state}"]`)).toHaveText(count);
    await expect(works(page).locator(".works-active")).toHaveText("Active 3");
    await expect(page.locator(".agents-grid, .agent-tile, .agents-tabs")).toHaveCount(0);
    await expect(page.getByRole("spinbutton", { name: "Agent columns" })).toHaveCount(0);
    await expect(works(page).locator('[data-metric="today-input"]')).toHaveText("—");
    await expect(works(page).locator('[data-metric="today-cache"]')).toHaveText("—");
    await expect(works(page).locator('[data-metric="today-output"]')).toHaveText("—");
    await expect(works(page).locator('[data-metric="today-cost"]')).toHaveText("—");
    await expect(works(page).locator('[data-metric="today-landed"]')).toHaveText("—");
    await expect(works(page).locator('[data-availability="today"]')).toContainText("No data");
    await expect(works(page).locator('[data-availability="fleet"]')).toContainText("No data");
    await expect(works(page).locator(".works-source-note")).toContainText("per-session USD");
    await expect(works(page).locator(".works-source-note")).toContainText("daily merge data");
    await expect(works(page).locator(".works-fleet svg")).toHaveAttribute("aria-label", /last 30 minutes/);
    await expect(works(page).locator(".works-fleet svg")).toHaveAttribute("data-known-count", "0");
    await expect(workRow(page, "agent-one").locator(".work-stats")).toContainText("Context remaining");
    await expect(workRow(page, "agent-one").locator(".work-stats")).toContainText("Verification");
    await expect(workRow(page, "agent-one").locator(".work-stats")).toContainText("Last action");
    await expect(works(page).locator('.work-runtime-cache[data-window-id="agent-one"]')).toHaveAttribute("inert", "");
    await expect(works(page).locator('.work-runtime-cache[data-window-id="agent-one"]')).toHaveAttribute("aria-hidden", "true");
    await expect(works(page).locator('.work-runtime-cache[data-window-id="agent-one"] .xterm')).toHaveCount(1);
    await works(page).locator('.work-runtime-cache[data-window-id="agent-one"] .xterm').evaluate(node => { (window as any).__worksNativeTerminal = node; });
    await page.evaluate(() => (window as any).__worksAgentsFixture.output("agent-one", "discarded line\r\n<span>literal</span>\r\n\u001b[32mgreen line\u001b[0m\r\nfinal line\r\n"));
    const tail = workRow(page, "agent-one").locator(".work-tail > .terminal-text-preview");
    await expect(tail).toHaveText("<span>literal</span>\ngreen line\nfinal line", { useInnerText: false });
    await expect(tail.locator("span")).toHaveCount(0);
    await page.evaluate(() => {
      (window as any).__worksAgentsFixture.output("pm-window", "Project supervision\r\nReviewing current work\r\nWaiting for agent updates\r\n");
      (window as any).__worksAgentsFixture.output("agent-two", "Reading the assigned issue\r\nChanges saved\r\nWaiting for input\r\n");
    });
    await expect(workRow(page, "pm-window").locator(".work-tail > .terminal-text-preview")).toHaveText("Project supervision\nReviewing current work\nWaiting for agent updates");
    await expect(workRow(page, "agent-two").locator(".work-tail > .terminal-text-preview")).toHaveText("Reading the assigned issue\nChanges saved\nWaiting for input");
    await page.evaluate(() => {
      const target = document.querySelector('.works-surface:not(.works-surface--preview)')!;
      const row = target.querySelector('[data-window-id="agent-one"].work-row')!;
      const probe = { active: true, frames: 0, empty: [] as string[], row,
        output: row.querySelector(".terminal-text-preview"),
        terminal: target.querySelector('.work-runtime-cache[data-window-id="agent-one"] .xterm') };
      (window as any).__worksReactivationProbe = probe;
      const sample = () => {
        if (!probe.active) return;
        if ((target as HTMLElement).getClientRects().length && !(target as HTMLElement).hidden) {
          probe.frames++;
          const current = target.querySelector('[data-window-id="agent-one"].work-row');
          if (!current?.textContent?.includes("Aster")
            || !current.querySelector(".terminal-text-preview")?.textContent?.includes("final line")) probe.empty.push("missing live output");
        }
        requestAnimationFrame(sample);
      };
      requestAnimationFrame(sample);
    });
    for (let turn = 0; turn < 3; turn++) {
      await openRail(page, "agents");
      await openRail(page, "works");
      await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    }
    const continuity = await page.evaluate(() => {
      const probe = (window as any).__worksReactivationProbe; probe.active = false;
      const target = document.querySelector('.works-surface:not(.works-surface--preview)')!;
      return { frames: probe.frames, empty: probe.empty,
        sameRow: target.querySelector('[data-window-id="agent-one"].work-row') === probe.row,
        sameOutput: target.querySelector('[data-window-id="agent-one"] .terminal-text-preview') === probe.output,
        sameTerminal: target.querySelector('.work-runtime-cache[data-window-id="agent-one"] .xterm') === probe.terminal };
    });
    expect(continuity.frames).toBeGreaterThan(0);
    expect(continuity.empty).toEqual([]);
    expect(continuity.sameRow && continuity.sameOutput && continuity.sameTerminal).toBe(true);
    await info.attach("works-reactivation-frames", { body: JSON.stringify(continuity), contentType: "application/json" });
    await page.screenshot({ path: info.outputPath("works-approved-list.png") });

    await workRow(page, "agent-one").focus(); await page.keyboard.press("Enter");
    await expect(full(page)).toBeVisible();
    expect(await full(page).locator(".xterm").evaluate(node => node === (window as any).__worksNativeTerminal)).toBe(true);
    const input = full(page).locator(".xterm-helper-textarea"); await input.focus();
    await input.press("Escape"); await input.press("q");
    await expect.poll(async () => (await sent(page)).some(message => message.kind === "terminal_input" && message.id === "agent-one" && message.data === "\u001b")).toBe(true);
    await expect.poll(async () => (await sent(page)).some(message => message.kind === "terminal_input" && message.id === "agent-one" && message.data === "q")).toBe(true);
    await expect(full(page)).toBeVisible();
    await expect(full(page).locator("input, form, [data-action='close'], [data-action='stop']")).toHaveCount(0);
    await full(page).getByRole("button", { name: "Back to Works" }).click();
    await expect(workRow(page, "agent-one")).toBeFocused();
    expect(await works(page).locator('.work-runtime-cache[data-window-id="agent-one"] .xterm').evaluate(node => node === (window as any).__worksNativeTerminal)).toBe(true);
    await page.keyboard.press("Space"); await expect(full(page)).toBeVisible();
    await full(page).getByRole("button", { name: "Previous work" }).click();
    await expect(full(page).locator(".works-full-name .work-name")).toHaveText("Yuu");
    await expect(full(page).locator(".works-full-name .window-role-badge")).toHaveText("PM");
    await full(page).getByRole("button", { name: "Next work" }).click();
    await expect(full(page).locator(".works-full-name")).toHaveText("Aster");
    await page.screenshot({ path: info.outputPath("works-native-terminal.png") });
    await full(page).getByRole("button", { name: "View Aster in Agents" }).press("Enter");
    await expect(main(page)).toHaveAttribute("data-surface", "agents");
    await expect(roster(page).locator('.roster-row[data-member-id="m-one"]')).toHaveAttribute("aria-selected", "true");
    await roster(page).locator('.roster-row[data-member-id="m-one"] .roster-current').press("Enter");
    await expect(main(page)).toHaveAttribute("data-surface", "works");
    expect(await full(page).locator(".xterm").evaluate(node => node === (window as any).__worksNativeTerminal)).toBe(true);
    await full(page).getByRole("button", { name: "Back to Works" }).click();
    await workRow(page, "agent-one").getByRole("button", { name: "View Aster in Agents" }).press("Enter");
    await expect(main(page)).toHaveAttribute("data-surface", "agents");
    const purpose = roster(page).locator('.roster-row[data-member-id="m-one"] .roster-purpose');
    await purpose.focus();
    await purpose.evaluate(node => { (window as any).__focusedRosterPurpose = node; });
    await page.evaluate(() => (window as any).__worksAgentsFixture.workspaceTitle("Updated purpose before Enter"));
    await expect(purpose).toHaveText("Updated purpose before Enter");
    expect(await purpose.evaluate(node => node === (window as any).__focusedRosterPurpose)).toBe(true);
    await expect(purpose).toBeFocused();
    await purpose.press("Enter");
    await expect(main(page)).toHaveAttribute("data-surface", "works");
    await expect(full(page).locator(".works-full-name")).toHaveText("Aster");

    await full(page).getByRole("button", { name: "Back to Works" }).click();
    await workRow(page, "agent-one").getByRole("button", { name: "View Aster in Agents" }).press("Enter");
    const now = roster(page).locator(".roster-now button");
    await now.focus();
    await now.evaluate(node => { (window as any).__focusedRosterNow = node; });
    await page.keyboard.down("Space");
    await page.evaluate(() => (window as any).__worksAgentsFixture.workspaceTitle("Updated current work while Space held"));
    await expect(now).toContainText("Updated current work while Space held");
    expect(await now.evaluate(node => node === (window as any).__focusedRosterNow)).toBe(true);
    await expect(now).toBeFocused();
    await page.keyboard.up("Space");
    await expect(main(page)).toHaveAttribute("data-surface", "works");
    expect(await full(page).locator(".xterm").evaluate(node => node === (window as any).__worksNativeTerminal)).toBe(true);

    await full(page).getByRole("button", { name: "Back to Works" }).click();
    await workRow(page, "agent-one").getByRole("button", { name: "View Aster in Agents" }).press("Enter");
    const memberOrder = () => roster(page).locator(".roster-row").evaluateAll(nodes => nodes.map(node => (node as HTMLElement).dataset.memberId));
    await page.evaluate(() => (window as any).__worksAgentsFixture.reorderMembers(["m-two", "m-one", "m-idle"]));
    await expect.poll(memberOrder).toEqual(["m-two", "m-one", "m-idle"]);
    await purpose.focus();
    await page.keyboard.down("Space");
    await page.evaluate(() => (window as any).__worksAgentsFixture.reorderMembers(["m-one", "m-two", "m-idle"]));
    await expect.poll(memberOrder).toEqual(["m-one", "m-two", "m-idle"]);
    expect(await purpose.evaluate(node => node === (window as any).__focusedRosterPurpose)).toBe(true);
    await expect(purpose).toBeFocused();
    await page.keyboard.up("Space");
    await expect(main(page)).toHaveAttribute("data-surface", "works");
    await expect(full(page).locator(".works-full-name")).toHaveText("Aster");
    expect(await full(page).locator(".xterm").evaluate(node => node === (window as any).__worksNativeTerminal)).toBe(true);
    await full(page).getByRole("button", { name: "Back to Works" }).click();
    for (const [state, label] of [["idle", "Idle"], ["error", "Error"], ["waiting", "Waiting"]]) {
      await page.evaluate(status => (window as any).__worksAgentsFixture.status("agent-two", status), state);
      await expect(workRow(page, "agent-two").locator(".work-state")).toHaveText(label);
      expect(await workRow(page, "agent-two").locator(".work-spark").evaluate(node => getComputedStyle(node).color))
        .toBe(await workRow(page, "agent-two").locator(".work-state").evaluate(node => getComputedStyle(node).color));
      await expect(works(page).locator(`[data-count="${state}"]`)).toHaveText("1");
    }
    for (const state of ["unavailable", "partial"]) {
      await page.evaluate(state => (window as any).__worksAgentsFixture.usage(state), state);
      await expect(works(page).locator('[data-availability="today"]')).toHaveAttribute("data-state", state);
      await expect(works(page).locator('[data-availability="fleet"]')).toHaveAttribute("data-state", state);
      await expect(works(page).locator('[data-availability="today"]')).toContainText(state === "unavailable"
        ? "Fixture transcript cannot be read" : "incomplete");
      await expect(works(page).locator('[data-metric="today-total"]')).toHaveText("—");
      await expect(works(page).locator('[data-metric="fleet-now"]')).toHaveText("—");
    }
    await page.evaluate(() => (window as any).__worksAgentsFixture.usage());
    await expect(works(page).locator('[data-availability="today"]')).toHaveAttribute("data-state", "measured");
    await expect(works(page).locator('[data-availability="fleet"]')).toHaveAttribute("data-state", "measured");
    await expect(works(page).locator('[data-metric="today-input"]')).toHaveText("0");
    await expect(works(page).locator('[data-metric="fleet-now"]')).toHaveText("0");
    await expect(works(page).locator(".works-fleet svg")).toHaveAttribute("data-known-count", "30");
    await expect(workRow(page, "agent-one").locator('[data-stat="context"]')).toHaveText("0%");
    await page.screenshot({ path: info.outputPath("works-measured-zero.png") });
  });

  test("telemetry updates retain native host focus and unchanged SVG children", async ({ page }, info) => {
    await page.evaluate(() => (window as any).__worksAgentsFixture.usage());
    const metric = works(page).locator('[data-metric="today-input"]');
    await expect(metric).toHaveText("0");
    await expect(works(page).locator(".works-fleet svg")).toHaveAttribute("data-known-count", "30");
    await workRow(page, "agent-one").focus(); await page.keyboard.press("Enter");
    const input = full(page).locator(".xterm-helper-textarea"); await input.focus();
    await full(page).locator(".works-terminal-host").evaluate(host => {
      const root = host.firstElementChild, terminal = host.querySelector(".xterm");
      const input = host.querySelector(".xterm-helper-textarea");
      const surface = host.closest(".works-surface")!;
      const svgs = [...surface.querySelectorAll(".works-fleet svg, .work-spark")];
      const observation = { host, root, terminal, input, svgs, removedHosts: 0, svgMutations: 0,
        observers: [] as MutationObserver[] };
      const native = new MutationObserver(records => {
        for (const record of records) if ([...record.removedNodes].includes(root!)) observation.removedHosts++;
      });
      native.observe(host, { childList: true }); observation.observers.push(native);
      for (const svg of svgs) {
        const observer = new MutationObserver(records => {
          for (const record of records) observation.svgMutations += record.addedNodes.length + record.removedNodes.length;
        });
        observer.observe(svg, { childList: true, subtree: true }); observation.observers.push(observer);
      }
      (window as any).__telemetryObservation = observation;
    });
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Performance.enable");
    const before = await cdp.send("Performance.getMetrics");
    for (let tick = 1; tick <= 20; tick++) {
      const previous = await metric.textContent();
      await page.evaluate(tick => (window as any).__worksAgentsFixture.usage("measured", tick), tick);
      await expect(metric).not.toHaveText(previous!);
    }
    const observation = await full(page).locator(".works-terminal-host").evaluate(host => {
      const state = (window as any).__telemetryObservation;
      state.observers.forEach((observer: MutationObserver) => observer.disconnect());
      return { sameHost: host === state.host, sameRoot: host.firstElementChild === state.root,
        sameTerminal: host.querySelector(".xterm") === state.terminal,
        sameSvgs: state.svgs.every((svg: Element) => svg.isConnected),
        focused: document.activeElement === state.input, removedHosts: state.removedHosts, svgMutations: state.svgMutations };
    });
    expect(observation).toEqual({ sameHost: true, sameRoot: true, sameTerminal: true, sameSvgs: true,
      focused: true, removedHosts: 0, svgMutations: 0 });
    await info.attach("renderer-telemetry-observation", { contentType: "application/json",
      body: JSON.stringify({ scope: "Chromium renderer metrics only; excludes native GUI CPU/RSS", updates: 20,
        observation, before, after: await cdp.send("Performance.getMetrics") }) });
    await cdp.detach();
    await full(page).locator(".works-full-header").evaluate(target => {
      (window as any).__gwtAttachmentUploader = async ({ file }: { file: File }) => ({
        upload_id: "telemetry-upload", filename: file.name, mime_type: file.type, size: file.size,
      });
      const transfer = new DataTransfer();
      transfer.items.add(new File(["fixture contents"], "fixture.txt", { type: "text/plain" }));
      target.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: transfer }));
      target.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: transfer }));
    });
    await expect.poll(async () => (await sent(page)).filter(message =>
      message.kind === "attach_files" && message.id === "agent-one")).toHaveLength(1);
    await page.evaluate(() => (window as any).__worksAgentsFixture.attachmentProgress());
    await expect(full(page).locator(".attachment-progress")).toContainText("Staging");
    await page.evaluate(() => (window as any).__worksAgentsFixture.remountInKanban());
    await expect(page.locator('.agent-kanban-root[data-board-id="telemetry-kanban"]')).toBeAttached();
    await page.evaluate(() => (window as any).__worksAgentsFixture.usage("measured", 21));
    await expect.poll(() => full(page).locator(".works-terminal-host").evaluate(host => {
      const state = (window as any).__telemetryObservation;
      return host.firstElementChild === state.root && host.querySelector(".xterm") === state.terminal
        && !!host.querySelector(".attachment-progress");
    })).toBe(true);
    await full(page).locator(".xterm-helper-textarea").press("k");
    await expect.poll(async () => (await sent(page)).some(message =>
      message.kind === "terminal_input" && message.id === "agent-one" && message.data === "k")).toBe(true);
    await full(page).getByRole("button", { name: "Back to Works" }).click();
    await page.screenshot({ path: info.outputPath("works-telemetry-preserved.png") });
  });

  test("saved Works output lines preserve raw text, readonly mirrors and native controls", async ({ page }, info) => {
    // Keep each source line short enough for the narrow split terminal grid;
    // the preview intentionally preserves parsed visual wraps, not source rows.
    const lines = ["body01", "body02", "body03", "body04", "body05", "body06", "body07", "body08", "<x>raw", "footer", "> input", "last"];
    const output = async () => page.evaluate(text => (window as any).__worksAgentsFixture.output("agent-one", `\u001b[2J\u001b[H${text}\r\n`), lines.join("\r\n"));
    const tail = () => page.locator('.works-surface:not(.works-surface--preview) .work-row[data-window-id="agent-one"] .terminal-text-preview');
    const expectLines = async (count: number, mirror = false) => {
      const surfaces = page.locator(mirror ? ".works-surface" : ".works-surface:not(.works-surface--preview)");
      await expect(surfaces).toHaveCount(mirror ? 2 : 1);
      for (const surface of await surfaces.all()) {
        await expect(surface).toHaveCSS("--works-preview-lines", String(count));
        await expect(surface.locator('.work-row[data-window-id="agent-one"] .terminal-text-preview'))
          .toHaveText(lines.slice(-count).join("\n"), { useInnerText: false });
        await expect(surface.locator('.work-row[data-window-id="agent-one"] .terminal-text-preview x')).toHaveCount(0);
        await expect(surface.locator(".work-row")).toHaveCount(3);
        await expect(surface.locator(".works-counts [data-count]")).toHaveCount(4);
        await expect(surface.locator(".works-today [data-metric]")).toHaveCount(6);
        await expect(surface.locator(".works-fleet svg")).toHaveAttribute("data-known-count", "0");
        await expect(surface.locator('.work-row[data-window-id="agent-one"] .work-stats [data-stat]')).toHaveCount(8);
      }
    };
    await output(); await expectLines(3);
    await works(page).locator('.work-runtime-cache[data-window-id="agent-one"] .xterm').evaluate(node => {
      (window as any).__worksLineTerminal = node;
    });
    await openRail(page, "settings");
    const input = page.getByRole("spinbutton", { name: "Works output lines", exact: true });
    await expect(input).toHaveValue("3");
    for (const count of [5, 10]) {
      await input.fill(String(count)); await input.press("Tab");
      await expect(page.locator('[data-role="system-settings-status"]')).toContainText("Saved system settings");
      await openRail(page, "works"); await expectLines(count);
      expect(await works(page).locator('.work-runtime-cache[data-window-id="agent-one"] .xterm')
        .evaluate(node => node === (window as any).__worksLineTerminal)).toBe(true);
      await openRail(page, "settings");
    }
    await page.screenshot({ path: info.outputPath("settings-works-output-lines.png") });
    const saves = (await sent(page)).filter(message => message.kind === "update_system_settings").length;
    await input.fill("2"); await input.press("Tab");
    await expect(page.locator('[data-role="system-settings-status"]')).toContainText("whole number from 3 to 10");
    expect((await sent(page)).filter(message => message.kind === "update_system_settings")).toHaveLength(saves);
    await openRail(page, "works"); await expectLines(10);
    await tail().click();
    await tail().evaluate(node => {
      const range = document.createRange(); range.selectNodeContents(node);
      const selection = window.getSelection()!; selection.removeAllRanges(); selection.addRange(range);
    });
    await tail().hover(); await page.mouse.wheel(200, 0);
    await expect(full(page)).toBeHidden();
    await page.getByRole("button", { name: "Split view", exact: true }).click();
    await page.getByRole("combobox", { name: "Right pane surface" }).selectOption("works");
    await expectLines(10, true);
    await page.setViewportSize({ width: 400, height: 1000 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await expectLines(10, true);
    const graphLabels = await page.locator(".works-surface .work-side small").evaluateAll(nodes => nodes.map(node => {
      const style = getComputedStyle(node);
      return { height: node.getBoundingClientRect().height, line: parseFloat(style.lineHeight) || parseFloat(style.fontSize) * 1.5 };
    }));
    for (const label of graphLabels) expect(label.height, "graph explanation remains readable in each narrow split pane").toBeLessThanOrEqual(label.line * 4);
    for (const surface of await page.locator(".works-surface").all()) {
      await surface.locator('.work-row[data-window-id="agent-one"]').scrollIntoViewIfNeeded();
    }
    await page.screenshot({ path: info.outputPath("works-output-lines-split-400.png") });
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.reload(); await openRail(page, "works"); await output(); await expectLines(10);
    await workRow(page, "agent-one").getByRole("button", { name: "Open terminal" }).click();
    await expect(full(page)).toBeVisible();
    const nativeInput = full(page).locator(".xterm-helper-textarea");
    await nativeInput.press("Escape"); await nativeInput.press("q");
    await expect.poll(async () => (await sent(page)).filter(message => message.kind === "terminal_input" && message.id === "agent-one").map(message => message.data))
      .toEqual(expect.arrayContaining(["\u001b", "q"]));
    await full(page).getByRole("button", { name: "Previous work" }).click();
    await expect(full(page).locator(".works-full-name .window-role-badge")).toHaveText("PM");
    await full(page).getByRole("button", { name: "Next work" }).click();
    await expect(full(page).locator(".works-full-name")).toHaveText("Aster");
    await full(page).getByRole("button", { name: "Back to Works" }).click();
    await expectLines(10);
    await page.screenshot({ path: info.outputPath("works-output-lines-10.png") });
  });

  test("Team/Past, revision readback, idle-safe links and 400px layouts retain the approved roster", async ({ page }, info) => {
    await openRail(page, "agents");
    await expect(roster(page).getByRole("tab", { name: "Team", exact: true })).toHaveAttribute("aria-selected", "true");
    await expect(roster(page).locator(".roster-count")).toHaveText("3");
    await expect(roster(page).locator(".roster-capacity-value")).toHaveText("3");
    await expect(roster(page).locator(".roster-capacity-note")).toContainText("Recommended capacity 6");
    await expect(roster(page).locator(".roster-active")).toHaveText("Active 3");
    await expect(roster(page).locator(".roster-detail .roster-stat")).toHaveCount(4);
    await expect(roster(page).locator(".roster-detail")).toContainText("Codex");
    await expect(roster(page).locator(".roster-detail")).toContainText("Member since 2026-10-20");
    await expect(roster(page).locator(".roster-detail-heading")).not.toContainText("Fixture model");
    await expect(roster(page).locator(".roster-saved-settings")).toContainText("Model: saved-manual-model");
    await expect(roster(page).locator(".roster-saved-settings")).toContainText("Automatic launches use tier settings");
    await expect(roster(page).locator(".roster-current-config")).toHaveText("Recorded name: Launch author · Tool: codex · Recorded model: launch-model");
    expect(await roster(page).locator(".roster-history li").allTextContents()).toEqual([
      expect.stringMatching(/Now.*#4777.*Current issue/), expect.stringContaining("Later slice"), expect.stringContaining("Earlier slice"),
    ]);
    await page.screenshot({ path: info.outputPath("agents-approved-team.png") });
    await roster(page).locator('.roster-row[data-member-id="m-idle"]').focus(); await page.keyboard.press("Enter");
    await expect(roster(page).locator(".roster-detail h3")).toHaveText("Cedar");
    await expect(roster(page).locator('.roster-row[data-member-id="m-idle"]')).toBeFocused();
    await expect(roster(page).locator('.roster-row[data-member-id="m-idle"] .roster-purpose')).not.toHaveAttribute("role", "button");
    expect(await roster(page).locator('.roster-row[data-member-id="m-idle"] .roster-purpose').evaluate(node => node.tagName)).toBe("SPAN");
    await expect(roster(page).locator(".roster-detail .roster-open, .roster-detail .roster-now, .roster-row[data-member-id='m-idle'] .roster-current")).toHaveCount(0);
    await expect(roster(page).locator(".roster-detail .roster-stat b")).toHaveText(["—", "—", "—", "—"]);
    await roster(page).getByRole("tab", { name: "Past", exact: true }).click();
    await expect(roster(page).locator(".roster-past .roster-stat")).toHaveCount(4);
    await expect(roster(page).locator(".roster-past tbody tr")).toHaveCount(1);
    await expect(roster(page).locator(".roster-past tbody tr")).toContainText("Earlier author");
    await expect(roster(page).locator(".roster-past tbody tr")).toContainText("Implementation");
    await expect(roster(page).locator(".roster-past tbody tr")).toContainText("#4000");
    await expect(roster(page).locator(".roster-past tbody tr")).toContainText("2026-10-20");
    await page.screenshot({ path: info.outputPath("agents-approved-past.png") });
    await roster(page).getByRole("tab", { name: "Team", exact: true }).click();
    const newMemberTool = roster(page).getByRole("combobox", { name: "Tool for new member", exact: true });
    await expect(newMemberTool.locator("option")).toHaveText(["Codex", "Claude Code"]);
    await newMemberTool.selectOption("codex");
    await roster(page).getByLabel("Name for new member", { exact: true }).fill("Dahlia");
    await roster(page).locator('.roster-capacity [data-action="add"]').click();
    await expect(roster(page).locator(".roster-count")).toHaveText("4");
    await expect(roster(page).locator(".roster-capacity-value")).toHaveText("4");
    expect(await page.evaluate(() => (window as any).__worksAgentsFixture.members.find(member => member.agent_member_id === "m-added").agent_id)).toBe("codex");
    // This is the fixture WebSocket roster, not a native backend Session.
    await page.evaluate(() => {
      const fixture = (window as any).__worksAgentsFixture;
      const member = fixture.members.find((value: any) => value.agent_member_id === "m-added");
      member.saved_manual_profile = { agent_id: "codex", agent_member_id: "m-added", model: "retained-model", reasoning: "high", runtime_target: "Host", skip_permissions: false };
      member.saved_implementation_core = { agent_id: "codex", model: "retained-model", reasoning: "high", version: null, fast_mode: false };
      member.saved_implementation_core_revision = "core-added-1";
      member.history = [
        { session_id: "s-added-old", issue_number: 4100, title: "Earlier retained", result: "landed", started_at: "2026-10-20T00:00:00Z", ended_at: "2026-10-20T02:00:00Z" },
        { session_id: "s-added-new", issue_number: 4200, title: "Later retained", result: "landed", started_at: "2026-10-22T00:00:00Z", ended_at: "2026-10-22T02:00:00Z" },
      ];
      fixture.reorderMembers(["m-one", "m-two", "m-idle", "m-added"]);
    });
    const beforeRest = await page.evaluate(() => structuredClone((window as any).__worksAgentsFixture.members.find(member => member.agent_member_id === "m-added")));
    expect(beforeRest.saved_implementation_core.model).toBe("retained-model");
    expect(beforeRest.history).toHaveLength(2);
    await roster(page).locator('.roster-row[data-member-id="m-added"]').click();
    await expect(roster(page).locator(".roster-saved-settings")).toContainText("Model: retained-model");
    expect(await roster(page).locator(".roster-history li").allTextContents()).toEqual([
      expect.stringContaining("Later retained"), expect.stringContaining("Earlier retained"),
    ]);
    await roster(page).locator('.roster-detail [data-action="rest"]').click();
    await expect(roster(page).locator(".roster-count")).toHaveText("3");
    await expect(roster(page).locator(".roster-capacity-value")).toHaveText("3");
    const mutations = (await sent(page)).filter(message => message.kind === "agent_roster" && message.action.kind !== "read");
    expect(mutations.map(message => [message.action, message.expected_revision])).toEqual([[{ kind: "add", agent_id: "codex", name: "Dahlia", pm_eligible: false }, 3], [{ kind: "rest", member_id: "m-added" }, 5]]);
    expect(await page.evaluate(() => (window as any).__worksAgentsFixture.rejectedRevisions)).toEqual([]);
    expect(await page.evaluate(() => (window as any).__worksAgentsFixture.members.find(member => member.agent_member_id === "m-added").enabled)).toBe(false);
    const resting = roster(page).locator('.roster-row[data-member-id="m-added"]');
    const restingRecord = { ...beforeRest, enabled: false };
    expect(await page.evaluate(() => (window as any).__worksAgentsFixture.members.find(member => member.agent_member_id === "m-added"))).toEqual(restingRecord);
    await expect(roster(page).locator(".roster-saved-settings")).toContainText("Model: retained-model");
    expect(await roster(page).locator(".roster-history li").allTextContents()).toEqual([
      expect.stringContaining("Later retained"), expect.stringContaining("Earlier retained"),
    ]);
    await expect(resting).toBeVisible();
    await expect(resting.locator(".roster-state")).toHaveText("Resting");
    await expect(roster(page).locator(".roster-capacity-note")).toContainText("Registered 4");
    await page.evaluate(() => (window as any).__worksAgentsFixture.census(null));
    await expect(roster(page).locator(".roster-active")).toHaveText("Active —");
    await expect(resting).toBeVisible();
    await page.evaluate(() => (window as any).__worksAgentsFixture.census(["agent-one", "agent-two", "pm-window"]));
    await expect(roster(page).locator(".roster-active")).toHaveText("Active 3");

    await roster(page).locator('.roster-row[data-member-id="m-one"]').click();
    await roster(page).locator(".roster-now button").evaluate(button => {
      (window as any).__worksAgentsFixture.retire("agent-one", true);
      (button as HTMLButtonElement).click();
    });
    await expect(main(page)).toHaveAttribute("data-surface", "agents");
    await expect(roster(page).locator(".roster-now, .roster-row[data-member-id='m-one'] .roster-current")).toHaveCount(0);
    await page.evaluate(() => (window as any).__worksAgentsFixture.error("Roster readback unavailable"));
    await expect(roster(page).locator(":scope > .roster-error")).toHaveText("Roster readback unavailable");
    await expect(roster(page).locator(".roster-count")).toHaveText("3");

    await page.setViewportSize({ width: 400, height: 1000 });
    await expectNoHorizontalOverflow(page, ".agents-surface:not([hidden]):not(.agents-surface--preview)");
    await page.screenshot({ path: info.outputPath("agents-approved-400.png") });
    await page.getByRole("button", { name: "Split view", exact: true }).click();
    await page.getByRole("combobox", { name: "Right pane surface" }).selectOption("agents");
    const previewName = page.locator('.agents-surface--preview [aria-label="Name for new member"]');
    await expect(previewName).toBeDisabled();
    await expect(previewName).toHaveAttribute("tabindex", "-1");
    await page.getByRole("button", { name: "Close split", exact: true }).click();
    await roster(page).getByRole("tab", { name: "Past", exact: true }).click();
    await expectNoHorizontalOverflow(page, ".agents-surface:not([hidden]):not(.agents-surface--preview)");
    await openRail(page, "works");
    await expect(works(page).locator(".work-row")).toHaveCount(2);
    await expectNoHorizontalOverflow(page, ".works-surface:not([hidden]):not(.works-surface--preview)");
    await page.screenshot({ path: info.outputPath("works-approved-400.png") });
    await expect(works(page).locator('[data-availability="today"]')).toBeVisible();
    await expect(works(page).locator('[data-availability="fleet"]')).toBeVisible();
    await expect(works(page).locator(".works-source-note")).toBeVisible();
    await page.reload();
    await openRail(page, "agents");
    await expect(resting).toBeVisible();
    await expect(resting.locator(".roster-state")).toHaveText("Resting");
    expect(await page.evaluate(() => (window as any).__worksAgentsFixture.members.find(member => member.agent_member_id === "m-added"))).toEqual(restingRecord);
    await resting.click();
    await expect(roster(page).locator(".roster-saved-settings")).toContainText("Model: retained-model");
    expect(await roster(page).locator(".roster-history li").allTextContents()).toEqual([
      expect.stringContaining("Later retained"), expect.stringContaining("Earlier retained"),
    ]);
    await expect(roster(page).locator(".roster-count")).toHaveText("3");
    await expect(roster(page).locator(".roster-capacity-value")).toHaveText("3");
    await expect(roster(page).locator(".roster-capacity-note")).toContainText("Registered 4");
    expect(await roster(page).locator(".roster-row").evaluateAll(rows => rows.map(row => (row as HTMLElement).dataset.memberId)))
      .toEqual(["m-one", "m-two", "m-idle", "m-added"]);
    await expectNoHorizontalOverflow(page, ".agents-surface:not([hidden]):not(.agents-surface--preview)");
    await resting.scrollIntoViewIfNeeded();
    await page.screenshot({ path: info.outputPath("agents-resting-reload-400.png") });
  });

  test("inline member names preserve drafts, IME, current work and history through conflict retry", async ({ page }, info) => {
    await openRail(page, "agents");
    const name = roster(page).getByRole("textbox", { name: "Display name", exact: true });
    const status = roster(page).locator(".roster-name-status");
    const writes = async () => (await sent(page)).filter(message => message.kind === "agent_roster" && message.action.kind === "rename");
    const identity = await page.evaluate(() => (window as any).__worksAgentsFixture.currentIdentity());
    await page.evaluate(() => (window as any).__worksAgentsFixture.deferRename());
    await name.fill("Maple current");
    await expect(status).toContainText("Saving");
    await name.fill("Oak current");
    expect(await writes()).toHaveLength(1);
    await name.focus();
    await page.evaluate(() => (window as any).__worksAgentsFixture.workspaceStatus("idle"));
    await expect(name).toBeFocused();
    await expect(name).toHaveValue("Oak current");
    // A rejected CAS can read back the requested name already saved by another
    // writer. Confirm that result and continue the newer draft without Retry.
    await page.evaluate(() => {
      (window as any).__worksAgentsFixture.renameElsewhere("Maple current");
      (window as any).__worksAgentsFixture.releaseRename();
    });
    await expect.poll(async () => (await writes()).length).toBe(2);
    await page.evaluate(() => (window as any).__worksAgentsFixture.releaseRename());
    await expect(status).toHaveText("Saved");
    await page.evaluate(() => (window as any).__worksAgentsFixture.replayRename());
    await expect(name).toHaveValue("Oak current");
    await expect(status).toHaveText("Saved");
    await expect(roster(page).getByRole("button", { name: "Retry name", exact: true })).toHaveCount(0);
    await expect(roster(page).locator(":scope > .roster-error")).toBeHidden();
    await expect(roster(page).locator(".roster-detail h3")).toHaveText("Oak current");
    await name.fill("  ");
    await expect(name).toHaveAttribute("aria-invalid", "true");
    await name.fill(" BIRCH ");
    await expect(name).toHaveAttribute("aria-invalid", "true");
    expect(await writes()).toHaveLength(2);
    await name.dispatchEvent("compositionstart");
    await name.fill("楓");
    expect(await writes()).toHaveLength(2);
    await name.dispatchEvent("compositionend");
    await expect.poll(async () => (await writes()).length).toBe(3);
    await page.evaluate(() => (window as any).__worksAgentsFixture.releaseRename());
    await expect(status).toHaveText("Saved");
    await name.fill("Draft after conflict");
    await expect.poll(async () => (await writes()).length).toBe(4);
    await page.evaluate(() => { (window as any).__worksAgentsFixture.renameElsewhere("Remote name"); (window as any).__worksAgentsFixture.releaseRename(); });
    await expect(status).toContainText("changed");
    await expect(name).toHaveValue("Draft after conflict");
    await expect(roster(page).locator(".roster-detail h3")).toHaveText("Remote name");
    await roster(page).getByRole("button", { name: "Retry name", exact: true }).click();
    await expect.poll(async () => (await writes()).length).toBe(5);
    await page.evaluate(() => (window as any).__worksAgentsFixture.releaseRename());
    await expect(status).toHaveText("Saved");
    const renamedIdentity = await page.evaluate(() => (window as any).__worksAgentsFixture.currentIdentity());
    // The fixture's explicit status refresh is unrelated to the rename.
    expect(renamedIdentity.agents).toEqual(identity.agents);
    expect(renamedIdentity.assignments).toEqual(identity.assignments);
    await expect(roster(page).locator(".roster-history-config")).toHaveText([
      "Recorded name: — · Tool: — · Recorded model: —",
      "Recorded name: Recorded author · Tool: claude · Recorded model: recorded-model",
    ]);
    await roster(page).getByRole("tab", { name: "Past", exact: true }).click();
    await expect(roster(page).locator(".roster-past tbody")).toContainText("Earlier author");
    await roster(page).getByRole("tab", { name: "Team", exact: true }).click();
    await openRail(page, "works");
    await expect(workRow(page, "agent-one").locator(".work-name")).toHaveText("Draft after conflict");
    await workRow(page, "agent-one").getByRole("button", { name: "Open terminal", exact: true }).click();
    await expect(full(page).locator(".works-full-name")).toContainText("Draft after conflict");
    await openRail(page, "agents");
    await page.setViewportSize({ width: 400, height: 1000 });
    await expectNoHorizontalOverflow(page, ".agents-surface:not([hidden]):not(.agents-surface--preview)");
    await name.scrollIntoViewIfNeeded();
    await page.screenshot({ path: info.outputPath("inline-name-400.png") });
    await page.getByRole("button", { name: "Split view", exact: true }).click();
    await page.getByRole("combobox", { name: "Right pane surface" }).selectOption("agents");
    await expect(page.locator('.agents-surface--preview [aria-label="Display name"]')).toBeDisabled();
    await expect(page.locator(".agents-surface--preview .roster-history-config")).toHaveText([
      "Recorded name: — · Tool: — · Recorded model: —",
      "Recorded name: Recorded author · Tool: claude · Recorded model: recorded-model",
    ]);
    await expect(page.locator(".agents-surface--preview .roster-saved-settings")).toContainText("Model: saved-manual-model");
    await expect(page.locator(".agents-surface--preview .roster-current-config")).toHaveText("Recorded name: Launch author · Tool: codex · Recorded model: launch-model");
  });

  test("ordered member plans keep current work through pending readback, stale rejection and reassignment", async ({ page }, info) => {
    await openRail(page, "agents");
    const plans = roster(page).locator(".roster-plans");
    const plan = (number: number) => plans.locator(`.roster-plan[data-issue-number="${number}"]`);
    const currentIdentity = await page.evaluate(() => (window as any).__worksAgentsFixture.currentIdentity());
    await expect(plans).toBeVisible();
    await expect(roster(page).locator(".roster-current-work")).toBeVisible();
    await expect(roster(page).locator(".roster-current-work + .roster-plans")).toBeVisible();
    await expect(roster(page).locator(".roster-row")).toHaveCount(3);
    await roster(page).locator('.roster-row[data-member-id="m-idle"]').click();
    await expect(plans).toContainText("No queued issue");
    await expect(plans.locator(".roster-plan")).toHaveCount(0);
    await roster(page).locator('.roster-row[data-member-id="m-one"]').click();
    expect(await plans.locator(".roster-plan").evaluateAll(nodes => nodes.map(node => (node as HTMLElement).dataset.issueNumber))).toEqual(["5002", "5003"]);
    await expect(plan(5002).locator(".roster-plan-reason")).toHaveText("Same SPEC continuation");
    await expect(plan(5003).locator(".roster-plan-reason")).toHaveText("Queue order");
    const target = plan(5002).getByRole("combobox", { name: "Assign issue #5002 to member", exact: true });
    expect(await target.evaluate(node => node.tagName)).toBe("SELECT");
    expect(await target.locator("option").evaluateAll(nodes => nodes.map(node => (node as HTMLOptionElement).value))).not.toContain("m-one");
    await page.evaluate(() => (window as any).__worksAgentsFixture.deferSchedule());
    await target.selectOption("m-idle");
    await target.focus();
    await target.evaluate(node => { (window as any).__t20aSelectedTarget = node; });
    const previousCatalogVersion = await roster(page).getAttribute("data-catalog-version");
    await page.evaluate(() => (window as any).__worksAgentsFixture.status("agent-two", "running"));
    await page.waitForFunction(previous => document.querySelector(".agents-surface:not(.agents-surface--preview)")?.getAttribute("data-catalog-version") !== previous, previousCatalogVersion);
    await expect(target).toHaveValue("m-idle");
    await expect(target).toBeFocused();
    expect(await target.evaluate(node => node === (window as any).__t20aSelectedTarget)).toBe(true);
    const updatedCatalogVersion = await roster(page).getAttribute("data-catalog-version");
    await page.evaluate(() => (window as any).__worksAgentsFixture.status("agent-two", "waiting"));
    await page.waitForFunction(previous => document.querySelector(".agents-surface:not(.agents-surface--preview)")?.getAttribute("data-catalog-version") !== previous, updatedCatalogVersion);
    await plan(5002).getByRole("button", { name: "Reassign issue #5002", exact: true }).click();
    await expect.poll(async () => (await sent(page)).filter(message => message.kind === "agent_schedule" && message.action.kind === "reassign").length).toBe(1);
    const first = (await sent(page)).filter(message => message.kind === "agent_schedule" && message.action.kind === "reassign")[0];
    expect(first.action).toEqual({ kind: "reassign", issue_number: 5002, source_member_id: "m-one", target_member_id: "m-idle", expected_roster_revision: 3, expected_queue_revision: "queue-v1:fixture-1", expected_schedule_revision: 7 });
    await expect(plan(5002)).toBeVisible();
    await expect(plan(5002).locator(".roster-plan-reason")).toHaveText("Same SPEC continuation");
    await expect(plans.getByRole("button", { name: "Refresh plans", exact: true })).toBeEnabled();
    await page.evaluate(() => { (window as any).__worksAgentsFixture.changeQueueRevision(); (window as any).__worksAgentsFixture.releaseSchedule(); });
    await expect(plans).toContainText("Queue changed; refresh plans before retrying");
    await expect(plan(5002)).toBeVisible();
    await expect(plan(5002).getByRole("button", { name: "Reassign issue #5002", exact: true })).toBeDisabled();
    await plans.getByRole("button", { name: "Refresh plans", exact: true }).click();
    await expect(plans).not.toContainText("Queue changed; refresh plans before retrying");

    await plan(5002).getByRole("combobox", { name: "Assign issue #5002 to member", exact: true }).selectOption("m-idle");
    await plan(5002).getByRole("button", { name: "Reassign issue #5002", exact: true }).click();
    await expect.poll(async () => (await sent(page)).filter(message => message.kind === "agent_schedule" && message.action.kind === "reassign").length).toBe(2);
    await expect(plan(5002)).toBeVisible();
    await page.evaluate(() => (window as any).__worksAgentsFixture.releaseSchedule());
    await expect(plan(5002)).toHaveCount(0);
    await expect(plan(5003)).toBeVisible();
    await roster(page).locator('.roster-row[data-member-id="m-idle"]').click();
    await expect(plan(5002).locator(".roster-plan-reason")).toHaveText("Assigned by you");
    await expect(roster(page).locator(".roster-now, .roster-detail .roster-open")).toHaveCount(0);
    await page.screenshot({ path: info.outputPath("agents-plans-reassigned.png") });
    await roster(page).locator('.roster-row[data-member-id="m-two"]').click();
    await expect(plan(5004).locator(".roster-plan-reason")).toHaveText("Queue order");
    await expect(roster(page).locator(".roster-now")).toContainText("Second issue");
    expect(await page.evaluate(() => (window as any).__worksAgentsFixture.currentIdentity())).toEqual(currentIdentity);
    const mutations = (await sent(page)).filter(message => message.kind === "agent_schedule" && message.action.kind === "reassign");
    expect(mutations[1].action).toEqual({ ...first.action, expected_queue_revision: "queue-v1:fixture-2" });
    expect(await page.evaluate(() => (window as any).__worksAgentsFixture.rejectedScheduleRevisions)).toHaveLength(1);
    await page.setViewportSize({ width: 400, height: 1000 });
    await expectNoHorizontalOverflow(page, ".agents-surface:not([hidden]):not(.agents-surface--preview)");
    await plans.scrollIntoViewIfNeeded();
    await expect(plan(5004).getByRole("combobox", { name: "Assign issue #5004 to member", exact: true })).toBeInViewport();
    await expect(plan(5004).getByRole("button", { name: "Reassign issue #5004", exact: true })).toBeInViewport();
    await page.screenshot({ path: info.outputPath("agents-plans-400.png") });
  });
  test("shared member and PM tuning retain identity, history and narrow layout", async ({ page }, info) => {
    await openRail(page, "agents");
    await roster(page).locator('.roster-row[data-member-id="m-idle"]').click();
    const detail = roster(page).locator('.roster-detail');
    await detail.locator('[data-member-tuning-field="model"]').fill("unlisted-implementation");
    await expect(detail.locator('[data-member-tuning-status]')).toHaveText("Saved");
    let writes = (await sent(page)).filter(message => message.kind === "agent_roster" && message.action.kind === "set_tuning");
    expect(writes).toHaveLength(1); expect(writes[0].action.member_id).toBe("m-idle");
    expect(writes[0].action).not.toHaveProperty("agent_id"); expect(writes[0]).not.toHaveProperty("expected_revision");
    await page.evaluate(() => (window as any).__worksAgentsFixture.deferEligibility());
    await detail.locator('[data-action="pm-eligible"]').check();
    await expect(detail.locator('[data-action="select-pm"]')).toBeDisabled();
    await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    await expect(detail.locator('[data-action="pm-eligible"]')).toBeChecked();
    await page.evaluate(() => (window as any).__worksAgentsFixture.releaseEligibility());
    await expect(detail.locator('[data-action="select-pm"]')).toBeEnabled();
    await detail.locator('[data-action="select-pm"]').click();
    await expect(detail.locator('[data-action="select-pm"]')).toHaveText("Configured Project Manager");
    await expect(roster(page).locator('.roster-row[data-member-id="m-idle"]')).toContainText("Project Manager");
    await expect(detail.locator('[data-action="rest"]')).toBeDisabled();
    await detail.locator('[aria-label="Display name"]').fill("Cedar PM");
    await expect(detail.locator('.roster-name-status')).toHaveText("Saved");
    await detail.locator('[data-member-tuning-field="model"]').fill("opusplan");
    await expect(detail.locator('[data-member-tuning-status]')).toHaveText("Saved");
    const pmWrites = (await sent(page)).filter(message => message.kind === "set_pm_launch_profile");
    expect(pmWrites).toHaveLength(1); expect(pmWrites[0]).toMatchObject({ project_key: APP_PROJECT_KEY, member_id: "m-idle", model: "opusplan" });
    expect(pmWrites[0].expected_profile_revision).toBeTruthy(); expect(pmWrites[0].request_id).toBeTruthy();
    await page.evaluate(() => (window as any).__worksAgentsFixture.pmUnknown());
    await expect(detail.locator('[data-member-tuning-field="model"]')).toBeDisabled();
    await expect(detail.locator('[data-member-tuning-field="model"]')).toHaveValue("opusplan");
    await detail.locator('[data-action="reload-tuning"]').click();
    await expect(detail.locator('[data-member-tuning-field="model"]')).toBeEnabled();
    await expect(detail.locator('.roster-history').filter({ hasText: "Assignment history" })).toBeVisible();
    await page.setViewportSize({ width: 400, height: 1000 });
    await expectNoHorizontalOverflow(page, ".agents-surface:not([hidden]):not(.agents-surface--preview)");
    await detail.locator('[data-member-tuning-field="reasoning"]').scrollIntoViewIfNeeded();
    await expect(detail.locator('[data-member-tuning-field="reasoning"]')).toBeInViewport();
    await page.screenshot({ path: info.outputPath("agents-shared-pm-tuning-400.png") });
  });

});

async function installWorksAgentsBackend(page: Page) {
  await page.addInitScript(({ projectKey }) => {
    (window as any).__gwtPlaywrightTestBridge = true;
    const baseWindow = (id: string, overrides: Record<string, unknown>) => ({
      id, title: id, preset: "agent", geometry: { x: 2400, y: 1400, width: 560, height: 340 }, geometry_revision: 0,
      z_index: 1, status: "running", persist: true, minimized: false, maximized: false,
      purpose_title: null, dynamic_title: null, dynamic_title_detail: null, agent_id: null, session_id: null,
      tab_group_id: "legacy-agent-group", tab_group_active: false, placement: { kind: "canvas" }, ...overrides,
    });
    const windows: any[] = [
      baseWindow("board-window", { title: "Board", preset: "board", z_index: 100 }),
      baseWindow("agent-one", { preset: "codex", agent_id: "codex", session_id: "s-one", agent_name: "Aster", dynamic_title: "Current issue", linked_issue_number: 4777 }),
      baseWindow("agent-two", { preset: "claude", agent_id: "claude", session_id: "s-two", agent_name: "Birch", dynamic_title: "Second issue", linked_issue_number: 4001, status: "waiting" }),
      baseWindow("pm-window", { preset: "claude", agent_id: "claude", session_id: "s-pm", is_pm: true, agent_name: "Yuu", title: "Project Manager", dynamic_title: "Project supervision" }),
      baseWindow("recorded-window", { preset: "codex", session_id: "s-recorded", agent_name: "Recorded only", status: "running" }),
    ];
    const assignment = (session_id: string, issue_number: number) => ({ reservation_id: `r-${session_id}`, session_id, issue_number, spec_number: null, delivery_id: null, host_id: "fixture-host", host_pid: 12345, host_started_at: 1 });
    const savedRoster = JSON.parse(sessionStorage.getItem("fixture:works-roster") || "null");
    const members: any[] = savedRoster?.members ?? [
      { agent_member_id: "m-one", agent_id: "codex", name: "Aster", enabled: true, created_at: "2026-10-20T00:00:00Z", last_issue_number: 4777, last_spec_number: null, assignment: assignment("s-one", 4777), provider: "Codex", model: "Fixture model", saved_manual_profile: { agent_id: "codex", agent_member_id: "m-one", model: "saved-manual-model", reasoning: "high", runtime_target: "Host", skip_permissions: false }, issue_count: 3, landed_count: 2, active_seconds: 7200, tokens: 5000, history: [
        { session_id: "s-one", name: "Launch author", provider: "codex", model: "launch-model", issue_number: 4777, title: "Current issue", result: "Running", started_at: "2026-10-23T00:00:00Z", ended_at: null },
        { session_id: "s-earlier", name: "Recorded author", provider: "claude", model: "recorded-model", issue_number: 4000, title: "Earlier slice", result: "landed", started_at: "2026-10-20T00:00:00Z", ended_at: "2026-10-20T02:00:00Z" },
        { session_id: "s-later", issue_number: 4600, title: "Later slice", result: null, started_at: "2026-10-22T00:00:00Z", ended_at: "2026-10-22T02:00:00Z" },
      ] },
      { agent_member_id: "m-two", agent_id: "claude", name: "Birch", enabled: true, created_at: "2026-10-20T00:00:00Z", last_issue_number: 4001, last_spec_number: null, assignment: assignment("s-two", 4001), provider: "Claude", model: "Fixture model", issue_count: null, landed_count: null, active_seconds: null, tokens: null, history: [] },
      { agent_member_id: "m-idle", agent_id: "codex", name: "Cedar", enabled: true, created_at: "2026-10-20T00:00:00Z", last_issue_number: null, last_spec_number: null, assignment: null, provider: null, model: null, issue_count: null, landed_count: null, active_seconds: null, tokens: null, history: [] },
    ];
    for (const member of members) {
      member.saved_implementation_core ||= { agent_id: member.agent_id, model: member.saved_manual_profile?.model || null, reasoning: null, version: null, fast_mode: false };
      member.saved_implementation_core_revision ||= `core-${member.agent_member_id}-1`;
    }
    let pmMemberId: string | null = null, pmProfileRevision = 1;
    let pmProfile = { agent_id: "claude", model: null as string | null, reasoning: null as string | null };
    const pmStatus = () => ({ kind: "pm_status", project_key: projectKey, configured_member_id: pmMemberId,
      configured_member_name: members.find(member => member.agent_member_id === pmMemberId)?.name,
      profile_revision: `pm-${pmProfileRevision}`, configured_agent_id: pmProfile.agent_id, configured_model: pmProfile.model,
      configured_reasoning: pmProfile.reasoning, running_agent_id: "claude", running_model: "running-model", running_reasoning: "high", is_running: true, available: true, paused: false, auto_start: true, loop_interval_secs: 60,
      agent_options: [{ id: "claude", name: "Claude Code" }, { id: "codex", name: "Codex" }] });
    // Fixture-only backing storage models backend persistence across page reload.
    let worksPreviewLines = Number(sessionStorage.getItem("fixture:works-preview-lines") || 3);
    let revision = savedRoster?.revision ?? 3, activityRevision = 0;
    let agents: any[] = [
      { window_id: "agent-two", session_id: "s-two", runtime_incarnation: 2, issue_number: 4001, is_pm: false },
      { window_id: "agent-one", session_id: "s-one", runtime_incarnation: 1, issue_number: 4777, is_pm: false },
      { window_id: "pm-window", session_id: "s-pm", runtime_incarnation: 3, issue_number: null, is_pm: true },
    ];
    const sentMessages: any[] = [], rejectedRevisions: any[] = [];
    const rejectedScheduleRevisions: any[] = [];
    let queueRevision = "queue-v1:fixture-1", scheduleRevision = 7, deferredSchedule = false;
    let pendingSchedule: any = null, pendingRename: any = null, deferredRename = false, firstRenameReceipt: any = null;
    let deferredEligibility = false, pendingEligibilityReceipt: string | null = null;
    const planned = [
      { agent_member_id: "m-one", issues: [{ issue_number: 5002, reason: "same_spec_continuation" }, { issue_number: 5003, reason: "queue_order" }] },
      { agent_member_id: "m-two", issues: [{ issue_number: 5004, reason: "queue_order" }] },
      { agent_member_id: "m-idle", issues: [] },
    ];
    let socket: FixtureWebSocket, online = true, echoInput = false, resyncText: string | null = null;
    const sockets: FixtureWebSocket[] = [];
    const workspace = () => ({ kind: "workspace_state", workspace: { app_version: "playwright", active_tab_id: "tab-works", recent_projects: [], tabs: [{ id: "tab-works", title: "Works Agents Fixture", project_root: "/fixture", project_key: projectKey, kind: "git", workspace: { windows, viewport: { x: 0, y: 0, zoom: 1 } } }] } });
    const rosterSnapshot = () => ({ kind: "agent_roster", roster: { project_key: projectKey, revision, revision_decimal: String(revision), members, capacity: members.filter(member => member.enabled).length, recommended_capacity: 6, past_complete: true, past_contributors: [{ session_id: "s-earlier", name: "Earlier author", kind: "implementation", issue_numbers: [4000], first_posted_at: "2026-10-20T00:00:00Z", last_posted_at: "2026-10-20T02:00:00Z", post_count: 4 }], by_session: Object.fromEntries(agents.map(agent => [agent.session_id, { branch: `work/${agent.window_id}`, ahead_of_develop: 0, model: "Fixture model", provider: agent.is_pm || agent.session_id === "s-two" ? "Claude" : "Codex", last_action: "Fixture observation", elapsed_seconds: 120, warning_count: 0 }])), error: null } });
    const applyRename = (message: any) => {
      const member = members.find(member => member.agent_member_id === message.action.member_id);
      const name = message.action.name.trim();
      let error: string | null = null;
      if (message.expected_revision !== revision) error = "Agent roster changed; reload before retrying";
      else if (!member || !name || members.some(other => other !== member && other.name.trim().toLowerCase() === name.toLowerCase())) error = "Invalid display name";
      else if (member.name !== name) { member.name = name; revision++; }
      const receipt = { ...rosterSnapshot(), request_id: message.request_id };
      receipt.roster.error = error as any;
      if (!firstRenameReceipt) firstRenameReceipt = JSON.parse(JSON.stringify(receipt));
      socket.emit(receipt);
    };
    const scheduleSnapshot = () => ({ kind: "agent_schedule", schedule: { project_key: projectKey, roster_revision: revision, queue_revision: queueRevision, schedule_revision: scheduleRevision, members: members.filter(member => member.enabled).map(member => ({ agent_member_id: member.agent_member_id, issues: planned.find(plan => plan.agent_member_id === member.agent_member_id)?.issues || [], empty_reason: planned.find(plan => plan.agent_member_id === member.agent_member_id)?.issues.length ? null : "No queued issue" })) } });
    const applySchedule = (action: any) => {
      if (action.expected_roster_revision !== revision || action.expected_queue_revision !== queueRevision || action.expected_schedule_revision !== scheduleRevision) {
        rejectedScheduleRevisions.push(action);
        socket.emit({ kind: "agent_schedule_error", project_key: projectKey, message: "Queue changed; refresh plans before retrying" });
        return;
      }
      const source = planned.find(plan => plan.agent_member_id === action.source_member_id);
      const target = planned.find(plan => plan.agent_member_id === action.target_member_id);
      const issue = source?.issues.find(issue => issue.issue_number === action.issue_number);
      if (!source || !target || !issue) throw new Error("fixture received an invalid schedule move");
      source.issues = source.issues.filter(issue => issue.issue_number !== action.issue_number);
      target.issues.push({ ...issue, reason: "assigned_by_you" }); scheduleRevision++;
      socket.emit(scheduleSnapshot());
    };
    const activity = () => ({ kind: "active_work_projection", projection: { id: "works-agents-fixture", active_works: [], local_agent_activity: { project_key: projectKey, host_id: "fixture-host", observed_at: new Date(1792886400000 + activityRevision++).toISOString(), agents } } });
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
      readyState = FixtureWebSocket.CONNECTING;
      inbound: string[] = [];
      constructor(public readonly url: string) {
        super(); socket = this; sockets.push(this);
        setTimeout(() => { if (online) this.open(); }, 0);
      }
      open() { this.readyState = FixtureWebSocket.OPEN; this.dispatchEvent(new Event("open")); this.emit(workspace()); this.emit(rosterSnapshot()); this.emit(activity()); }
      send(raw: string) {
        const message = JSON.parse(raw); sentMessages.push(message);
        if (message.kind === "frontend_ready" && resyncText !== null) {
          this.emit(workspace()); this.emit(activity());
          this.emit({ kind: "terminal_snapshot", id: "agent-one", data_base64: btoa(resyncText) });
        }
        if (echoInput && message.kind === "terminal_input" && !message.data.startsWith("\u001b")) this.emit({ kind: "terminal_output", id: message.id, data_base64: btoa(message.data === "\r" ? "\r\n" : message.data) });
        if (message.kind === "create_window" && message.preset === "settings" && !windows.some(data => data.preset === "settings")) {
          windows.push(baseWindow("settings-window", { title: "Settings", preset: "settings", z_index: 101, surface_owner: message.surface_owner }));
          this.emit(workspace());
        }
        if (message.kind === "get_system_settings") this.emit({ kind: "system_settings", language: "auto", works_preview_lines: worksPreviewLines });
        if (message.kind === "update_system_settings") {
          if (Number.isInteger(message.works_preview_lines) && message.works_preview_lines >= 3 && message.works_preview_lines <= 10) {
            worksPreviewLines = message.works_preview_lines;
            sessionStorage.setItem("fixture:works-preview-lines", String(worksPreviewLines));
            this.emit({ kind: "system_settings_updated", language: message.language, works_preview_lines: worksPreviewLines });
          } else this.emit({ kind: "system_settings_error", message: "Invalid fixture Works output lines" });
        }
        if (message.kind === "list_supported_agents") this.emit({ kind: "supported_agent_list", agents: [
          { id: "codex", name: "Codex", installed: true }, { id: "claude", name: "Claude Code", installed: true },
        ] });
        if (message.kind === "list_custom_agents") this.emit({ kind: "custom_agent_list", agents: [] });
        if (message.kind === "set_pm_launch_profile") {
          const success = message.project_key === projectKey && message.member_id === pmMemberId && message.expected_profile_revision === `pm-${pmProfileRevision}`;
          if (success) { pmProfile = { agent_id: message.agent_id, model: message.model, reasoning: message.reasoning }; pmProfileRevision++; }
          this.emit({ kind: "pm_launch_profile_result", project_key: projectKey, member_id: message.member_id, request_id: message.request_id, success, retryable: true,
            profile_revision: `pm-${pmProfileRevision}`, profile: pmProfile, message: success ? null : "Profile changed" }); this.emit(pmStatus());
        }
        if (message.kind === "agent_roster") {
          if (message.action.kind === "set_tuning") {
            const member = members.find(member => member.agent_member_id === message.action.member_id);
            if (!member || member.saved_implementation_core_revision !== message.action.expected_core_revision) {
              this.emit({ kind: "agent_roster_error", project_key: projectKey, request_id: message.request_id, message: "Core changed" }); return;
            }
            member.saved_implementation_core = { ...member.saved_implementation_core, model: message.action.model, reasoning: message.action.reasoning };
            revision++; member.saved_implementation_core_revision = `core-${member.agent_member_id}-${revision}`;
            this.emit({ ...rosterSnapshot(), request_id: message.request_id }); return;
          }
          if (message.action.kind === "rename") {
            if (deferredRename) pendingRename = message;
            else applyRename(message);
            return;
          }
          if (message.action.kind !== "read" && message.expected_revision !== revision) { rejectedRevisions.push(message); this.emit(rosterSnapshot()); return; }
          if (message.action.kind === "set_pm_eligibility") { const member = members.find(member => member.agent_member_id === message.action.member_id); if (member) { member.pm_eligible = message.action.eligible; revision++; } }
          if (message.action.kind === "select_pm") { const member = members.find(member => member.agent_member_id === message.action.member_id); if (member?.pm_eligible && member.enabled && !member.assignment) { pmMemberId = member.agent_member_id; member.is_pm = true; member.pm_project_key = projectKey; revision++; this.emit(pmStatus()); } }
          if (message.action.kind === "add") { members.push({ agent_member_id: "m-added", agent_id: message.action.agent_id, name: message.action.name, pm_eligible: message.action.pm_eligible === true, enabled: true, created_at: "2026-10-25T00:00:00Z", last_issue_number: null, last_spec_number: null, assignment: null, provider: null, model: null, history: [], issue_count: 0, landed_count: 0, active_seconds: 0, tokens: 0 }); revision++; }
          if (message.action.kind === "rest") { const member = members.find(member => member.agent_member_id === message.action.member_id); if (member) { member.enabled = false; revision++; } }
          if (message.action.kind !== "read") sessionStorage.setItem("fixture:works-roster", JSON.stringify({ members, revision }));
          if (message.action.kind === "set_pm_eligibility" && deferredEligibility) {
            pendingEligibilityReceipt = JSON.stringify({ ...rosterSnapshot(), request_id: message.request_id }); return;
          }
          this.emit({ ...rosterSnapshot(), request_id: message.request_id });
          if (message.action.kind === "read" && pmMemberId) this.emit(pmStatus());
        }
        if (message.kind === "agent_schedule") {
          if (message.action.kind === "read") this.emit(scheduleSnapshot());
          else if (deferredSchedule) pendingSchedule = message.action;
          else applySchedule(message.action);
        }
      }
      emit(payload: unknown) {
        // Keep one ordered WebSocket stream across Playwright clock ticks.
        this.inbound.push(JSON.stringify(payload));
        if (this.inbound.length !== 1) return;
        const deliver = () => {
          this.dispatchEvent(new MessageEvent("message", { data: this.inbound[0] }));
          // Reentrant emit must see this packet until dispatch finishes.
          this.inbound.shift();
          if (this.inbound.length) setTimeout(deliver, 0);
        };
        setTimeout(deliver, 0);
      }
      close() { this.readyState = FixtureWebSocket.CLOSED; this.dispatchEvent(new CloseEvent("close")); }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
    (window as any).__worksAgentsFixture = {
      sent: sentMessages, members, rejectedRevisions, rejectedScheduleRevisions,
      echoInputForWorks() { echoInput = true; },
      rebindForWorks() {
        windows.find(data => data.id === "agent-one").session_id = "rebound-session";
        agents = [{ window_id: "agent-one", session_id: "rebound-session", runtime_incarnation: 9, issue_number: 4777, is_pm: false }];
        resyncText = "fresh rebound snapshot\r\n";
        socket.emit(workspace()); socket.emit(activity());
      },
      loseWorksProof(phase: string) {
        if (phase === "clear") { online = false; sockets.filter(item => item.readyState === FixtureWebSocket.OPEN).forEach(item => item.close()); return; }
        const event: any = phase === "unknown"
          ? { kind: "local_agent_activity", activity: { ...activity().projection.local_agent_activity, agents: null } }
          : workspace();
        if (phase === "catalog") event.workspace.tabs[0].workspace.windows = windows.filter(data => data.id !== "agent-one");
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: event }));
      },
      restoreWorksProof(phase: string) {
        if (phase === "clear") {
          online = true;
          const connecting = sockets.filter(item => item.readyState === FixtureWebSocket.CONNECTING);
          if (!connecting.length) throw new Error("transport has not created its reconnect socket");
          connecting.forEach(item => item.open());
        } else socket.emit(phase === "catalog" ? workspace() : activity());
      },
      pmUnknown() { socket.emit({ ...pmStatus(), profile_revision: null, configured_agent_id: null }); },
      deferEligibility() { deferredEligibility = true; },
      releaseEligibility() {
        if (!pendingEligibilityReceipt) throw new Error("No pending eligibility receipt");
        const receipt = pendingEligibilityReceipt; pendingEligibilityReceipt = null; deferredEligibility = false;
        socket.emit(JSON.parse(receipt));
      },
      deferRename() { deferredRename = true; },
      releaseRename() { const message = pendingRename; pendingRename = null; if (message) applyRename(message); },
      replayRename() { if (firstRenameReceipt) socket.emit(firstRenameReceipt); },
      renameElsewhere(name: string) { members[0].name = name; revision++; socket.emit(rosterSnapshot()); },
      activityForWorks(session_id: string, runtime_incarnation: number) {
        agents = [{ window_id: "agent-one", session_id, runtime_incarnation, issue_number: 4777,
          is_pm: false, current_task: `Live ${session_id}` }];
        socket.emit(activity());
      },
      catalogForWorks(session_id: string, placement: string) {
        const event = workspace();
        const tab = event.workspace.tabs[0];
        tab.workspace.windows = windows.map(data => data.id === "agent-one" ? { ...data, session_id } : data);
        if (placement === "background") {
          event.workspace.tabs.unshift({ ...tab, id: "empty-tab", workspace: { ...tab.workspace, windows: [] } });
          event.workspace.active_tab_id = "empty-tab";
        } else if (placement === "empty") {
          event.workspace.tabs = [];
          event.workspace.active_tab_id = "";
        }
        socket.emit(event);
      },
      workspaceTitle(title: string) { windows.find(data => data.id === "agent-one").dynamic_title = title; socket.emit(workspace()); },
      workspaceStatus(status: string) { windows.find(data => data.id === "agent-one").status = status; socket.emit(workspace()); },
      attachmentProgress() {
        const requests = sentMessages.filter(message => message.kind === "attach_files" && message.id === "agent-one");
        if (requests.length !== 1) throw new Error("Expected one telemetry attachment request");
        const [request] = requests;
        if (!request.operation_id || !request.expected_target) throw new Error("Telemetry attachment request has no captured identity");
        socket.emit({ kind: "attachment_progress", id: request.id,
          operation_id: request.operation_id, expected_target: request.expected_target,
          phase: "staging", file_index: 0, file_count: 1, filename: "fixture.txt", bytes_done: 8, bytes_total: 16, message: null });
      },
      remountInKanban() {
        windows.push(baseWindow("telemetry-kanban", { preset: "agent_kanban", title: "Telemetry Kanban" }));
        windows.find(data => data.id === "agent-one").placement = {
          kind: "agent_kanban", board_id: "telemetry-kanban", lane_id: "active", order: 0, collapsed: false,
        };
        socket.emit(workspace());
      },
      reorderMembers(ids: string[]) { members.splice(0, members.length, ...ids.map(id => members.find(member => member.agent_member_id === id))); revision++; socket.emit(rosterSnapshot()); },
      currentIdentity() { return { windows: windows.filter(data => data.session_id).map(({ id, session_id, status }) => ({ id, session_id, status })), agents, assignments: members.map(member => ({ member_id: member.agent_member_id, assignment: member.assignment })) }; },
      deferSchedule() { deferredSchedule = true; },
      changeQueueRevision() { queueRevision = "queue-v1:fixture-2"; },
      releaseSchedule() { if (!pendingSchedule) throw new Error("no schedule mutation is pending"); const action = pendingSchedule; pendingSchedule = null; applySchedule(action); },
      output(id: string, text: string) { socket.emit({ kind: "terminal_output", id, data_base64: btoa(text) }); },
      census(ids: string[] | null) {
        if (ids !== null) agents = agents.filter(agent => ids.includes(agent.window_id));
        const snapshot = activity().projection.local_agent_activity;
        socket.emit({ kind: "local_agent_activity", activity: { ...snapshot, agents: ids === null ? null : agents } });
      },
      monitorActive(active_count: number) {
        socket.emit({ kind: "issue_monitor_status", status: {
          enabled: true, state: "active", queue_len: 0, active_count, max_active_agents: 3,
        } });
      },
      status(id: string, status: string) { const data = windows.find(data => data.id === id); if (data) data.status = status; socket.emit({ kind: "terminal_status", id, status }); },
      retire(id: string, immediate: boolean) { agents = agents.filter(agent => agent.window_id !== id); const event = activity(); if (immediate) window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: event })); else socket.emit(event); },
      error(message: string) { socket.emit({ kind: "agent_roster_error", project_key: projectKey, message }); },
      usage(quality = "measured", inputTokens = 0) {
        const measured = quality === "measured", value = measured ? 0 : null;
        socket.emit({ kind: "provider_usage", accounts: [], consumption: [], sessions: agents.map(agent => ({
          session_id: agent.session_id, provider_session_id: `provider-${agent.session_id}`,
          provider: agent.is_pm || agent.session_id === "s-two" ? "claude_code" : "codex", project_path: "/fixture",
          model: "Fixture model", active: true, input_tokens: 0, cache_read_tokens: 0, cache_creation_tokens: 0,
          output_tokens: 0, total_tokens: 0, reported_context_left_pct: 0, context_used_tokens: null,
          context_limit_tokens: null, context_left_pct: null, limit_reached: false, eligible: true,
          state: quality === "unavailable" ? { kind: "unavailable", reason: "Fixture transcript cannot be read" } : { kind: "ok" },
          telemetry: quality === "unavailable" ? null : { source: "fixture-reported", quality,
            observed_at: "2026-10-25T00:00:00Z", today_tokens: measured ? inputTokens : null, today_input_tokens: measured ? inputTokens : null,
            today_cache_read_tokens: value, today_cache_creation_tokens: value, today_output_tokens: value,
            minute_buckets: Array.from({ length: 30 }, (_, index) => ({
              started_at: new Date(1792884600000 + index * 60000).toISOString(), tokens: value,
            })),
          },
        })) });
      },
    };
  }, { projectKey: APP_PROJECT_KEY });
}
