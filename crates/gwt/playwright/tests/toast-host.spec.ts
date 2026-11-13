/* SPEC #3206 — real-browser E2E for the shared toast-host primitive and the
 * v2 notification center.
 *
 * v1 routed the bottom-right `alerts` region (completion / attention /
 * board-mention) through `createToastStack`; the first describe below keeps
 * that regression coverage. v2 retired the top-right autonomous `log` region
 * in favour of the operator-rail bell + unread badge + history drawer, so the
 * second describe boots the full frontend against a fixture WebSocket and
 * asserts what only a real browser proves: the retired region is absent, the
 * bell/badge/drawer wiring works end to end (click, Esc, ×, clear-all), the
 * history scrolls inside the drawer, and FR-017 Issue-window errors are read
 * only in the notification center — never rendered in the Issue window.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { gotoLiveGwt, readLiveHubCatalog } from "./_helpers/live-gwt";

test("Issue #5333: Issues naming across Issues, Settings and notifications (isolated checkout)", async ({ page }, info) => {
  test.setTimeout(120_000);
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  test.skip(!base || !process.env.GWT_PLAYWRIGHT_CHECK_HOME, "requires browser-check checkout and fresh HOME");
  await page.setViewportSize({ width: 1440, height: 1000 });
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await page.addInitScript(({ theme, origin }) => {
    if (location.origin === origin) localStorage.setItem("gwt:ui:theme", theme);
  }, { theme, origin: new URL(base!).origin });
  await gotoLiveGwt(page, base!, { enableTestBridge: true });
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  const capture = async (surface: string) => {
    const path = info.outputPath(`issues-naming-${surface}-${theme}.png`);
    await page.screenshot({ path, fullPage: true });
    await info.attach(`${surface}-${theme}`, { path, contentType: "image/png" });
  };

  await page.locator('#op-rail [data-surface="issues"]').click();
  await expect(page.locator('.workspace-window[data-preset="issue"]')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator("#op-strip-issue-monitor")).toHaveAttribute("title", /Issues:/);
  await capture("issues");

  await page.locator('#op-rail [data-surface="settings"]').click();
  const settings = page.locator('.workspace-window[data-preset="settings"]');
  await expect(settings).toBeVisible();
  await settings.getByRole("tab", { name: "Issues execution settings", exact: true }).click();
  await expect(settings.locator('[data-settings-panel="issues"]')).toBeVisible();
  await capture("settings");

  await inject(page, { kind: "issue_monitor_toast", level: "error",
    message: "Issues scan stalled; last scan at 2026-11-10T00:00:00Z", issue_number: 5333 });
  await page.locator("#op-notifications-button").click();
  await expect(page.locator("#notification-center")).toBeVisible();
  await expect(page.locator("#notification-center")).toContainText("Issues");
  await expect(page.locator("#notification-center")).toContainText("#5333");
  await capture("notifications");
  const retiredName = new RegExp(["issue", "monitor"].join("\\s+"), "i");
  expect(await page.locator("body").innerText()).not.toMatch(retiredName);
  expect(errors, "zero console and page errors across the renamed surfaces").toEqual([]);
});

test("Issue #6659 / Issue #6932 removes resolved Agent Error rows for unmounted and mounted windows (isolated checkout)", async ({ page }, testInfo) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  test.skip(!base || !process.env.GWT_PLAYWRIGHT_CHECK_HOME, "requires browser-check checkout and fresh HOME");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const catalog = await readLiveHubCatalog(page, base!);
  const primary = catalog.projects[0];
  expect(primary).toBeTruthy();
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  const agent = (id: string) => ({
    id, title: id, preset: "codex", status: "error", agent_id: "codex",
    geometry: { x: 40, y: 60, width: 800, height: 500 }, z_index: 1,
    minimized: false, maximized: false, persist: true,
  });
  const orphan = agent("issue-6659-unmounted");
  const keeper = agent("issue-6659-retained");
  const mounted = agent("issue-6659-mounted");
  const snapshot = {
    app_version: "playwright",
    tabs: [
      { ...primary, project_root: process.env.GWT_PLAYWRIGHT_PROJECT_ROOT, kind: "git", workspace: {
        viewport: { x: 0, y: 0, zoom: 1 }, windows: [] as ReturnType<typeof agent>[],
      } },
      { id: "issue-6659-background", project_key: "6659665966596659", title: "Background fixture", project_root: "/fixture/background", kind: "git", workspace: {
        viewport: { x: 0, y: 0, zoom: 1 }, windows: [orphan, keeper],
      } },
    ],
    active_tab_id: primary.id,
    recent_projects: [],
  };
  // Only transport is deterministic: HTML/JS/CSS still come from the fresh
  // checkout binary. No window is launched and no backend state is rewritten.
  await page.addInitScript(({ selectedTheme, initial }) => {
    localStorage.setItem("gwt:ui:theme", selectedTheme);
    class SnapshotSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      readyState = 0;
      constructor(_url: string) {
        super();
        setTimeout(() => {
          this.readyState = SnapshotSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }
      send(raw: string) {
        if (JSON.parse(raw).kind !== "frontend_ready") return;
        for (const payload of [
          { kind: "hub_state", hub: { app_version: initial.app_version, projects: initial.tabs, recent_projects: [] } },
          { kind: "workspace_state", workspace: initial },
        ]) {
          this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) }));
        }
      }
      close() { this.readyState = SnapshotSocket.CLOSED; }
    }
    // @ts-ignore - deterministic transport over binary-served frontend assets.
    window.WebSocket = SnapshotSocket;
  }, { selectedTheme: theme, initial: snapshot });
  await gotoLiveGwt(page, new URL(`/p/${primary.project_key}`, base!).href);
  await expect(page.locator("#app-version")).toHaveText("vplaywright");
  const toast = (id: string) => page.locator(`.toast-alerts__item[data-toast-id="attention-${id}"]`);
  const alerts = page.locator(".toast-alerts__item");
  const rows = page.locator("#notification-center .notification-center__item");
  const publish = () => inject(page, { kind: "workspace_state", workspace: snapshot });
  const diagnose = (id: string) => inject(page, { kind: "terminal_status", id, status: "error", detail: "Local runtime failed" });

  // Saved status alone is not evidence that this machine ran the Agent.
  await expect(alerts).toHaveCount(0);
  await diagnose(orphan.id);
  await diagnose(keeper.id);
  await expect(toast(orphan.id)).toHaveCount(1);
  await expect(toast(keeper.id)).toHaveAttribute("data-level", "error");
  await expect(page.locator(`.workspace-window[data-id="${orphan.id}"]`)).toHaveCount(0);
  await expect(rows).toHaveCount(2);
  await publish();
  await expect(alerts).toHaveCount(2);
  await expect(rows).toHaveCount(2);

  snapshot.tabs[1].workspace.windows = [keeper];
  await publish();
  await expect(toast(orphan.id)).toHaveCount(0);
  await expect(toast(keeper.id)).toHaveCount(1);
  await expect(rows).toHaveCount(1);

  snapshot.tabs[0].workspace.windows = [mounted];
  await publish();
  await diagnose(mounted.id);
  await expect(page.locator(`.workspace-window[data-id="${mounted.id}"]`)).toHaveCount(1);
  await expect(toast(mounted.id)).toHaveCount(1);
  await publish();
  await expect(rows).toHaveCount(2);
  snapshot.tabs[0].workspace.windows = [];
  await publish();
  await expect(toast(mounted.id)).toHaveCount(0);
  await expect(rows).toHaveCount(1);

  for (let cycle = 0; cycle < 3; cycle += 1) {
    snapshot.tabs[1].workspace.windows = [orphan, keeper];
    await publish();
    await diagnose(orphan.id);
    await expect(toast(orphan.id)).toHaveCount(1);
    await expect(rows).toHaveCount(2);
    await publish();
    await expect(rows).toHaveCount(2);
    snapshot.tabs[1].workspace.windows = [keeper];
    await publish();
    await expect(toast(orphan.id)).toHaveCount(0);
    await expect(toast(keeper.id)).toHaveCount(1);
  }
  // Removing the inactive Project from the authoritative catalog is also
  // removal authority, even without a new runtime-status snapshot.
  await inject(page, { kind: "hub_state", hub: {
    app_version: snapshot.app_version, projects: [snapshot.tabs[0]], recent_projects: [],
  } });
  await expect(alerts).toHaveCount(0);
  await page.locator("#op-notifications-button").click();
  await expect(rows).toHaveCount(0);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  const screenshot = testInfo.outputPath(`removed-window-attention-${theme}.png`);
  await page.screenshot({ path: screenshot, fullPage: true });
  await testInfo.attach(`removed-window-attention-${theme}`, { path: screenshot, contentType: "image/png" });
  expect(errors).toEqual([]);
});

test("Issue #6729 keeps only unresolved local errors across launch retry and foreign snapshots (isolated checkout)", async ({ page }, testInfo) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  test.skip(!base || !process.env.GWT_PLAYWRIGHT_CHECK_HOME, "requires browser-check checkout and fresh HOME");
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  const soakSeconds = theme === "dark" ? Number(process.env.GWT_PLAYWRIGHT_ATTENTION_SOAK_SECONDS || 0) : 0;
  test.setTimeout((soakSeconds + 120) * 1000);
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const catalog = await readLiveHubCatalog(page, base!);
  const primary = catalog.projects[0];
  expect(primary).toBeTruthy();
  const agent = (id: string, issue: number, startedAt?: number | null) => ({
    id, linked_issue_number: issue, title: id, preset: "codex", status: "error", agent_id: "codex",
    runtime_started_at_ms: startedAt,
    geometry: { x: 40, y: 60, width: 800, height: 500 }, z_index: 1,
    minimized: false, maximized: false, persist: true,
  });
  const first = agent("timeout-first", 6729, 1000);
  const successor = agent("timeout-successor", 6729);
  const real = agent("real-local-error", 6730, null);
  const unprovenSuccessor = { ...agent("unknown-order-successor", 6730), status: "running" };
  const recoveredOld = agent("recovered-old", 6731, 1000);
  const recoveredNew = { ...agent("recovered-new", 6731, 2000), status: "running" };
  const snapshot = {
    app_version: "playwright", active_tab_id: primary.id, recent_projects: [],
    tabs: [
      { ...primary, kind: "git", workspace: { viewport: { x: 0, y: 0, zoom: 1 }, windows: [] } },
      { id: "notifications-background", project_key: "6729672967296729", title: "Issues Off", project_root: "/fixture/foreign",
        kind: "git", workspace: { viewport: { x: 0, y: 0, zoom: 1 },
          windows: [first, successor, real, unprovenSuccessor, agent("foreign-machine", 9999), recoveredOld, recoveredNew] } },
    ],
  };
  // Use binary-served assets with a deterministic runtime stream. No Agent is
  // launched and the isolated Issues remains Off throughout the measurement.
  await page.addInitScript(({ theme, snapshot }) => {
    localStorage.setItem("gwt:ui:theme", theme);
    class SnapshotSocket extends EventTarget {
      static OPEN = 1;
      static CLOSED = 3;
      readyState = 0;
      constructor() {
        super();
        setTimeout(() => { this.readyState = 1; this.dispatchEvent(new Event("open")); }, 0);
      }
      send(raw: string) {
        if (JSON.parse(raw).kind !== "frontend_ready") return;
        for (const payload of [
          { kind: "hub_state", hub: { app_version: snapshot.app_version, projects: snapshot.tabs, recent_projects: [] } },
          { kind: "workspace_state", workspace: snapshot },
        ]) this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) }));
      }
      close() { this.readyState = 3; }
    }
    // @ts-ignore - deterministic transport; frontend assets come from the owned checkout GUI.
    window.WebSocket = SnapshotSocket;
  }, { theme, snapshot });
  await gotoLiveGwt(page, new URL(`/p/${primary.project_key}`, base!).href);
  await expect(page.locator("#app-version")).toHaveText("vplaywright");
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  const alerts = page.locator(".toast-alerts__item");
  const rows = page.locator("#notification-center .notification-center__item");
  const status = (id: string, state: string, detail: string | null, retryable?: boolean) =>
    inject(page, { kind: "terminal_status", id, status: state, detail, retryable });
  await expect(alerts).toHaveCount(0);
  await expect(rows).toHaveCount(0);
  await status(recoveredOld.id, "error", "Retained old local diagnostic");
  await expect(alerts).toHaveCount(0);
  snapshot.tabs[1].workspace.windows = snapshot.tabs[1].workspace.windows.filter(w => w.id !== recoveredNew.id);
  await inject(page, { kind: "workspace_state", workspace: snapshot });
  await status(recoveredOld.id, "error", "Retained old local diagnostic");
  await expect(alerts).toHaveCount(0);
  await page.reload();
  await expect(page.locator("#app-version")).toHaveText("vplaywright");
  await status(recoveredOld.id, "error", "Retained old local diagnostic");
  await expect(alerts).toHaveCount(0);
  await status(first.id, "starting", "Launching...");
  await inject(page, { kind: "window_state", window_id: first.id, state: "error" });
  await expect(alerts).toHaveCount(0);
  await status(first.id, "error", "Launch timed out: no window was created within 120s");
  await expect(alerts).toHaveCount(1);
  await expect(alerts).toContainText("Launch timed out");
  await expect(alerts).not.toContainText("Launching...");
  successor.runtime_started_at_ms = 2000;
  await inject(page, { kind: "workspace_state", workspace: snapshot });
  await status(successor.id, "error", "Second local launch failed");
  await expect(alerts).toHaveCount(1);
  await expect(alerts).toContainText(successor.id);
  await expect(rows).toHaveCount(1);
  await status(first.id, "stopped", "Late old window exit");
  await status(first.id, "running", null);
  await expect(alerts).toHaveCount(1);
  await expect(alerts).toContainText(successor.id);
  await status(successor.id, "error", "Retry scheduled", true);
  await expect(alerts).toHaveCount(0);
  await expect(rows).toHaveCount(0);
  await status(real.id, "error", "Process exited with status 1");
  unprovenSuccessor.runtime_started_at_ms = 3000;
  await inject(page, { kind: "workspace_state", workspace: snapshot });
  await expect(alerts).toHaveCount(1);
  await expect(alerts).toContainText(real.id);
  await status(successor.id, "error", "Retry attempt failed");
  await status(successor.id, "running", null);
  await expect(alerts).toHaveCount(1);
  await expect(alerts).toContainText(real.id);
  await expect(rows).toHaveCount(1);
  await expect(page.locator("#op-notifications-button .op-rail__badge")).toHaveText("1");

  const startedAt = new Date().toISOString();
  const started = performance.now();
  let cycles = 0;
  do {
    await inject(page, { kind: "workspace_state", workspace: snapshot });
    await status(successor.id, "starting", "Launching...");
    await inject(page, { kind: "window_state", window_id: successor.id, state: "error" });
    await status(successor.id, "error", "Automatic retry pending", true);
    await status(successor.id, "running", null);
    await status(first.id, "error", "Launch timed out: no window was created within 120s");
    await status(recoveredOld.id, "error", "Retained old local diagnostic");
    await expect(alerts).toHaveCount(1);
    await expect(alerts).toContainText(real.id);
    await expect(rows).toHaveCount(1);
    expect(errors, "console and page errors").toEqual([]);
    cycles++;
    if (soakSeconds > 0 && cycles % 6 === 0) {
      console.log(`Issue #6729 soak: ${Math.floor((performance.now() - started) / 1000)}s, ${cycles} checked cycles, zero spurious toasts`);
    }
    const remaining = soakSeconds * 1000 - (performance.now() - started);
    if (remaining > 0) await page.waitForTimeout(Math.min(10_000, remaining));
  } while (performance.now() - started < soakSeconds * 1000);
  await testInfo.attach("attention-soak", { body: JSON.stringify({
    theme, startedAt, finishedAt: new Date().toISOString(), elapsedMs: performance.now() - started,
    cycles, spuriousToasts: 0, unresolvedRealErrors: 1, consoleAndPageErrors: errors,
  }), contentType: "application/json" });
  await page.screenshot({ path: testInfo.outputPath(`local-error-attention-${theme}.png`), fullPage: true });
  expect(errors).toEqual([]);
});

test("Issue #6932 Issues notifications group repeated failures and continued/stalled diagnoses (isolated checkout)", async ({ page }, testInfo) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  test.skip(!base || !process.env.GWT_PLAYWRIGHT_CHECK_HOME, "requires browser-check checkout and fresh HOME");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await gotoLiveGwt(page, base!);
  expect(await (await page.request.get(new URL("/app.js", base!).href)).text())
    .toContain("issueMonitorNotificationModel.update(() => ({ ...event }))");
  const bell = page.locator("#op-notifications-button");
  const badge = bell.locator(".op-rail__badge");
  const drawer = page.locator("#notification-center");
  const rows = drawer.locator(".notification-center__item");
  await bell.click();
  await drawer.getByRole("button", { name: "Clear all", exact: true }).click();
  await page.getByRole("button", { name: "Close notifications", exact: true }).click();
  // Deterministic receive events exercise binary-served app wiring, without
  // triggering real autonomous launches or changing the backend protocol.
  await page.evaluate(() => {
    const launch = { kind: "issue_monitor_toast", level: "info", message: "Issues launch requested", issue_number: 5016 };
    for (let i = 0; i < 4; i += 1) {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: launch }));
    }
  });
  await expect(badge).toHaveText("1");
  await page.evaluate(() => {
    const notice = { kind: "issue_monitor_toast", level: "warn", message: "Repeated notice", issue_number: 5016 };
    for (const payload of [
      { ...notice, level: "info", message: "Issues launch requested", issue_number: 5017 },
      notice, notice,
      { ...notice, level: "error", message: "Issues launch requested" },
      { ...notice, notification_transition: "needs_human", level: "error", message: "Decision needed" },
    ]) {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
    }
  });
  await expect(badge).toHaveText("5");
  await expect(badge).toHaveAttribute("data-has-error", "true");
  await bell.click();
  await expect(rows).toHaveCount(5);
  await expect(rows.first()).toContainText("Needs human #5016");
  await expect(rows.first()).toHaveAttribute("data-level", "error");
  await expect(rows.filter({ hasText: "Repeated notice" })).toHaveCount(1);
  await expect(rows.filter({ hasText: "Repeated notice" }).locator(".notification-center__count")).toHaveText("×2");
  await expect(rows.filter({ hasText: "Issues launch requested" })).toHaveCount(3);
  await expect(rows.last()).toContainText("Issues #5016");
  await expect(badge).toBeHidden();
  const launchRow = drawer.locator('[data-error-key="issue-monitor:launch-requested:5016"]');
  await expect(launchRow).toHaveCount(1);
  await launchRow.locator(".notification-center__dismiss").click();
  await expect(rows).toHaveCount(4);
  await page.evaluate(() => window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
    detail: { kind: "issue_monitor_toast", level: "info", message: "Issues launch requested", issue_number: 5016 },
  })));
  await expect(launchRow).toHaveCount(1);
  await expect(rows).toHaveCount(5);
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  const screenshot = testInfo.outputPath(`monitor-notifications-${theme}.png`);
  await page.screenshot({ path: screenshot, fullPage: true });
  await testInfo.attach(`monitor-notifications-${theme}`, { path: screenshot, contentType: "image/png" });
  await drawer.getByRole("button", { name: "Clear all", exact: true }).click();
  await expect(rows).toHaveCount(0);
  await page.getByRole("button", { name: "Close notifications", exact: true }).click();
  await expect(drawer).toBeHidden();
  const failure = { kind: "issue_monitor_toast", level: "error",
    message: "Issues scan failed at open-pr-readback stage: GitHub connection failed" };
  for (let i = 0; i < 5; i++) await inject(page, failure);
  await expect(rows).toHaveCount(1);
  await expect(rows.locator(".notification-center__count")).toHaveText("×5");
  await bell.click();
  await drawer.getByRole("button", { name: "Clear all", exact: true }).click();
  await page.getByRole("button", { name: "Close notifications", exact: true }).click();
  for (const disposition of ["stale_readback", "previous_candidates"]) {
    const message = `scan continued: candidate-load failed (continued_with_${disposition}): offline`;
    await inject(page, { ...failure, message });
    await inject(page, { kind: "issue_monitor_status", status: { enabled: true, state: "error",
      queue_len: 0, active_count: 0, max_active_agents: 1, last_error: message } });
  }
  await expect(rows).toHaveCount(0);
  const stalled = "Issues scan stalled; last scan at 2026-11-09T00:00:00Z; previous failure: issue #42: needs_human: decision needed";
  await inject(page, { ...failure, message: stalled });
  await bell.click();
  await page.getByRole("button", { name: "Close notifications", exact: true }).click();
  await inject(page, { ...failure, message: stalled.replace("00:00:00Z", "00:01:00Z") });
  await expect(rows).toHaveCount(1);
  await expect(badge).toBeHidden();
  await inject(page, { ...failure, message: stalled.replace("#42", "#43") });
  await expect(rows).toHaveCount(2);
  await expect(badge).toHaveText("1");
  expect(errors).toEqual([]);
});

test.describe("toast-host alerts region (real browser, SPEC #3206 P1)", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      class NoopSocket {
        constructor() {}
        send() {}
        close() {}
        addEventListener() {}
        removeEventListener() {}
      }
      // @ts-ignore
      window.WebSocket = NoopSocket;
    });
    await installEmbeddedRoutes(page);
    await page.goto(APP_URL);
    await page.evaluate(async () => {
      const mod = await import("/toast-host.js");
      document.body.replaceChildren();
      const host = document.createElement("div");
      host.id = "operator-notice-stack";
      host.className = "operator-notice-stack";
      document.body.appendChild(host);
      (window as any).__activated = [];
      const stack = (mod as any).createToastStack({
        document,
        className: "toast-alerts",
        ariaRole: "status",
        ariaLive: "polite",
        animateDismiss: true,
        levels: ["neutral", "info", "warn", "error", "done"],
        defaultLevel: "neutral",
      });
      stack.mount(host);
      (window as any).__alerts = stack;
    });
  });

  test("the three former systems share ONE bottom-right stack (no overlap)", async ({ page }) => {
    await page.evaluate(() => {
      const s = (window as any).__alerts;
      s.push({ id: "agent-completion", level: "neutral", title: "Done", message: "ok", dismissible: false });
      s.push({ id: "attention-w1", level: "warn", title: "Needs input", message: "y", dismissible: true });
      s.push({ id: "board-mention", level: "info", title: "Board reply", dismissible: false });
    });

    // All three live in the SAME single container (no per-system fixed offsets).
    expect(await page.locator(".toast-alerts").count()).toBe(1);
    const items = page.locator(".toast-alerts__list .toast-alerts__item");
    await expect(items).toHaveCount(3);
    await expect(items.first()).toContainText("Board reply"); // newest on top

    const layout = await page.evaluate(() => {
      const host = getComputedStyle(
        document.querySelector(".operator-notice-stack") as HTMLElement,
      );
      const alerts = getComputedStyle(
        document.querySelector(".toast-alerts") as HTMLElement,
      );
      return { hostPosition: host.position, alertsPosition: alerts.position };
    });
    expect(layout.hostPosition).toBe("fixed");
    expect(layout.alertsPosition).not.toBe("fixed");
  });

  // SPEC #6476 T-005c (AC-1): the dismiss control was a 20x19px target on a
  // phone. Below 601px it is a 44px tap target; wider layouts keep 20px.
  test.describe("on a phone", () => {
    test.use({ hasTouch: true });
    test("the dismiss control is a 44px tap target that dismisses only its toast", async ({ page }) => {
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(String(error)));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const push = (id: string) => page.evaluate(toastId => (window as any).__alerts.push({
        id: toastId, level: "error", title: `Alert ${toastId}`, message: "detail", timeoutMs: 0, dismissible: true,
      }), id);
      const dismiss = (id: string) => page.locator(`.toast-alerts__item[data-toast-id="${id}"] .toast-alerts__dismiss`);
      await page.setViewportSize({ width: 601, height: 900 });
      await push("desktop");
      expect((await dismiss("desktop").boundingBox())!.width, "601px keeps the desktop width").toBe(20);
      const layout = (id: string) => page.locator(`.toast-alerts__item[data-toast-id="${id}"]`).evaluate(item => {
        const box = (selector: string) => item.querySelector(selector)!.getBoundingClientRect();
        const outer = item.getBoundingClientRect();
        const close = box(".toast-alerts__dismiss");
        return { gap: box(".toast-alerts__message").top - box(".toast-alerts__title").bottom,
          inside: close.top >= outer.top && close.bottom <= outer.bottom && close.right <= outer.right };
      });
      const desktopGap = (await layout("desktop")).gap;
      for (const width of [320, 390]) {
        await page.setViewportSize({ width, height: 844 });
        await push(`keep-${width}`);
        await push(`phone-${width}`);
        const box = (await dismiss(`phone-${width}`).boundingBox())!;
        expect(box.width, `dismiss width at ${width}px`).toBeGreaterThanOrEqual(44);
        expect(box.height, `dismiss height at ${width}px`).toBeGreaterThanOrEqual(44);
        await expect(dismiss(`phone-${width}`)).toBeInViewport({ ratio: 1 });
        const phone = await layout(`phone-${width}`);
        expect(phone.inside, "the whole 44px target stays inside its toast").toBe(true);
        expect(phone.gap, "the message stays as close to the title as on desktop").toBeLessThanOrEqual(desktopGap + 1);
        await dismiss(`phone-${width}`).tap();
        await expect(page.locator(`.toast-alerts__item[data-toast-id="phone-${width}"]`)).toHaveCount(0);
        await expect(page.locator(`.toast-alerts__item[data-toast-id="keep-${width}"]`)).toHaveCount(1);
      }
      expect(errors, "zero console/page errors").toEqual([]);
    });
  });

  test("sticky alerts and the update CTA remain visible and operable without overlap", async ({
    page,
  }) => {
    await page.evaluate(async () => {
      const mod = await import("/update-cta.js");
      (window as any).__updateSent = [];
      const controller = (mod as any).createUpdateCtaController({
        document,
        send: (message: unknown) => (window as any).__updateSent.push(message),
        setVersionState: () => {},
      });
      controller.showAvailable("9.99.0");
      (window as any).__updateController = controller;
      (window as any).__alerts.push({
        id: "sticky-error",
        level: "error",
        title: "Background task failed",
        message: "Open the affected work item for details.",
        timeoutMs: 0,
        dismissible: true,
        onActivate: () => (window as any).__activated.push("sticky-error"),
      });
    });

    const host = page.locator("#operator-notice-stack");
    const alerts = page.locator(".toast-alerts");
    const shell = page.locator("#update-cta-shell");
    await expect(host).toBeVisible();
    await expect(alerts).toHaveAttribute("role", "status");
    await expect(shell).toBeVisible();

    const ownership = await page.evaluate(() => {
      const hostNode = document.getElementById("operator-notice-stack")!;
      const alertsNode = document.querySelector(".toast-alerts")!;
      const shellNode = document.getElementById("update-cta-shell")!;
      return {
        children: Array.from(hostNode.children).map((node) => node.className),
        alertsParentIsHost: alertsNode.parentElement === hostNode,
        shellParentIsHost: shellNode.parentElement === hostNode,
        ctaInsideAlertsLiveRegion: alertsNode.contains(shellNode),
        hostRole: hostNode.getAttribute("role"),
        hostAriaLive: hostNode.getAttribute("aria-live"),
      };
    });
    expect(ownership.alertsParentIsHost).toBe(true);
    expect(ownership.shellParentIsHost).toBe(true);
    expect(ownership.children).toEqual(["toast-alerts", "update-cta-shell"]);
    expect(ownership.ctaInsideAlertsLiveRegion).toBe(false);
    expect(ownership.hostRole).toBeNull();
    expect(ownership.hostAriaLive).toBeNull();

    await page.evaluate(() => {
      const alerts = (window as any).__alerts;
      alerts.push({
        id: "queued-warning",
        level: "warn",
        title: "Needs input",
        timeoutMs: 0,
        dismissible: true,
      });
      alerts.push({
        id: "queued-info",
        level: "info",
        title: "Board reply",
        timeoutMs: 0,
        dismissible: true,
      });
      alerts.push({
        id: "queued-warning",
        level: "warn",
        title: "Needs input (updated)",
        timeoutMs: 0,
        dismissible: true,
      });
    });
    const alertItems = page.locator(".toast-alerts__item");
    await expect(alertItems).toHaveCount(3);
    await expect(page.locator('#update-cta')).toBeVisible();

    for (const viewport of [
      { width: 1280, height: 900 },
      { width: 390, height: 844 },
    ]) {
      await page.setViewportSize(viewport);
      const geometry = await page.evaluate(() => {
        const alertBoxes = Array.from(document.querySelectorAll(".toast-alerts__item"))
          .map((node) => node.getBoundingClientRect());
        const ctaBox = document
          .getElementById("update-cta-shell")!
          .getBoundingClientRect();
        const overlapAreas = alertBoxes.map((alertBox) => {
          const overlapWidth = Math.max(
            0,
            Math.min(alertBox.right, ctaBox.right) - Math.max(alertBox.left, ctaBox.left),
          );
          const overlapHeight = Math.max(
            0,
            Math.min(alertBox.bottom, ctaBox.bottom) - Math.max(alertBox.top, ctaBox.top),
          );
          return overlapWidth * overlapHeight;
        });
        return {
          maxOverlapArea: Math.max(...overlapAreas),
          scrollWidth: document.documentElement.scrollWidth,
          viewportWidth: window.innerWidth,
          maxAlertBottom: Math.max(...alertBoxes.map((box) => box.bottom)),
          ctaTop: ctaBox.top,
        };
      });
      expect(geometry.maxOverlapArea).toBe(0);
      expect(geometry.maxAlertBottom).toBeLessThanOrEqual(geometry.ctaTop);
      expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.viewportWidth);
    }

    const stickyAlert = page.locator(
      '.toast-alerts__item[data-toast-id="sticky-error"]',
    );
    await stickyAlert.focus();
    await expect(stickyAlert).toBeFocused();
    expect(await stickyAlert.evaluate((node) => node.matches(":focus-visible"))).toBe(true);
    await page.locator("#update-cta").focus();
    await expect(page.locator("#update-cta")).toBeFocused();
    expect(
      await page.locator("#update-cta").evaluate((node) => node.matches(":focus-visible")),
    ).toBe(true);

    await page.evaluate(() => (window as any).__alerts.dismiss("queued-info"));
    await expect(alertItems).toHaveCount(2);
    await expect(page.locator("#update-cta")).toBeVisible();

    await stickyAlert.click();
    expect(await page.evaluate(() => (window as any).__activated)).toContain("sticky-error");
    await expect(page.locator('.toast-alerts__item[data-toast-id="sticky-error"]')).toHaveCount(0);

    await page.evaluate(() =>
      (window as any).__alerts.push({
        id: "dismiss-error",
        level: "error",
        title: "Dismiss me",
        timeoutMs: 0,
        dismissible: true,
      }),
    );
    await page.locator('.toast-alerts__item[data-toast-id="dismiss-error"] .toast-alerts__dismiss').click();
    await expect(page.locator('.toast-alerts__item[data-toast-id="dismiss-error"]')).toHaveCount(0);

    await page.locator("[data-update-cta-dismiss]").click();
    await expect(shell).toHaveCount(0);
    await page.evaluate(() => (window as any).__updateController.showAvailable("9.99.0"));
    await page.locator("#update-cta").click();
    expect(await page.evaluate(() => (window as any).__updateSent)).toContainEqual({
      kind: "apply_update_start",
    });
    await expect(page.locator('#update-modal[data-state="downloading"]')).toBeVisible();
  });

  test("dedup by id replaces; onActivate jumps then dismisses", async ({ page }) => {
    await page.evaluate(() => {
      const s = (window as any).__alerts;
      s.push({ id: "attention-w1", level: "warn", title: "first" });
      s.push({ id: "attention-w1", level: "error", title: "second", dismissible: true });
    });
    const attention = page.locator('.toast-alerts__item[data-toast-id="attention-w1"]');
    await expect(attention).toHaveCount(1, { timeout: 1000 });
    await expect(attention).toContainText("second");
    await expect(attention).toHaveAttribute("data-level", "error");

    await page.evaluate(() =>
      (window as any).__alerts.push({
        id: "agent-completion",
        title: "Done",
        dismissible: false,
        onActivate: () => (window as any).__activated.push("completion"),
      }),
    );
    await page.locator('.toast-alerts__item[data-toast-id="agent-completion"]').click();
    expect(await page.evaluate(() => (window as any).__activated)).toContain("completion");
  });
});

/* SPEC #3206 v2 — notification center. Boots the full frontend against a
 * fixture WebSocket (one Issue window, monitor status) and drives backend
 * events through the app's `__gwt_test_inject` hook, which feeds receive()
 * exactly like a WebSocket frame. */
async function installNotificationCenterBackend(page, theme: "dark" | "light") {
  await page.addInitScript((selectedTheme: string) => {
    localStorage.setItem("gwt:ui:theme", selectedTheme);
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-issue",
            title: "Fixture Project",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                {
                  id: "issue-kanban",
                  title: "Issue",
                  preset: "issue",
                  geometry: { x: 40, y: 60, width: 1100, height: 700 },
                  z_index: 1,
                  status: "running",
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
                },
              ],
            },
          },
        ],
        active_tab_id: "tab-issue",
        recent_projects: [],
      },
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
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }
      emit(payload: unknown) {
        this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) }));
      }
      send(raw: string) {
        const message = JSON.parse(raw);
        if (message.kind === "frontend_ready") {
          this.emit(workspaceState);
          return;
        }
        if (message.kind === "list_issue_monitor") {
          this.emit({
            kind: "issue_monitor_status",
            status: {
              enabled: false,
              state: "disabled",
              queue_len: 0,
              active_count: 0,
              max_active_agents: 1,
              total_candidates: 1,
              autonomous_mode: false,
              launch_profile_source: "saved",
              launch_profile_summary: "codex / host",
            },
          });
          return;
        }
        if (message.kind === "load_knowledge_bridge") {
          this.emit({
            kind: "knowledge_entries",
            id: message.id,
            knowledge_kind: message.knowledge_kind,
            request_id: message.request_id,
            entries: [
              {
                number: 3206,
                title: "Notification center",
                state: "open",
                meta: "fixture",
                labels: ["gwt-spec"],
                linked_branch_count: 0,
                match_score: 100,
                phase: "implementation",
                has_unknown_phase: false,
                is_spec: true,
                monitor_state: "queued",
                queue_position: 1,
                exclusion_reason: null,
              },
            ],
            selected_number: null,
            empty_message: "",
            refresh_enabled: true,
          });
        }
      }
      close() {
        this.readyState = FixtureWebSocket.CLOSED;
      }
    }
    // @ts-ignore
    window.WebSocket = FixtureWebSocket;
  }, theme);
}

function inject(page, payload: Record<string, unknown>) {
  return page.evaluate((detail) => {
    window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail }));
  }, payload);
}

test.describe("notification center (real browser, SPEC #3206 v2)", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  let pageErrors: string[] = [];

  test.beforeEach(async ({ page }, testInfo) => {
    pageErrors = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") pageErrors.push(message.text());
    });
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await installNotificationCenterBackend(page, theme);
    await installEmbeddedRoutes(page);
    await page.goto(APP_URL);
    await expect(page.locator("#op-notifications-button")).toBeVisible();
    await expect(page.locator(".surface-knowledge .issue-bridge-root")).toBeVisible();
  });

  test.afterEach(() => {
    expect(pageErrors).toEqual([]);
  });

  test("the top-right log region is gone; autonomous events land in the bell badge and the drawer (newest on top, scrollable)", async ({
    page,
  }) => {
    const bell = page.locator("#op-notifications-button");
    const badge = bell.locator(".op-rail__badge");
    await expect(badge).toBeHidden();

    for (let i = 0; i < 12; i += 1) {
      await inject(page, {
        kind: "issue_monitor_toast",
        level: i === 11 ? "error" : i % 2 ? "success" : "info",
        message: `autonomous event ${i}`,
        issue_number: 3000 + i,
      });
    }

    // FR-012 / Sc 6: no floating top-right region anywhere in the page.
    await expect(page.locator(".autonomous-notifications")).toHaveCount(0);
    // FR-009 / Sc 5: unread badge with error emphasis while the drawer is closed.
    await expect(badge).toBeVisible();
    await expect(badge).toHaveText("12");
    await expect(badge).toHaveAttribute("data-has-error", "true");
    await expect(bell).toHaveAttribute("aria-label", /12 unread, includes errors/);

    await bell.click();
    const drawer = page.locator("#notification-center");
    await expect(drawer).toHaveAttribute("data-open", "true");
    await expect(drawer).toBeVisible();
    await expect(bell).toHaveAttribute("aria-expanded", "true");
    // FR-014: opening reads everything.
    await expect(badge).toBeHidden();

    const rows = drawer.locator(".notification-center__item");
    await expect(rows).toHaveCount(12);
    await expect(rows.first()).toContainText("Issues #3011");
    await expect(rows.first()).toContainText("autonomous event 11");
    await expect(rows.first()).toHaveAttribute("data-level", "error");
    await expect(rows.last()).toContainText("#3000");

    // Issue #3979 — triage rows. Resolved in the real engine so the named grid
    // areas, the per-severity marker shapes and the color-mix tints are all
    // exercised in whichever theme this project runs.
    await expect(rows.first()).toHaveAttribute("data-severity", "critical");
    const marker = rows.first().locator(".notification-center__severity");
    await expect(marker).toHaveAttribute("data-severity", "critical");
    await expect(marker).toHaveAttribute("aria-label", "Critical");
    await expect(marker).toHaveText("!");

    const triage = await page.evaluate(() => {
      const markerFor = (severity: string) =>
        document.querySelector(
          `.notification-center__item[data-severity="${severity}"] .notification-center__severity`,
        ) as HTMLElement;
      const critical = markerFor("critical");
      const row = critical.closest(".notification-center__item") as HTMLElement;
      const title = row.querySelector(".notification-center__title") as HTMLElement;
      const time = row.querySelector(".notification-center__time") as HTMLElement;
      const box = (el: HTMLElement) => el.getBoundingClientRect();
      return {
        severities: [...document.querySelectorAll(".notification-center__item")].map(
          (item) => (item as HTMLElement).dataset.severity,
        ),
        radii: ["critical", "good", "neutral"].map(
          (severity) => getComputedStyle(markerFor(severity)).borderTopLeftRadius,
        ),
        glyphs: ["critical", "good", "neutral"].map(
          (severity) => markerFor(severity).textContent?.trim(),
        ),
        markerLeadsTitle: box(critical).left < box(title).left,
        timeFollowsTitle: box(time).left > box(title).left,
        titleAndTimeShareALine: Math.abs(box(title).top - box(time).top) < box(title).height,
        messageBelowTitle:
          box(row.querySelector(".notification-center__message") as HTMLElement).top
          >= box(title).bottom,
        stripe: getComputedStyle(row).borderLeftColor,
      };
    });
    expect(new Set(triage.severities)).toEqual(new Set(["critical", "good", "neutral"]));
    expect(new Set(triage.radii).size).toBeGreaterThanOrEqual(2);
    expect(new Set(triage.glyphs).size).toBe(3);
    expect(triage.markerLeadsTitle).toBe(true);
    expect(triage.timeFollowsTitle).toBe(true);
    expect(triage.titleAndTimeShareALine).toBe(true);
    expect(triage.messageBelowTitle).toBe(true);
    expect(triage.stripe).not.toBe("");

    // The history scrolls inside the drawer body instead of growing the page.
    const scroll = await page.evaluate(() => {
      const body = document.querySelector("#notification-center .notification-center__body") as HTMLElement;
      const drawerNode = document.getElementById("notification-center") as HTMLElement;
      const styles = getComputedStyle(body);
      return {
        overflowY: styles.overflowY,
        drawerPosition: getComputedStyle(drawerNode).position,
        drawerZ: getComputedStyle(drawerNode).zIndex,
        noticeStackZ: getComputedStyle(document.getElementById("operator-notice-stack") as HTMLElement).zIndex,
        pageScrollHeight: document.documentElement.scrollHeight,
        viewportHeight: window.innerHeight,
      };
    });
    expect(scroll.overflowY).toBe("auto");
    expect(scroll.drawerPosition).toBe("fixed");
    // FR-015: persistent drawer never outranks the transient notice stack.
    expect(Number(scroll.drawerZ)).toBeLessThan(Number(scroll.noticeStackZ));
    expect(scroll.pageScrollHeight).toBeLessThanOrEqual(scroll.viewportHeight);

    // Esc closes and returns aria-expanded.
    await page.keyboard.press("Escape");
    await expect(drawer).toHaveAttribute("data-open", "false");
    await expect(bell).toHaveAttribute("aria-expanded", "false");
  });

  test("per-row dismiss and clear-all work on real clicks; the backdrop closes the drawer", async ({ page }) => {
    await inject(page, { kind: "issue_monitor_toast", level: "info", message: "first", issue_number: 1 });
    await inject(page, { kind: "issue_monitor_toast", level: "warn", message: "second", issue_number: 2 });
    await page.locator("#op-notifications-button").click();
    const drawer = page.locator("#notification-center");
    const rows = drawer.locator(".notification-center__item");
    await expect(rows).toHaveCount(2);

    await rows.first().locator(".notification-center__dismiss").click();
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText("first");

    await drawer.locator(".notification-center__clear").click();
    await expect(rows).toHaveCount(0);
    await expect(drawer.locator(".notification-center__empty")).toBeVisible();

    await page.locator("#notification-center-backdrop").click({ position: { x: 20, y: 20 } });
    await expect(drawer).toHaveAttribute("data-open", "false");
  });

  test("FR-017: an Issues error is read only in the notification center, never in the Issue window", async ({
    page,
  }) => {
    const issueWindow = page.locator(".surface-knowledge .issue-bridge-root");
    // User ruling 2026-09-04: errors are read in ONE place. The Issue window
    // carries neither the old red banner nor any indicator of its own.
    await expect(issueWindow.locator(".knowledge-monitor-error")).toHaveCount(0);
    await expect(issueWindow.locator(".surface-error-indicator")).toHaveCount(0);

    const status = (lastError: string | null, recordedAt = "2026-11-09T00:00:00Z") => ({
      kind: "issue_monitor_status",
      status: {
        enabled: true,
        state: lastError ? "error" : "idle",
        queue_len: 0,
        active_count: 0,
        max_active_agents: 1,
        total_candidates: 1,
        autonomous_mode: false,
        launch_profile_source: "saved",
        launch_profile_summary: "codex / host",
        last_error: lastError,
        last_error_source: lastError ? "scan_failure" : null,
        last_scan_error: lastError ? { message: lastError, phase: "candidate-load", recorded_at: recordedAt } : null,
      },
    });

    await inject(page, status("issue #3785: gh issue list: github_rate_limited (retry)"));
    const badge = page.locator("#op-notifications-button .op-rail__badge");
    await expect(badge).toHaveText("1");
    await expect(badge).toHaveAttribute("data-has-error", "true");
    // Still nothing rendered inside the Issue window.
    await expect(issueWindow.locator(".surface-error-indicator")).toHaveCount(0);
    await expect(issueWindow.locator(".knowledge-status.error")).toHaveCount(0);

    // A later scan failure with the same cause updates one row to x2.
    await inject(page, status("issue #3785: gh issue list: github_rate_limited (retry)", "2026-11-09T00:01:00Z"));
    await page.locator("#op-notifications-button").click();
    const drawer = page.locator("#notification-center");
    await expect(drawer).toHaveAttribute("data-open", "true");
    const row = drawer.locator('.notification-center__item[data-error-key^="issue-monitor:"]').filter({ hasText: "issue #3785: gh issue list: github_rate_limited (retry)" });
    await expect(row).toHaveCount(1);
    await expect(row).toHaveAttribute("data-level", "error");
    await expect(row.locator(".notification-center__count")).toHaveText("\u00d72");
    await expect(row).toContainText("(retry)");
    await page.keyboard.press("Escape");

    // Recovery: the row falls to read/resolved and the badge clears.
    await inject(page, status(null));
    await expect(row).toHaveAttribute("data-resolved", "true");
    await expect(badge).toBeHidden();

    const rendered = await row.evaluate((rowNode) => {
      return {
        theme: document.documentElement.getAttribute("data-theme"),
        rim: getComputedStyle(rowNode).borderLeftColor,
      };
    });
    expect(rendered.rim).not.toBe("");
    expect(["dark", "light", null]).toContain(rendered.theme);
  });
});
