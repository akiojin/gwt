/* SPEC-2009 Phase 3 - Branch Cleanup E2E coverage.
 *
 * Exercises the embedded Branches window with a deterministic WebSocket
 * backend so the cleanup confirmation, force-delete option, progress stream,
 * and result modal are verified without touching a real Git repository.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { gotoLiveGwt } from "./_helpers/live-gwt";

// SPEC #5016 AC11: exercise the real app receiver and its synchronous view.
test("branch_entries shared state preserves count ordering and stale isolation", async ({ page }, info) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = info.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  if (!base) await installEmbeddedRoutes(page);
  await installBranchCleanupBackend(page);
  await page.goto(base ? new URL(new URL(APP_URL).pathname, base).toString() : APP_URL);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  const control = page.locator("#op-strip-issue-monitor");
  await control.focus();
  await page.evaluate(() => {
    const shell = (window as any).__operatorShell;
    const apply = shell.applyTelemetryCounts;
    (window as any).__branchCounts = [];
    shell.applyTelemetryCounts = (counts: any) => {
      if ("branches" in counts) (window as any).__branchCounts.push(counts.branches);
      return apply(counts);
    };
  });
  const emit = (id: string, loadId: number, count: number) => page.evaluate(({ id, loadId, count }) => {
    (window as any).__branchCleanupFixture.emit({ kind: "branch_entries", id, load_id: loadId,
      phase: "hydrated", entries: Array.from({ length: count }, (_, index) => ({
        name: `work/fixture-${index}`, scope: "local", cleanup_ready: false,
      })),
    });
  }, { id, loadId, count });
  const count = page.locator("#op-strip-branches");
  await emit("branches-1", 10, 2);
  await expect(count).toHaveText("2");
  await emit("branches-1", 11, 3);
  await expect(count).toHaveText("3");
  await emit("branches-1", 10, 1); // stale: must not publish a receipt
  // This following event is a FIFO marker and has an independent load sequence.
  await emit("branches-other", 1, 4);
  await expect(count).toHaveText("4");
  expect(await page.evaluate(() => (window as any).__branchCounts)).toEqual([2, 3, 4]);
  await expect(control).toBeFocused();
  await expect(page.locator(".render-degradation-banner")).toHaveCount(0);
  const screenshot = info.outputPath(`branch-entries-${theme}.png`);
  await page.screenshot({ path: screenshot });
  await info.attach("branch-entries", { path: screenshot, contentType: "image/png" });
  expect(errors).toEqual([]);
});

// SPEC #5016 AC11: receive receipts drive the existing list/modal appliers.
test("branch_error receipts preserve branch controls and prioritize running cleanup", async ({ page }, info) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = info.project.name.includes("light") ? "light" : "dark";
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  if (base) {
    await gotoLiveGwt(page, base, { suppressProjectSurfaces: true });
  } else {
    await installEmbeddedRoutes(page);
    await installBranchCleanupBackend(page);
    await page.goto(APP_URL);
  }
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  const style = await page.addStyleTag({ content: `
    body > :not(#app):not(#branch-error-host):not(script), #app > :not(#branch-cleanup-modal) { display: none !important; }
    #app { display: contents !important; }
    #branch-error-host { display: grid; grid-template-columns: 1fr 1fr; gap: var(--space-3); padding: var(--space-3); height: 680px; background: var(--color-canvas); color: var(--color-text); }
    #branch-error-host > section { position: relative; min-width: 0; border: 1px solid var(--color-border); background: var(--color-surface); }
    #branch-error-host .window-body { position: relative; inset: auto; height: 100%; }
  ` });
  try {
    // The list has no navigation path today. Mount the checkout's production
    // factory and markup directly, without reviving the three legacy fixmes.
    await page.evaluate(async () => {
      const { createBranchesCleanupSurface } = await import("/branches-cleanup-surface.js");
      const host = document.createElement("main");
      host.id = "branch-error-host";
      document.body.append(host);
      const windowMap = new Map();
      for (const id of ["branch-error-fresh", "branch-error-loading"]) {
        const root = document.createElement("section");
        root.dataset.windowId = id;
        const body = document.createElement("div");
        body.className = "window-body";
        root.append(body);
        host.append(root);
        windowMap.set(id, root);
      }
      const settled: Array<{ prefix: string; loading: boolean }> = [];
      const noop = () => {};
      const surface = createBranchesCleanupSurface({
        send: noop,
        createNode: (tag: string, className = "", text?: string) => {
          const node = document.createElement(tag);
          node.className = className;
          if (text !== undefined) node.textContent = text;
          return node;
        },
        windowMap, focusWindowLocally: noop, sendWindowFocus: noop,
        branchCleanupModal: document.querySelector("#branch-cleanup-modal"),
        branchCleanupDialog: document.querySelector("#branch-cleanup-modal [role='dialog']"),
        launchPending: { settleWhere: (prefix: string) => settled.push({
          prefix, loading: surface.ensureBranchListState("branch-error-fresh").loading,
        }) },
        visibleBounds: () => ({ x: 0, y: 0, width: 1440, height: 900 }),
        getActiveWorkProjection: () => null, renderWorkspaceWindows: noop,
      });
      for (const [id, root] of windowMap) {
        surface.mountBranchesWindow({ id }, root.querySelector(".window-body"));
      }
      const state = surface.ensureBranchListState("branch-error-fresh");
      state.entries = [{ name: "work/retained", scope: "local", is_head: false,
        ahead: 0, behind: 0, cleanup_ready: true, cleanup: {
          availability: "safe", execution_branch: "work/retained", risks: [],
          upstream: null, merge_target: { kind: "develop", reference: "origin/develop" },
        } }];
      state.receivedFreshEntries = true;
      state.loading = false;
      surface.renderBranches("branch-error-fresh");
      (window as any).__branchError = { surface, state, settled, host,
        row: host.querySelector(".branch-row"),
        literal: '<img src=x onerror="window.__branchErrorExecuted = true">' };
    });
    const fresh = page.locator("[data-window-id='branch-error-fresh']");
    const loading = page.locator("[data-window-id='branch-error-loading']");
    const row = fresh.locator(".branch-row");
    await expect(row).toBeVisible();
    await row.focus();
    const immediate = await page.evaluate(() => {
      const f = (window as any).__branchError;
      f.state.loading = true;
      f.surface.applyBranchCleanupReceiveEvent({ kind: "branch_error", id: "branch-error-fresh", message: f.literal });
      return { loading: f.state.loading, error: f.state.error, settled: f.settled.slice(),
        retained: f.row === f.host.querySelector(".branch-row"), focused: document.activeElement === f.row,
        otherLoading: f.surface.ensureBranchListState("branch-error-loading").loading };
    });
    expect(immediate).toEqual({ loading: false, error: "", settled: [{ prefix: "branch:", loading: true }],
      retained: true, focused: true, otherLoading: true });
    const literal = await page.evaluate(() => (window as any).__branchError.literal);
    await expect(fresh.locator(".branch-notice-detail")).toHaveText(literal);
    await expect(row).toBeFocused();
    await expect(loading.locator(".branch-notice")).toBeHidden();

    // An equal-valued receipt still applies: its pending/notice effect must
    // not be suppressed by a selector that compares only the message text.
    const repeated = await page.evaluate(() => {
      const f = (window as any).__branchError;
      f.state.notice = "";
      f.state.loading = true;
      f.surface.renderBranches("branch-error-fresh");
      f.surface.applyBranchCleanupReceiveEvent({ kind: "branch_error", id: "branch-error-fresh", message: f.literal });
      return { settled: f.settled.length, loading: f.state.loading,
        retained: f.row === f.host.querySelector(".branch-row") };
    });
    expect(repeated).toEqual({ settled: 2, loading: false, retained: true });
    await expect(fresh.locator(".branch-notice-detail")).toHaveText(literal);
    await expect(row).toBeFocused();

    await page.evaluate(() => {
      const f = (window as any).__branchError;
      f.surface.applyBranchCleanupReceiveEvent({ kind: "branch_error", id: "branch-error-loading", message: f.literal });
    });
    await expect(loading.locator(".branch-notice-title")).toHaveText("Branches unavailable");
    await expect(loading.locator(".branch-empty")).toHaveText(literal);
    await expect(fresh.locator(".branch-notice-title")).toHaveText("Branch notice");
    await expect(row).toBeFocused();

    await row.locator(".branch-cleanup-toggle").click();
    await fresh.getByRole("button", { name: "Clean Up (1)" }).click();
    const modal = page.locator("#branch-cleanup-modal");
    await modal.getByRole("button", { name: "Run cleanup" }).click();
    await expect(modal.locator("h2")).toHaveText("Cleaning up branches");
    const failure = await page.evaluate(() => {
      const f = (window as any).__branchError;
      f.state.loading = true;
      f.surface.applyBranchCleanupReceiveEvent({ kind: "branch_error", id: "branch-error-fresh", message: f.literal });
      return { loading: f.state.loading, error: f.state.error, notice: f.state.notice,
        stage: f.surface.readBranchCleanupState("branch-error-fresh").stage,
        retained: f.row === f.host.querySelector(".branch-row"), settled: f.settled.length };
    });
    expect(failure).toEqual({ loading: false, error: "", notice: "", stage: "result", retained: true, settled: 4 });
    await expect(modal.locator("h2")).toHaveText("Cleanup result");
    await expect(modal).toContainText("success 0 · partial 0 · failed 1");
    await expect(modal).toContainText(literal);
    await expect(fresh.locator(".branch-notice")).toBeHidden();
    await expect(page.locator("#branch-error-host img, #branch-cleanup-modal img")).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).__branchErrorExecuted)).toBeUndefined();
    await page.screenshot({ path: info.outputPath(`branch-error-${theme}.png`) });
    expect(errors).toEqual([]);
  } finally {
    await page.evaluate(() => {
      const f = (window as any).__branchError;
      if (!f) return;
      f.surface.markRunningBranchCleanupConnectionInterrupted("branch-error-fresh");
      f.surface.closeBranchCleanupModal();
      f.host.remove();
      delete (window as any).__branchError;
    });
    await style.evaluate(node => node.remove());
  }
});

test.describe("Branch Cleanup E2E", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  // SPEC-2359 W-13 fused the Work / Git Branches tabs into a single Workspace
  // List+Detail surface; preset "branches" now maps to surface "work" and the
  // standalone branches surface markup is unreachable until SPEC-2009 Phase 7
  // #3 (Branches re-integration) restores a navigation path. These three specs
  // drive the old `[data-work-tab='branches']` tab, so they cannot pass on
  // develop today (CI does not run Playwright; the breakage landed silently
  // with the fusion). The state-layer behavior they covered is unit-tested in
  // __tests__/branch-list-state.test.mjs and branch-cleanup.smoke.test.mjs.
  // Re-enable these together with the #3 navigation work.
  test.fixme("shows cleanup progress and sends force filesystem delete", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installBranchCleanupBackend(page);

    await page.goto(APP_URL);

    const branchesWindow = page.locator(".workspace-window.surface-work");
    await expect(branchesWindow).toBeVisible({ timeout: 10_000 });
    await branchesWindow.locator("[data-work-tab='branches']").click();

    const branchSurface = branchesWindow.locator("[data-work-section='branches']");
    await expect(branchSurface).toBeVisible();
    const notice = branchSurface.locator(".branch-notice");
    await expect(notice).toBeHidden();
    await expect(notice).toHaveCSS("display", "none");

    const firstBranch = branchSurface.locator(
      ".branch-row[data-branch-name='work/cleanup-one']",
    );
    const secondBranch = branchSurface.locator(
      ".branch-row[data-branch-name='work/cleanup-two']",
    );
    await expect(firstBranch).toContainText("safe");
    await expect(secondBranch).toContainText("risky");

    await firstBranch.locator(".branch-cleanup-toggle").click();
    await secondBranch.locator(".branch-cleanup-toggle").click();

    const cleanupTrigger = branchSurface.getByRole("button", {
      name: "Clean Up (2)",
    });
    await expect(cleanupTrigger).toBeEnabled();
    await cleanupTrigger.click();

    const modal = page.locator("#branch-cleanup-modal");
    await expect(modal).toHaveClass(/open/);
    await expect(modal.getByRole("dialog", { name: "Branch cleanup" }))
      .toContainText("Delete 2 selected branches.");
    await expect(modal).toContainText("work/cleanup-one");
    await expect(modal).toContainText("work/cleanup-two");

    const forceToggle = modal.getByLabel("Force remove remaining worktree files");
    await expect(forceToggle).not.toBeChecked();
    await forceToggle.check();
    await expect(forceToggle).toBeChecked();

    await modal.getByRole("button", { name: "Run cleanup" }).click();
    await page.waitForFunction(() =>
      window.__branchCleanupCalls?.some(
        (call) => call.kind === "run_branch_cleanup",
      ),
    );

    const cleanupCall = await page.evaluate(() =>
      window.__branchCleanupCalls.find(
        (call) => call.kind === "run_branch_cleanup",
      ),
    );
    expect(cleanupCall).toMatchObject({
      kind: "run_branch_cleanup",
      id: "branches-1",
      branches: ["work/cleanup-one", "work/cleanup-two"],
      delete_remote: false,
      force_filesystem_delete: true,
    });

    await page.evaluate(() => {
      window.__branchCleanupFixture.emit({
        kind: "branch_cleanup_progress",
        id: "branches-1",
        branch: "work/cleanup-one",
        execution_branch: "work/cleanup-one",
        index: 1,
        total: 2,
        phase: "running",
        message: "Removing worktree for work/cleanup-one",
      });
    });
    await expect(modal).toContainText("Cleaning 1 of 2: work/cleanup-one");
    await expect(modal.locator(".branch-cleanup-progress-item.running"))
      .toContainText("work/cleanup-one");
    await expect(modal).toContainText("Removing worktree for work/cleanup-one");

    await page.evaluate(() => {
      window.__branchCleanupFixture.emit({
        kind: "branch_cleanup_progress",
        id: "branches-1",
        branch: "work/cleanup-one",
        execution_branch: "work/cleanup-one",
        index: 1,
        total: 2,
        phase: "done",
        message: "Deleted local branch",
      });
      window.__branchCleanupFixture.emit({
        kind: "branch_cleanup_progress",
        id: "branches-1",
        branch: "work/cleanup-two",
        execution_branch: "work/cleanup-two",
        index: 2,
        total: 2,
        phase: "running",
        message: "Removing worktree for work/cleanup-two",
      });
    });
    await expect(modal).toContainText("Cleaning 2 of 2: work/cleanup-two");
    await expect(modal.locator(".branch-cleanup-progress-item.done"))
      .toContainText("work/cleanup-one");

    await page.evaluate(() => {
      window.__branchCleanupFixture.emit({
        kind: "branch_cleanup_result",
        id: "branches-1",
        results: [
          {
            branch: "work/cleanup-one",
            execution_branch: "work/cleanup-one",
            status: "success",
            message: "Deleted local branch and worktree",
          },
          {
            branch: "work/cleanup-two",
            execution_branch: "work/cleanup-two",
            status: "success",
            message: "Deleted local branch and worktree",
          },
        ],
      });
    });
    await expect(modal).toContainText("Cleanup result");
    await expect(modal).toContainText("success 2 · partial 0 · failed 0");
    await expect(modal.getByRole("button", { name: "Close" })).toBeVisible();
  });

  test.fixme("explains branch detail checks and self-heals after interruption", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installBranchDetailCheckBackend(page);

    await page.goto(APP_URL);

    const branchesWindow = page.locator(".workspace-window.surface-work");
    await expect(branchesWindow).toBeVisible({ timeout: 10_000 });
    await branchesWindow.locator("[data-work-tab='branches']").click();

    const branchSurface = branchesWindow.locator("[data-work-section='branches']");
    const notice = branchSurface.locator(".branch-notice");
    const branch = branchSurface.locator(
      ".branch-row[data-branch-name='work/detail-pending']",
    );

    await expect(notice.locator(".branch-notice-title")).toHaveText(
      "Checking branch details",
    );
    await expect(notice).toContainText(
      "Cleanup selection unlocks after verification.",
    );
    await expect(branch.locator(".branch-cleanup-badge")).toHaveText("checking");
    await expect(branch.locator(".branch-cleanup-toggle")).toHaveAttribute(
      "title",
      "Checking cleanup safety",
    );
    await expect
      .poll(() =>
        notice.evaluate(
          (element) =>
            window.getComputedStyle(element, "::before").animationName,
        ),
      )
      .toContain("branch-detail-check-sweep");
    await expect
      .poll(() =>
        branch
          .locator(".branch-cleanup-badge")
          .evaluate((element) => window.getComputedStyle(element).animationName),
      )
      .toContain("branch-cleanup-checking-pulse");

    await page.evaluate(() => window.__branchDetailFixture.close());

    // SPEC-2009 FR-064/FR-066: the detail check self-heals on reconnect. The
    // Branches surface re-requests automatically and returns to the checking
    // state — there is no alarming "Branch detail check interrupted" banner and
    // never a manual "Refresh to verify cleanup safety" step.
    await expect(notice.locator(".branch-notice-title")).toHaveText(
      "Checking branch details",
    );
    await expect(notice).not.toContainText("Refresh to verify cleanup safety");
    await expect(branch.locator(".branch-cleanup-badge")).toHaveText("checking");
    await expect(branch.locator(".branch-cleanup-toggle")).toHaveAttribute(
      "title",
      "Checking cleanup safety",
    );
  });

  test.fixme("honors reduced motion for branch detail check animations", async ({
    page,
  }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await installEmbeddedRoutes(page);
    await installBranchDetailCheckBackend(page);

    await page.goto(APP_URL);

    const branchesWindow = page.locator(".workspace-window.surface-work");
    await expect(branchesWindow).toBeVisible({ timeout: 10_000 });
    await branchesWindow.locator("[data-work-tab='branches']").click();

    const branchSurface = branchesWindow.locator("[data-work-section='branches']");
    const notice = branchSurface.locator(".branch-notice");
    const branch = branchSurface.locator(
      ".branch-row[data-branch-name='work/detail-pending']",
    );

    await expect(notice.locator(".branch-notice-title")).toHaveText(
      "Checking branch details",
    );
    await expect(branch.locator(".branch-cleanup-badge")).toHaveText("checking");
    await expect
      .poll(() =>
        notice.evaluate(
          (element) =>
            window.getComputedStyle(element, "::before").animationName,
        ),
      )
      .toBe("none");
    await expect
      .poll(() =>
        branch
          .locator(".branch-cleanup-badge")
          .evaluate((element) => window.getComputedStyle(element).animationName),
      )
      .toBe("none");
  });
});

async function installBranchCleanupBackend(page) {
  await page.addInitScript(() => {
    const branchEntries = [
      cleanupBranch({
        name: "work/cleanup-one",
        availability: "safe",
        mergeTarget: { kind: "develop", reference: "origin/develop" },
        risks: [],
      }),
      cleanupBranch({
        name: "work/cleanup-two",
        availability: "risky",
        mergeTarget: { kind: "develop", reference: "origin/develop" },
        risks: ["unmerged"],
      }),
      cleanupBranch({
        name: "develop",
        availability: "blocked",
        isHead: true,
        mergeTarget: null,
        blockedReason: "current_head",
        risks: [],
      }),
    ];

    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Fixture",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                {
                  id: "branches-1",
                  title: "Branches",
                  preset: "branches",
                  geometry: { x: 96, y: 96, width: 760, height: 620 },
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
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };

    const recordedCalls = [];
    window.__branchCleanupCalls = recordedCalls;

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        window.__branchCleanupFixture = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw) {
        const message = JSON.parse(raw);
        recordedCalls.push(message);
        switch (message.kind) {
          case "frontend_ready":
            this.emit(workspaceState);
            break;
          case "load_branches":
            this.emit({
              kind: "branch_entries",
              id: message.id,
              phase: "hydrated",
              entries: branchEntries,
            });
            break;
          default:
            break;
        }
      }

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload) {
        setTimeout(() => {
          this.dispatchEvent(
            new MessageEvent("message", { data: JSON.stringify(payload) }),
          );
        }, 0);
      }
    }

    function cleanupBranch({
      name,
      availability,
      isHead = false,
      mergeTarget,
      blockedReason = null,
      risks,
    }) {
      return {
        name,
        scope: "local",
        is_head: isHead,
        upstream: null,
        ahead: 0,
        behind: 0,
        last_commit_date: "2026-05-20",
        cleanup_ready: true,
        cleanup: {
          availability,
          execution_branch: name,
          merge_target: mergeTarget,
          upstream: null,
          blocked_reason: blockedReason,
          risks,
        },
      };
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}

async function installBranchDetailCheckBackend(page) {
  await page.addInitScript(() => {
    const branchEntries = [
      {
        name: "work/detail-pending",
        scope: "local",
        is_head: false,
        upstream: null,
        ahead: 0,
        behind: 0,
        last_commit_date: "2026-05-20",
        cleanup_ready: false,
        cleanup: null,
      },
    ];

    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Fixture",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                {
                  id: "branches-1",
                  title: "Branches",
                  preset: "branches",
                  geometry: { x: 96, y: 96, width: 760, height: 620 },
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
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        window.__branchDetailFixture = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw) {
        const message = JSON.parse(raw);
        switch (message.kind) {
          case "frontend_ready":
            this.emit(workspaceState);
            break;
          case "load_branches":
            this.emit({
              kind: "branch_entries",
              id: message.id,
              phase: "loading",
              entries: branchEntries,
            });
            break;
          default:
            break;
        }
      }

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload) {
        setTimeout(() => {
          this.dispatchEvent(
            new MessageEvent("message", { data: JSON.stringify(payload) }),
          );
        }, 0);
      }
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}

/* Issue #4433 — cleanup progress/result used to be replied to the originating
 * client only, so a WebView reload mid-cleanup left the new client with no
 * events and the UI rendered a failure that never happened. This spec drives
 * the reachable Issue Other cleanup surface in a real Chromium (dark + light
 * projects) and proves the reconnect path.
 */
test.describe("Issue #4433 branch cleanup reconnect", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("a reconnect mid-cleanup re-syncs instead of showing a false failure", async ({
    page,
  }, testInfo) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(String(error)));

    const configuredUrl = process.env.GWT_PLAYWRIGHT_BASE_URL;
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
    if (configuredUrl) {
      // Exercise real checkout startup first; cleanup itself uses a deterministic
      // backend so this verification cannot delete a repository branch.
      await gotoLiveGwt(page, configuredUrl, { enableTestBridge: true });
      const asset = await page.request.get(new URL("/branches-cleanup-surface.js", configuredUrl).toString());
      expect(asset.ok()).toBe(true);
      expect(await asset.text()).toContain("const cleanupModel = createUiStateStore");
    } else {
      await installEmbeddedRoutes(page);
    }
    await installWorkspaceCleanupBackend(page);
    await page.goto(configuredUrl ? new URL(new URL(APP_URL).pathname, configuredUrl).toString() : APP_URL);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await page.locator(".issue-other-summary").click();

    await expect(page.locator(".workspace-overview-root")).toBeVisible({
      timeout: 10_000,
    });

    const cleanupTrigger = page.locator("[data-action='cleanup-merged-workspaces']");
    await cleanupTrigger.click();
    const modal = page.locator("#branch-cleanup-modal");
    await expect(modal).toHaveClass(/open/);
    await modal.getByRole("button", { name: "Cancel" }).click();
    await expect(cleanupTrigger).toBeFocused();
    await cleanupTrigger.click();
    await expect(modal).toHaveClass(/open/);
    const force = modal.getByLabel("Force remove remaining worktree files");
    await force.focus();
    await force.check();
    await expect(force).toBeFocused();
    await expect(force).toBeChecked();
    await modal.getByRole("button", { name: "Run cleanup" }).click();

    // AC-1: the run is tagged so the backend can address it by operation.
    await page.waitForFunction(() =>
      (window as any).__sent?.some((m: any) => m.kind === "run_branch_cleanup"),
    );
    const runCall = await page.evaluate(() =>
      (window as any).__sent.find((m: any) => m.kind === "run_branch_cleanup"),
    );
    expect(typeof runCall.operation_id).toBe("string");
    expect(runCall.operation_id.length).toBeGreaterThan(0);
    expect(runCall.force_filesystem_delete).toBe(true);

    await page.evaluate((operationId) => {
      (window as any).__cleanupFixture.emit({
        kind: "branch_cleanup_progress",
        id: "workspace-window-1",
        operation_id: operationId,
        branch: "work/cleanup-one",
        execution_branch: "work/cleanup-one",
        index: 1,
        total: 1,
        phase: "running",
        message: "Removing worktree for work/cleanup-one",
      });
    }, runCall.operation_id);
    await expect(modal).toContainText("Cleaning 1 of 1: work/cleanup-one");

    // The socket drops mid-cleanup, exactly as a WebView reload would.
    await page.evaluate(() => (window as any).__cleanupFixture.close());

    // AC-5: the cleanup is still running on the backend, so the UI must not
    // claim it failed.
    await expect(modal).toContainText("Reconnecting to cleanup status");
    await expect(modal).not.toContainText("Cleanup result");
    await expect(modal).not.toContainText("Connection lost");
    await expect(modal.locator("h2")).toHaveText("Cleaning up branches");

    // AC-2: the reconnected client re-subscribes and the fixture replays the
    // operation's final state to it.
    await page.waitForFunction(
      (operationId) =>
        (window as any).__sent?.some(
          (m: any) =>
            m.kind === "sync_branch_cleanup" && m.operation_id === operationId,
        ),
      runCall.operation_id,
      { timeout: 15_000 },
    );
    await expect(modal).toContainText("Cleanup result");
    await expect(modal).toContainText("success 1");
    await expect(modal).toContainText("Deleted local branch and worktree");

    await page.screenshot({ path: testInfo.outputPath("cleanup-shared-result.png") });

    // AC-3: closing the result releases the backend snapshot.
    await modal.getByRole("button", { name: "Close" }).click();
    const clearCall = await page.evaluate(() =>
      (window as any).__sent.find(
        (m: any) => m.kind === "clear_branch_cleanup_status",
      ),
    );
    expect(clearCall).toMatchObject({
      kind: "clear_branch_cleanup_status",
      id: "workspace-window-1",
    });
    expect(clearCall.operation_id).toBe(runCall.operation_id);

    expect(consoleErrors.join(" | ")).toBe("");
    expect(pageErrors.join(" | ")).toBe("");
  });
});

async function installWorkspaceCleanupBackend(page: any) {
  await page.addInitScript(() => {
    (window as any).__sent = [];

    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Fixture Project",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                {
                  id: "workspace-window-1",
                  title: "Workspace",
                  preset: "work",
                  geometry: { x: 80, y: 80, width: 1280, height: 760 },
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
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };

    const projection = {
      kind: "active_work_projection",
      projection: {
        id: "workspace-current",
        title: "Fixture",
        status_category: "active",
        status_text: "",
        summary: "",
        owner: null,
        branch: null,
        workspaces: [],
        active_works: [
          {
            id: "work-cleanup-one",
            linked_issue_numbers: [],
            title: "Merged work",
            status_category: "idle",
            lifecycle_state: "paused",
            branch: "work/cleanup-one",
            worktree_path: "/fixture/work/cleanup-one",
            merged_into_base: true,
            active_agents: 0,
            blocked_agents: 0,
            agents: [],
            cleanup_candidate: {
              branch: "work/cleanup-one",
              remote_delete_available: false,
              reason: "merged",
            },
          },
        ],
        unassigned_agents: [],
        agents: [],
        journal_entries: [],
      },
    };

    const cleanupResult = {
      kind: "branch_cleanup_result",
      id: "workspace-window-1",
      results: [
        {
          branch: "work/cleanup-one",
          execution_branch: "work/cleanup-one",
          status: "success",
          message: "Deleted local branch and worktree",
        },
      ],
    };

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      url: string;
      readyState: number;

      constructor(url: string) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        (window as any).__cleanupFixture = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw: string) {
        const message = JSON.parse(raw);
        (window as any).__sent.push(message);
        if (message.kind === "frontend_ready") {
          this.emit(workspaceState);
          setTimeout(() => this.emit(projection), 0);
          return;
        }
        if (message.kind === "sync_branch_cleanup") {
          // The backend replays the operation's current state to whichever
          // client asks, not just the one that started it.
          this.emit({ ...cleanupResult, operation_id: message.operation_id });
        }
      }

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload: unknown) {
        setTimeout(() => {
          if (this.readyState !== FixtureWebSocket.OPEN) return;
          this.dispatchEvent(
            new MessageEvent("message", { data: JSON.stringify(payload) }),
          );
        }, 0);
      }
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}
