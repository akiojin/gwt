/* SPEC-1939 Phase 13+15 — project-bar Index badge withdrawn. The remaining
 * coverage exercises the dedicated Index window health and search panels
 * using the SPEC-2017 Kanban fixture pattern:
 * the embedded frontend is served via `installEmbeddedRoutes`
 * (`_helpers/embedded-frontend.ts`) and the WebSocket is stubbed with a
 * deterministic backend that emits canned `workspace_state` +
 * `project_index_status` events. The live reconnect case uses a real backend;
 * fixture cases can also fetch binary-served assets when a base URL is set.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { acquireLiveGwtBackendLock, gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

test("live backend replays fresh index health after reconnect and completes offline index search", async ({ page }, testInfo) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  test.skip(!base, "requires browser-check isolated backend");
  await page.setViewportSize({ width: 1440, height: 900 });
  const release = await acquireLiveGwtBackendLock(base!, testInfo);
  const errors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  page.on("pageerror", (error) => errors.push(String(error)));
  let controlledHealth = false;
  await page.routeWebSocket(/\/ws(?:\?|$)/, socket => {
    const server = socket.connectToServer();
    server.onMessage(message => {
      const event = typeof message === "string" ? JSON.parse(message) : null;
      if (!(controlledHealth && event?.kind === "project_index_status")) socket.send(message);
    });
    socket.onMessage(message => server.send(message));
  });
  try {
    await gotoLiveGwt(page, base!, { enableTestBridge: true });
    const { table } = await openIndexHealthPanel(page, true, true);
    await expect(table.locator("tr[data-scope='issues']")).toBeVisible();
    const status = await page.waitForFunction(() => {
      const messages = (window as any).__gwtPlaywrightMessages ?? [];
      return messages.map((entry: any) => entry.payload)
        .findLast((payload: any) => payload.kind === "project_index_status" && payload.status?.scopes?.issues);
    }).then((handle) => handle.jsonValue());
    // A fresh HOME can legitimately have an index awaiting repair. Verify the
    // backend's actual health rather than requiring the user's populated index.
    const cellClass = status.status.scopes.issues.healthy ? 'ready' : 'unhealthy';
    await expect(table.locator(`tr[data-scope='issues'] .settings-index-cell.${cellClass}`)).toBeVisible();
    await sendLiveGwtEvent(page, { kind: "refresh_index_status", project_root: status.project_root });
    await expect(table.locator(`tr[data-scope='issues'] .settings-index-cell.${cellClass}`)).toBeVisible();

    await gotoLiveGwt(page, page.url(), { enableTestBridge: true });
    const reconnected = await openIndexHealthPanel(page, false, true);
    await expect(reconnected.table.locator(`tr[data-scope='issues'] .settings-index-cell.${cellClass}`)).toBeVisible();
    // Keep the real request/reply and receiver path offline: only the wire
    // query is empty, which returns before index runtime/model provisioning.
    // Nonempty result bodies and errors are controlled in the fixture below.
    await page.evaluate(() => {
      const send = WebSocket.prototype.send;
      WebSocket.prototype.send = function (data) {
        if (typeof data === "string") {
          const payload = JSON.parse(data);
          if (payload.kind === "search_project_index") {
            (window as any).__gwtLiveIndexSearchRequest = payload;
            return send.call(this, JSON.stringify({ ...payload, query: "" }));
          }
        }
        return send.call(this, data);
      };
    });
    await reconnected.root.locator("[data-index-tab='search']").click();
    await reconnected.root.locator(".index-search-input").fill("offline receiver acceptance");
    const reply = await page.waitForFunction(() => {
      const request = (window as any).__gwtLiveIndexSearchRequest;
      const messages = (window as any).__gwtPlaywrightMessages ?? [];
      return request && messages.map((entry: any) => entry.payload).findLast((payload: any) =>
        payload.kind === "project_index_search_results" && payload.id === request.id
        && payload.request_id === request.request_id);
    }, undefined, { timeout: 10_000 }).then(handle => handle.jsonValue());
    expect(reply).toMatchObject({ query: "", results: [] });
    // The wire contract omits suggestions when the vector is empty.
    expect(reply.suggestions ?? []).toEqual([]);
    await expect(reconnected.root.locator(".index-search-input")).toHaveValue("offline receiver acceptance");
    await expect(reconnected.root.locator(".index-search-status")).toHaveText("No indexed results");
    await expect(reconnected.root.locator(".index-search-layout")).toHaveAttribute("aria-busy", "false");
    await expect(reconnected.root.locator(".index-run-button")).toBeEnabled();
    // Only search response bodies are controlled; the page and transport above
    // still come from the isolated checkout and exercise a real reconnect.
    controlledHealth = true;
    await page.evaluate(() => {
      const send = WebSocket.prototype.send;
      WebSocket.prototype.send = function(data) {
        if (typeof data === "string") {
          const request = JSON.parse(data);
          if ((window as any).__captureIndexActivation && request.kind === "create_window") {
            (window as any).__capturedIndexActivation = request;
            return;
          }
          if (request.kind === "search_project_index") {
            const root = document.querySelector(`[data-id="${CSS.escape(request.id)}"] .index-search-root`)!;
            (window as any).__indexFrameRequest = { ...request,
              disabledBeforeSend: (root.querySelector(".index-run-button") as HTMLButtonElement).disabled,
              busyBeforeSend: root.querySelector(".index-search-layout")?.getAttribute("aria-busy") };
            return;
          }
        }
        return send.call(this, data);
      };
    });
    await reconnected.root.locator(".index-search-input").hover();
    await reconnected.root.locator(".index-search-input").focus();
    const searchFrame = async (fallback: boolean) => reconnected.root.evaluate(async (root, fallback) => {
      const input = root.querySelector(".index-search-input") as HTMLInputElement;
      const list = root.querySelector(".index-result-list")!;
      const status = root.querySelector(".index-search-status")!;
      const focused = document.activeElement;
      const originalRaf = window.requestAnimationFrame;
      let renders = 0;
      let observer: MutationObserver | undefined;
      let deadline: ReturnType<typeof setTimeout> | undefined;
      if (fallback) window.requestAnimationFrame = () => 0;
      try {
        input.value = fallback ? "frame error" : "frame result";
        input.dispatchEvent(new Event("input", { bubbles: true }));
        (root.querySelector("form") as HTMLFormElement).requestSubmit();
        const request = (window as any).__indexFrameRequest;
        const previousNode = list.firstChild;
        const expected = fallback ? "Controlled search unavailable" : "Frame result";
        const started = performance.now();
        const rendered = new Promise<void>((resolve, reject) => {
          deadline = setTimeout(() => reject(new Error("search response render exceeded one second")), 1000);
          observer = new MutationObserver(records => {
            renders += records.filter(record => record.target === list)
              .flatMap(record => Array.from(record.addedNodes)).length;
            if (fallback ? status.textContent === expected : list.textContent?.includes(expected)) resolve();
          });
          observer.observe(root, { childList: true, subtree: true, characterData: true });
        });
        const event = fallback
          ? { kind: "project_index_search_error", id: request.id, request_id: request.request_id, message: expected }
          : { kind: "project_index_search_results", id: request.id, request_id: request.request_id,
            results: [{ scope: "issues", title: expected, preview: "Committed frame response", target: { kind: "memory" } }], suggestions: [] };
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: event }));
        // Duplicate responses are stale after the first accepted response.
        for (let count = 0; count < 49; count++) window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: event }));
        const unchanged = list.firstChild === previousNode;
        await rendered;
        return { unchanged, renders, elapsedMs: performance.now() - started,
          disabledBeforeSend: request.disabledBeforeSend, busyBeforeSend: request.busyBeforeSend,
          busyAfter: root.querySelector(".index-search-layout")?.getAttribute("aria-busy"),
          focusPreserved: document.activeElement === focused, hoverPreserved: root.matches(":hover") };
      } finally {
        clearTimeout(deadline);
        observer?.disconnect();
        window.requestAnimationFrame = originalRaf;
      }
    }, fallback);
    for (const fallback of [false, true]) {
      const measured = await searchFrame(fallback);
      expect(measured).toMatchObject({ unchanged: true, renders: 1, disabledBeforeSend: true,
        busyBeforeSend: "true", busyAfter: "false", focusPreserved: true, hoverPreserved: true });
      expect(measured.elapsedMs).toBeLessThan(1000);
      await testInfo.attach(`index-search-frame-${fallback ? "fallback" : "normal"}`, {
        body: JSON.stringify(measured), contentType: "application/json" });
      await testInfo.attach(`Index search ${fallback ? "error" : "results"}`, {
        body: await reconnected.root.screenshot(), contentType: "image/png" });
      if (!fallback) {
        const activation = await reconnected.root.evaluate(root => {
          const input = root.querySelector(".index-search-input") as HTMLInputElement;
          input.value = "pending activation";
          input.dispatchEvent(new Event("input", { bubbles: true }));
          (root.querySelector("form") as HTMLFormElement).requestSubmit();
          const request = (window as any).__indexFrameRequest;
          window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
            kind: "project_index_search_results", id: request.id, request_id: request.request_id,
            results: [{ title: "Unseen Board", target: { kind: "board" } }], suggestions: [],
          } }));
          const shown = root.querySelector(".index-detail-title")?.textContent;
          (window as any).__captureIndexActivation = true;
          (window as any).__capturedIndexActivation = null;
          try {
            (root.querySelector("[data-action='open-index-result']") as HTMLButtonElement).click();
            return { shown, attemptedCreate: (window as any).__capturedIndexActivation };
          } finally { (window as any).__captureIndexActivation = false; }
        });
        expect(activation).toEqual({ shown: "Frame result", attemptedCreate: null });
        await expect(reconnected.root.locator(".index-detail-title")).toHaveText("Unseen Board");
      }
    }
    await reconnected.root.locator(".index-search-health-summary").hover();
    await reconnected.root.locator(".index-search-input").focus();
    controlledHealth = true;
    const burst = await reconnected.root.evaluate(async (root, projectRoot) => {
      const summary = root.querySelector(".index-search-health-summary")!;
      const health = root.querySelector(".index-health-table")!;
      const before = summary.innerHTML;
      const focused = document.activeElement;
      const renders = { summary: 0, table: 0 };
      const summaryObserver = new MutationObserver(records => {
        renders.summary += records.reduce((count, record) => count + record.addedNodes.length, 0);
      });
      const tableObserver = new MutationObserver(records => {
        renders.table += records.flatMap(record => Array.from(record.addedNodes))
          .filter(node => node instanceof Element && node.matches("[data-role='index-settings-table']")).length;
      });
      summaryObserver.observe(summary, { childList: true });
      tableObserver.observe(health, { childList: true });
      try {
        for (let count = 1; count <= 50; count++) {
          window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
            kind: "project_index_status", project_root: projectRoot, status: {
              state: "ready", scopes: {
                issues: { healthy: true, document_count: count },
                specs: { healthy: true, document_count: 6 },
              },
            },
          } }));
        }
        // A trailing inactive project must not erase this frame's active data.
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
          kind: "project_index_status", project_root: `${projectRoot}/inactive`,
          status: { state: "error", scopes: {} },
        } }));
        const unchanged = summary.innerHTML === before;
        await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
        return { unchanged, renders, summary: summary.textContent,
          table: health.querySelector("tr[data-scope='issues']")?.textContent,
          focusPreserved: document.activeElement === focused };
      } finally {
        summaryObserver.disconnect();
        tableObserver.disconnect();
      }
    }, status.project_root);
    expect(burst.unchanged).toBe(true);
    expect(burst.renders).toEqual({ summary: 1, table: 1 });
    expect(burst.summary).toContain("2 ready");
    expect(burst.table).toContain("50 docs");
    expect(burst.focusPreserved).toBe(true);
    await testInfo.attach("index-health-frame-burst", { body: JSON.stringify(burst), contentType: "application/json" });
    const fallbackStarted = Date.now();
    await page.evaluate(projectRoot => {
      (window as any).__indexRestoreRaf = window.requestAnimationFrame;
      window.requestAnimationFrame = () => 0;
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
        kind: "project_index_status", project_root: projectRoot,
        status: { state: "ready", scopes: { issues: { healthy: true, document_count: 51 } } },
      } }));
    }, status.project_root);
    try {
      await expect(reconnected.table.locator("tr[data-scope='issues']")).toContainText("51 docs", { timeout: 1000 });
      expect(Date.now() - fallbackStarted).toBeLessThan(1000);
      await expect(reconnected.root.locator(".index-search-health-summary")).toContainText("1 ready");
      await expect(reconnected.root.locator(".index-search-input")).toBeFocused();
    } finally {
      await page.evaluate(() => { window.requestAnimationFrame = (window as any).__indexRestoreRaf; });
    }
    await reconnected.root.locator("[data-index-tab='health']").click();
    await expect(reconnected.table.locator("tr[data-scope='issues'] .ready")).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme",
      testInfo.project.name.includes("light") ? "light" : "dark");
    const screenshot = testInfo.outputPath("index-search-live-reconnect.png");
    await page.screenshot({ path: screenshot });
    await testInfo.attach("Index health frame", { path: screenshot, contentType: "image/png" });
    expect(errors).toEqual([]);
  } finally {
    await release();
  }
});

test.describe("Project Index status surface", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("project-bar Index badge has been withdrawn (Phase 13)", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, { state: "repair_required" });

    await page.goto(APP_URL);

    // The Project shell mounts without the retired Index badge.
    await expect(page.locator("#close-project-button")).toBeVisible({ timeout: 10_000 });
    await expect(page.locator("#index-status")).toHaveCount(0);
  });

  test("Index window Health renders the scope health table from project_index_status (T-IDX-106)", async ({
    page,
  }, testInfo) => {
    const errors: string[] = [];
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("pageerror", error => errors.push(String(error)));
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "repair_required",
      scopes: {
        specs: {
          healthy: false,
          repair_required: true,
          document_count: 5,
          reason: "count_mismatch",
        },
        files: {
          wtAhash: {
            healthy: false,
            repair_required: true,
            document_count: 0,
            reason: "manifest_missing",
          },
        },
      },
      worktrees: {
        wtAhash: { branch: "develop", path: "/abs/wtA" },
      },
    });

    await page.goto(APP_URL);
    const { root, table } = await openIndexHealthPanel(page);
    await expect(page.locator("html")).toHaveAttribute("data-theme",
      testInfo.project.name.includes("light") ? "light" : "dark");
    await expect(table).toBeVisible();

    const specsRow = table.locator("tr[data-scope='specs']");
    await expect(specsRow.locator(".settings-index-cell.unhealthy"))
      .toContainText("count_mismatch");

    const filesRow = table.locator("tr[data-scope='files']");
    await expect(filesRow.locator(".settings-index-cell[data-worktree-hash='wtAhash']"))
      .toContainText("manifest_missing");

    // Worktree column header should reflect the supplied branch label.
    await expect(table.locator("thead th[data-worktree-hash='wtAhash']"))
      .toContainText("develop");

    // The same committed status updates the visible Search summary and the
    // hidden Health table; opening Health must not be the update trigger.
    await root.locator("[data-index-tab='search']").click();
    await page.evaluate(() => window.__gwtFixtureWebSocket.setIndexStatus({
      state: "ready", scopes: {
        issues: { healthy: true, repair_required: false, document_count: 27 },
        specs: { healthy: true, repair_required: false, document_count: 6 },
      }, worktrees: {},
    }));
    await expect(root.locator(".index-search-health-summary")).toContainText("2 ready");
    await expect(table.locator("tr[data-scope='specs'] .settings-index-cell.ready")).toHaveCount(1);
    await root.locator("[data-index-tab='health']").click();
    await expect(table.locator("tr[data-scope='issues'] .settings-index-cell.ready")).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath("index-health-shared-state.png") });
    expect(errors).toEqual([]);
  });

  test("Index window Health tab and refresh button request full status refresh", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "ready",
      scopes: {
        issues: {
          healthy: true,
          repair_required: false,
          document_count: 12,
          reason: "ready",
        },
      },
    });

    await page.goto(APP_URL);
    const { root } = await openIndexHealthPanel(page);
    const afterTabOpen = await readRefreshIndexStatusSends(page);
    expect(afterTabOpen.length).toBeGreaterThanOrEqual(1);
    expect(afterTabOpen.at(-1)).toMatchObject({
      kind: "refresh_index_status",
      project_root: "/fixture",
    });

    await root.locator("[data-action='refresh-index-status']").click();
    const afterManualRefresh = await readRefreshIndexStatusSends(page);
    expect(afterManualRefresh.length).toBeGreaterThan(afterTabOpen.length);
    expect(afterManualRefresh.at(-1)).toMatchObject({
      kind: "refresh_index_status",
      project_root: "/fixture",
    });
  });

  test("Index window Search summarizes degraded health and keeps search controls usable", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "repair_required",
      scopes: {
        issues: {
          healthy: true,
          repair_required: false,
          document_count: 12,
          reason: "ready",
        },
        specs: {
          healthy: false,
          repair_required: true,
          document_count: 5,
          reason: "count_mismatch",
        },
        files: {
          wtAhash: {
            healthy: false,
            repair_required: true,
            document_count: 0,
            reason: "manifest_missing",
          },
        },
      },
      worktrees: {
        wtAhash: { branch: "develop", path: "/abs/wtA" },
      },
    });

    await page.goto(APP_URL);
    const { root, panel } = await openIndexSearchPanel(page);

    const summary = panel.locator("[data-role='index-search-health-summary']");
    await expect(summary).toBeVisible({ timeout: 10_000 });
    await expect(summary).toContainText("1 ready");
    await expect(summary).toContainText("2 degraded");
    await expect(summary).toContainText("specs");
    await expect(summary).toContainText("files · develop");
    await expect(panel.locator(".settings-index-rebuild")).toHaveCount(0);

    await root.locator(".index-search-input").fill("workspace lifecycle");
    await expect(root.locator(".index-run-button")).toBeEnabled();
    await expect(panel.locator(".index-scope-button[data-scope='specs']")).toBeEnabled();

    await summary.locator("[data-action='open-index-health']").click();
    await expect(root.locator("[data-index-panel='health']")).toBeVisible();

    const refreshes = await readRefreshIndexStatusSends(page);
    expect(refreshes.at(-1)).toMatchObject({
      kind: "refresh_index_status",
      project_root: "/fixture",
    });
  });

  test("Index window Search refreshes missing health before showing unavailable status", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "ready",
      scopes: {
        issues: {
          healthy: true,
          repair_required: false,
          document_count: 12,
          reason: "ready",
        },
      },
      worktrees: {},
    }, { emitInitialStatus: false, deferRefreshStatus: true });

    await page.goto(APP_URL);
    const { panel } = await openIndexSearchPanel(page);

    const summary = panel.locator("[data-role='index-search-health-summary']");
    await expect(summary).toBeVisible({ timeout: 10_000 });
    await expect(summary).toContainText("Refreshing status");

    await expect.poll(async () => {
      const refreshes = await readRefreshIndexStatusSends(page);
      return refreshes.length;
    }).toBeGreaterThan(0);

    const refreshes = await readRefreshIndexStatusSends(page);
    expect(refreshes.at(-1)).toMatchObject({
      kind: "refresh_index_status",
      project_root: "/fixture",
    });
    await page.evaluate(() => {
      window.__gwtFixtureWebSocket.flushRefreshStatus();
    });
    await expect(summary).toContainText("1 ready", { timeout: 10_000 });
  });

  test("Index window Health scope-row Rebuild all dispatches without worktree_hash", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "repair_required",
      scopes: {
        issues: {
          healthy: false,
          repair_required: true,
          document_count: 0,
          reason: "manifest_missing",
        },
      },
    });

    await page.goto(APP_URL);
    const { table } = await openIndexHealthPanel(page);
    const issuesRow = table.locator("tr[data-scope='issues']").first();
    await expect(issuesRow).toBeVisible({ timeout: 10_000 });

    // The scope-row Rebuild button lives in the row header (`th`),
    // distinct from the per-cell Rebuild button inside `td`.
    const rebuildAll = issuesRow.locator(".settings-index-rebuild-all[data-scope='issues']");
    await rebuildAll.click();

    const lastRebuild = await page.evaluate(() => {
      const sends = (window.__gwtFixtureWebSocket && window.__gwtFixtureWebSocket.recordedSends) || [];
      return sends
        .map((raw) => {
          try {
            return JSON.parse(raw);
          } catch (e) {
            return null;
          }
        })
        .filter((m) => m && m.kind === "rebuild_index_cell")
        .pop();
    });

    expect(lastRebuild).toMatchObject({
      kind: "rebuild_index_cell",
      project_root: "/fixture",
      scope: "issues",
    });
    // Repo-shared scopes (`issues`, `specs`) must NOT carry a worktree_hash
    // since they are not per-worktree.
    expect(lastRebuild).not.toHaveProperty("worktree_hash");
  });

  test("Index window Health renders the memory scope row and dispatches rebuild_index_cell (SPEC-2805)", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "repair_required",
      scopes: {
        memory: {
          healthy: false,
          repair_required: true,
          document_count: 0,
          reason: "manifest_missing",
        },
      },
    });

    await page.goto(APP_URL);
    const { table } = await openIndexHealthPanel(page);
    const memoryRow = table.locator("tr[data-scope='memory']").first();
    await expect(memoryRow).toBeVisible({ timeout: 10_000 });
    await expect(memoryRow.locator(".settings-index-cell.unhealthy")).toContainText(
      "manifest_missing",
    );

    const rebuildAll = memoryRow.locator(".settings-index-rebuild-all[data-scope='memory']");
    await rebuildAll.click();

    const lastRebuild = await page.evaluate(() => {
      const sends = (window.__gwtFixtureWebSocket && window.__gwtFixtureWebSocket.recordedSends) || [];
      return sends
        .map((raw) => {
          try {
            return JSON.parse(raw);
          } catch (e) {
            return null;
          }
        })
        .filter((m) => m && m.kind === "rebuild_index_cell")
        .pop();
    });

    expect(lastRebuild).toMatchObject({
      kind: "rebuild_index_cell",
      project_root: "/fixture",
      scope: "memory",
    });
    // Memory is repo-scoped just like issues/specs, so worktree_hash must
    // not be included in the dispatched IPC payload.
    expect(lastRebuild).not.toHaveProperty("worktree_hash");
  });

  test("Index window Health shows Work and Reports status and Search opens Reports", async ({ page }, testInfo) => {
    const configuredUrl = process.env.GWT_PLAYWRIGHT_BASE_URL;
    const errors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });
    page.on("pageerror", (error) => errors.push(String(error)));
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await page.addInitScript((value) => localStorage.setItem("gwt:ui:theme", value), theme);
    if (!configuredUrl) await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "ready",
      scopes: {
        works: { healthy: true, repair_required: false, document_count: 7, reason: "ready" },
        reports: { healthy: true, repair_required: false, document_count: 3, reason: "ready" },
      },
    }, { includePm: true });

    const url = configuredUrl
      ? new URL(new URL(APP_URL).pathname, configuredUrl).toString()
      : APP_URL;
    await page.goto(url);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const pm = page.locator(".workspace-window[data-id='pm-fixture']");
    await page.locator("#op-pm-entry").click();
    await pm.getByRole("button", { name: "Execution log", exact: true }).click();
    const { root, table } = await openIndexHealthPanel(page);
    const worksRow = table.locator("tr[data-scope='works']");
    await expect(worksRow.locator(".settings-index-cell.ready")).toContainText("7 docs");
    await worksRow.locator(".settings-index-rebuild-all[data-scope='works']").click();
    const lastRebuild = await page.evaluate(() =>
      window.__gwtFixtureWebSocket.recordedSends.map((raw) => JSON.parse(raw))
        .filter((message) => message.kind === "rebuild_index_cell").pop(),
    );
    expect(lastRebuild).toEqual({ kind: "rebuild_index_cell", project_root: "/fixture", scope: "works" });
    const reportsRow = table.locator("tr[data-scope='reports']");
    await expect(reportsRow.locator(".settings-index-cell.ready")).toContainText("3 docs");
    await reportsRow.locator(".settings-index-rebuild-all").click();
    const reportRebuild = await page.evaluate(() =>
      window.__gwtFixtureWebSocket.recordedSends.map((raw) => JSON.parse(raw))
        .filter((message) => message.kind === "rebuild_index_cell").pop(),
    );
    expect(reportRebuild).toEqual({ kind: "rebuild_index_cell", project_root: "/fixture", scope: "reports" });
    await root.locator("[data-index-tab='search']").click();
    await expect(root.locator("[data-role='index-search-health-summary']")).toContainText("2 ready");
    await expect(root.locator(".index-scope-button[data-scope='reports']")).toHaveAttribute("aria-pressed", "true");
    await root.locator(".index-search-input").fill("release decision");
    const request = await page.waitForFunction(() =>
      window.__gwtFixtureWebSocket.recordedSends.map((raw) => JSON.parse(raw))
        .filter((message) => message.kind === "search_project_index").pop(),
    ).then((handle) => handle.jsonValue());
    expect(request.scopes).toContain("reports");
    await page.evaluate((payload) => window.__gwtFixtureWebSocket.emit(payload), {
      kind: "project_index_search_results", project_root: "/fixture", id: request.id,
      request_id: request.request_id, query: request.query, scope: "reports", suggestions: [],
      results: [{ scope: "reports", title: "Release decision", subtitle: "qa · 2026-10-15T08:00:00Z · Issue #5429",
        preview: "Q: Can we release?\n\nA: Required checks pass.", distance: 0.08,
        target: { kind: "report", report_id: "report-fixture", report_type: "qa", source_kind: "qa",
          created_at: "2026-10-15T08:00:00Z", question: "Can we release?", body: "Required checks pass.", issue_numbers: [5429] } }],
    });
    const result = root.locator(".index-result-row");
    await expect(result).toContainText("qa · 2026-10-15T08:00:00Z · Issue #5429");
    await expect(root.locator(".index-detail-preview")).toContainText("Can we release?");
    await expect(root.locator(".index-detail-preview")).toContainText("Required checks pass.");
    await root.locator("[data-action='open-index-result']").click();
    await expect(pm).toHaveClass(/focused/);
    await expect(pm.getByRole("button", { name: "Reports", exact: true })).toHaveAttribute("aria-pressed", "true");
    await expect(pm.getByRole("log", { name: "PM reports" })).toBeVisible();
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("Index window Search shows animated loading feedback in the results pane", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "ready",
      scopes: {
        issues: { healthy: true, repair_required: false, document_count: 214 },
        specs: { healthy: true, repair_required: false, document_count: 774 },
        memory: { healthy: true, repair_required: false, document_count: 260 },
        board: { healthy: true, repair_required: false, document_count: 1471 },
      },
      worktrees: {},
    });

    await page.goto(APP_URL);
    const { root } = await openIndexSearchPanel(page);
    await root.locator(".index-search-input").fill("memory search loading");

    const loading = root.locator(".index-search-loading");
    await expect(loading).toBeVisible({ timeout: 5_000 });
    await expect(loading).toHaveAttribute("role", "status");
    await expect(loading).toContainText("Searching semantic index");
    await expect(root.locator(".index-search-layout")).toHaveAttribute("aria-busy", "true");

    const animatedDot = loading.locator(".index-search-loading-dot").first();
    await expect(animatedDot).toBeVisible();
    await expect(animatedDot).toHaveCSS("animation-name", /index-search-loading/);
  });

  test("Index window Search all-terms mode separates strict results and suggestions", async ({
    page,
  }, testInfo) => {
    const configuredUrl = process.env.GWT_PLAYWRIGHT_BASE_URL;
    const errors: string[] = [];
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("pageerror", error => errors.push(String(error)));
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
    // Responses remain deterministic; a configured URL serves the actual
    // checkout binary's embedded HTML/modules rather than filesystem routes.
    if (!configuredUrl) await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "ready",
      scopes: {
        specs: { healthy: true, repair_required: false, document_count: 774 },
        discussions: { healthy: true, repair_required: false, document_count: 91 },
      },
      worktrees: {},
    });

    await page.goto(configuredUrl ? new URL(new URL(APP_URL).pathname, configuredUrl).toString() : APP_URL);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const { root } = await openIndexSearchPanel(page);
    await expect(root.locator(".index-search-input")).toHaveAttribute(
      "placeholder",
      "Search by meaning, e.g. work lifecycle",
    );
    await root.locator("[data-match-mode='all_terms']").click();
    await expect(root.locator(".index-search-input")).toHaveAttribute(
      "placeholder",
      "All terms required, e.g. Work discussion",
    );
    await root.locator(".index-scope-button[data-scope='board']").click();
    await expect(root.locator(".index-scope-button[data-scope='board']")).toHaveAttribute("aria-pressed", "false");
    await root.locator(".index-search-input").fill("Workspace volatile");

    const request = await page.waitForFunction(() => {
      const sends = (window.__gwtFixtureWebSocket && window.__gwtFixtureWebSocket.recordedSends) || [];
      return sends
        .map((raw) => {
          try {
            return JSON.parse(raw);
          } catch (e) {
            return null;
          }
        })
        .filter((m) => m && m.kind === "search_project_index")
        .pop();
    });
    const requestPayload = await request.jsonValue();
    expect(requestPayload).toMatchObject({
      kind: "search_project_index",
      match_mode: "all_terms",
      query: "Workspace volatile",
    });
    expect(requestPayload.scopes).not.toContain("board");
    await expect(root.locator(".index-search-layout")).toHaveAttribute("aria-busy", "true");

    await page.evaluate((payload) => {
      window.__gwtFixtureWebSocket.emit(payload);
    }, {
      kind: "project_index_search_results",
      project_root: "/fixture",
      id: requestPayload.id,
      request_id: requestPayload.request_id,
      query: requestPayload.query,
      scope: "all",
      results: [
        {
          scope: "specs",
          title: "Workspace volatility decision",
          subtitle: "SPEC #1939",
          preview: "Workspace is current state; Work is durable.",
          distance: 0.08,
          match_mode: "all_terms",
          matched_terms: ["Workspace", "volatile"],
          missing_terms: [],
        },
      ],
      suggestions: [
        {
          scope: "discussions",
          title: "Workspace naming discussion",
          subtitle: "discussion",
          preview: "Workspace was confusing and Work may be a better durable unit.",
          distance: 0.18,
          match_mode: "all_terms",
          matched_terms: ["Workspace"],
          missing_terms: ["volatile"],
        },
      ],
    });

    await expect(root.locator(".index-search-status")).toContainText(
      "1 strict results · 1 semantic suggestions",
    );
    await expect(root.locator(".index-result-group-label").first()).toContainText("Strict results");
    await expect(root.locator(".index-result-row").first()).toContainText(
      "Workspace volatility decision",
    );
    await expect(root.locator(".index-result-row.is-suggestion")).toContainText(
      "Workspace naming discussion",
    );
    await expect(root.locator(".index-result-row.is-suggestion")).toContainText(
      "Matched: Workspace",
    );
    await root.locator(".index-result-row.is-suggestion").click();
    await expect(root.locator(".index-detail-meta", { hasText: "Missing: volatile" })).toBeVisible();
    await expect(root.locator(".index-detail-title")).toHaveText("Workspace naming discussion");
    await expect(root.locator(".index-search-layout")).toHaveAttribute("aria-busy", "false");
    await root.locator(".index-result-row.is-suggestion").press("ArrowUp");
    await expect(root.locator(".index-detail-title")).toHaveText("Workspace volatility decision");

    await root.locator(".index-search-input").fill("Workspace recovery");
    const failedRequest = await page.waitForFunction(() =>
      window.__gwtFixtureWebSocket.recordedSends.map(raw => JSON.parse(raw))
        .findLast(message => message.kind === "search_project_index" && message.query === "Workspace recovery"),
    ).then(handle => handle.jsonValue());
    await page.evaluate(payload => window.__gwtFixtureWebSocket.emit(payload), {
      kind: "project_index_search_error", id: failedRequest.id,
      request_id: failedRequest.request_id, query: failedRequest.query, message: "Index search fixture unavailable",
    });
    await expect(root.locator(".index-search-status")).toHaveText("Index search fixture unavailable");
    await expect(root.locator(".index-result-list")).toHaveText("Index search fixture unavailable");
    await expect(root.locator(".index-search-layout")).toHaveAttribute("aria-busy", "false");
    await expect(root.locator(".index-run-button")).toBeEnabled();

    await root.locator(".index-search-input").fill("Workspace restored");
    const recoveredRequest = await page.waitForFunction(() =>
      window.__gwtFixtureWebSocket.recordedSends.map(raw => JSON.parse(raw))
        .findLast(message => message.kind === "search_project_index" && message.query === "Workspace restored"),
    ).then(handle => handle.jsonValue());
    await page.evaluate(payload => window.__gwtFixtureWebSocket.emit(payload), {
      kind: "project_index_search_results", id: recoveredRequest.id,
      request_id: recoveredRequest.request_id, query: recoveredRequest.query, suggestions: [],
      results: [{ scope: "specs", title: "Workspace restored result", preview: "Fresh committed response", distance: 0.1 }],
    });
    await expect(root.locator(".index-search-status")).toContainText("1 strict results · 0 semantic suggestions");
    await expect(root.locator(".index-result-row")).toHaveCount(1);
    await expect(root.locator(".index-detail-title")).toHaveText("Workspace restored result");
    await expect(root.locator(".index-detail-preview")).toHaveText("Fresh committed response");
    await expect(root.locator(".index-search-layout")).toHaveAttribute("aria-busy", "false");
    await page.screenshot({ path: testInfo.outputPath("index-search-shared-state.png") });

    const workspace = await page.evaluate(() => window.__gwtFixtureWebSocket.workspaceSnapshot());
    const closed = structuredClone(workspace);
    closed.workspace.tabs[0].workspace.windows = [];
    await page.evaluate(payload => window.__gwtFixtureWebSocket.emit(payload), closed);
    await expect(root).toHaveCount(0);
    await page.evaluate(payload => window.__gwtFixtureWebSocket.emit(payload), workspace);
    await expect(root).toHaveCount(1);
    // Restored windows stay parked until the normal entrypoint selects them.
    await openIndexSearchPanel(page);
    await expect(root.locator(".index-search-input")).toHaveValue("");
    await expect(root.locator(".index-result-row")).toHaveCount(0);
    await expect(root.locator(".index-result-detail")).toHaveText("Select a result");
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("Index window Health per-cell Rebuild dispatches rebuild_index_cell IPC (T-IDX-102/T-IDX-110)", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIndexStatusBackend(page, {
      state: "error",
      scopes: {
        files: {
          wtAhash: {
            healthy: false,
            repair_required: true,
            document_count: 0,
            reason: "rebuild_failed",
          },
        },
      },
      worktrees: {
        wtAhash: { branch: "develop", path: "/abs/wtA" },
      },
    });

    await page.goto(APP_URL);
    const { table } = await openIndexHealthPanel(page);

    // Wait for Index Health to render the row, then click the per-cell
    // Rebuild button.
    const filesCell = table
      .locator("tr[data-scope='files'] .settings-index-cell[data-worktree-hash='wtAhash']")
      .first();
    await expect(filesCell).toBeVisible({ timeout: 10_000 });

    await filesCell.locator(".settings-index-rebuild").click();

    // The fixture WebSocket has captured every send() call; the click
    // must enqueue a `rebuild_index_cell` payload with the right scope +
    // worktree hash.
    const lastRebuild = await page.evaluate(() => {
      const sends = (window.__gwtFixtureWebSocket && window.__gwtFixtureWebSocket.recordedSends) || [];
      return sends
        .map((raw) => {
          try {
            return JSON.parse(raw);
          } catch (e) {
            return null;
          }
        })
        .filter((m) => m && m.kind === "rebuild_index_cell")
        .pop();
    });

    expect(lastRebuild).toMatchObject({
      kind: "rebuild_index_cell",
      project_root: "/fixture",
      scope: "files",
      worktree_hash: "wtAhash",
    });
  });

});

async function openIndexHealthPanel(page, createWindow = true, live = false) {
  await expect(page.locator("#close-project-button")).toBeVisible({ timeout: 10_000 });

  // SPEC-1939 Phase 15: settings target=index is now the compatibility
  // entrypoint for the dedicated Index window, not a Settings tab.
  if (createWindow) await page.evaluate(() => {
    document.dispatchEvent(
      new CustomEvent("settings:open", { detail: { target: "index" }, bubbles: true }),
    );
  });

  const root = page.locator(live
    ? ".workspace-window.surface-index.focused .index-search-root"
    : ".index-search-root").first();
  await expect(root).toBeVisible({ timeout: 10_000 });
  await root.locator("[data-index-tab='health']").click();

  const panel = root.locator("[data-index-panel='health']");
  await expect(panel).toBeVisible({ timeout: 10_000 });
  const table = panel.locator("[data-role='index-settings-table']");
  return { root, panel, table };
}

async function openIndexSearchPanel(page) {
  await expect(page.locator("#close-project-button")).toBeVisible({ timeout: 10_000 });

  await page.evaluate(() => {
    document.dispatchEvent(
      new CustomEvent("settings:open", { detail: { target: "index" }, bubbles: true }),
    );
  });

  const root = page.locator(".index-search-root").first();
  await expect(root).toBeVisible({ timeout: 10_000 });
  await root.locator("[data-index-tab='search']").click();

  const panel = root.locator("[data-index-panel='search']");
  await expect(panel).toBeVisible({ timeout: 10_000 });
  return { root, panel };
}

async function readRefreshIndexStatusSends(page) {
  return page.evaluate(() => {
    const sends =
      (window.__gwtFixtureWebSocket && window.__gwtFixtureWebSocket.recordedSends) || [];
    return sends
      .map((raw) => {
        try {
          return JSON.parse(raw);
        } catch (e) {
          return null;
        }
      })
      .filter((message) => message && message.kind === "refresh_index_status");
  });
}

async function installIndexStatusBackend(page, indexStatus, options = {}) {
  await page.addInitScript(({ indexStatusPayload, optionsPayload }) => {
    const projectRoot = "/fixture";
    const baseTabState = {
      id: "tab-1",
      title: "Fixture Project",
      project_root: projectRoot,
      kind: "git",
      workspace: {
        viewport: { x: 0, y: 0, zoom: 1 },
        windows: optionsPayload.includePm ? [{
          id: "pm-fixture", title: "Project Manager", preset: "agent", is_pm: true,
          session_id: "pm-session", status: "running", persist: true, z_index: 1,
          geometry: { x: 20, y: 20, width: 760, height: 620 },
        }, {
          id: "index-fixture", title: "Index", preset: "index", status: "running", persist: true, z_index: 2,
          geometry: { x: 96, y: 76, width: 720, height: 540 },
        }] : [],
      },
    };
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [JSON.parse(JSON.stringify(baseTabState))],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };
    const projectIndexStatus = {
      kind: "project_index_status",
      project_root: projectRoot,
      status: {
        state: indexStatusPayload.state,
        // Empty detail by default so the formatted title strings from
        // index-status-controller.js (`Auto-rebuild not started`,
        // `Auto-rebuild failed`, etc.) drive the title attribute. Tests
        // that need a specific detail can pass one explicitly.
        detail: indexStatusPayload.detail || "",
        progress: indexStatusPayload.progress || null,
        scopes: indexStatusPayload.scopes || {},
        worktrees: indexStatusPayload.worktrees || {},
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
        // SPEC-1939 T-IDX-102 — record every payload the frontend sends so
        // tests can assert on `rebuild_index_cell` and similar dispatches.
        this.recordedSends = [];
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw) {
        this.recordedSends.push(raw);
        let message;
        try {
          message = JSON.parse(raw);
        } catch (e) {
          return;
        }
        if (message.kind === "frontend_ready") {
          this.emit(workspaceState);
          if (optionsPayload.emitInitialStatus !== false) {
            this.emit(projectIndexStatus);
          }
          return;
        }
        if (message.kind === "refresh_index_status") {
          if (optionsPayload.deferRefreshStatus) {
            this.pendingRefreshStatus = projectIndexStatus;
          } else {
            this.emit(projectIndexStatus);
          }
          return;
        }
        // SPEC-1939 T-IDX-106/Phase 15: simulate the backend create_window
        // behaviour for `preset === "index"` so settings:open target=index →
        // focusOrSpawnPreset("index") can drive a real Index window mount
        // end-to-end. The fixture appends an Index window to the
        // current tab's workspace and re-emits workspace_state.
        if (message.kind === "create_window" && message.preset === "index") {
          const tab = workspaceState.workspace.tabs[0];
          tab.workspace.windows = (tab.workspace.windows || []).concat([
            {
              id: `index-${Date.now()}`,
              title: "Index",
              preset: "index",
              geometry: { x: 96, y: 76, width: 720, height: 540 },
              z_index: tab.workspace.windows.length + 1,
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
          ]);
          this.emit(workspaceState);
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

      flushRefreshStatus() {
        if (!this.pendingRefreshStatus) return;
        const payload = this.pendingRefreshStatus;
        this.pendingRefreshStatus = null;
        this.emit(payload);
      }

      setIndexStatus(status) {
        projectIndexStatus.status = status;
        this.emit(projectIndexStatus);
      }

      workspaceSnapshot() {
        return JSON.parse(JSON.stringify(workspaceState));
      }
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });

    // Expose the most recently constructed FixtureWebSocket on window so
    // transition tests can drive additional `project_index_status` events
    // without needing a second WebSocket. The wrapper guards against a
    // race where the test calls `.emit` before app.js has constructed the
    // WebSocket — in that case `__gwtFixtureWebSocket` is undefined and
    // the test fails fast.
    const originalConstructor = FixtureWebSocket;
    const FixtureWebSocketWithTracking = function (url) {
      const instance = new originalConstructor(url);
      window.__gwtFixtureWebSocket = instance;
      return instance;
    };
    FixtureWebSocketWithTracking.CONNECTING = 0;
    FixtureWebSocketWithTracking.OPEN = 1;
    FixtureWebSocketWithTracking.CLOSING = 2;
    FixtureWebSocketWithTracking.CLOSED = 3;
    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocketWithTracking,
    });
  }, { indexStatusPayload: indexStatus, optionsPayload: options });
}
