/* Issue #4777 T-1 — the rail picks one of four surfaces.
 *
 * The PM sits alone above a rule; Issues / Agents / Board / Settings follow
 * with an icon and a visible label. The pressed entry is the surface of the focused window, a
 * click opens (or focuses) that surface, and at 860px and below the rail is a
 * strip across the top of the canvas. Runs against the embedded frontend with
 * a stub backend, in both themes, with zero console / page errors.
 */
import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// browser-check supplies an isolated checkout server; fixture mode remains
// useful for fast development while both modes exercise the same UI code.
const surfaceAppUrl = process.env.GWT_PLAYWRIGHT_BASE_URL || APP_URL;
async function installSurfaceAssets(page: Page) {
  if (!process.env.GWT_PLAYWRIGHT_BASE_URL) await installEmbeddedRoutes(page);
}

type SentMessage = {
  kind?: string;
  id?: string;
  preset?: string;
};

test.describe("Surface rail", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("splits two independent surfaces, keeps agents live, and restores the canvas", async ({ page }, testInfo) => {
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
    await page.getByRole("button", { name: "Split view", exact: true }).click();
    const left = page.getByRole("region", { name: "Left pane", exact: true });
    const right = page.getByRole("region", { name: "Right pane", exact: true });
    await expect(left).toHaveAttribute("data-surface", "board");
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
    await expect(page.locator("#split-surfaces")).toBeHidden();
    await expect(page.locator("#canvas-stage > .workspace-window")).toHaveCount(6);
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
      const preset = surface === "issues" ? "issue" : surface;
      const view = `.workspace-window[data-preset='${preset}']`;
      await expect(left.locator(view)).toBeVisible();
      await expect(right.locator(view)).toBeVisible();
      expect(await left.locator(view).getAttribute("data-id")).not.toBe(await right.locator(view).getAttribute("data-id"));
      await expect(right).toHaveAttribute("data-active", "true");
    }
    for (const pane of [left, right]) {
      const language = pane.getByLabel("Output Language", { exact: true });
      await expect(language).toBeVisible();
      expect(await language.evaluate(node => node.closest(".split-pane")?.getAttribute("aria-label")))
        .toBe(await pane.getAttribute("aria-label"));
      await pane.getByRole("tab", { name: "Custom Agents", exact: true }).click();
      await expect(pane.getByRole("tab", { name: "Custom Agents", exact: true })).toHaveAttribute("aria-selected", "true");
    }
    expect((await sentMessages(page)).filter(message => message.kind === "create_window").length).toBe(4);
    await clearMessages(page);
    await page.getByRole("button", { name: "Close split", exact: true }).click();
    await expect(page.locator("#canvas-stage > .workspace-window")).toHaveCount(9);
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
    await page.getByRole("tab", { name: "agent-one", exact: true }).click();
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
    await page.evaluate(() => (window as any).__updateSurfaceAgent("agent-one", { dynamic_title: "Updated agent" }));
    await expect(right.getByRole("tab", { name: "Updated agent", exact: true })).toBeVisible();
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
    await right.getByRole("button", { name: "Close Updated agent tab", exact: true }).click();
    await expect(left.locator('.agent-tile[data-agent-id="agent-two"]')).toBeVisible();
    await right.getByRole("button", { name: "Open Updated agent tab", exact: true }).click();
    await page.evaluate(() => (window as any).__removeSurfaceAgent("agent-one"));
    await expect(left.locator('.agent-tile[data-agent-id="agent-one"]')).toHaveCount(0);
    await expect(right.getByRole("tab", { name: "All agents", exact: true })).toHaveAttribute("aria-selected", "true");
    await page.getByRole("button", { name: "Close split", exact: true }).click();
    await expect(page.locator(".agents-surface--preview")).toHaveCount(0);
    expect((await sentMessages(page)).filter(message => ["close_window", "stop_agent"].includes(message.kind || ""))).toEqual([]);
    expect(errors).toEqual([]);
  });

  test("Agents uses equal columns, keyboard spin control, live output and per-session input", async ({ page }, testInfo) => {
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
    const one = page.getByRole("tab", { name: "agent-one", exact: true });
    const two = page.getByRole("tab", { name: "agent-two", exact: true });
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
    const one = page.getByRole("tab", { name: "agent-one", exact: true });
    const all = page.getByRole("tab", { name: "All agents", exact: true });
    const tile = page.locator('.agent-tile[data-agent-id="agent-one"]');
    await one.click();
    await tile.locator(".xterm").evaluate(node => { (window as any).__closedTabTerminal = node; });
    await clearMessages(page);
    await page.getByRole("button", { name: "Close agent-one tab", exact: true }).click();
    await expect(one).toHaveCount(0);
    await expect(all).toBeFocused();
    await expect(all).toHaveAttribute("aria-selected", "true");
    await expect(page.locator(".agent-tile")).toHaveCount(2);
    await page.evaluate(() => (window as any).__surfaceRailSocket().emit({ kind: "terminal_output", id: "agent-one", data_base64: btoa("LIVE AFTER TAB CLOSE\r\n") }));
    await expect(tile.locator(".xterm-rows")).toContainText("LIVE AFTER TAB CLOSE");
    const reopen = page.getByRole("button", { name: "Open agent-one tab", exact: true });
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
    await page.getByRole("combobox", { name: "Right pane surface" }).selectOption("settings");
    await expect(page.locator(".split-pane[data-surface='board'] [data-id='board-window']")).toBeVisible();
    await expect(page.locator(".split-pane[data-surface='settings'] [data-id='settings-grouped']")).toBeVisible();
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

    // Agents tiles every agent, excludes the PM and leaves the camera unchanged.
    await clearMessages(page);
    const stage = page.locator("#canvas-stage");
    const before = await stage.evaluate((element) => (element as HTMLElement).style.transform);
    await entry("agents").click();
    await expect.poll(() => pressedSurfaces(page)).toEqual(["agents"]);
    expect(await stage.evaluate((element) => (element as HTMLElement).style.transform)).toBe(before);
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
    await expect.poll(() => inCanvasView(page, "pm-window")).toBe(true);
    await windowById(page, "pm-window").locator(".titlebar").click();
    await expect.poll(() => pressedSurfaces(page)).toEqual(["pm"]);

    // Issues focuses the existing Issue window instead of spawning another.
    await clearMessages(page);
    await entry("issues").click();
    await expect.poll(() => focusTargets(page)).toContain("issue-window");
    await expect.poll(() => pressedSurfaces(page)).toEqual(["issues"]);
    expect(await createdPresets(page)).toEqual([]);

    // Settings has no window yet, so the rail asks the backend for one.
    await clearMessages(page);
    await entry("settings").click();
    await expect.poll(() => createdPresets(page)).toEqual(["settings"]);

    // Every entry explains itself on hover.
    for (const surface of ["issues", "agents", "board", "settings"]) {
      await expect(entry(surface)).toHaveAttribute("title", /\S/);
    }
    await expect(entry("issues")).toHaveAttribute("title", /⌘G/);

    // At 860px and below the rail is a strip across the top of the canvas.
    await page.setViewportSize({ width: 800, height: 900 });
    await expect
      .poll(() => rail.evaluate((element) => element.getBoundingClientRect().width))
      .toBe(800);
    const railBox = await rail.boundingBox();
    const canvasBox = await page.locator(".canvas-area").boundingBox();
    expect(railBox && canvasBox).toBeTruthy();
    expect(railBox!.height).toBeLessThan(160);
    expect(railBox!.y + railBox!.height).toBeLessThanOrEqual(canvasBox!.y + 1);
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

async function inCanvasView(page: Page, id: string): Promise<boolean> {
  return page.evaluate((windowId) => {
    const canvas = document.getElementById("canvas")!.getBoundingClientRect();
    const target = document
      .querySelector(`.workspace-window[data-id='${windowId}']`)
      ?.getBoundingClientRect();
    if (!target) return false;
    return (
      target.left >= canvas.left - 1 &&
      target.right <= canvas.right + 1 &&
      target.top >= canvas.top - 1 &&
      target.bottom <= canvas.bottom + 1
    );
  }, id);
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

async function focusTargets(page: Page): Promise<string[]> {
  return (await sentMessages(page))
    .filter((message) => message.kind === "focus_window")
    .map((message) => message.id ?? "");
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

async function installSurfaceRailBackend(page: Page, groupedSettings = false): Promise<void> {
  await page.addInitScript((groupedSettings) => {
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
      agent_color: null,
      tab_group_id: null,
      tab_group_active: false,
      placement: { kind: "canvas" },
      ...overrides,
    });

    // Agents sit far to the right so framing them moves the camera; the PM
    // (a claude pane marked is_pm) sits far to the left so framing the
    // agents leaves it out of view.
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
        title: "Issues",
        preset: "issue",
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
        session_id: "session-one",
        agent_color: "cyan",
        geometry: { x: 2400, y: 1400, width: 560, height: 340 },
        z_index: 5,
      }),
      canvasWindow("agent-two", {
        status: "waiting",
        agent_id: "agent-two",
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

    let zCounter = 20;
    let socket: FixtureWebSocket | null = null;

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
        ],
        active_tab_id: "tab-1",
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
        if (message.kind === "create_window" && message.preset) {
          zCounter += 1;
          const id = `${message.preset}-new`;
          windows.push(canvasWindow(windows.some(data => data.id === id) ? `${id}-${zCounter}` : id, { preset: message.preset, z_index: zCounter }));
          this.emit(workspaceState());
        }
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
    (window as any).__removeSurfaceAgent = (id: string) => {
      const index = windows.findIndex(data => data.id === id);
      if (index >= 0) windows.splice(index, 1);
      socket?.emit(workspaceState());
    };
    (window as any).__updateSurfaceAgent = (id: string, updates: Record<string, unknown>) => {
      const data = windows.find(data => data.id === id);
      if (data) Object.assign(data, updates);
      socket?.emit(workspaceState());
    };
  }, groupedSettings);
}
