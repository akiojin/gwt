/* Issue #4777 T-19: approved Works list and reusable Agents roster.
 * Uses the surface-rail fixture's real assets + EventTarget WebSocket boundary.
 * No process liveness is inferred from this transport fixture; live provider
 * exit and owned GUI restart are covered by the separate live acceptance suite.
 */
import { expect, test, type Page } from "@playwright/test";
import { APP_PROJECT_KEY, APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

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

  test.beforeEach(async ({ page }, info) => {
    const errors: string[] = []; capturedErrors.set(page, errors);
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await page.addInitScript(theme => {
      localStorage.setItem("gwt:ui:theme", theme);
      localStorage.setItem("gwt:ui:agent-columns", "3");
    }, info.project.name.includes("light") ? "light" : "dark");
    if (!process.env.GWT_PLAYWRIGHT_BASE_URL) await installEmbeddedRoutes(page);
    await installWorksAgentsBackend(page);
    await page.goto(appUrl);
    await openRail(page, "works");
    await expect(main(page)).toHaveAttribute("data-surface", "works");
    await expect(workRow(page, "agent-one")).toBeVisible();
  });

  test.afterEach(async ({ page }, info) => {
    await expect(page.locator(".render-degradation-banner")).toHaveCount(0);
    await expect(page.locator("html")).toHaveAttribute("data-theme", info.project.name.includes("light") ? "light" : "dark");
    expect(capturedErrors.get(page), "zero console/page errors, including readback and navigation").toEqual([]);
    expect((await sent(page)).filter(message => ["close_window", "stop_agent", "stop_window", "restart_window"].includes(message.kind))).toEqual([]);
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
    await page.evaluate(() => (window as any).__worksAgentsFixture.usage());
    await expect(works(page).locator('[data-metric="today-input"]')).toHaveText("0");
    await expect(works(page).locator('[data-metric="fleet-now"]')).toHaveText("0");
    await expect(works(page).locator(".works-fleet svg")).toHaveAttribute("data-known-count", "30");
    await expect(workRow(page, "agent-one").locator('[data-stat="context"]')).toHaveText("0%");
    await page.screenshot({ path: info.outputPath("works-measured-zero.png") });
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
    await roster(page).locator('.roster-capacity [data-action="add"]').click();
    await expect(roster(page).locator(".roster-count")).toHaveText("4");
    await expect(roster(page).locator(".roster-capacity-value")).toHaveText("4");
    await roster(page).locator('.roster-row[data-member-id="m-added"]').click();
    await roster(page).locator('.roster-detail [data-action="rest"]').click();
    await expect(roster(page).locator(".roster-count")).toHaveText("3");
    await expect(roster(page).locator(".roster-capacity-value")).toHaveText("3");
    const mutations = (await sent(page)).filter(message => message.kind === "agent_roster" && message.action.kind !== "read");
    expect(mutations.map(message => [message.action, message.expected_revision])).toEqual([[{ kind: "add" }, 3], [{ kind: "rest", member_id: "m-added" }, 4]]);
    expect(await page.evaluate(() => (window as any).__worksAgentsFixture.rejectedRevisions)).toEqual([]);
    expect(await page.evaluate(() => (window as any).__worksAgentsFixture.members.find(member => member.agent_member_id === "m-added").enabled)).toBe(false);

    await roster(page).locator('.roster-row[data-member-id="m-one"]').click();
    await roster(page).locator(".roster-now button").evaluate(button => {
      (window as any).__worksAgentsFixture.retire("agent-one", true);
      (button as HTMLButtonElement).click();
    });
    await expect(main(page)).toHaveAttribute("data-surface", "agents");
    await expect(roster(page).locator(".roster-now, .roster-row[data-member-id='m-one'] .roster-current")).toHaveCount(0);
    await page.evaluate(() => (window as any).__worksAgentsFixture.error("Roster readback unavailable"));
    await expect(roster(page).locator(".roster-error")).toHaveText("Roster readback unavailable");
    await expect(roster(page).locator(".roster-count")).toHaveText("3");

    await page.setViewportSize({ width: 400, height: 1000 });
    await expectNoHorizontalOverflow(page, ".agents-surface:not([hidden]):not(.agents-surface--preview)");
    await page.screenshot({ path: info.outputPath("agents-approved-400.png") });
    await roster(page).getByRole("tab", { name: "Past", exact: true }).click();
    await expectNoHorizontalOverflow(page, ".agents-surface:not([hidden]):not(.agents-surface--preview)");
    await openRail(page, "works");
    await expect(works(page).locator(".work-row")).toHaveCount(2);
    await expectNoHorizontalOverflow(page, ".works-surface:not([hidden]):not(.works-surface--preview)");
    await page.screenshot({ path: info.outputPath("works-approved-400.png") });
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
    const members: any[] = [
      { agent_member_id: "m-one", name: "Aster", enabled: true, created_at: "2026-10-20T00:00:00Z", last_issue_number: 4777, last_spec_number: null, assignment: assignment("s-one", 4777), provider: "Codex", model: "Fixture model", issue_count: 3, landed_count: 2, active_seconds: 7200, tokens: 5000, history: [
        { session_id: "s-earlier", issue_number: 4000, title: "Earlier slice", result: "landed", started_at: "2026-10-20T00:00:00Z", ended_at: "2026-10-20T02:00:00Z" },
        { session_id: "s-later", issue_number: 4600, title: "Later slice", result: null, started_at: "2026-10-22T00:00:00Z", ended_at: "2026-10-22T02:00:00Z" },
      ] },
      { agent_member_id: "m-two", name: "Birch", enabled: true, created_at: "2026-10-20T00:00:00Z", last_issue_number: 4001, last_spec_number: null, assignment: assignment("s-two", 4001), provider: "Claude", model: "Fixture model", issue_count: null, landed_count: null, active_seconds: null, tokens: null, history: [] },
      { agent_member_id: "m-idle", name: "Cedar", enabled: true, created_at: "2026-10-20T00:00:00Z", last_issue_number: null, last_spec_number: null, assignment: null, provider: null, model: null, issue_count: null, landed_count: null, active_seconds: null, tokens: null, history: [] },
    ];
    let revision = 3, activityRevision = 0;
    let agents: any[] = [
      { window_id: "agent-two", session_id: "s-two", runtime_incarnation: 2, issue_number: 4001, is_pm: false },
      { window_id: "agent-one", session_id: "s-one", runtime_incarnation: 1, issue_number: 4777, is_pm: false },
      { window_id: "pm-window", session_id: "s-pm", runtime_incarnation: 3, issue_number: null, is_pm: true },
    ];
    const sentMessages: any[] = [], rejectedRevisions: any[] = [];
    const rejectedScheduleRevisions: any[] = [];
    let queueRevision = "queue-v1:fixture-1", scheduleRevision = 7, deferredSchedule = false;
    let pendingSchedule: any = null;
    const planned = [
      { agent_member_id: "m-one", issues: [{ issue_number: 5002, reason: "same_spec_continuation" }, { issue_number: 5003, reason: "queue_order" }] },
      { agent_member_id: "m-two", issues: [{ issue_number: 5004, reason: "queue_order" }] },
      { agent_member_id: "m-idle", issues: [] },
    ];
    let socket: FixtureWebSocket;
    const workspace = () => ({ kind: "workspace_state", workspace: { app_version: "playwright", active_tab_id: "tab-works", recent_projects: [], tabs: [{ id: "tab-works", title: "Works Agents Fixture", project_root: "/fixture", project_key: projectKey, kind: "git", workspace: { windows, viewport: { x: 0, y: 0, zoom: 1 } } }] } });
    const rosterSnapshot = () => ({ kind: "agent_roster", roster: { project_key: projectKey, revision, members, capacity: members.filter(member => member.enabled).length, recommended_capacity: 6, past_complete: true, past_contributors: [{ session_id: "s-earlier", name: "Earlier author", kind: "implementation", issue_numbers: [4000], first_posted_at: "2026-10-20T00:00:00Z", last_posted_at: "2026-10-20T02:00:00Z", post_count: 4 }], by_session: Object.fromEntries(agents.map(agent => [agent.session_id, { branch: `work/${agent.window_id}`, ahead_of_develop: 0, model: "Fixture model", provider: agent.is_pm || agent.session_id === "s-two" ? "Claude" : "Codex", last_action: "Fixture observation", elapsed_seconds: 120, warning_count: 0 }])), error: null } });
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
      constructor(public readonly url: string) {
        super(); socket = this;
        setTimeout(() => { this.readyState = FixtureWebSocket.OPEN; this.dispatchEvent(new Event("open")); this.emit(workspace()); this.emit(rosterSnapshot()); this.emit(activity()); }, 0);
      }
      send(raw: string) {
        const message = JSON.parse(raw); sentMessages.push(message);
        if (message.kind === "agent_roster") {
          if (message.action.kind !== "read" && message.expected_revision !== revision) { rejectedRevisions.push(message); this.emit(rosterSnapshot()); return; }
          if (message.action.kind === "add") { members.push({ agent_member_id: "m-added", name: "Dahlia", enabled: true, created_at: "2026-10-25T00:00:00Z", last_issue_number: null, last_spec_number: null, assignment: null, provider: null, model: null, history: [], issue_count: 0, landed_count: 0, active_seconds: 0, tokens: 0 }); revision++; }
          if (message.action.kind === "rest") { const member = members.find(member => member.agent_member_id === message.action.member_id); if (member) { member.enabled = false; revision++; } }
          this.emit(rosterSnapshot());
        }
        if (message.kind === "agent_schedule") {
          if (message.action.kind === "read") this.emit(scheduleSnapshot());
          else if (deferredSchedule) pendingSchedule = message.action;
          else applySchedule(message.action);
        }
      }
      emit(payload: unknown) { const serialized = JSON.stringify(payload); setTimeout(() => this.dispatchEvent(new MessageEvent("message", { data: serialized })), 0); }
      close() { this.readyState = FixtureWebSocket.CLOSED; this.dispatchEvent(new CloseEvent("close")); }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
    (window as any).__worksAgentsFixture = {
      sent: sentMessages, members, rejectedRevisions, rejectedScheduleRevisions,
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
      usage() { socket.emit({ kind: "provider_usage", accounts: [], consumption: [], sessions: agents.map(agent => ({ session_id: agent.session_id, provider_session_id: `provider-${agent.session_id}`, provider: agent.is_pm || agent.session_id === "s-two" ? "claude_code" : "codex", project_path: "/fixture", model: "Fixture model", active: true, input_tokens: 0, cache_read_tokens: 0, cache_creation_tokens: 0, output_tokens: 0, total_tokens: 0, reported_context_left_pct: 0, context_used_tokens: null, context_limit_tokens: null, context_left_pct: null, limit_reached: false, eligible: true, state: { kind: "ok" }, telemetry: { source: "fixture-reported", quality: "measured", observed_at: "2026-10-25T00:00:00Z", today_tokens: 0, today_input_tokens: 0, today_cache_read_tokens: 0, today_cache_creation_tokens: 0, today_output_tokens: 0, minute_buckets: Array.from({ length: 30 }, (_, index) => ({ started_at: new Date(1792884600000 + index * 60000).toISOString(), tokens: 0 })) } })) }); },
    };
  }, { projectKey: APP_PROJECT_KEY });
}
