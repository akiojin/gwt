import { expect, test, type Page, type Locator } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { join } from "node:path";

// #4499: real Chromium and checkout assets, deterministic protocol projections.
// This suite proves the queue UI contract, not a real agent launch.
const configuredUrl = process.env.GWT_PLAYWRIGHT_BASE_URL;
// A browser-check URL may point at the Hub; these fixtures exercise Project assets.
const liveUrl = configuredUrl
  ? new URL(new URL(configuredUrl).pathname === "/" ? new URL(APP_URL).pathname : new URL(configuredUrl).pathname, configuredUrl).toString()
  : undefined;
const errors = new WeakMap<Page, string[]>();
test.use({ viewport: { width: 1600, height: 1100 } });
test.beforeEach(async ({ page }, info) => {
  const captured: string[] = [];
  errors.set(page, captured);
  page.on("pageerror", error => captured.push(error.message));
  page.on("console", message => { if (message.type() === "error") captured.push(message.text()); });
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme),
    info.project.name.includes("light") ? "light" : "dark");
  const narrowSettings = info.title.includes("Settings integrates");
  if (narrowSettings) await page.setViewportSize({ width: 1280, height: 1000 });
  if (!liveUrl) await installEmbeddedRoutes(page);
  await installBackend(page, {
    narrowSettings,
    inboxFirst: info.title.startsWith("Issue #5287"),
    includeAgent: info.title.startsWith("T-7a:") || info.title.startsWith("T-7b.2b.2:"),
    observeSkipped: info.title.startsWith("four columns preserve"),
  });
  await page.goto(liveUrl || APP_URL);
  await expect(page.locator(".issue-queue-board")).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", info.project.name.includes("light") ? "light" : "dark");
});
test.afterEach(async ({ page }, info) => {
  const screenshotPath = info.outputPath(`queue-board-${info.project.name}.png`);
  await page.screenshot({path:screenshotPath});
  await info.attach(`queue-board-${info.project.name}`, {path:screenshotPath, contentType:"image/png"});
  expect(errors.get(page), "zero console/page errors").toEqual([]);
});
const column = (page: Page, phase: string) => page.locator(`[data-queue-column="${phase}"]`);
const row = (page: Page, number: number) => page.locator(`.knowledge-row[data-issue-number="${number}"]`);
const messages = (page: Page) => page.evaluate(() => (window as any).__queueMessages.filter((message: any) =>
  ["issue_monitor_queue_push", "issue_monitor_queue_remove", "issue_monitor_queue_move", "set_issue_monitor_auto_refill"].includes(message.kind)));
async function confirm(page: Page, numbers: number[], extra = {}) {
  await page.evaluate(({numbers, extra}) => (window as any).__queueConfirm(numbers, extra), {numbers, extra});
}
async function drag(page: Page, number: number, target: Locator) {
  await row(page, number).dragTo(target);
}
async function titleMetrics(title: Locator) {
  return title.evaluate(element => {
    const oneLine = element.cloneNode(false) as HTMLElement;
    oneLine.textContent = "M";
    Object.assign(oneLine.style, { position: "absolute", visibility: "hidden", width: `${element.clientWidth}px`, height: "auto" });
    element.parentElement!.appendChild(oneLine);
    const lineHeight = oneLine.getBoundingClientRect().height;
    oneLine.remove();
    return { height: element.getBoundingClientRect().height, lineHeight,
      fits: element.scrollHeight <= element.clientHeight && element.scrollWidth <= element.clientWidth };
  });
}

test("Issue #5400 unchanged entry revisions retain the list and current detail", async ({ page }) => {
  await page.evaluate(() => (window as any).__queueEntries("entries-v1"));
  await row(page, 2).locator(".knowledge-row-select").click();
  await expect(page.locator(".knowledge-detail-title")).toHaveText("Second backlog issue");
  await expect(page.locator(".knowledge-detail-pane")).toContainText("Description for #2");
  await row(page, 1).evaluate(element => { (window as any).__revisionRow = element; });

  await page.evaluate(() => (window as any).__queueEntries("entries-v1"));
  expect(await row(page, 1).evaluate(element => element === (window as any).__revisionRow),
    "same revision preserves the exact list node").toBe(true);
  await expect(row(page, 2)).toHaveClass(/selected/);
  await expect(page.locator(".knowledge-detail-title")).toHaveText("Second backlog issue");
  await expect(page.locator("[data-action='refresh-knowledge']")).toBeEnabled();

  await page.evaluate(() => (window as any).__queueEntries("entries-v2", { number: 1, title: "Changed first issue" }));
  await expect(row(page, 1)).toContainText("Changed first issue");
  expect(await row(page, 1).evaluate(element => element === (window as any).__revisionRow),
    "changed revision updates the list").toBe(false);
  const search = page.locator(".knowledge-search");
  await search.fill("Second backlog");
  await expect(row(page, 1)).toHaveCount(0);
  await expect(row(page, 2)).toBeVisible();
  await row(page, 2).evaluate(element => { (window as any).__searchRevisionRow = element; });
  await page.evaluate(() => (window as any).__queueEntries("entries-v2"));
  expect(await row(page, 2).evaluate(element => element === (window as any).__searchRevisionRow),
    "same revision preserves the current search result node").toBe(true);
  await expect(row(page, 1)).toHaveCount(0);
  await expect(page.locator(".knowledge-detail-title")).toHaveText("Second backlog issue");
  await search.fill("");
  await expect(row(page, 1)).toContainText("Changed first issue");
  await expect(page.getByText("Issue load failed", { exact: true })).toHaveCount(0);
});

test("Issue #5401 replays retain focus and only changed Issue keys replace rows", async ({ page }, info) => {
  const select = row(page, 2).locator(".knowledge-row-select");
  await select.focus();
  await page.evaluate(() => {
    (window as any).__keptRow = document.querySelector('[data-issue-number="2"].knowledge-row');
    (window as any).__rowAdds = 0;
    const observer = new MutationObserver(records => {
      for (const record of records) for (const node of record.addedNodes) {
        if (node instanceof Element) (window as any).__rowAdds += Number(node.matches(".knowledge-row")) + node.querySelectorAll(".knowledge-row").length;
      }
    });
    observer.observe(document.querySelector(".knowledge-list")!, { childList: true, subtree: true });
    (window as any).__queueEntries(undefined);
  });
  await expect(select).toBeFocused();
  expect(await page.evaluate(() => (window as any).__rowAdds)).toBe(0);
  await page.evaluate(() => (window as any).__queueEntries("new-revision-same-content"));
  await expect(select).toBeFocused();
  expect(await page.evaluate(() => (window as any).__rowAdds)).toBe(0);
  await page.evaluate(() => (window as any).__queueTitle(1, "Updated key one"));
  await expect(row(page, 1)).toContainText("Updated key one");
  expect(await row(page, 2).evaluate(node => node === (window as any).__keptRow)).toBe(true);
  await expect(select).toBeFocused();
  expect(await page.evaluate(() => (window as any).__rowAdds)).toBe(1);
  await info.attach("keyed-row-counts", { body: JSON.stringify({ identical_replay_created: 0, one_changed_key_created: 1 }), contentType: "application/json" });
  await row(page, 1).locator(".knowledge-row-select").focus();
  await page.evaluate(() => (window as any).__queueTitle(1, "Updated focused key"));
  await expect(row(page, 1)).toContainText("Updated focused key");
  await expect(row(page, 1).locator(".knowledge-row-select")).toBeFocused();
});

test("Issue #5401 large collections retain scroll, selection and native keyboard across virtual rows", async ({ page }, info) => {
  await page.evaluate(() => (window as any).__queueSetEntries(Array.from({ length: 1545 }, (_, index) => ({
    number: index + 1000, title: `Virtual issue ${index + 1000}`, state: "open", labels: [], is_spec: false,
  }))));
  const pane = page.locator(".knowledge-list-pane");
  const mounted = () => page.locator(".knowledge-row").count();
  await expect(column(page, "backlog").locator("h3")).toHaveText("Backlog · 1545");
  expect(await mounted()).toBeLessThan(200);
  await pane.evaluate(node => { node.scrollTop = 10000; });
  await expect.poll(() => row(page, 1000).count()).toBe(0);
  const position = await pane.evaluate(node => node.scrollTop);
  const visible = column(page, "backlog").locator(".knowledge-row:visible").first();
  const number = Number(await visible.getAttribute("data-issue-number"));
  await page.evaluate(() => (window as any).__queueEntries(undefined));
  expect(await pane.evaluate(node => node.scrollTop)).toBe(position);
  expect(await mounted()).toBeLessThan(200);
  const select = row(page, number).locator(".knowledge-row-select");
  await expect(select).toHaveAttribute("aria-description", /open on GitHub/);
  await page.mouse.move(1590, 1090);
  await select.focus();
  await select.press("Enter");
  const detail = page.locator(".knowledge-detail-pane");
  await expect(detail).toBeFocused();
  await detail.press("Escape");
  await expect(row(page, number).locator(".knowledge-row-select")).toBeFocused();
  await pane.evaluate(node => { node.scrollTop = node.scrollHeight; });
  await expect(row(page, 2544)).toBeAttached();
  await pane.evaluate(node => { node.scrollTop = 0; });
  await expect(row(page, 1000)).toBeAttached();
  const boundary = column(page, "backlog").locator(".knowledge-row").last();
  const boundaryNumber = Number(await boundary.getAttribute("data-issue-number"));
  const lastControl = row(page, boundaryNumber).locator("button:visible, input:visible, summary:visible").last();
  await lastControl.focus();
  await lastControl.press("Tab");
  await expect(row(page, boundaryNumber + 1).locator('[data-action="queue-select"]')).toBeFocused();
  await info.attach("virtual-row-counts", { body: JSON.stringify({ total: 1545, mounted: await mounted(), limit: 200 }), contentType: "application/json" });
});

test("Issue #5401 preserves focused controls across column moves and identical detail replays", async ({ page }) => {
  const selection = row(page, 1).locator('[data-action="queue-select"]');
  await selection.focus();
  await confirm(page, [1, 3, 4]);
  await expect(column(page, "queued").locator('[data-issue-number="1"]')).toBeVisible();
  await expect(selection).toBeFocused();
  await row(page, 2).locator(".knowledge-row-select").click();
  const detail = page.locator(".knowledge-detail-pane");
  await expect(detail).toContainText("Description for #2");
  const detailTab = detail.getByRole("button", { name: "Issue", exact: true });
  await detailTab.focus();
  await page.evaluate(() => (window as any).__queueEntries(undefined));
  await expect(detailTab).toBeFocused();
});

test("Issue #5321 responsive summary and on-demand preview", async ({ page }, info) => {
  const before = process.env.GWT_ISSUE_5321_PHASE === "before";
  const phase = before ? "before" : "after";
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const root = page.locator(".issue-bridge-root");
  const pane = root.locator(".knowledge-list-pane");
  const detail = root.locator(".knowledge-detail-pane");
  const capture = async (name: string) => {
    const path = process.env.GWT_ISSUE_5321_EVIDENCE
      ? join(process.env.GWT_ISSUE_5321_EVIDENCE, `${phase}-${theme}-${name}.png`)
      : info.outputPath(`${phase}-${theme}-${name}.png`);
    await page.screenshot({ path });
    await info.attach(`${phase}-${theme}-${name}`, { path, contentType: "image/png" });
  };
  for (const width of [900, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    await page.locator(".workspace-window").evaluate((element, width) => {
      // The fixture's initial wide window auto-fits the canvas. Use physical
      // CSS pixels for the responsive acceptance instead of camera scaling.
      document.getElementById("canvas-stage")!.style.transform = "none";
      Object.assign((element as HTMLElement).style, { left: "8px", top: "8px", width: `${width - 80}px`, height: "860px" });
    }, width);
    await capture(`${width}-default`);
    if (before) continue;
    await expect(root).toHaveAttribute("data-preview-hidden", "true");
    await expect(detail).not.toBeVisible();
    const paneBox = (await pane.boundingBox())!;
    const shellBox = (await root.locator(".issue-list-shell").boundingBox())!;
    expect(Math.abs(paneBox.width - shellBox.width), "default Kanban uses the full list width").toBeLessThan(2);
    const statusBox = (await root.getByRole("group", { name: "Monitor status", exact: true }).boundingBox())!;
    const controlsBox = (await root.getByRole("group", { name: "Monitor controls", exact: true }).boundingBox())!;
    expect(Math.abs(statusBox.y + statusBox.height / 2 - controlsBox.y - controlsBox.height / 2), "summary status and controls share one row").toBeLessThan(2);
    expect(controlsBox.x - statusBox.x - statusBox.width, "summary has no large central gap").toBeLessThan(40);
    expect((await root.locator(".knowledge-monitor-bar").boundingBox())!.height, "summary stays compact").toBeLessThan(60);
    const title = "First backlog issue with a long descriptive title that preserves every word in the preview while each Kanban card stays compact and readable across both viewport sizes";
    await page.evaluate(title => (window as any).__queueTitle(1, title), title);
    const cardTitle = row(page, 1).locator(".knowledge-row-title");
    await expect(cardTitle).toHaveText(title);
    const cardMetrics = await titleMetrics(cardTitle);
    expect(cardMetrics.height, "long card title is at most two lines").toBeLessThanOrEqual(cardMetrics.lineHeight * 2 + 1);
    await row(page, 1).locator(".knowledge-row-select").click();
    await expect(root).toHaveAttribute("data-preview-hidden", "false");
    await expect(detail).toBeVisible();
    const previewTitle = detail.locator(".knowledge-detail-title");
    await expect(previewTitle).toHaveText(title);
    await expect(previewTitle).toBeVisible();
    const previewMetrics = await titleMetrics(previewTitle);
    expect(previewMetrics.fits, "preview shows the complete title without clipping").toBe(true);
    expect(previewMetrics.height, "preview title wraps naturally").toBeGreaterThan(previewMetrics.lineHeight * 2);
    if (width === 900) {
      const openPaneBox = (await pane.boundingBox())!;
      expect(Math.abs(openPaneBox.width - paneBox.width), "narrow preview overlays the full width board").toBeLessThan(2);
    } else {
      expect((await pane.boundingBox())!.width, "wide preview shares the list shell").toBeLessThan(paneBox.width - 100);
    }
    await capture(`${width}-preview`);
    await detail.locator('[data-action="close-issue-preview"]').click();
    await expect(root).toHaveAttribute("data-preview-hidden", "true");
    expect(Math.abs((await pane.boundingBox())!.width - shellBox.width), "closing preview restores full width").toBeLessThan(2);
    await page.evaluate(() => (window as any).__queueTitle(1, "First backlog issue"));
  }
  if (before) return;
  const statusPill = root.locator(".knowledge-monitor-pill");
  for (const [extra, label, tone] of [
    [{ enabled: false, state: "disabled", active_count: 0 }, "Stopped", "idle"],
    [{ enabled: true, state: "idle", active_count: 0, queue_len: 0 }, "Waiting · Empty", "idle"],
    [{ enabled: true, state: "active", active_count: 1, max_active_agents: 2, queue_len: 2 }, "Running", "active"],
    [{ enabled: true, state: "active", active_count: 2, max_active_agents: 2, queue_len: 2 }, "Waiting · Full", "idle"],
    [{ enabled: true, state: "error", active_count: 0, last_error: "Fixture current failure" }, "⚠ Error", "blocked"],
  ] as const) {
    await page.evaluate(extra => (window as any).__queueStatus({ last_error: null, quota_hold: null, ...extra }), extra);
    await expect(statusPill).toHaveText(label);
    await expect(statusPill).toHaveAttribute("data-tone", tone);
  }
  await statusPill.click();
  await expect(page.locator('[data-error-key="issue-monitor:last_error"]')).toContainText("Fixture current failure");
  await page.getByRole("button", { name: "Close notifications", exact: true }).click();
  await page.evaluate(() => (window as any).__queueStatus({ state: "idle", last_error: null, active_count: 0, queue_len: 0 }));
  await expect(statusPill).toHaveText("Waiting · Empty");
  await expect(statusPill).toHaveAttribute("data-tone", "idle");
  await page.setViewportSize({ width: 900, height: 1000 });
  await page.locator(".workspace-window").evaluate(element => { (element as HTMLElement).style.width = "820px"; });
  await root.getByRole("button", { name: "Split", exact: true }).click();
  await expect(root).toHaveAttribute("data-view-mode", "split");
  const splitList = (await pane.boundingBox())!;
  const splitDetail = (await detail.boundingBox())!;
  expect(splitList.x + splitList.width, "narrow Split keeps its interactive list beside the detail pane").toBeLessThanOrEqual(splitDetail.x + 1);
});

test("Issue #5396 lazy preview retains DOM across unchanged list and detail replies", async ({ page }, info) => {
  const detail = page.locator(".knowledge-detail-pane");
  expect(await page.evaluate(() => (window as any).__queueMessages.filter((message: any) =>
    message.kind === "select_knowledge_bridge_entry").length)).toBe(0);
  await row(page, 1).locator(".knowledge-row-select").click();
  await expect(detail.getByText("Description for #1", { exact: true })).toBeVisible();
  const metrics = await page.evaluate(async () => {
    const fixture = window as any;
    const pane = document.querySelector(".knowledge-detail-pane")!;
    const title = pane.querySelector(".knowledge-detail-title")!;
    const disclosure = pane.querySelector("details")!;
    disclosure.open = false;
    let replacements = 0;
    const observer = new MutationObserver(() => { if (!title.isConnected) replacements++; });
    observer.observe(pane, { childList: true, subtree: true });
    const before = fixture.__queueMessages.length;
    const started = performance.now();
    for (let index = 0; index < 20; index++) {
      fixture.__queueTitle(1, title.textContent);
      fixture.__queueReplayDetail();
    }
    await new Promise(resolve => setTimeout(resolve, 500));
    observer.disconnect();
    return { replies: 40, replacements, retained: title.isConnected,
      disclosureOpen: disclosure.open, elapsedMs: performance.now() - started,
      extraSelections: fixture.__queueMessages.slice(before).filter((message: any) =>
        message.kind === "select_knowledge_bridge_entry").length };
  });
  await info.attach("preview-refresh-metrics", { body: JSON.stringify(metrics), contentType: "application/json" });
  expect(metrics.retained).toBe(true);
  expect(metrics.replacements).toBe(0);
  expect(metrics.disclosureOpen).toBe(false);
  expect(metrics.extraSelections).toBe(0);
  await page.evaluate(() => (window as any).__queueRefreshDetail());
  await expect(detail.locator(".knowledge-detail-title")).toHaveText("Refreshed issue");
});

test("Issue #5321 Settings integrates Issues at 1280px without duplicate surfaces", async ({ page }, info) => {
  test.skip(process.env.GWT_ISSUE_5321_PHASE === "before", "AC-M covers the approved Settings integration");
  const settings = page.locator('.workspace-window[data-preset="settings"]');
  await expect(settings).toBeInViewport({ ratio: 1 });
  const issues = settings.locator('[data-settings-panel="issues"]');
  const root = issues.locator(".issue-bridge-root");
  const options = issues.locator(".settings-issues-options");
  const controls = options.locator("[data-settings-issues-controls]");
  const candidates = options.locator("[data-settings-candidates]");
  const capture = async (name: string) => {
    const path = process.env.GWT_ISSUE_5321_EVIDENCE
      ? join(process.env.GWT_ISSUE_5321_EVIDENCE, `settings-1280-${info.project.name}-${name}.png`)
      : info.outputPath(`settings-1280-${name}.png`);
    await page.screenshot({ path });
    await info.attach(`settings-1280-${name}`, { path, contentType: "image/png" });
  };
  const assertSingleSurface = async () => {
    await expect(settings).toHaveCount(1);
    await expect(page.locator('.workspace-window[data-preset="issue"], .workspace-window[data-preset="issue_monitor"]')).toHaveCount(0);
    await expect(page.locator(".issue-bridge-root")).toHaveCount(1);
  };
  await assertSingleSurface();
  const issuesRail = page.locator('.op-rail [data-surface="issues"]');
  await expect(issuesRail).toHaveAttribute("aria-pressed", "true");
  await settings.locator('[data-settings-tab="system"]').click();
  await expect(page.locator('.op-rail [data-surface="settings"]')).toHaveAttribute("aria-pressed", "true");
  await settings.locator('[data-settings-tab="issues"]').click();
  await expect(issuesRail).toHaveAttribute("aria-pressed", "true");
  const navigation = settings.getByRole("tablist", { name: "Settings sections" });
  await expect(navigation).toBeVisible();
  await expect(navigation).toHaveAttribute("aria-orientation", "vertical");
  await expect(settings.locator(".settings-tabs")).toContainText("Application settings");
  await expect(settings.locator(".settings-tabs")).toContainText("Project settings");
  await expect(settings.locator('[data-settings-tab="issues"]')).toHaveAttribute("aria-selected", "true");
  await expect(settings.locator('[data-settings-tab="agents"]')).toHaveCount(0);
  const navBox = (await settings.locator(".settings-tabs").boundingBox())!;
  const issueBox = (await issues.boundingBox())!;
  expect(navBox.width, "grouped navigation retains usable width").toBeGreaterThan(100);
  expect(navBox.x + navBox.width, "grouped navigation sits beside Issues").toBeLessThanOrEqual(issueBox.x + 1);
  await expect(options).not.toHaveAttribute("open", "");
  await expect(controls).toBeHidden();
  await expect(candidates).toBeHidden();
  for (const action of ["monitor-autonomous", "monitor-toggle"]) {
    await expect(root.locator(`[data-action="${action}"]`)).toBeVisible();
    await expect(options.locator(`[data-action="${action}"]`)).toHaveCount(0);
  }
  await expect(root.locator(".knowledge-search")).toBeVisible();
  await expect(root.locator('[data-action="issue-new"]')).toBeVisible();
  await expect(root.locator('[data-action="refresh-knowledge"]')).toBeVisible();
  await expect(root.locator("[data-queue-column]")).toHaveCount(4);
  await expect(root.locator(".knowledge-detail-pane")).toBeHidden();
  await capture("default");
  await row(page, 1).locator(".knowledge-row-select").click();
  await expect(root.locator(".knowledge-detail-pane")).toBeVisible();
  await expect(root.locator(".knowledge-detail-title")).toHaveText("First backlog issue");
  await capture("preview");
  await root.locator('[data-action="close-issue-preview"]').click();
  await expect(root).toHaveAttribute("data-preview-hidden", "true");
  await expect(root.locator(".knowledge-detail-pane")).toBeHidden();
  await root.locator('[data-action="monitor-settings"]').click();
  await expect(options).toHaveAttribute("open", "");
  await expect(controls).toBeVisible();
  await expect(candidates).toBeVisible();
  const mode = controls.getByLabel("Max active mode", { exact: true });
  const max = controls.getByLabel("Max active", { exact: true });
  await expect(mode).toHaveValue("auto");
  await expect(max).toBeDisabled();
  await mode.selectOption("manual");
  await expect(max).toBeEnabled();
  await max.fill("3");
  await max.press("Tab");
  await expect(max).toHaveValue("3");
  // Wait for the saved Manual status before requesting Auto; the input's
  // optimistic value alone does not acknowledge the fixture's async reply.
  await expect(root.locator('[data-metric="active"]')).toHaveText("Active 1/3");
  await mode.selectOption("auto");
  await expect(max).toBeDisabled();
  await expect(settings.getByLabel("Auto-apply updates", { exact: true })).toHaveCount(1);
  await expect(settings.locator('[data-settings-panel="system"]').getByLabel("Auto-apply updates", { exact: true })).toHaveCount(1);
  await expect(settings.locator('[data-settings-panel="issues"]').getByLabel("Auto-apply updates", { exact: true })).toHaveCount(0);
  await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "agents" } })));
  await expect(settings.locator('[data-settings-tab="issues"]')).toHaveAttribute("aria-selected", "true");
  await expect(candidates).toBeVisible();
  await expect(candidates).toBeFocused();
  await assertSingleSurface();
  await capture("options");
  await root.locator('[data-action="monitor-settings"]').click();
  await expect(options).not.toHaveAttribute("open", "");
  await expect(root.locator(".issue-queue-board")).toBeVisible();
  await page.locator('.op-rail [data-surface="agents"]').click();
  await expect(page.locator('.op-rail [data-surface="agents"]')).toHaveAttribute("aria-pressed", "true");
  await issuesRail.click();
  await expect(root.locator(".issue-queue-board")).toBeVisible();
  await expect(issuesRail).toHaveAttribute("aria-pressed", "true");
  await settings.locator(".titlebar").click();
  await expect(issuesRail).toHaveAttribute("aria-pressed", "true");
  await assertSingleSurface();
  await page.getByRole("button", { name: "Split view", exact: true }).click();
  const left = page.getByRole("region", { name: "Left pane", exact: true });
  await left.locator('[data-settings-tab="system"]').click();
  await expect(left.locator('[data-settings-panel="issues"]')).toBeHidden();
  await confirm(page, [3, 4]);
  await expect(left.locator('[data-settings-tab="system"]')).toHaveAttribute("aria-selected", "true");
  await issuesRail.click();
  await expect(left.locator('[data-settings-tab="issues"]')).toHaveAttribute("aria-selected", "true");
  await expect(left.locator(".issue-queue-board")).toBeVisible();
  await page.getByRole("button", { name: "Close split", exact: true }).click();
});

test("Issue #5321 preview keyboard focus opens and returns with Escape", async ({ page }) => {
  const select = row(page, 1).locator(".knowledge-row-select");
  const detail = page.locator(".knowledge-detail-pane");
  // Keep native hover from opening a tooltip that consumes the first Escape.
  await page.mouse.move(1590, 1090);
  await select.focus();
  await select.press("Enter");
  await expect(detail).toBeFocused();
  await detail.press("Escape");
  await expect(detail).not.toBeVisible();
  await expect(select).toBeFocused();
});

test("pending SessionStart diagnosis clears when late readiness arrives", async ({page}) => {
  const diagnosis = "SessionStart readiness pending: hooks.json missing; check hook discovery and works.lock contention";
  await page.evaluate(diagnosis => (window as any).__queueInbox([
    {issue:{number:5},state:"launching",error_message:diagnosis}]), diagnosis);
  await expect(row(page,5)).toContainText(diagnosis);
  await expect(column(page,"active").locator('[data-issue-number="5"]')).toBeVisible();
  const pendingScreenshot = test.info().outputPath("readiness-pending.png");
  await page.screenshot({path:pendingScreenshot});
  await test.info().attach("readiness-pending", {path:pendingScreenshot,contentType:"image/png"});
  await page.evaluate(() => (window as any).__queueInbox([{issue:{number:5},state:"launched"}]));
  await expect(row(page,5)).not.toContainText("SessionStart readiness pending:");
  await expect(column(page,"active").locator('[data-issue-number="5"]')).toBeVisible();
});

test("Issue #5415 failed initial startup requeues once and releases its Active slot", async ({ page }) => {
  // Runtime/Rust tests prove the timeout and exact authority cleanup. This
  // protocol projection proves the resulting Monitor recovery is visible.
  const diagnosis = "Issue Monitor startup timed out after 180s: initial prompt was not submitted; last output: Could not read Windows sandbox configuration and requirements from the app server.; gwt retries this launch automatically without spending an attempt";
  await confirm(page, [3, 4], { enabled: true, state: "active", active_count: 1 });
  await page.evaluate(() => (window as any).__queueInbox([{ issue: { number: 5 }, state: "launched" }]));
  await expect(column(page, "active").locator('[data-issue-number="5"]')).toBeVisible();
  await expect(page.locator('[data-metric="active"]')).toHaveText("Active 1/1");
  await confirm(page, [5, 3, 4], { state: "active", active_count: 0, last_error: null });
  await page.evaluate(diagnosis => (window as any).__queueInbox([
    { issue: { number: 5 }, state: "queued", error_message: diagnosis },
  ]), diagnosis);
  await expect(page.locator('[data-metric="active"]')).toHaveText("Active 0/1");
  await expect(column(page, "queued").locator('[data-issue-number="5"]')).toBeVisible();
  await expect(column(page, "active").locator('[data-issue-number="5"]')).toHaveCount(0);
  await expect(row(page, 5)).toHaveCount(1);
  await page.evaluate(message => (window as any).__queueEvent({
    kind: "issue_monitor_toast", level: "error", issue_number: 5, message,
  }), diagnosis);
  await page.locator("#op-notifications-button").click();
  await expect(page.locator("#notification-center")).toContainText(diagnosis);
});

test("Issue #5287 inbox-first restart, GUI error, load failure and renderer recovery", async ({ page }) => {
  const assertInbox = async () => {
    await expect(column(page, "backlog").locator(".knowledge-row")).toHaveCount(2);
    await expect(column(page, "queued").locator(".knowledge-row")).toHaveCount(2);
    await expect(column(page, "active").locator(".knowledge-row")).toHaveCount(1);
    await expect(row(page, 5)).toContainText("Running issue");
  };
  await assertInbox();
  await page.reload();
  await assertInbox();
  await page.evaluate(() => (window as any).__queueLoadError());
  const loadBadge = page.getByRole("button", { name: "Issue load failed · Details", exact: true }).first();
  const loadNotice = page.locator('[data-error-key="issue-window:tab-queue::issue-1:load"]');
  await expect(loadBadge).toBeVisible();
  await expect(page.locator(".knowledge-list")).not.toContainText("Issue cache read failed");
  // A headed browser retains its physical pointer across reload. Keep this
  // notification journey clear of an unrelated card-hover explanation.
  await page.mouse.move(1590, 1090);
  await expect(page.locator('.issue-control-explanation[role="tooltip"]')).not.toBeVisible();
  await loadBadge.click();
  await expect(loadNotice).toBeFocused();
  await expect(loadNotice).toContainText("Issue cache read failed");
  await page.getByRole("button", { name: "Close notifications", exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__queueMessages
    .filter((message: any) => message.kind === "report_knowledge_error").map((message: any) => message.stage))).toContain("load");
  await page.evaluate(() => (window as any).__queueKnowledgeComplete());
  await expect(loadBadge).toHaveCount(0);
  await expect(loadNotice).toHaveAttribute("data-resolved", "true");
  await expect(row(page, 3)).toContainText("Enriched queued issue");
  await expect(page.locator(".knowledge-row")).toHaveCount(6);
  await assertInbox();
  await page.evaluate(() => {
    const original = Element.prototype.insertBefore;
    Element.prototype.insertBefore = function <T extends Node>(child: T, before: Node | null): T {
      if (child instanceof Element && child.classList.contains("knowledge-row")) {
        Element.prototype.insertBefore = original;
        throw new Error("Issue card render failed");
      }
      return original.call(this, child, before) as T;
    };
    (window as any).__queueTitle(1, "Changed entry exercises keyed insertion");
  });
  const renderBadge = page.getByRole("button", { name: "Issue rendering failed · Details", exact: true });
  const renderNotice = page.locator('[data-error-key="issue-window:tab-queue::issue-1:render"]');
  await expect(renderBadge).toBeVisible();
  await expect(page.locator(".knowledge-list")).not.toContainText("Issue card render failed");
  await page.mouse.move(1590, 1090);
  await expect(page.locator('.issue-control-explanation[role="tooltip"]')).not.toBeVisible();
  await renderBadge.click();
  await expect(renderNotice).toBeFocused();
  await expect(renderNotice).toContainText("Issue card render failed");
  await page.getByRole("button", { name: "Close notifications", exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__queueMessages
    .filter((message: any) => message.kind === "report_knowledge_error").map((message: any) => message.stage))).toContain("render");
  await confirm(page, [3, 4]);
  await expect(renderBadge).toHaveCount(0);
  await expect(renderNotice).toHaveAttribute("data-resolved", "true");
  await assertInbox();
});

test("Issue #4862 a live delivered slice stays Active while its window binding is recovered", async ({ page }) => {
  await page.evaluate(() => (window as any).__queueInbox([{
    issue: {number: 5, title: "Running issue"}, state: "launched",
    completion_reason: "delivered_by_pr_5019", launched_window_id: null,
  }]));
  await expect(column(page, "active").locator('[data-issue-number="5"]')).toBeVisible();
  await expect(column(page, "backlog").locator('[data-issue-number="5"]')).toHaveCount(0);
  await expect(column(page, "queued").locator('[data-issue-number="5"]')).toHaveCount(0);
});

test("four columns preserve labelled controls, provenance, empty guidance and narrow scrolling", async ({ page }) => {
  await expect(page.locator("[data-queue-column]")).toHaveCount(4);
  await expect(column(page,"backlog")).toContainText("First backlog issue");
  await expect(row(page,1)).toContainText("Skipped");
  await expect(row(page,1)).toContainText("not selected in this terminal queue");
  await expect(row(page,1)).toBeVisible();
  await page.screenshot({path:test.info().outputPath("queue-observations.png")});
  await expect(column(page,"queued")).toContainText("operator");
  await expect(column(page,"queued")).toContainText("auto-refill");
  await expect(column(page,"active")).toContainText("Running issue");
  await expect(column(page,"done")).toContainText("Completed issue");
  await expect(page.locator('[data-issue-filter], [data-issue-lane-filter]')).toHaveCount(0);
  await expect(page.getByRole("button", {name:"Kanban",exact:true})).toBeVisible();
  await expect(page.getByRole("button", {name:"Split",exact:true})).toBeVisible();
  const searchBox=await page.locator(".knowledge-search").boundingBox();
  const modeBox=await page.locator(".knowledge-view-mode").boundingBox();
  expect(Math.abs(searchBox!.y-modeBox!.y), "search and view mode share one filter row").toBeLessThan(2);
  await expect(page.locator('[data-action="monitor-settings"]')).toHaveAttribute("aria-label", "Issues settings");
  await expect(page.locator('[data-action="refresh-knowledge"]')).toHaveText("↻ Refresh");
  await expect(page.getByRole("group",{name:"Monitor status",exact:true})).toBeVisible();
  await expect(page.getByRole("group",{name:"Monitor controls",exact:true})).toBeVisible();
  const pane=page.locator(".issue-bridge-root .knowledge-list-pane");
  await expect(page.locator(".issue-bridge-root")).toHaveAttribute("data-preview-hidden","true");
  const fullWidth=(await pane.boundingBox())!.width;
  await row(page,3).locator(".knowledge-row-select").click();
  const visibleWidth=(await pane.boundingBox())!.width;
  expect(fullWidth).toBeGreaterThan(visibleWidth+100);
  await page.locator('.knowledge-detail-pane [data-action="close-issue-preview"]').click();
  await expect(page.locator(".issue-bridge-root")).toHaveAttribute("data-preview-hidden","true");
  const hiddenWidth=(await pane.boundingBox())!.width;
  expect(hiddenWidth).toBeGreaterThan(visibleWidth+100);
  expect(Math.abs(hiddenWidth-(await page.locator(".issue-list-shell").boundingBox())!.width), "hidden preview gives the board full width").toBeLessThan(2);
  await expect(page.locator(".knowledge-detail-pane")).not.toBeVisible();
  await page.locator(".workspace-window").evaluate(element => { (element as HTMLElement).style.width="850px"; });
  for (const phase of ["backlog","queued","active","done"]) {
    expect((await column(page,phase).boundingBox())!.width).toBeGreaterThanOrEqual(262);
  }
  expect(await pane.evaluate(element=>element.scrollWidth>element.clientWidth)).toBe(true);
  await column(page,"done").scrollIntoViewIfNeeded();
  await expect(column(page,"done")).toBeVisible();
  await confirm(page,[]);
  await expect(column(page,"queued").locator(".knowledge-row")).toHaveCount(0);
  await column(page,"queued").scrollIntoViewIfNeeded();
  await expect(column(page,"queued")).toContainText("Nothing will launch until an issue is queued.");
});

test("T-7a: current labels explain themselves with keyboard focus and hover", async ({ page }) => {
  const popup = page.locator('.issue-control-explanation[role="tooltip"]');
  const controlMessages = () => page.evaluate(() => (window as any).__queueMessages.filter((message: any) =>
    message.kind.startsWith("set_issue_monitor_") || message.kind.startsWith("issue_monitor_queue_")
      || message.kind === "update_viewport"));
  const before = await controlMessages();
  const queueMetric = page.locator('[data-metric="queue"]');
  await queueMetric.focus();
  await page.keyboard.press("Tab");
  const autonomous = page.getByRole("switch", { name: "Autonomous mode" });
  await expect(autonomous).toBeFocused();
  await expect(popup).toBeVisible();
  await expect(popup).toHaveText("Allow eligible issues to run without waiting for human approval.");
  await expect(autonomous).toHaveAttribute("aria-describedby", await popup.getAttribute("id") as string);
  await autonomous.hover();
  // Capture can refresh native hover, so finish the keyboard checks first.
  await page.screenshot({ path: test.info().outputPath("issue-control-explanation.png") });
  await page.keyboard.press("Escape");
  await expect(popup).not.toBeVisible();
  await page.mouse.move(1590, 1090);
  await expect(popup).not.toBeVisible();
  await expect(autonomous).toBeFocused();

  for (const phase of ["backlog", "queued", "active", "done"]) {
    const heading = column(page, phase).locator(".issue-queue-heading");
    await heading.focus();
    await expect(heading).toBeFocused();
    await expect(popup).toBeVisible();
    await expect(popup).toHaveText(await heading.getAttribute("aria-description") as string);
  }
  const queued = column(page, "queued").locator(".issue-queue-heading");
  await queued.focus();
  const backlog = column(page, "backlog").locator(".issue-queue-heading");
  await backlog.hover();
  await expect(popup).toHaveText(await backlog.getAttribute("aria-description") as string);
  const hoverPopup = await popup.boundingBox();
  await page.mouse.move(hoverPopup!.x + 20, hoverPopup!.y + 10);
  await expect(popup).toHaveText(await backlog.getAttribute("aria-description") as string);
  await confirm(page, [3, 4, 1]);
  await expect(column(page, "queued").locator(".issue-queue-heading")).toHaveText("Queued · 3");
  await expect(column(page, "queued").locator(".issue-queue-heading")).toBeFocused();
  await expect(popup).toHaveText(await backlog.getAttribute("aria-description") as string);
  await page.mouse.move(1590, 1090);
  await expect(popup).toBeVisible();
  await page.keyboard.press("Escape");
  await confirm(page, [3, 4, 1, 2]);
  await expect(column(page, "queued").locator(".issue-queue-heading")).toHaveText("Queued · 4");
  await expect(column(page, "queued").locator(".issue-queue-heading")).toBeFocused();
  await expect(popup).not.toBeVisible();
  await expect(popup).toContainText("launch order");
  await page.locator(".knowledge-search").focus();
  await page.mouse.move(1590, 1090);
  await queued.hover();
  await expect(popup).toBeVisible();
  await expect(popup).toContainText("launch order");
  const box = await popup.boundingBox();
  expect(box).not.toBeNull();
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.y).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(1600);
  expect(box!.y + box!.height).toBeLessThanOrEqual(1100);
  await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
  await expect(popup).toBeVisible();
  await confirm(page, [3, 4]);
  await expect(queued).toHaveText("Queued · 2");
  await expect(popup).toBeVisible();
  await expect(queued).toHaveAttribute("aria-describedby", await popup.getAttribute("id") as string);
  await page.mouse.move(1590, 1090);
  await expect(popup).not.toBeVisible();
  await page.locator("#op-notifications-button").focus();
  await queued.hover();
  await expect(popup).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(popup).not.toBeVisible();
  // Leave the hover target before checking keyboard-only focus explanations.
  await page.mouse.move(1590, 1090);
  for (const selector of [
    '.knowledge-monitor-pill', '[data-metric="active"]', '[data-metric="queue"]',
    '[data-action="monitor-settings"]', '[data-action="monitor-toggle"]',
    '[data-action="monitor-autonomous"]',
  ]) {
    const target = page.locator(selector);
    await target.focus();
    await expect(target).toBeFocused();
    await expect(popup).toBeVisible();
    await expect(popup).toHaveText(await target.getAttribute("aria-description") as string);
  }
  const select = row(page, 4).locator(".knowledge-row-select");
  await expect(page.locator('.knowledge-monitor-pill')).toHaveCSS("cursor", "default");
  await select.focus();
  await expect(popup).toContainText("Auto-refill added this issue");
  expect(await controlMessages()).toEqual(before);
  await page.keyboard.press("Enter");
  // Wait for the fixture's authoritative detail, not the immediate local
  // selection preview, before testing dismissal across a later refresh.
  await expect(page.locator(".knowledge-detail-pane").getByText("Description for #4", {exact:true})).toBeVisible();
  const source = page.locator(".issue-detail-provenance");
  await source.focus();
  await expect(source).toBeFocused();
  await expect(popup).toBeVisible();
  await expect(source).toHaveAttribute("aria-describedby", await popup.getAttribute("id") as string);
  await expect(popup).toContainText("Auto-refill added this issue");
  await page.keyboard.press("Escape");
  await expect(popup).not.toBeVisible();
  await page.evaluate(() => (window as any).__queueRefreshDetail());
  await expect(page.locator(".issue-detail-title")).toHaveText("Refreshed issue");
  await expect(source).toBeFocused();
  await expect(popup).not.toBeVisible();
  await expect(popup).toContainText("Auto-refill added this issue");
  await page.locator(".knowledge-search").focus();
  await row(page, 4).locator(".knowledge-row-badge").hover();
  await expect(popup).toBeVisible();
  const cardBox = await row(page, 4).boundingBox();
  const cardPopup = await popup.boundingBox();
  await page.mouse.move(cardBox!.x + 20, cardBox!.y + cardBox!.height - 1);
  await expect(popup).toBeVisible();
  await page.mouse.move(cardPopup!.x + 20, cardPopup!.y + 10);
  await expect(popup).toBeVisible();
  await source.focus();
  await expect(popup).toContainText("Auto-refill added this issue");
  await expect(popup).toBeVisible();
  await row(page, 5).locator(".knowledge-row-select").focus();
  await expect(popup).toHaveText("The agent is working on this issue.");
  await row(page, 5).locator(".knowledge-row-select").click();
  await page.getByRole("button", { name: "Output", exact: true }).click();
  for (const selector of [".issue-preview .knowledge-monitor-chip", ".issue-split-pair .knowledge-row-badge"]) {
    if (selector.startsWith(".issue-split")) await page.getByRole("button", { name: "Split", exact: true }).click();
    const badge = page.locator(selector);
    await badge.focus();
    await expect(badge).toBeFocused();
    await expect(popup).toHaveText("The agent is working on this issue.");
    await badge.hover();
    await expect(popup).toBeVisible();
    await page.keyboard.press("Escape");
    await page.mouse.move(1590, 1090);
    await expect(popup).not.toBeVisible();
  }
});


test("T-7b.2a: tier minimums and failures explain reported facts and clear on omission", async ({page}) => {
  await confirm(page, [3, 4], { tier_facts: [
    { issue_number: 3, recorded_launch_tier: 0, history_floor: 1, minimum_tier_override: 0,
      agent_failures: 2, next_minimum_tier: 1 },
    { issue_number: 4, history_floor: 0, minimum_tier_override: 2, agent_failures: 3, next_minimum_tier: 2 },
  ] });
  const chips = row(page, 3).locator('[data-kind="tier"]');
  await expect(chips).toHaveText(["Min tier 0", "2 agent failures → ≥ tier 1"]);
  await expect(row(page, 4).locator('[data-kind="tier"]')).toHaveCount(0);
  const select = row(page, 3).locator(".knowledge-row-select");
  const popup = page.locator('.issue-control-explanation[role="tooltip"]');
  await page.mouse.move(1590, 1090);
  await select.focus();
  await expect(popup).toContainText("zero-based");
  await expect(popup).toContainText("not a profile lock");
  await expect(popup).toContainText("availability can select a higher tier");
  await expect(select).toHaveAttribute("aria-describedby", await popup.getAttribute("id") as string);
  await expect(select.locator('[tabindex="0"]')).toHaveCount(0);
  await chips.last().hover();
  await expect(popup).toContainText("2 classified agent failures");
  await expect(popup).toContainText("Infrastructure and unknown failures are excluded");
  await page.screenshot({path: test.info().outputPath("issue-tier-chips.png")});
  const before = await page.evaluate(() => (window as any).__queueMessages.length);
  await page.keyboard.press("Escape");
  await expect(popup).not.toBeVisible();
  await page.evaluate(() => (window as any).__queueOmitTierFacts());
  await expect(row(page, 3).locator('[data-kind="tier"]')).toHaveCount(0);
  // Refresh restores row focus; move away so returning actually opens its new explanation.
  await page.locator(".knowledge-search").focus();
  await select.focus();
  await expect(popup).toBeVisible();
  await expect(popup).not.toContainText("zero-based");
  const controls = await page.evaluate(before => (window as any).__queueMessages.slice(before).filter((message: any) =>
    /^(set_|issue_monitor_queue_|update_viewport|input$)/.test(message.kind)), before);
  expect(controls, "reading and dismissing facts sends no controls").toEqual([]);
});

test("T-7b.2b.2: Running tier uses exact live window and clears stale explanations", async ({page}) => {
  const facts = [{ issue_number: 5, recorded_launch_tier: 2,
    confirmed_launch: { tier: 0, window_id: "tab-queue::agent-5", delivery_id: "delivery-5" } }];
  await confirm(page, [3, 4], { tier_facts: facts });
  const chip = row(page, 5).locator('[data-key="tier-running"]');
  const select = row(page, 5).locator(".knowledge-row-select");
  const popup = page.locator('.issue-control-explanation[role="tooltip"]');
  await expect(chip).toHaveText("Running tier 0");
  await expect(select.locator('[tabindex="0"]')).toHaveCount(0);
  // Settle the existing preview's initial agent uptime projection before pressing a row.
  await page.evaluate(() => (window as any).__queueFocusResponse());
  await select.hover();
  const pressedRow = await select.elementHandle();
  await page.mouse.down();
  await page.evaluate(() => (window as any).__queueFocusResponse());
  expect(await pressedRow!.evaluate(node => node.isConnected), "focus response preserves the pressed row").toBe(true);
  await page.mouse.up();
  const detail = page.locator(".knowledge-detail-pane");
  await expect(detail.getByText("Description for #5", {exact:true})).toBeVisible();
  await detail.locator('[data-action="close-issue-preview"]').click();
  await page.mouse.move(1590, 1090);
  await select.focus();
  await expect(popup).toContainText("Acknowledged launch tier 0 (zero-based tiers)");
  await expect(select).toHaveAttribute("aria-describedby", await popup.getAttribute("id") as string);
  await page.screenshot({path: test.info().outputPath("running-tier-reported.png")});
  // Establish the tier anchor after capture: native pointer hit-testing during
  // a screenshot can enter another card before the runtime invalidation.
  await chip.hover();
  await expect(popup).toHaveText("Acknowledged launch tier 0 (zero-based tiers); the matching agent window currently reports running.");
  const before = await page.evaluate(() => (window as any).__queueMessages.length);
  // The persisted window remains running: only the actual runtime event changes.
  await page.evaluate(() => (window as any).__queueAgentState("stopped"));
  await expect(chip).toHaveCount(0);
  await expect(popup).not.toBeVisible();
  await expect(select).not.toHaveAttribute("aria-description", /Acknowledged/);
  await page.locator(".knowledge-search").focus();
  await select.focus();
  await expect(popup).toBeVisible();
  await expect(popup).not.toContainText("Acknowledged launch tier");
  await page.screenshot({path: test.info().outputPath("running-tier-cleared.png")});
  await page.evaluate(() => (window as any).__queueAgentState("running"));
  await expect(chip).toHaveText("Running tier 0");
  await page.evaluate(() => (window as any).__queueAgentWindow({id:"other-agent"}));
  await expect(chip).toHaveCount(0);
  await page.evaluate(() => (window as any).__queueAgentWindow({id:"tab-queue::agent-5",placement:{kind:"canvas"},linked_issue_number:5}));
  await expect(chip).toHaveText("Running tier 0");
  await page.evaluate(() => (window as any).__queueOmitTierFacts());
  await expect(chip).toHaveCount(0);
  await confirm(page, [3, 4], { tier_facts: facts });
  await expect(chip).toHaveText("Running tier 0");
  await page.evaluate(() => (window as any).__queueStatus({tier_facts:[{issue_number:5,recorded_launch_tier:2}]}));
  await expect(chip).toHaveCount(0);
  await confirm(page, [3, 4], { tier_facts: facts });
  await expect(chip).toHaveText("Running tier 0");
  await page.evaluate(() => (window as any).__queueAgentWindow(null));
  await expect(chip).toHaveCount(0);
  const controls = await page.evaluate(before => (window as any).__queueMessages.slice(before).filter((message: any) =>
    /^(set_|issue_monitor_queue_|input$|stop_window$|close_window$)/.test(message.kind)), before);
  expect(controls, "observing launch facts sends no controls").toEqual([]);
});

test("card selection keeps issue body, acceptance and provenance beside switchable output", async ({page}) => {
  const detail=page.locator(".knowledge-detail-pane");
  for (const [number,source] of [[3,"Operator"],[4,"Auto-refill"]] as const) {
    await row(page,number).locator(".knowledge-row-select").click();
    await expect(page.getByRole("button",{name:"Issue",exact:true})).toHaveAttribute("aria-pressed","true");
    await expect(detail.getByText(`Description for #${number}`, {exact:true})).toBeVisible();
    await detail.locator(".knowledge-section").filter({hasText:"Acceptance criteria"}).locator("summary").click();
    await expect(detail.getByText(`AC-${number}: expected behavior`, {exact:true})).toBeVisible();
    await expect(detail.locator(".issue-detail-provenance")).toHaveText(`Queued by: ${source}`);
  }
  await page.getByRole("button",{name:"Output",exact:true}).click();
  await expect(detail.locator(".issue-preview-empty")).toHaveText("Waiting in queue. No agent has started.");
  await row(page,6).locator(".knowledge-row-select").click();
  await expect(page.getByRole("button",{name:"Output",exact:true})).toHaveAttribute("aria-pressed","true");
  await expect(detail.locator(".issue-preview-empty")).toHaveText("This issue is completed. No agent is running.");
  await page.getByRole("button",{name:"Issue",exact:true}).click();
  await expect(detail).toContainText("Description for #6");
  await expect(page.locator(".issue-queue-board")).toBeVisible();
});

test("urgent priority, cap fallback and PM demotion preserve assignment provenance", async ({page}) => {
  await confirm(page, [3, 4, 1], { terminal_queue: [
    {number: 3, queued_by: "operator", priority: "urgent", priority_reason: "urgent_label", assigned_by: "alice", assigned_at: "2026-10-03T01:00:00Z"},
    {number: 4, queued_by: "auto-refill", priority: "normal", priority_reason: "urgent_limit_reached"},
    {number: 1, queued_by: "operator", priority: "normal", priority_reason: "pm_demoted", assigned_by: "pm-session", assigned_at: "2026-10-03T02:00:00Z"},
  ]});
  await expect.poll(() => column(page, "queued").locator(".knowledge-row").evaluateAll(rows => rows.map(row => row.getAttribute("data-issue-number")))).toEqual(["3", "4", "1"]);
  for (const [number, label, actor, time] of [
    [3, "Urgent", "alice", "2026-10-03T01:00:00Z"],
    [4, "Normal · Urgent limit reached", "Unknown", "Not observed"],
    [1, "Normal · PM demoted", "pm-session", "2026-10-03T02:00:00Z"],
  ] as const) {
    const card = row(page, number);
    const priority = card.locator('[data-key="queue-priority"]');
    await expect(priority).toHaveText(label);
    const cardBounds = (await card.boundingBox())!;
    const priorityBounds = (await priority.boundingBox())!;
    expect(priorityBounds.x + priorityBounds.width, "priority chip fits within queued card").toBeLessThanOrEqual(cardBounds.x + cardBounds.width);
    expect(await priority.evaluate(element => element.scrollWidth <= element.clientWidth), "priority text fits without overflowing").toBe(true);
    await row(page, number).locator(".knowledge-row-select").click();
    await expect(page.locator(".issue-detail-priority")).toHaveText(`Priority: ${label}`);
    await expect(page.locator(".issue-detail-priority-assignment")).toHaveText(`Priority assigned by: ${actor} · Assigned at: ${time}`);
  }
});

test("bulk drag and reorder wait for confirmation, forbidden and rejected changes retain cards", async ({ page }) => {
  await row(page,1).getByRole("checkbox").check();
  await row(page,2).getByRole("checkbox").check();
  await drag(page,1,column(page,"queued").locator("h3"));
  await expect.poll(()=>messages(page)).toEqual([{kind:"issue_monitor_queue_push",issue_numbers:[1,2]}]);
  await expect(column(page,"backlog").locator('[data-issue-number="1"]')).toHaveCount(1);
  await confirm(page,[3,4,1,2]);
  await expect(column(page,"queued").locator(".knowledge-row")).toHaveCount(4);
  await row(page,1).getByRole("checkbox").uncheck();
  await row(page,2).getByRole("checkbox").uncheck();
  await drag(page,2,row(page,3));
  await expect.poll(async()=>(await messages(page)).at(-1)).toEqual({kind:"issue_monitor_queue_move",issue_number:2,position:0});
  await expect(column(page,"queued").locator(".knowledge-row").first()).toHaveAttribute("data-issue-number","3");
  await confirm(page,[2,3,4,1]);
  await expect(column(page,"queued").locator(".knowledge-row").first()).toHaveAttribute("data-issue-number","2");
  await drag(page,2,column(page,"backlog").locator("h3"));
  await expect.poll(async()=>(await messages(page)).at(-1)).toEqual({kind:"issue_monitor_queue_remove",issue_numbers:[2]});
  await confirm(page,[3,4,1]);
  await expect(column(page,"backlog").locator('[data-issue-number="2"]')).toHaveCount(1);
  const before=(await messages(page)).length;
  for(const phase of ["active","done"]) {
    await drag(page,2,column(page,phase).locator("h3"));
    await expect(page.locator(".issue-queue-feedback")).toContainText("controlled by the monitor");
  }
  expect((await messages(page)).length).toBe(before);
  await drag(page,2,column(page,"queued").locator("h3"));
  await confirm(page,[3,4,1],{last_error:"Queue change rejected: issue is on hold"});
  await expect(column(page,"backlog").locator('[data-issue-number="2"]')).toHaveCount(1);
  await page.locator("#op-notifications-button").click();
  await expect(page.locator("#notification-center")).toContainText("Queue change rejected: issue is on hold");
});

test("auto-refill starts off and toggle and limit use server-confirmed values", async ({ page }) => {
  await page.getByRole("button", {name:"Issues settings",exact:true}).click();
  const issues = page.locator('[data-settings-panel="issues"]');
  await expect(issues.locator(".settings-issues-options")).toHaveAttribute("open", "");
  await expect(issues.locator('[data-action="monitor-settings"]')).toBeFocused();
  const toggle=issues.getByLabel("Auto-refill",{exact:true});
  await expect(toggle).not.toBeChecked();
  await toggle.click();
  await expect.poll(()=>messages(page)).toEqual([{kind:"set_issue_monitor_auto_refill",enabled:true,limit:3}]);
  await confirm(page,[3,4],{terminal_queue_auto_refill:true});
  await expect(toggle).toBeChecked();
  await issues.getByLabel("Refill limit",{exact:true}).fill("5");
  await issues.getByLabel("Refill limit",{exact:true}).press("Tab");
  await expect.poll(async()=>(await messages(page)).at(-1)).toEqual({kind:"set_issue_monitor_auto_refill",enabled:true,limit:5});
  await confirm(page,[3,4],{terminal_queue_auto_refill_limit:5});
  await expect(issues.getByLabel("Refill limit",{exact:true})).toHaveValue("5");
});

test("live launched state survives stale cache and retains full queue positions", async ({ page }) => {
  await confirm(page, [99, 3, 4]);
  await page.evaluate(() => (window as any).__queueInbox([{issue:{number:3},state:"launched"}]));
  await expect(column(page, "queued").locator('[data-issue-number="3"]')).toHaveCount(0);
  await expect(column(page, "active").locator('[data-issue-number="3"]')).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).__queueMessages.filter((message:any) =>
    message.kind === "load_knowledge_bridge").length)).toBeGreaterThan(1);
  await expect(column(page, "active").locator('[data-issue-number="3"]')).toBeVisible();
  const move = row(page, 4).locator('[data-action="move-up"]');
  if (!await move.isVisible()) await row(page, 4).locator('.knowledge-row-menu summary').click();
  await move.click();
  await expect.poll(async () => (await messages(page)).at(-1)).toEqual({kind:"issue_monitor_queue_move",issue_number:4,position:1});
});

test("an unfocused Issue window reflects a launch while animation frames are suspended", async ({ page }, info) => {
  await page.evaluate(() => (window as any).__queueAddForeground());
  await expect(page.locator('[data-id="tab-queue::console"]')).toHaveClass(/focused/);
  await expect(page.locator('[data-id="tab-queue::issue-1"]')).not.toHaveClass(/focused/);
  const elapsed = await page.evaluate(async () => {
    const scheduleFrame = window.requestAnimationFrame;
    window.requestAnimationFrame = () => 0;
    const start = performance.now();
    const rendered = new Promise<number>(resolve => {
      const observer = new MutationObserver(() => {
        if (!document.querySelector('[data-queue-column="active"] [data-issue-number="3"]')) return;
        observer.disconnect();
        resolve(performance.now() - start);
      });
      observer.observe(document.body, {childList:true,subtree:true});
    });
    (window as any).__queueInbox([{issue:{number:3},state:"launched"}]);
    try { return await rendered; } finally { window.requestAnimationFrame = scheduleFrame; }
  });
  expect(elapsed, 'model-to-view update without a focus or animation frame').toBeLessThanOrEqual(1000);
  await info.attach('unfocused-render-timing', {body:JSON.stringify({elapsed_ms:elapsed,limit_ms:1000}),contentType:'application/json'});
  await expect(page.locator('[data-id="tab-queue::issue-1"]')).not.toHaveClass(/focused/);
  await expect(column(page, "queued").locator('[data-issue-number="3"]')).toHaveCount(0);
});

async function installBackend(page: Page, { includeAgent = false, observeSkipped = false, inboxFirst = false, narrowSettings = false } = {}) {
  const projectKey=new URL(liveUrl||APP_URL).pathname.split("/")[2];
  await page.addInitScript(({projectKey, includeAgent, observeSkipped, inboxFirst, narrowSettings})=>{
    const fixture=window as any;
    fixture.__queueMessages=[];
    const status:any={enabled:false,state:"disabled",active_count:1,max_active_agents:1,
      max_active_agents_mode:"auto",max_active_agents_override:null,
      launch_profile_source:"saved",launch_profile_summary:"Fixture agent",terminal_queue_auto_refill:false,
      terminal_queue_auto_refill_limit:3,terminal_queue:[{number:3,queued_by:"operator"},{number:4,queued_by:"auto-refill"}],queue_len:2};
    const entries=[
      {number:1,title:"First backlog issue",...(observeSkipped ? {labels:["gwt-queued"],monitor_state:"skipped",
        exclusion_reason:"not selected in this terminal queue"} : {})},{number:2,title:"Second backlog issue"},
      {number:3,title:"Operator queued issue",monitor_state:"queued",queue_position:1,queued_by:"operator"},
      {number:4,title:"Auto-refilled issue",monitor_state:"queued",queue_position:2,queued_by:"auto-refill"},
      {number:5,title:"Running issue",monitor_state:"launched"},{number:6,title:"Completed issue",state:"closed"},
    ].map(entry=>({state:"open",labels:[],is_spec:false,linked_branch_count:0,related_work_refs:[],...entry}));
    if (inboxFirst) Object.assign(status, {state:"error",last_error:"stale unbound Monitor pane"});
    class FixtureSocket extends EventTarget {
      static CONNECTING=0; static OPEN=1; static CLOSING=2; static CLOSED=3;
      readyState=0;
      constructor(public readonly url:string) { super();
        fixture.__queueEvent=(event:any)=>this.emit(event);
        fixture.__queueEntries=(revision:string, patch?:{number:number,title:string})=>{
          const entry=entries.find(entry=>entry.number===patch?.number);
          if (entry && patch) entry.title=patch.title;
          this.dispatchEvent(new MessageEvent("message",{data:JSON.stringify({kind:"knowledge_entries",
            ...fixture.__queueBaseLoad,revision,entries,selected_number:null,refresh_enabled:true})}));
        };
        fixture.__queueSetEntries=(next:any[])=>{
          entries.splice(0, entries.length, ...next);
          fixture.__queueEntries(undefined);
        };
        fixture.__queueLoadError=()=>this.emit({kind:"knowledge_error",...fixture.__queueLoad,
          message:"Issue cache read failed"});
        fixture.__queueKnowledgeComplete=()=>this.emit({kind:"knowledge_entries",...fixture.__queueLoad,
          entries:[{...entries[2],state:"closed",title:"Enriched queued issue"}],refresh_enabled:true});
        fixture.__queueConfirm=(numbers:number[],extra:any)=>{
        Object.assign(status,{terminal_queue:numbers.map(number=>({number,queued_by:number===4?"auto-refill":"operator"})),queue_len:numbers.length},extra);
        this.emit({kind:"issue_monitor_status",status});
      }; fixture.__queueStatus=(extra:any)=>{
        Object.assign(status,extra);
        this.emit({kind:"issue_monitor_status",status});
      }; fixture.__queueTitle=(number:number,title:string)=>{
        const entry=entries.find(entry=>entry.number===number);
        if(entry) entry.title=title;
        this.emit({kind:"knowledge_entries",...fixture.__queueLoad,entries,selected_number:null,refresh_enabled:true});
      }; fixture.__queueOmitTierFacts=()=>{
        delete status.tier_facts;
        this.emit({kind:"issue_monitor_status",status});
      }; fixture.__queueAgentState=(state:string)=>{
        this.emit({kind:"terminal_status",id:"tab-queue::agent-5",status:state});
      }; fixture.__queueFocusResponse=()=>{
        const workspace=fixture.__queueWorkspace;
        const issue=workspace.tabs[0].workspace.windows.find((item:any)=>item.id==="tab-queue::issue-1");
        issue.z_index+=1;
        this.dispatchEvent(new MessageEvent("message",{data:JSON.stringify({kind:"workspace_state",workspace})}));
      }; fixture.__queueAgentWindow=(extra:any)=>{
        const workspace=fixture.__queueWorkspace;
        const windows=workspace.tabs[0].workspace.windows;
        const agent=windows.find((item:any)=>item.preset==="agent");
        if(extra===null) workspace.tabs[0].workspace.windows=windows.filter((item:any)=>item!==agent);
        else Object.assign(agent,extra);
        this.emit({kind:"workspace_state",workspace});
      }; fixture.__queueAddForeground=()=>{
        const workspace=fixture.__queueWorkspace;
        workspace.tabs[0].workspace.windows.push({id:"tab-queue::console",title:"Console",preset:"console",
          geometry:{x:1000,y:80,width:500,height:500},z_index:2,status:"running",persist:true,minimized:false,maximized:false});
        this.emit({kind:"workspace_state",workspace});
      }; fixture.__queueInbox=(items:unknown[])=>this.dispatchEvent(new MessageEvent("message",{
        data:JSON.stringify({kind:"issue_monitor_inbox",items})
      })); fixture.__queueReplayDetail=()=>this.emit(fixture.__queueLastDetail);
      fixture.__queueRefreshDetail=()=>this.emit({...fixture.__queueLastDetail,
        detail:{...fixture.__queueLastDetail.detail,title:"Refreshed issue"}});
        setTimeout(()=>{this.readyState=1;this.dispatchEvent(new Event("open"));},0); }
      emit(payload:unknown) {const data=JSON.stringify(payload);setTimeout(()=>this.dispatchEvent(new MessageEvent("message",{data})),0);}
      send(raw:string) {
        const message=JSON.parse(raw);fixture.__queueMessages.push(message);
        if(message.kind==="frontend_ready") { fixture.__queueWorkspace={app_version:"playwright",tabs:[{
          id:"tab-queue",title:"Queue fixture",project_root:"/fixture",project_key:projectKey,kind:"git",
          workspace:{viewport:{x:0,y:0,zoom:1},windows:[{id:"tab-queue::issue-1",title:"Settings",preset:"settings",
            geometry:{x:40,y:40,width:narrowSettings ? 1160 : 1470,height:narrowSettings ? 880 : 950},z_index:1,status:"running",persist:true,minimized:false,maximized:false},
            ...(includeAgent ? [{id:"tab-queue::agent-5",session_id:"agent-5-session",title:"Issue 5 agent",preset:"agent",agent_id:"codex",
              geometry:{x:80,y:80,width:900,height:600},z_index:2,status:"running",persist:true,minimized:false,maximized:false,
              placement:{kind:"issue_preview",issue_window_id:"tab-queue::issue-1",issue_number:5}}] : [])]} }],active_tab_id:"tab-queue",recent_projects:[]};
          this.emit({kind:"workspace_state",workspace:fixture.__queueWorkspace}); }
        else if(message.kind==="create_window" && message.preset==="settings") {
          const workspace=fixture.__queueWorkspace;
          workspace.tabs[0].workspace.windows.push({id:"tab-queue::settings",title:"Settings",preset:"settings",
            geometry:{x:100,y:80,width:700,height:800},z_index:3,status:"running",persist:true,minimized:false,maximized:false});
          this.emit({kind:"workspace_state",workspace});
        }
        else if(message.kind==="set_issue_monitor_max_active_agents") {
          Object.assign(status, {max_active_agents:message.max_active_agents ?? 1,
            max_active_agents_override:message.max_active_agents,
            max_active_agents_mode:message.max_active_agents == null ? "auto" : "manual"});
          this.emit({kind:"issue_monitor_status",status});
        }
        else if(message.kind==="get_issue_monitor_profiles") {
          this.emit({kind:"issue_monitor_profiles",request_id:message.request_id,project_root:"/fixture",
            profiles:[{agent_id:"codex",model:"gpt-6-sol",prefer_for:[]}],usage_threshold_percent:80});
        }
        else if(message.kind==="list_issue_monitor") {
          this.emit({kind:"issue_monitor_status",status});
          if (inboxFirst) this.emit({kind:"issue_monitor_inbox",items:entries.map(issue=>({issue,
            state:issue.number===5?"launched":issue.number===6?"released":"queued"}))});
        }
        else if(["load_knowledge_bridge","search_knowledge_bridge"].includes(message.kind)) {
          fixture.__queueLoad={id:message.id,knowledge_kind:"issue",request_id:message.request_id};
          if (message.kind==="load_knowledge_bridge") fixture.__queueBaseLoad=fixture.__queueLoad;
          if (!inboxFirst) this.emit({kind:"knowledge_entries",...fixture.__queueLoad,entries,selected_number:null,refresh_enabled:true});
        }
        else if(message.kind==="select_knowledge_bridge_entry") {
          fixture.__queueLastDetail={kind:"knowledge_detail",id:message.id,knowledge_kind:"issue",request_id:message.request_id,detail:{number:message.number,title:entries.find(e=>e.number===message.number)?.title,state:message.number===6?"closed":"open",labels:[],sections:[{title:"Description",body:`Description for #${message.number}`,body_html:`<p>Description for #${message.number}</p>`},{title:"Acceptance criteria",body:`AC-${message.number}: expected behavior`,body_html:`<p>AC-${message.number}: expected behavior</p>`}],related_works:[]}};
          this.emit(fixture.__queueLastDetail);
        }
      }
      close(){this.readyState=3;this.dispatchEvent(new CloseEvent("close"));}
    }
    Object.defineProperty(window,"WebSocket",{configurable:true,value:FixtureSocket});
  },{projectKey, includeAgent, observeSkipped, inboxFirst, narrowSettings});
}
