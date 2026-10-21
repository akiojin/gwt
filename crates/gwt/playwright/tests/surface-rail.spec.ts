/* Issue #4777 T-1 — the rail picks one of four surfaces.
 *
 * The PM sits alone above a rule; Issues / Agents / Board / Settings follow
 * with an icon and a visible label. The pressed entry is the surface of the focused window, a
 * click opens (or focuses) that surface, and at 860px and below the rail is a
 * strip across the top of the surfaces. Runs against the embedded frontend with
 * a stub backend, in both themes, with zero console / page errors.
 */
import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// browser-check supplies an isolated checkout server; fixture mode remains
// useful for fast development while both modes exercise the same UI code.
const surfaceAppUrl = process.env.GWT_PLAYWRIGHT_BASE_URL
  ? new URL(new URL(APP_URL).pathname, process.env.GWT_PLAYWRIGHT_BASE_URL).href
  : APP_URL;
async function installSurfaceAssets(page: Page) {
  if (!process.env.GWT_PLAYWRIGHT_BASE_URL) await installEmbeddedRoutes(page);
}

type SentMessage = {
  kind?: string;
  id?: string;
  preset?: string;
  surface_owner?: string;
  origin_session_id?: string;
  bounds?: { width: number; height: number };
};

test.describe("Surface rail", () => {
  test.setTimeout(60_000);
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test.afterEach(async ({ page }) => {
    await expect(page.locator(".render-degradation-banner")).toHaveCount(0);
  });

  for (const [surface, focusedId] of [["settings", "issue-window"], ["board", "board-window"], ["issues", "legacy-issues-window"]] as const) {
    test(`restoring ${surface} ignores legacy canvas geometry and grouping`, async ({ page }, testInfo) => {
      const errors = captureBrowserErrors(page);
      await installSurfaceAssets(page);
      await installSurfaceRailBackend(page, true, false, focusedId);
      await page.goto(surfaceAppUrl);
      const main = page.getByRole("region", { name: "Main pane", exact: true });
      // Settings retains its existing default Issues tab; the rail follows
      // that visible content rather than introducing new tab persistence.
      const visibleSurface = surface === "settings" ? "issues" : surface;
      await expect(main).toHaveAttribute("data-surface", visibleSurface);
      await expect(main.locator(`.workspace-window[data-id='${focusedId}']`)).toBeVisible();
      if (surface === "settings") {
        await expect(main.getByRole("tab", { name: "Issues", exact: true })).toHaveAttribute("aria-selected", "true");
      }
      await expectSurfaceOnly(page);
      await page.reload();
      await expect(main).toHaveAttribute("data-surface", visibleSurface);
      await expectSurfaceOnly(page);
      // A project change closes the previous layout before restoring the
      // next catalog. Returning must restore the saved surface as well.
      await page.evaluate(() => (window as any).__switchSurfaceProject("tab-2"));
      await expect(main).toHaveAttribute("data-surface", "board");
      await expect(main.locator('[data-id="second-board"]')).toBeVisible();
      await expectSurfaceOnly(page);
      await page.evaluate(() => (window as any).__switchSurfaceProject("tab-1"));
      await expect(main).toHaveAttribute("data-surface", visibleSurface);
      await expect(main.locator(`.workspace-window[data-id='${focusedId}']`)).toBeVisible();
      await expect(page.locator("#split-surfaces")).toHaveAttribute("data-split", "false");
      await expectSurfaceOnly(page);
      await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.includes("light") ? "light" : "dark");
      await page.screenshot({ path: testInfo.outputPath(`restored-${surface}.png`) });
      expect(errors).toEqual([]);
    });
  }

  test("Settings events and notification framing retain the fixed surface layout", async ({ page }, testInfo) => {
    const errors = captureBrowserErrors(page);
    await installSurfaceAssets(page);
    await installSurfaceRailBackend(page, true);
    await page.goto(surfaceAppUrl);
    const main = page.getByRole("region", { name: "Main pane", exact: true });
    await expect(main).toHaveAttribute("data-surface", "board");
    await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "system" } })));
    await expect(main).toHaveAttribute("data-surface", "settings");
    await expect(main.getByRole("tab", { name: "System", exact: true })).toHaveAttribute("aria-selected", "true");
    await expectSurfaceOnly(page);
    // Runtime attention uses the real frameWindow entry point.
    await page.evaluate(() => (window as any).__surfaceRailSocket().emit({ kind: "terminal_status", id: "agent-one", status: "waiting" }));
    const notice = page.locator(".toast-alerts__item").filter({ hasText: "agent-one" });
    await expect(notice).toBeVisible();
    await notice.click();
    await expect(main).toHaveAttribute("data-surface", "agents");
    await expect(main.locator('[data-agent-id="agent-one"] .xterm')).toBeVisible();
    await expectSurfaceOnly(page);
    await page.getByRole("button", { name: "Split view", exact: true }).click();
    await page.getByRole("combobox", { name: "Right pane surface" }).selectOption("board");
    await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "issues" } })));
    await expect(page.getByRole("region", { name: "Right pane", exact: true })).toHaveAttribute("data-surface", "issues");
    await expect(page.locator("#split-surfaces")).toHaveAttribute("data-split", "true");
    await expectSurfaceOnly(page);
    await page.screenshot({ path: testInfo.outputPath("fixed-layout-entry-points.png") });
    expect(errors).toEqual([]);
  });

  test("Board origin Agent links reveal live and resumed agents in the rail layout", async ({ page }) => {
    const errors = captureBrowserErrors(page);
    await installSurfaceAssets(page);
    await installSurfaceRailBackend(page);
    await page.goto(surfaceAppUrl);
    const main = page.getByRole("region", { name: "Main pane", exact: true });
    await expect(main).toHaveAttribute("data-surface", "board");
    await page.evaluate(() => (window as any).__surfaceRailSocket().emit({
      kind: "board_entries", id: "board-window", has_more_before: false,
      entries: ["session-one", "session-resumed"].map((session, index) => ({
        id: session, author_kind: "agent", author: "Codex", body: `Agent update ${index}`,
        origin_agent_id: "codex", origin_session_id: session,
        created_at: `2026-10-16T00:00:0${index}Z`,
      })),
    }));
    await main.locator('[data-board-entry-id="session-one"] .board-origin-button').click();
    await expect(main).toHaveAttribute("data-surface", "agents");
    await expect(main.locator('[data-agent-id="agent-one"] .xterm')).toBeVisible();
    await expect(main.getByRole("tab", { name: /Aster/ })).toHaveAttribute("aria-selected", "true");
    await page.locator('.op-rail__surface[data-surface="board"]').click();
    await main.locator('[data-board-entry-id="session-resumed"] .board-origin-button').click();
    await expect(main).toHaveAttribute("data-surface", "agents");
    await expect(main.getByRole("tab", { name: "All agents", exact: true })).toHaveAttribute("aria-selected", "true");
    await expect(main.locator('[data-agent-id="agent-resumed"] .xterm')).toBeVisible();
    expect((await sentMessages(page)).filter(message => message.kind === "open_board_origin_agent")
      .map(message => message.origin_session_id)).toEqual(["session-one", "session-resumed"]);
    await expectSurfaceOnly(page);
    expect(errors).toEqual([]);
  });

  test("PM owns only its terminal and auxiliary Settings across reload with nearby agents and Start Work", async ({ page }, testInfo) => {
    const errors: string[] = [];
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("pageerror", error => errors.push(String(error)));
    await installSurfaceAssets(page);
    await installSurfaceRailBackend(page, false, true);
    await page.goto(surfaceAppUrl);
    const main = page.getByRole("region", { name: "Main pane", exact: true });
    const openPm = () => page.locator("#op-pm-entry").click();
    const expectPm = async () => {
      await expect(main).toHaveAttribute("data-surface", "pm");
      await expect(main.locator('[data-id="pm-window"]')).toBeVisible();
      await expect(main.locator('.workspace-window')).toHaveCount(1);
      for (const id of ["agent-one", "agent-two", "naming-window", "start-work-shell"]) await expect(windowById(page, id)).toBeHidden();
      await expectSurfaceOnly(page);
      await expect(page.locator("#split-view-button")).toBeDisabled();
    };
    await openPm();
    await expectPm();
    await page.evaluate(() => (window as any).__updateSurfaceAgent("pm-window", {}));
    await expectPm();
    await page.evaluate(() => (window as any).__surfaceRailSocket().emit({ kind: "hub_state", hub: {
      app_version: "playwright", projects: [{ id: "tab-1", title: "Surface Rail Fixture", project_root: "/fixture", kind: "git" }], recent_projects: [],
    } }));
    await expectPm();
    await main.locator('[data-id="pm-window"] .xterm').evaluate(node => { (window as any).__ownedPmTerminal = node; });
    await page.locator("#op-pm-settings-button").click();
    const auxiliary = main.locator('.workspace-window[data-preset="settings"]');
    await expect(auxiliary).toBeVisible();
    const auxiliaryId = await auxiliary.getAttribute("data-id");
    expect((await sentMessages(page)).filter(message => message.kind === "create_window" && message.surface_owner === "pm")).toHaveLength(1);
    await auxiliary.getByRole("tab", { name: "Project Manager", exact: true }).click();
    await openPm();
    expect(await main.locator('[data-id="pm-window"] .xterm').evaluate(node => node === (window as any).__ownedPmTerminal)).toBe(true);
    await page.locator('.op-rail__surface[data-surface="agents"]').click();
    await expect(main.locator(".agent-tile")).toHaveCount(4);
    await expect(main.locator('[data-agent-id="start-work-shell"]')).toContainText("Interactive");
    await page.locator('[data-agent-id="start-work-shell"] .xterm-helper-textarea').press("s");
    await expect.poll(async () => (await sentMessages(page)).filter(message => message.kind === "terminal_input")).toContainEqual({ kind: "terminal_input", id: "start-work-shell", data: "s" });
    await openPm();
    await page.locator("#op-pm-settings-button").click();
    await page.reload();
    await openPm();
    await expectPm();
    await page.locator("#op-pm-settings-button").click();
    await expect(auxiliary).toHaveAttribute("data-id", auxiliaryId!);
    await auxiliary.locator('[data-action="close"]').click();
    const close = page.locator("#window-close-confirm-modal");
    await close.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(auxiliary).toBeVisible();
    await auxiliary.locator('[data-action="close"]').click();
    await close.getByRole("button", { name: "Close window", exact: true }).click();
    await expectPm();
    await page.locator("#op-pm-settings-button").click();
    await expect(auxiliary).toBeVisible();
    await openPm();
    await expectPm();
    await page.locator('.op-rail__surface[data-surface="agents"]').click();
    await expect(main.locator(".agent-tile")).toHaveCount(3);
    await expect(main.locator('[data-agent-id="start-work-shell"]')).toHaveCount(0);
    await expect(main.locator('[data-agent-id="pm-window"]')).toHaveCount(0);
    await expect(page.locator("#split-view-button")).toBeEnabled();
    await page.evaluate(() => (window as any).__surfaceRailSocket().emit({ kind: "launch_wizard_open_error", title: "Start Work", message: "Fixture opening state" }));
    await expect(page.locator("#wizard-modal")).toBeVisible();
    await expect(main).toHaveAttribute("data-surface", "agents");
    await page.locator("#wizard-cancel-button").click();
    await openPm();
    await expectPm();
    await page.screenshot({ path: testInfo.outputPath("pm-surface-ownership.png") });
    expect((await sentMessages(page)).filter(message => message.kind === "open_pm_agent")).toEqual([]);
    expect(errors).toEqual([]);
  });

  test("rail surfaces fill one fixed pane and split collapses to the active live view", async ({ page }, testInfo) => {
    const errors: string[] = [];
    page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
    page.on("pageerror", (error) => errors.push(String(error)));
    await installSurfaceAssets(page);
    await installSurfaceRailBackend(page);
    await page.goto(surfaceAppUrl);
    await expect(windowById(page, "board-window")).toBeVisible();
    const geometry = () => page.locator(".workspace-window").evaluateAll((elements) =>
      Object.fromEntries(elements.map((element) => {
        const node = element as HTMLElement;
        return [node.dataset.id, [node.style.left, node.style.top, node.style.width, node.style.height]];
      })),
    );
    const original = await geometry();
    await page.locator('.op-rail__surface[data-surface="board"]').click();
    const main = page.getByRole("region", { name: "Main pane", exact: true });
    await expect(main.locator('[data-id="board-window"]')).toBeVisible();
    await expectSurfaceOnly(page);
    const areaBox = await page.locator(".surface-area").boundingBox();
    const mainBox = await main.boundingBox();
    expect(mainBox!.width).toBeCloseTo(areaBox!.width, 0);
    await main.locator('[data-id="board-window"]').evaluate(node => { (window as any).__singleBoardView = node; });
    await page.getByRole("button", { name: "Split view", exact: true }).click();
    const left = page.getByRole("region", { name: "Left pane", exact: true });
    const right = page.getByRole("region", { name: "Right pane", exact: true });
    await expect(left).toHaveAttribute("data-surface", "board");
    expect(await left.locator('[data-id="board-window"]').evaluate(node => node === (window as any).__singleBoardView)).toBe(true);
    await expect(right).toHaveAttribute("data-surface", "issues");
    const leftBox = await left.boundingBox();
    const rightBox = await right.boundingBox();
    expect(leftBox!.x + leftBox!.width).toBeLessThanOrEqual(rightBox!.x);
    expect(Math.abs(leftBox!.width - rightBox!.width)).toBeLessThan(2);
    await right.getByRole("combobox", { name: "Right pane surface" }).selectOption("agents");
    await expect(right).toHaveAttribute("data-active", "true");
    await expect(left).toHaveAttribute("data-surface", "board");
    await expect(right.locator(".agent-tile")).toHaveCount(2);
    await expect(right.locator("[data-agent-id='agent-one']")).toBeVisible();
    await expect(right.locator("[data-agent-id='agent-two']")).toBeVisible();
    await expect(right.locator("[data-id='pm-window']")).toHaveCount(0);
    // Focus selects which pane the fixed rail controls.
    await left.getByRole("combobox", { name: "Left pane surface" }).focus();
    await page.locator(".op-rail__surface[data-surface='settings']").click();
    await expect(left).toHaveAttribute("data-surface", "settings");
    await expect(left.locator(".workspace-window[data-preset='settings']")).toBeVisible();
    await expect(right).toHaveAttribute("data-surface", "agents");
    await expect(left.getByRole("option", { name: "Agents", exact: true })).toHaveJSProperty("disabled", false);
    await page.setViewportSize({ width: 1000, height: 800 });
    await expect(right.locator(".agent-tile").first()).toBeVisible();
    await expect(left).toHaveAttribute("data-surface", "settings");
    const splitGeometry = await geometry();
    for (const [id, value] of Object.entries(original)) expect(splitGeometry[id]).toEqual(value);
    await page.screenshot({ path: testInfo.outputPath("split-surfaces.png") });
    await clearMessages(page);
    await page.getByRole("button", { name: "Close split", exact: true }).click();
    await expect(page.locator("#split-surfaces")).toHaveAttribute("data-split", "false");
    await expect(main).toHaveAttribute("data-surface", "settings");
    await expect(main.locator(".workspace-window[data-preset='settings']")).toBeVisible();
    await expectSurfaceOnly(page);
    await page.screenshot({ path: testInfo.outputPath("single-surface.png") });
    expect((await sentMessages(page)).filter((message) => message.kind === "close_window")).toEqual([]);
    expect(errors).toEqual([]);
  });

  test("the same non-Agent surfaces open as two live views with pane-local Settings labels", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await installSurfaceAssets(page);
    await installSurfaceRailBackend(page);
    await page.goto(surfaceAppUrl);
    await page.getByRole("button", { name: "Split view", exact: true }).click();
    const left = page.getByRole("region", { name: "Left pane", exact: true });
    const right = page.getByRole("region", { name: "Right pane", exact: true });
    for (const surface of ["board", "issues", "settings"]) {
      await left.getByRole("combobox", { name: "Left pane surface" }).selectOption(surface);
      await right.getByRole("combobox", { name: "Right pane surface" }).selectOption(surface);
      const preset = surface === "issues" ? "settings" : surface;
      const view = `.workspace-window[data-preset='${preset}']`;
      await expect(left.locator(view)).toBeVisible();
      await expect(right.locator(view)).toBeVisible();
      expect(await left.locator(view).getAttribute("data-id")).not.toBe(await right.locator(view).getAttribute("data-id"));
      await expect(right).toHaveAttribute("data-active", "true");
    }
    for (const pane of [left, right]) {
      await pane.getByRole("tab", { name: "System", exact: true }).click();
      const language = pane.getByLabel("Output Language", { exact: true });
      await expect(language).toBeVisible();
      expect(await language.evaluate(node => node.closest(".split-pane")?.getAttribute("aria-label")))
        .toBe(await pane.getAttribute("aria-label"));
      await pane.getByRole("tab", { name: "Custom Agents", exact: true }).click();
      await expect(pane.getByRole("tab", { name: "Custom Agents", exact: true })).toHaveAttribute("aria-selected", "true");
    }
    expect((await sentMessages(page)).filter(message => message.kind === "create_window").length).toBe(2);
    for (const message of (await sentMessages(page)).filter(message => message.kind === "create_window")) {
      expect(message.bounds!.width).toBeGreaterThan(0);
      expect(message.bounds!.height).toBeGreaterThan(0);
    }
    const activeViewId = await right.locator(".workspace-window").getAttribute("data-id");
    await clearMessages(page);
    await page.getByRole("button", { name: "Close split", exact: true }).click();
    await expect(page.getByRole("region", { name: "Main pane", exact: true }).locator(".workspace-window")).toHaveAttribute("data-id", activeViewId!);
    await expectSurfaceOnly(page);
    expect((await sentMessages(page)).filter(message => message.kind === "close_window")).toEqual([]);
    expect(errors).toEqual([]);
  });

  test("the same agent stays live in both panes and input follows the selected pane", async ({ page }, testInfo) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await installSurfaceAssets(page);
    await installSurfaceRailBackend(page);
    await page.goto(surfaceAppUrl);
    await page.locator('.op-rail__surface[data-surface="agents"]').click();
    await page.getByRole("tab", { name: "Aster", exact: true }).click();
    await page.locator('.agent-tile[data-agent-id="agent-one"] .xterm').evaluate(node => { (window as any).__splitAgentTerminal = node; });
    await page.getByRole("button", { name: "Split view", exact: true }).click();
    const left = page.getByRole("region", { name: "Left pane", exact: true });
    const right = page.getByRole("region", { name: "Right pane", exact: true });
    await right.getByRole("combobox", { name: "Right pane surface" }).selectOption("agents");
    await expect(right.locator('.agent-tile[data-agent-id="agent-one"] .xterm')).toBeVisible();
    await expect(left.locator('.agent-tile[data-agent-id="agent-one"] pre')).toBeVisible();
    await expect(left.locator("textarea")).toHaveCount(0);
    const output = "SAME AGENT BOTH PANES";
    await page.evaluate(text => (window as any).__surfaceRailSocket().emit({ kind: "terminal_output", id: "agent-one", data_base64: btoa(text + "\r\n") }), output);
    await expect(left.locator('.agent-tile[data-agent-id="agent-one"] pre')).toContainText(output);
    await expect(right.locator('.agent-tile[data-agent-id="agent-one"] .xterm-rows')).toContainText(output);
    const previousRevision = Number(await right.locator(".agents-surface:not(.agents-surface--preview)").getAttribute("data-catalog-version"));
    await page.evaluate(() => {
      (window as any).__catalogRaf = window.requestAnimationFrame;
      window.requestAnimationFrame = () => 0;
      Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
      (window as any).__updateSurfaceAgent("agent-one", { dynamic_title: "Updated agent" });
    });
    await expect(right.getByRole("tab", { name: "Aster — Updated agent", exact: true })).toBeVisible();
    await expect(left.locator('.agent-tile[data-agent-id="agent-one"] h3')).toHaveText("Aster — Updated agent", { timeout: 1000 });
    const expectCatalogRevision = async () => {
      await expect.poll(() => page.evaluate(() => {
        const nodes = document.querySelectorAll(".agents-surface, #split-surfaces");
        const revisions = [...nodes].map(node => Number((node as HTMLElement).dataset.catalogVersion));
        return revisions.length === 3 && revisions.every(value => value > 0 && value === revisions[0]) ? revisions[0] : 0;
      }), { timeout: 1000 }).toBeGreaterThan(previousRevision);
    };
    await expectCatalogRevision();
    await page.evaluate(() => (window as any).__surfaceRailSocket().emit({ kind: "terminal_status", id: "agent-one", status: "stopped" }));
    await expect(right.locator('.agent-tile[data-agent-id="agent-one"] .agent-tile__mode')).toHaveText("Input unavailable", { timeout: 1000 });
    await expectCatalogRevision();
    await page.evaluate(() => {
      window.requestAnimationFrame = (window as any).__catalogRaf;
      delete (document as any).visibilityState;
      (window as any).__surfaceRailSocket().emit({ kind: "terminal_status", id: "agent-one", status: "running" });
    });
    await expect(right).toHaveAttribute("data-active", "true");
    await left.locator('.agent-tile[data-agent-id="agent-one"]').click();
    await expect(left.locator('.agent-tile[data-agent-id="agent-one"] .xterm-helper-textarea')).toBeFocused();
    await clearMessages(page);
    await left.locator('.agent-tile[data-agent-id="agent-one"] .xterm-helper-textarea').press("a");
    await expect.poll(async () => (await sentMessages(page)).filter(message => message.kind === "terminal_input"))
      .toEqual([{ kind: "terminal_input", id: "agent-one", data: "a" }]);
    await right.getByRole("combobox", { name: "Right pane surface" }).focus();
    await expect(right.locator('.agent-tile[data-agent-id="agent-one"] .xterm')).toBeVisible();
    expect(await right.locator('.agent-tile[data-agent-id="agent-one"] .xterm').evaluate(node => node === (window as any).__splitAgentTerminal)).toBe(true);
    await page.setViewportSize({ width: 1000, height: 800 });
    await expect(left.locator('.agent-tile[data-agent-id="agent-one"] pre')).toContainText(output);
    await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.includes("light") ? "light" : "dark");
    await page.screenshot({ path: testInfo.outputPath("same-agent-both-panes.png") });
    await right.getByRole("button", { name: "Close Aster — Updated agent tab", exact: true }).click();
    await expect(left.locator('.agent-tile[data-agent-id="agent-two"]')).toBeVisible();
    await right.getByRole("button", { name: "Open Aster — Updated agent tab", exact: true }).click();
    await page.evaluate(() => (window as any).__removeSurfaceAgent("agent-one", { id: "agent-two", status: "stopped" }));
    await expect(left.locator('.agent-tile[data-agent-id="agent-one"]')).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).__gwtTerminalTestApi.metrics("agent-one").hasRuntime)).toBe(false);
    await expect(right.locator('.agent-tile[data-agent-id="agent-two"] .agent-tile__mode')).toHaveText("Input unavailable");
    await expect(right.getByRole("tab", { name: "All agents", exact: true })).toHaveAttribute("aria-selected", "true");
    await page.getByRole("button", { name: "Close split", exact: true }).click();
    await expect(page.locator(".agents-surface--preview")).toHaveCount(0);
    await expect(page.getByRole("region", { name: "Main pane", exact: true }).locator('.agent-tile[data-agent-id="agent-two"]')).toBeVisible();
    expect((await sentMessages(page)).filter(message => ["close_window", "stop_agent"].includes(message.kind || ""))).toEqual([]);
    expect(errors).toEqual([]);
  });

  test("long agent titles stay on one line with a fixed name and hover or focus explanations", async ({ page }, testInfo) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await installSurfaceAssets(page);
    await installSurfaceRailBackend(page);
    await page.goto(surfaceAppUrl);
    await page.locator('.op-rail__surface[data-surface="agents"]').click();
    await page.getByRole("spinbutton", { name: "Agent columns" }).fill("4");
    const tile = page.locator('.agent-tile[data-agent-id="agent-one"]');
    const header = tile.locator(".agent-tile__header");
    const title = header.locator("h3");
    const terminal = tile.locator(".agent-tile__terminal");
    const originalHeader = await header.boundingBox();
    const originalTerminal = await terminal.boundingBox();
    const originalTab = await page.getByRole("tab", { name: "Aster", exact: true }).boundingBox();
    const purpose = "Investigate terminal output, preserve agent identity, and deliver the smallest verified change across the existing application surfaces.\n".repeat(2).trim();
    const fullTitle = `Aster — ${purpose}`;
    await page.evaluate(purpose => (window as any).__updateSurfaceAgent("agent-one", { dynamic_title: purpose }), purpose);
    await expect(title).toHaveText(fullTitle);
    expect((await header.boundingBox())!.height).toBe(originalHeader!.height);
    expect((await terminal.boundingBox())!.height).toBe(originalTerminal!.height);
    const name = title.locator(".agent-title__name");
    const body = title.locator(".agent-title__purpose");
    const nameBox = await name.boundingBox();
    const bodyBox = await body.boundingBox();
    expect(nameBox!.y).toBe(bodyBox!.y);
    expect(await title.evaluate(node => getComputedStyle(node).whiteSpace)).toBe("nowrap");
    expect(await body.evaluate(node => node.scrollWidth > node.clientWidth)).toBe(true);
    await body.hover();
    const explanation = page.locator("#agents-control-explanation");
    await expect(explanation).toBeVisible();
    await expect(explanation).toHaveText(fullTitle);
    await page.mouse.wheel(250, 0);
    await expect.poll(() => body.evaluate(node => node.scrollLeft)).toBeGreaterThan(0);
    expect((await name.boundingBox())!.x).toBe(nameBox!.x);
    await page.mouse.move(0, 0);
    await title.focus();
    await expect(title).toBeFocused();
    await expect(explanation).toBeVisible();
    await expect(title).toHaveAttribute("aria-describedby", "agents-control-explanation");
    await clearMessages(page);
    await page.keyboard.press("Escape");
    await expect(explanation).toBeHidden();
    expect((await sentMessages(page)).filter(message => message.kind === "update_viewport")).toEqual([]);
    const tab = page.getByRole("tab", { name: fullTitle, exact: true });
    expect((await tab.boundingBox())!.height).toBe(originalTab!.height);
    expect(await tab.evaluate(node => getComputedStyle(node).whiteSpace)).toBe("nowrap");
    await tab.focus();
    await expect(explanation).toBeVisible();
    await expect(explanation).toHaveText(fullTitle);
    const tabNameBox = await tab.locator(".agent-title__name").boundingBox();
    await tab.locator(".agent-title__purpose").hover();
    await page.mouse.wheel(250, 0);
    await expect.poll(() => tab.locator(".agent-title__purpose").evaluate(node => node.scrollLeft)).toBeGreaterThan(0);
    await expect(tab.locator(".agent-title__name")).toHaveText("Aster");
    expect((await tab.locator(".agent-title__name").boundingBox())!.x).toBe(tabNameBox!.x);
    await page.keyboard.press("Escape");
    await page.evaluate(() => (window as any).__updateSurfaceAgent("agent-one", { agent_name: "Vireo" }));
    await expect(title.locator(".agent-title__name")).toHaveText("Vireo");
    await page.getByRole("spinbutton", { name: "Agent columns" }).fill("2");
    await page.getByRole("button", { name: "Split view", exact: true }).click();
    const right = page.getByRole("region", { name: "Right pane", exact: true });
    await right.getByRole("combobox", { name: "Right pane surface" }).selectOption("agents");
    const preview = page.locator('.agents-surface--preview [data-agent-id="agent-one"] h3');
    await expect(preview).toHaveText(`Vireo — ${purpose}`);
    await expect(preview.locator(".agent-title__name")).toHaveText("Vireo");
    expect(await preview.evaluate(node => getComputedStyle(node).whiteSpace)).toBe("nowrap");
    await right.getByRole("spinbutton", { name: "Agent columns" }).fill("4");
    for (const heading of [preview, right.locator('[data-agent-id="agent-one"] h3')]) {
      const identity = await heading.locator(".agent-title__name").boundingBox();
      const mode = await heading.locator("..").locator(":scope > span").boundingBox();
      // Allow subpixel overlap of advance boxes; glyphs retain their side bearings.
      expect(identity!.x + identity!.width).toBeLessThanOrEqual(mode!.x + 0.5);
    }
    await right.getByRole("spinbutton", { name: "Agent columns" }).fill("2");
    await preview.hover();
    await expect(page.locator("#agents-preview-control-explanation")).toHaveText(`Vireo — ${purpose}`);
    await expect(page.locator("#agents-preview-control-explanation")).toBeVisible();
    await page.mouse.move(0, 0);
    await preview.focus();
    await expect(page.locator('.agents-surface:not(.agents-surface--preview) [data-agent-id="agent-one"] h3')).toBeFocused();
    await expect(page.locator("#agents-control-explanation")).toBeVisible();
    await expect(page.locator("#agents-control-explanation")).toHaveText(`Vireo — ${purpose}`);
    await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.includes("light") ? "light" : "dark");
    await page.screenshot({ path: testInfo.outputPath("agent-title-one-line.png") });
    expect(errors).toEqual([]);
  });

  test("Settings launch controls and PM controls share hover and keyboard explanations without changing settings", async ({ page }, testInfo) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await installSurfaceAssets(page);
    await installSurfaceRailBackend(page);
    await page.goto(surfaceAppUrl);
    await page.evaluate(() => {
      (window as any).__surfaceRailSocket().emit({ kind: "issue_monitor_status", status: {
        enabled: true, state: "active", active_count: 1, max_active_agents: 2, max_active_agents_override: 2,
        max_active_agents_mode: "manual", terminal_queue_auto_refill: true, terminal_queue_auto_refill_limit: 3,
        allowed_labels: ["bug"], terminal_queue: [], queue_len: 0,
      } });
      (window as any).__surfaceRailSocket().emit({ kind: "pm_status", available: true, paused: false,
        auto_start: true, configured_agent_id: "claude", loop_interval_secs: 60, running_agent_id: "claude", is_running: true,
        agent_options: [{ id: "claude", name: "Claude Code" }, { id: "codex", name: "Codex" }],
      });
    });
    await page.locator('.op-rail__surface[data-surface="settings"]').click();
    const settings = page.locator('.workspace-window[data-preset="settings"]');
    await settings.getByRole("tab", { name: "Issues", exact: true }).click();
    const issues = settings.locator('[data-settings-panel="issues"]');
    await issues.locator(".settings-issues-options > summary").click();
    await expect(issues.getByRole("spinbutton", { name: "Max active", exact: true })).toBeEnabled();
    await clearMessages(page);
    const explain = async (control: ReturnType<Page["locator"]>, pattern: RegExp) => {
      await control.hover();
      let ids = (await control.getAttribute("aria-describedby"))!.split(/\s+/);
      const popup = page.locator(`#${ids.at(-1)}`);
      await expect(popup).toBeVisible();
      await expect(popup).toContainText(pattern);
      await page.mouse.move(0, 0);
      await control.focus();
      await expect(control).toBeFocused();
      await expect(popup).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(popup).toBeHidden();
    };
    for (const [name, pattern] of [
      ["Max active mode", /Auto.*Manual/], ["Max active", /agent/i],
      ["Auto-refill", /queue/i], ["Refill limit", /0.*disable/i], ["Allowed labels", /matching issues/i],
    ] as const) await explain(issues.getByLabel(name, { exact: true }), pattern);
    await settings.getByRole("tab", { name: "Project Manager", exact: true }).click();
    const pm = settings.locator('[data-settings-panel="project-manager"]');
    for (const [role, pattern] of [
      ["pm-autonomy-status", /autonomous coordination/i], ["pm-pause-toggle", /Issue Monitor stays unchanged/],
      ["pm-agent-select", /after restart/], ["pm-model-input", /after restart/], ["pm-effort-select", /after restart/],
      ["pm-loop-interval", /Minimum: 10 seconds/], ["pm-auto-start", /automatically/],
    ] as const) await explain(pm.locator(`[data-role="${role}"]`), pattern);
    await page.screenshot({ path: testInfo.outputPath("settings-control-explanations.png") });
    await pm.locator('[data-role="pm-autonomy-status"]').focus();
    const popupId = (await pm.locator('[data-role="pm-autonomy-status"]').getAttribute("aria-describedby"))!.split(/\s+/).at(-1);
    await expect(page.locator(`#${popupId}`)).toBeVisible();
    await page.evaluate(() => (window as any).__removeSurfaceAgent("issue-window"));
    await expect(windowById(page, "issue-window")).toHaveCount(0);
    await expect(page.locator(`#${popupId}`)).toHaveCount(0);
    expect((await sentMessages(page)).filter(message => /^(set_issue_monitor|set_pm|restart_pm|update_viewport)/.test(message.kind || ""))).toEqual([]);
    await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.includes("light") ? "light" : "dark");
    expect(errors).toEqual([]);
  });

  test("Agents uses equal columns, keyboard spin control, live output and per-session input", async ({ page, browser }, testInfo) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await installSurfaceAssets(page);
    await installSurfaceRailBackend(page);
    await page.goto(surfaceAppUrl);
    await page.locator('.op-rail__surface[data-surface="agents"]').click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.includes("light") ? "light" : "dark");
    const tiles = page.locator(".agent-tile");
    await expect(tiles).toHaveCount(2);
    const columns = page.getByRole("spinbutton", { name: "Agent columns" });
    await columns.fill("1");
    const first = await tiles.nth(0).boundingBox();
    const second = await tiles.nth(1).boundingBox();
    expect(second!.y).toBeGreaterThan(first!.y);
    await columns.press("ArrowUp");
    await expect(columns).toHaveValue("2");
    const boxes = await tiles.evaluateAll(nodes => nodes.map(node => { const r = node.getBoundingClientRect(); return { y: r.y, width: r.width }; }));
    expect(boxes[0].y).toBe(boxes[1].y);
    expect(Math.abs(boxes[0].width - boxes[1].width)).toBeLessThan(1);
    await columns.fill("4");
    await columns.press("ArrowUp");
    await expect(columns).toHaveValue("4");
    await columns.press("ArrowDown");
    await expect(columns).toHaveValue("3");
    await page.locator('.op-rail__surface[data-surface="board"]').click();
    await page.locator('.op-rail__surface[data-surface="agents"]').click();
    await expect(columns).toHaveValue("3");
    await page.reload();
    await page.locator('.op-rail__surface[data-surface="agents"]').click();
    await expect(columns).toHaveValue("3");
    await expect(page.locator(".agents-grid")).toHaveCSS("grid-template-columns", /^\S+ \S+ \S+$/);
    await page.screenshot({ path: testInfo.outputPath("agents-columns-restored.png") });
    const reopened = await browser.newContext({ storageState: await page.context().storageState(),
      colorScheme: testInfo.project.use.colorScheme, viewport: testInfo.project.use.viewport });
    try {
      const restored = await reopened.newPage();
      restored.on("pageerror", error => errors.push(String(error)));
      restored.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await installSurfaceAssets(restored);
      await installSurfaceRailBackend(restored);
      await restored.goto(surfaceAppUrl);
      await restored.locator('.op-rail__surface[data-surface="agents"]').click();
      await expect(restored.getByRole("spinbutton", { name: "Agent columns" })).toHaveValue("3");
    } finally { await reopened.close(); }
    await columns.fill("2");
    await page.evaluate(() => (window as any).__surfaceRailSocket().emit({ kind: "terminal_output", id: "agent-one", data_base64: btoa("LIVE AGENT OUTPUT\r\n") }));
    await expect(tiles.first().locator(".xterm-rows")).toContainText("LIVE AGENT OUTPUT");
    await tiles.first().locator(".xterm-helper-textarea").press("a");
    await expect(windowById(page, "agent-one")).toHaveClass(/\bfocused\b/);
    await expect.poll(async () => (await sentMessages(page)).filter(message => message.kind === "terminal_input")).toContainEqual({ kind: "terminal_input", id: "agent-one", data: "a" });
    await expect(tiles.locator("form, .agent-tile__input")).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath("agents-grid.png") });
    expect(errors).toEqual([]);
  });

  test("agent tabs switch live terminals with direct input and keep the All agents grid", async ({ page }, testInfo) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await installSurfaceAssets(page);
    await installSurfaceRailBackend(page);
    await page.goto(surfaceAppUrl);
    await page.locator('.op-rail__surface[data-surface="agents"]').click();
    const all = page.getByRole("tab", { name: "All agents", exact: true });
    const one = page.getByRole("tab", { name: "Aster", exact: true });
    const two = page.getByRole("tab", { name: "Birch", exact: true });
    const first = page.locator('.agent-tile[data-agent-id="agent-one"]');
    const second = page.locator('.agent-tile[data-agent-id="agent-two"]');
    await expect(all).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("tablist", { name: "Agent views" }).getByRole("tab")).toHaveCount(3);
    await one.click();
    await expect(first).toBeVisible();
    await expect(second).toBeHidden();
    await page.evaluate(() => (window as any).__surfaceRailSocket().emit({ kind: "terminal_output", id: "agent-two", data_base64: btoa("OUTPUT WHILE HIDDEN\r\n") }));
    await two.click();
    await expect(two).toHaveAttribute("aria-selected", "true");
    await expect(first).toBeHidden();
    await expect(second.locator(".xterm-rows")).toContainText("OUTPUT WHILE HIDDEN");
    await second.locator(".xterm-helper-textarea").press("b");
    await expect.poll(async () => (await sentMessages(page)).filter(message => message.kind === "terminal_input")).toContainEqual({ kind: "terminal_input", id: "agent-two", data: "b" });
    await expect(page.locator(".agent-tile form, .agent-tile__input")).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath("agents-tab-selected.png") });
    await two.focus();
    await two.press("ArrowLeft");
    await expect(one).toBeFocused();
    await expect(first).toBeVisible();
    await one.press("Home");
    await expect(all).toBeFocused();
    await expect(first).toBeVisible();
    await expect(second).toBeVisible();
    await expect(page.locator(".agent-tile .xterm")).toHaveCount(2);
    await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.includes("light") ? "light" : "dark");
    await page.screenshot({ path: testInfo.outputPath("agents-tab-grid.png") });
    await one.click();
    await expect(one).toBeFocused();
    await page.evaluate(() => (window as any).__removeSurfaceAgent("agent-one"));
    await expect(all).toBeFocused();
    await expect(all).toHaveAttribute("aria-selected", "true");
    await expect(second).toBeVisible();
    expect(errors).toEqual([]);
  });

  test("closing an agent tab keeps its terminal live and allows reopening", async ({ page }, testInfo) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await installSurfaceAssets(page);
    await installSurfaceRailBackend(page);
    await page.goto(surfaceAppUrl);
    await page.locator('.op-rail__surface[data-surface="agents"]').click();
    const one = page.getByRole("tab", { name: "Aster", exact: true });
    const all = page.getByRole("tab", { name: "All agents", exact: true });
    const tile = page.locator('.agent-tile[data-agent-id="agent-one"]');
    await one.click();
    await tile.locator(".xterm").evaluate(node => { (window as any).__closedTabTerminal = node; });
    await clearMessages(page);
    await page.getByRole("button", { name: "Close Aster tab", exact: true }).click();
    await expect(one).toHaveCount(0);
    await expect(all).toBeFocused();
    await expect(all).toHaveAttribute("aria-selected", "true");
    await expect(page.locator(".agent-tile")).toHaveCount(2);
    await page.evaluate(() => (window as any).__surfaceRailSocket().emit({ kind: "terminal_output", id: "agent-one", data_base64: btoa("LIVE AFTER TAB CLOSE\r\n") }));
    await expect(tile.locator(".xterm-rows")).toContainText("LIVE AFTER TAB CLOSE");
    const reopen = page.getByRole("button", { name: "Open Aster tab", exact: true });
    await expect(reopen).toHaveText("Open tab");
    await reopen.focus();
    await reopen.press("Enter");
    await expect(one).toHaveAttribute("aria-selected", "true");
    await expect(one).toBeFocused();
    expect(await tile.locator(".xterm").evaluate(node => node === (window as any).__closedTabTerminal)).toBe(true);
    await tile.locator(".xterm-helper-textarea").press("c");
    await expect.poll(async () => (await sentMessages(page)).filter(message => message.kind === "terminal_input")).toContainEqual({ kind: "terminal_input", id: "agent-one", data: "c" });
    await one.focus();
    await one.press("Delete");
    await expect(one).toHaveCount(0);
    await expect(all).toBeFocused();
    expect((await sentMessages(page)).filter(message => message.kind === "close_window" || message.kind === "stop_agent")).toEqual([]);
    await page.screenshot({ path: testInfo.outputPath("agents-tab-closed.png") });
    expect(errors).toEqual([]);
  });

  test("opening an inactive grouped surface keeps the split open", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(String(error)));
    page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
    await installSurfaceAssets(page);
    await installSurfaceRailBackend(page, true);
    await page.goto(surfaceAppUrl);
    await expect(windowById(page, "board-window")).toBeVisible();
    await page.getByRole("button", { name: "Split view", exact: true }).click();
    await page.getByRole("combobox", { name: "Left pane surface" }).selectOption("settings");
    await expect(page.locator("#split-surfaces")).toBeVisible();
    await expect(page.locator(".split-pane[data-surface='settings'] [data-id='settings-grouped']")).toBeVisible();
    expect(errors).toEqual([]);
  });

  test("two surfaces in one tab group stay visible without changing the active tab", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(String(error)));
    page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
    await installSurfaceAssets(page);
    await installSurfaceRailBackend(page, true);
    await page.goto(surfaceAppUrl);
    await expect(windowById(page, "board-window")).toBeVisible();
    await clearMessages(page);
    await page.getByRole("button", { name: "Split view", exact: true }).click();
    // Keep the other Settings view in the right pane while selecting the
    // inactive grouped view, then show its Board peer beside it.
    await page.getByRole("combobox", { name: "Left pane surface" }).selectOption("settings");
    await page.getByRole("combobox", { name: "Right pane surface" }).selectOption("board");
    await expect(page.locator(".split-pane[data-surface='settings'] [data-id='settings-grouped']")).toBeVisible();
    await expect(page.locator(".split-pane[data-surface='board'] [data-id='board-window']")).toBeVisible();
    expect((await sentMessages(page)).filter((message) => message.kind === "activate_window_tab")).toEqual([]);
    await page.getByRole("button", { name: "Close split", exact: true }).click();
    await expect(windowById(page, "settings-grouped")).toBeHidden();
    await expect(windowById(page, "board-window")).toBeVisible();
    expect(errors).toEqual([]);
  });

  test("selects Issues / Agents / Board / Settings and folds to a top strip when narrow", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(String(error)));

    await installSurfaceAssets(page);
    await installSurfaceRailBackend(page);
    await page.goto(surfaceAppUrl);

    const rail = page.locator("#op-rail");
    const surfaces = rail.locator(".op-rail__surface");
    const entry = (surface: string) => rail.locator(`[data-surface='${surface}']`);

    await expect(windowById(page, "board-window")).toBeVisible({ timeout: 10_000 });
    await expect(surfaces).toHaveText(["Issues", "Agents", "Board", "Settings"]);
    // The PM leads the rail, cut off from the four surfaces by a rule.
    const railOrder = await rail.evaluate((element) =>
      Array.from(element.querySelectorAll("[data-surface], .op-rail__divider")).map((node) =>
        node.classList.contains("op-rail__divider")
          ? "|"
          : (node as HTMLElement).dataset.surface ?? "",
      ),
    );
    expect(railOrder.slice(0, 6)).toEqual(["pm", "|", "issues", "agents", "board", "settings"]);
    await expect(entry("pm")).toHaveText("PM");
    for (const surface of ["issues", "agents", "board", "settings"]) {
      await expect(entry(surface).locator("svg")).toBeVisible();
    }

    // The topmost window is the Board, so the Board surface starts pressed.
    await expect(windowById(page, "board-window")).toHaveClass(/focused/);
    await expect(pressedSurfaces(page)).resolves.toEqual(["board"]);
    const railWidth = await rail.evaluate((element) => element.getBoundingClientRect().width);
    expect(railWidth).toBe(64);
    // The pressed entry carries a 2px accent edge, not only a color change.
    await expect(entry("board")).toHaveCSS("border-left-width", "2px");
    const accent = await entry("board").evaluate(
      (element) => getComputedStyle(element).borderLeftColor,
    );
    const idleEdge = await entry("issues").evaluate(
      (element) => getComputedStyle(element).borderLeftColor,
    );
    expect(accent).not.toBe(idleEdge);

    // Agents tiles every agent and excludes the PM without canvas controls.
    await clearMessages(page);
    await entry("agents").click();
    await expect.poll(() => pressedSurfaces(page)).toEqual(["agents"]);
    await expectSurfaceOnly(page);
    expect(
      (await sentMessages(page)).filter(
        (message) => message.kind === "create_window" || message.kind === "focus_window",
      ),
    ).toEqual([]);
    await expect(page.locator("[data-agent-id=agent-one]")).toBeVisible();
    await expect(page.locator("[data-agent-id=agent-two]")).toBeVisible();
    await expect(windowById(page, "pm-window")).toBeHidden();
    await expect(page.locator(".agent-tile[data-agent-id=pm-window]")).toHaveCount(0);

    // The PM entry lands on the PM and presses itself, not Agents: the role
    // marker (is_pm), not the claude preset, decides where it belongs.
    await clearMessages(page);
    await entry("pm").click();
    await expect(page.getByRole("region", { name: "Main pane", exact: true }).locator('[data-id="pm-window"]')).toBeVisible();
    await windowById(page, "pm-window").locator(".titlebar").click();
    await expect.poll(() => pressedSurfaces(page)).toEqual(["pm"]);

    // Issues opens its section in the migrated Settings view, keeping its ID.
    await clearMessages(page);
    await entry("issues").click();
    await expect(page.getByRole("region", { name: "Main pane", exact: true }).locator('[data-id="issue-window"]')).toBeVisible();
    await expect.poll(() => pressedSurfaces(page)).toEqual(["issues"]);
    await expect(windowById(page, "issue-window").getByRole("tab", { name: "Issues", exact: true })).toHaveAttribute("aria-selected", "true");
    expect(await createdPresets(page)).toEqual([]);

    // Settings reuses that same view without creating a second host.
    await clearMessages(page);
    await entry("settings").click();
    await expect(page.getByRole("region", { name: "Main pane", exact: true }).locator('[data-id="issue-window"]')).toBeVisible();
    expect(await createdPresets(page)).toEqual([]);

    // Every entry explains itself on hover.
    for (const surface of ["issues", "agents", "board", "settings"]) {
      await expect(entry(surface)).toHaveAttribute("title", /\S/);
    }
    await expect(entry("issues")).toHaveAttribute("title", /⌘G/);

    // At 860px and below the rail is a strip across the top of the surfaces.
    await page.setViewportSize({ width: 800, height: 900 });
    await expect
      .poll(() => rail.evaluate((element) => element.getBoundingClientRect().width))
      .toBe(800);
    const railBox = await rail.boundingBox();
    const surfaceBox = await page.locator(".surface-area").boundingBox();
    expect(railBox && surfaceBox).toBeTruthy();
    expect(railBox!.height).toBeLessThan(160);
    expect(railBox!.y + railBox!.height).toBeLessThanOrEqual(surfaceBox!.y + 1);
    await expect(entry("issues")).toHaveCSS("border-bottom-width", "2px");
    await expect(entry("issues")).toHaveCSS("border-left-width", "0px");
    const surfaceTops = await surfaces.evaluateAll((elements) =>
      elements.map((element) => Math.round(element.getBoundingClientRect().top)),
    );
    expect(new Set(surfaceTops).size).toBe(1);

    expect(pageErrors).toEqual([]);
    expect(consoleErrors).toEqual([]);
  });
});

function windowById(page: Page, id: string) {
  return page.locator(`.workspace-window[data-id='${id}']`);
}

function captureBrowserErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(String(error)));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  return errors;
}

async function expectSurfaceOnly(page: Page): Promise<void> {
  await expect(page.locator("#canvas, #canvas-world-grid, #canvas-stage, #fleet-minimap, .resize-handle, [data-action='ungroup']")).toHaveCount(0);
  await expect(page.locator("#split-surfaces")).toBeVisible();
}

async function pressedSurfaces(page: Page): Promise<string[]> {
  return page.evaluate(() =>
    Array.from(document.querySelectorAll(".op-rail [data-surface][aria-pressed='true']")).map(
      (element) => (element as HTMLElement).dataset.surface ?? "",
    ),
  );
}

async function sentMessages(page: Page): Promise<SentMessage[]> {
  return page.evaluate(() => [...(((window as any).__surfaceRailSent ?? []) as SentMessage[])]);
}

async function createdPresets(page: Page): Promise<string[]> {
  return (await sentMessages(page))
    .filter((message) => message.kind === "create_window")
    .map((message) => message.preset ?? "");
}

async function clearMessages(page: Page): Promise<void> {
  await page.evaluate(async () => {
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    );
    (window as any).__surfaceRailSent.length = 0;
  });
}

async function installSurfaceRailBackend(page: Page, groupedSettings = false, ownership = false, focusedId = "board-window"): Promise<void> {
  await page.addInitScript(({ groupedSettings, ownership, focusedId }) => {
    (window as any).__gwtPlaywrightTestBridge = true;
    const canvasWindow = (id: string, overrides: Record<string, unknown>) => ({
      id,
      title: id,
      preset: "agent",
      geometry: { x: 120, y: 100, width: 560, height: 340 },
      geometry_revision: 0,
      z_index: 1,
      status: "idle",
      minimized: false,
      maximized: false,
      pre_maximize_geometry: null,
      persist: true,
      purpose_title: null,
      dynamic_title: null,
      dynamic_title_detail: null,
      agent_id: null,
      session_id: null as string | null,
      agent_color: null,
      tab_group_id: null,
      tab_group_active: false,
      placement: { kind: "canvas" },
      ...overrides,
    });

    // Persisted canvas locations deliberately sit outside the viewport. The
    // fixed surface layout must ignore them, including inactive legacy groups.
    const windows = [
      canvasWindow("pm-window", {
        title: "Project Manager",
        preset: "claude",
        is_pm: true,
        status: "running",
        agent_id: "pm-window",
        agent_color: "yellow",
        geometry: { x: -3200, y: 80, width: 560, height: 340 },
        z_index: 2,
      }),
      canvasWindow("issue-window", {
        title: "Settings",
        preset: "settings",
        geometry: { x: 80, y: 80, width: 640, height: 420 },
        z_index: 10,
      }),
      canvasWindow("board-window", {
        title: "Board",
        preset: "board",
        geometry: { x: 760, y: 80, width: 520, height: 420 },
        z_index: 20,
      }),
      canvasWindow("agent-one", {
        status: "running",
        agent_id: "agent-one",
        agent_name: "Aster",
        session_id: "session-one",
        agent_color: "cyan",
        geometry: { x: 2400, y: 1400, width: 560, height: 340 },
        z_index: 5,
      }),
      canvasWindow("agent-two", {
        status: "waiting",
        agent_id: "agent-two",
        agent_name: "Birch",
        session_id: "session-two",
        agent_color: "green",
        geometry: { x: 3000, y: 1400, width: 560, height: 340 },
        z_index: 6,
      }),
    ];
    if (groupedSettings) {
      Object.assign(windows.find((data) => data.id === "board-window")!, { tab_group_id: "group", tab_group_active: true });
      windows.push(canvasWindow("settings-grouped", { preset: "settings", tab_group_id: "group", tab_group_active: false }));
    }
    if (focusedId === "legacy-issues-window") {
      windows.push(canvasWindow(focusedId, { preset: "issue", title: "Issues" }));
    }
    const focused = windows.find(data => data.id === focusedId);
    if (focused) focused.z_index = 100;
    if (focusedId === "issue-window") {
      // An inline agent can have a newer persisted z-index without becoming
      // the project landing surface. Restore its owning Settings/Issues view.
      for (const data of windows.filter(data => ["agent-one", "agent-two"].includes(data.id))) {
        Object.assign(data, { z_index: 200, placement: { kind: "issue_preview", issue_window_id: "issue-window", issue_number: 5595 } });
      }
    }

    if (ownership) {
      windows.push(canvasWindow("naming-window", { title: "Naming…", status: "running", session_id: "session-naming" }));
      windows.push(canvasWindow("start-work-shell", { preset: "shell", status: "running", surface_owner: "agents", persist: false }));
      for (const data of windows) data.geometry = { x: 80, y: 80, width: 560, height: 340 };
      const saved = sessionStorage.getItem("surface-ownership-windows");
      if (saved) windows.splice(0, windows.length, ...JSON.parse(saved));
    }
    let zCounter = 100;
    let socket: FixtureWebSocket | null = null;
    let activeTabId = "tab-1";

    const workspaceState = () => ({
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Surface Rail Fixture",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: windows.map((windowData) => ({ ...windowData })),
            },
          },
          {
            id: "tab-2",
            title: "Second project",
            project_root: "/second-fixture",
            kind: "git",
            workspace: {
              viewport: { x: -5000, y: 3000, zoom: 0.4 },
              windows: [canvasWindow("second-board", {
                preset: "board", geometry: { x: -5000, y: 3000, width: 520, height: 420 },
              })],
            },
          },
        ],
        active_tab_id: activeTabId,
        recent_projects: [],
      },
    });

    (window as any).__surfaceRailSent = [];

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      url: string;
      readyState = FixtureWebSocket.CONNECTING;

      constructor(url: string) {
        super();
        this.url = url;
        socket = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(workspaceState());
        }, 0);
      }

      send(raw: string): void {
        let message: SentMessage;
        try {
          message = JSON.parse(raw) as SentMessage;
        } catch {
          return;
        }
        (window as any).__surfaceRailSent.push(message);
        if (message.kind === "focus_window") {
          const target = windows.find((windowData) => windowData.id === message.id);
          if (target) {
            zCounter += 1;
            target.z_index = zCounter;
            this.emit(workspaceState());
          }
        }
        if (message.kind === "open_board_origin_agent") {
          let target = windows.find(data => data.session_id === message.origin_session_id);
          if (!target) {
            target = canvasWindow("agent-resumed", { session_id: message.origin_session_id, status: "running" });
            windows.push(target);
          }
          target.z_index = ++zCounter;
          this.emit(workspaceState());
        }
        if (message.kind === "create_window" && message.preset) {
          zCounter += 1;
          const id = `${message.preset}-new`;
          windows.push(canvasWindow(windows.some(data => data.id === id) ? `${id}-${zCounter}` : id, { preset: message.preset, surface_owner: message.surface_owner, z_index: zCounter }));
          this.emit(workspaceState());
        }
        if (message.kind === "close_window") {
          const index = windows.findIndex(data => data.id === message.id);
          if (index >= 0) windows.splice(index, 1);
          this.emit(workspaceState());
        }
        if (ownership) sessionStorage.setItem("surface-ownership-windows", JSON.stringify(windows.filter(windowData => windowData.persist !== false)));
        if (message.kind === "activate_window_tab") {
          const target = windows.find((data) => data.id === message.id);
          if (target?.tab_group_id) {
            for (const data of windows) {
              if (data.tab_group_id === target.tab_group_id) data.tab_group_active = data.id === target.id;
            }
            this.emit(workspaceState());
          }
        }
      }

      close(): void {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload: unknown): void {
        setTimeout(() => {
          this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) }));
        }, 0);
      }
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
    (window as any).__surfaceRailSocket = () => socket;
    (window as any).__switchSurfaceProject = (id: string) => {
      activeTabId = id;
      socket?.emit(workspaceState());
    };
    (window as any).__removeSurfaceAgent = (id: string, update?: Record<string, unknown>) => {
      const index = windows.findIndex(data => data.id === id);
      if (index >= 0) windows.splice(index, 1);
      const changed = update && windows.find(data => data.id === update.id);
      if (changed) Object.assign(changed, update);
      socket?.emit(workspaceState());
    };
    (window as any).__updateSurfaceAgent = (id: string, updates: Record<string, unknown>) => {
      const data = windows.find(data => data.id === id);
      if (data) Object.assign(data, updates);
      socket?.emit(workspaceState());
    };
  }, { groupedSettings, ownership, focusedId });
}
