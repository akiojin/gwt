/* Knowledge Bridge Work Item compatibility coverage.
 *
 * The fixture serves the embedded frontend assets directly through Playwright
 * routes and replaces WebSocket with a deterministic cache-backed backend.
 * That keeps browser coverage active in CI without depending on a live gwt GUI
 * process, GitHub cache state, or the user's local workspace.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Legacy SPEC preset Work Item compatibility", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 3840, height: 1100 },
  });

  test("renders gwt-spec entries through the unified Work Item list", async ({
    page,
  }, testInfo) => {
    await installEmbeddedRoutes(page);
    await installSpecPresetBackend(page, {
      theme: testInfo.project.name.includes("light") ? "light" : "dark",
    });

    await page.goto(APP_URL);

    await expect(page.locator(".surface-knowledge .knowledge-list")).toBeVisible();
    await expect(page.locator(".surface-knowledge .knowledge-heading")).toHaveText(
      "Cached work items",
    );
    await expect(page.locator(".surface-knowledge .knowledge-search")).toHaveAttribute(
      "placeholder",
      "Semantic search work items",
    );
    await expect(page.locator(".surface-knowledge .kanban-board")).toHaveCount(0);
    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(5);
    await expect(page.getByText("SPEC Issue Kanban View")).toBeVisible();
    await expect(page.getByText("Merge Kanban implementation bundle")).toHaveCount(0);
  });
});

test.describe("Issue Bridge load recovery", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("renders cached issues as an Issue list instead of lifecycle Kanban", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssueBridgeBackend(page);

    await page.goto(APP_URL);

    await expect(page.locator(".surface-knowledge .knowledge-list")).toBeVisible();
    await expect(page.locator(".surface-knowledge .knowledge-list")).toHaveAttribute(
      "aria-label",
      "Cached work items",
    );
    await expect(page.locator(".surface-knowledge .kanban-board")).toHaveCount(0);
    await expect(
      page.locator(".surface-knowledge .kanban-column[data-phase='planning']"),
    ).toHaveCount(0);
    await expect(
      page.locator(".surface-knowledge .kanban-column[data-phase='implementation']"),
    ).toHaveCount(0);
    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(4);
    await expect(page.locator(".surface-knowledge .knowledge-heading")).toHaveText(
      "Cached work items",
    );
    await expect(page.locator(".surface-knowledge .knowledge-search")).toHaveAttribute(
      "placeholder",
      "Semantic search work items",
    );
    await expect(page.getByText("Closed issue hidden by default")).toHaveCount(0);
    await expect(page.getByText("Design-required work item shares Issue list")).toBeVisible();
    await expect(page.getByText("(plain)")).toHaveCount(0);
  });

  test("Issue state filter defaults to open and can show closed or all issues", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssueBridgeBackend(page);

    await page.goto(APP_URL);

    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(4);
    await expect(page.getByText("Closed issue hidden by default")).toHaveCount(0);

    await page.locator(".surface-knowledge [data-issue-filter='closed']").click();

    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(1);
    await expect(page.getByText("Closed issue hidden by default")).toBeVisible();

    await page.locator(".surface-knowledge [data-issue-filter='all']").click();

    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(5);
  });

  test("selecting an Issue row renders cached detail in the right pane", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssueBridgeBackend(page);

    await page.goto(APP_URL);

    await page.locator(".surface-knowledge .knowledge-row[data-issue-number='3096']").click();

    await expect(
      page.locator(".surface-knowledge .knowledge-detail-pane"),
    ).toContainText("Issue Bridge detail body");
    await expect(
      page.locator(".surface-knowledge .knowledge-detail-pane"),
    ).toContainText("Launch Agent");
  });

  test("Issue auto refresh stays cache-first while browsing cached issues", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssueBridgeBackend(page, {
      errorOnForcedRefresh: true,
      triggerAutoRefreshOnce: true,
    });

    await page.goto(APP_URL);

    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(4);
    await page.locator(".surface-knowledge .knowledge-row[data-issue-number='3095']").click();
    await expect(
      page.locator(".surface-knowledge .knowledge-detail-pane"),
    ).toContainText("Issue #3095");
    await page.waitForFunction(
      () => typeof window.__triggerKnowledgeAutoRefresh === "function",
    );
    await page.evaluate(() => window.__triggerKnowledgeAutoRefresh());
    await page.waitForFunction(() =>
      window.__knowledgeLoadMessages?.filter(
        (message) => message.kind === "load_knowledge_bridge",
      ).length >= 2,
    );

    await expect(page.locator(".surface-knowledge .knowledge-status.error")).toHaveCount(0);
    await expect(page.locator(".surface-knowledge .knowledge-status")).toHaveText("");
    const refreshFlags = await page.evaluate(() =>
      window.__knowledgeLoadMessages
        .filter((message) => message.kind === "load_knowledge_bridge")
        .map((message) => message.refresh),
    );
    expect(refreshFlags).not.toContain(true);
  });

  test("requests cached issues when a stale detail exists but the list is empty", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssueBridgeBackend(page, { staleDetailBeforeWorkspace: true });

    await page.goto(APP_URL);

    await expect(page.locator(".surface-knowledge .knowledge-list")).toBeVisible();
    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(4);
  });

  test("manual refresh recovers a stale empty loading state", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIssueBridgeBackend(page, { ignoreFirstLoad: true });

    await page.goto(APP_URL);

    const refresh = page.locator(
      ".surface-knowledge [data-action='refresh-knowledge']",
    );
    await expect(refresh).toBeEnabled();
    await refresh.click();

    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(4);
  });

  test("projects monitor state and controls through the canonical Issue surface", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(String(error)));

    await installEmbeddedRoutes(page);
    await installIssueBridgeBackend(page);

    await page.goto(APP_URL);

    const issueSurface = page.locator(".workspace-window.surface-knowledge");
    await expect(issueSurface).toBeVisible();
    await expect(page.locator(".surface-issue-monitor")).toHaveCount(0);
    await expect(
      issueSurface.locator('[data-issue-number="3273"] .knowledge-monitor-chip'),
    ).toHaveText("Queued");
    await expect(issueSurface.locator('[data-issue-number="3273"]')).toContainText(
      "Queue 1",
    );
    await expect(
      issueSurface.locator('[data-issue-number="3096"] .knowledge-monitor-chip'),
    ).toHaveText("Needs human");
    await expect(
      issueSurface.locator('[data-issue-number="3097"] .knowledge-monitor-chip'),
    ).toHaveText("On hold");
    await expect(issueSurface.locator('[data-issue-number="3097"]')).toContainText(
      "Excluded by label: hold",
    );
    await expect(issueSurface.locator("button button")).toHaveCount(0);
    await expect(issueSurface.locator(".knowledge-monitor-summary")).toContainText(
      "Queue 3 | Active 1",
    );

    await issueSurface
      .locator('[data-issue-number="3273"] [data-action="launch-now"]')
      .click();
    await issueSurface
      .locator('[data-issue-number="3095"] [data-action="move-up"]')
      .click();
    await expect(issueSurface.locator(".knowledge-monitor-max-active")).toHaveCount(0);
    await issueSurface.locator('[data-action="monitor-toggle"]').click();
    await issueSurface.locator('[data-action="monitor-autonomous"]').click();
    await issueSurface.locator('[data-action="monitor-settings"]').click();
    await issueSurface.locator(".knowledge-monitor-quick-title").fill(
      "Investigate flaky release gate",
    );
    await issueSurface.locator('[data-action="quick-register-launch"]').click();

    const messages = await page.evaluate(() => window.__knowledgeLoadMessages);
    expect(messages).toContainEqual({
      kind: "issue_monitor_launch_now",
      issue_number: 3273,
      linked_issue_kind: "spec",
    });
    expect(messages).toContainEqual({
      kind: "reorder_issue_monitor_issues",
      issue_numbers: [3095, 3273, 3094],
    });
    expect(messages).toContainEqual({
      kind: "set_issue_monitor_enabled",
      enabled: true,
    });
    expect(messages).toContainEqual({
      kind: "set_issue_monitor_autonomous_mode",
      enabled: true,
    });
    expect(messages).toContainEqual({ kind: "issue_monitor_configure_profile" });
    expect(messages).toContainEqual({
      kind: "quick_register_issue",
      title: "Investigate flaky release gate",
      launch: true,
    });
    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });

  test("redirects a persisted issue_monitor preset to the Issue surface", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(String(error)));

    await installEmbeddedRoutes(page);
    await installIssueBridgeBackend(page, { legacyPreset: true });

    await page.goto(APP_URL);

    await expect(page.locator(".workspace-window.surface-knowledge")).toBeVisible();
    await expect(page.locator(".surface-issue-monitor")).toHaveCount(0);
    await expect(page.locator(".surface-knowledge .knowledge-heading")).toHaveText(
      "Cached work items",
    );
    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(4);
    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });
});

// SPEC #3170 T-955 — both the Issue preset and the SPEC preset normalize to
// the backend `issue` knowledge kind, so every selection scenario runs twice
// against the same surface contract.
const SELECTION_SURFACES = [
  {
    label: "Issue preset",
    preset: "issue",
    tabId: "tab-issue-selection",
    windowId: "issue-selection",
    windowTitle: "Issue",
  },
  {
    label: "SPEC preset",
    preset: "spec",
    tabId: "tab-spec-selection",
    windowId: "spec-selection",
    windowTitle: "SPEC Kanban",
  },
];

const SELECTION_FIRST_NUMBER = 4001;
const SELECTION_ROW_COUNT = 500;
const SELECTION_P95_BUDGET_MS = 50;

test.describe("SPEC #3170 immediate selection under delayed and reversed responses", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  for (const surface of SELECTION_SURFACES) {
    test(`${surface.label}: click preview settles under the p95 budget while every detail reply lags`, async ({
      page,
    }, testInfo) => {
      await installEmbeddedRoutes(page);
      await installKnowledgeSelectionBackend(page, {
        ...surface,
        theme: testInfo.project.name.includes("light") ? "light" : "dark",
      });

      await page.goto(APP_URL);
      await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(
        SELECTION_ROW_COUNT,
      );

      // Distinct rows, striding across the list so consecutive clicks never
      // reuse the previous row and the stale 300 ms replies keep landing on
      // top of a newer selection.
      const measuredNumbers = [];
      for (let step = 0; step < 30; step += 1) {
        measuredNumbers.push(SELECTION_FIRST_NUMBER + ((step * 7) % SELECTION_ROW_COUNT));
      }
      const warmupNumbers = [0, 1, 2, 3, 4].map(
        (offset) => SELECTION_FIRST_NUMBER + SELECTION_ROW_COUNT - 1 - offset,
      );

      const samples = await measureKnowledgeSelectionLatency(page, {
        warmupNumbers,
        measuredNumbers,
        holdMs: 180,
        warmupHoldMs: 380,
      });

      expect(samples).toHaveLength(measuredNumbers.length);
      for (const sample of samples) {
        expect(sample.settleMs).toBeGreaterThanOrEqual(0);
        // The local preview is a synchronous transition: identity, selection
        // and the "Loading detail" placeholder are all present before the
        // click handler returns, long before the 300 ms detail reply.
        expect(sample.syncIdentity).toBe(sample.number);
        expect(sample.syncSelected).toBe(sample.number);
        expect(sample.syncLoadingPlaceholder).toBe(true);
        expect(sample.mismatchedFrames).toBe(0);
      }

      const settleTimes = samples.map((sample) => sample.settleMs);
      const stats = {
        surface: surface.label,
        project: testInfo.project.name,
        samples: settleTimes,
        min: Math.min(...settleTimes),
        median: percentile(settleTimes, 0.5),
        p95: percentile(settleTimes, 0.95),
        max: Math.max(...settleTimes),
        mismatchedFrames: samples.reduce(
          (total, sample) => total + sample.mismatchedFrames,
          0,
        ),
        observedFrames: samples.reduce((total, sample) => total + sample.frames, 0),
      };
      console.log(`[SPEC-3170] ${surface.label} selection latency`, JSON.stringify(stats));
      await testInfo.attach(
        `spec-3170-selection-latency-${surface.preset}.json`,
        { body: JSON.stringify(stats, null, 2), contentType: "application/json" },
      );

      expect(stats.mismatchedFrames).toBe(0);
      expect(stats.p95).toBeLessThan(SELECTION_P95_BUDGET_MS);
    });

    test(`${surface.label}: reversed detail completion keeps the last clicked row`, async ({
      page,
    }, testInfo) => {
      await installEmbeddedRoutes(page);
      await installKnowledgeSelectionBackend(page, {
        ...surface,
        theme: testInfo.project.name.includes("light") ? "light" : "dark",
        // The first request (row A) is answered last; row B's reply overtakes
        // it by 350 ms.
        detailDelayPlan: [400, 50],
      });

      await page.goto(APP_URL);
      await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(
        SELECTION_ROW_COUNT,
      );

      const first = SELECTION_FIRST_NUMBER + 3;
      const second = SELECTION_FIRST_NUMBER + 21;
      const result = await runReversedKnowledgeSelection(page, {
        first,
        second,
        observeMs: 1100,
      });

      expect(result.secondSettleMs).toBeGreaterThanOrEqual(0);
      expect(result.secondSettleMs).toBeLessThan(SELECTION_P95_BUDGET_MS);
      expect(result.finalIdentity).toBe(second);
      expect(result.finalSelected).toBe(second);
      expect(result.finalBodyHasSecond).toBe(true);
      expect(result.finalBodyHasFirst).toBe(false);
      expect(result.staleBodyFrames).toBe(0);
      expect(result.mismatchedFrames).toBe(0);
      // Both replies must have been delivered for the assertion above to mean
      // anything: B at ~50 ms and A's stale answer at ~400 ms.
      expect(result.deliveredDetails).toBe(2);
    });

    test(`${surface.label}: silent semantic retry keeps one search in flight and survives row clicks`, async ({
      page,
    }, testInfo) => {
      test.setTimeout(60_000);
      await installEmbeddedRoutes(page);
      await installKnowledgeSelectionBackend(page, {
        ...surface,
        theme: testInfo.project.name.includes("light") ? "light" : "dark",
        semanticRetry: true,
      });

      await page.goto(APP_URL);
      await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(
        SELECTION_ROW_COUNT,
      );

      await page
        .locator(".surface-knowledge .knowledge-search")
        .fill("  latency ladder  ");
      await page.waitForFunction(
        () => window.__kbFixture.searchRequests.length >= 1,
        undefined,
        { timeout: 10_000 },
      );

      // FR-099: a transient semantic failure is silent — no error, no
      // "Searching semantic index" spinner text on the Issue/SPEC surface.
      const status = page.locator(".surface-knowledge .knowledge-status");
      await expect(status).not.toContainText("Searching semantic index");
      await expect(page.locator(".surface-knowledge .knowledge-status.error")).toHaveCount(0);
      await expect(page.locator(".surface-knowledge .knowledge-row").first()).toBeVisible();

      // Row clicks inside the retry window must neither cancel the ladder nor
      // add a search burst, and they still preview synchronously.
      for (const number of [SELECTION_FIRST_NUMBER + 2, SELECTION_FIRST_NUMBER + 5]) {
        const preview = await clickKnowledgeRow(page, number);
        expect(preview.identity).toBe(number);
        expect(preview.selected).toBe(number);
        expect(preview.loadingPlaceholder).toBe(true);
      }
      expect(await page.evaluate(() => window.__kbFixture.searchRequests.length)).toBe(1);

      await page.waitForFunction(
        () => window.__kbFixture.searchRequests.length >= 2,
        undefined,
        { timeout: 15_000 },
      );
      const telemetry = await readSelectionFixtureTelemetry(page);
      expect(telemetry.searchQueries).toEqual(["latency ladder", "latency ladder"]);
      expect(telemetry.maxSearchOverlap).toBe(1);
      await expect(page.locator(".surface-knowledge .knowledge-status.error")).toHaveCount(0);
      await expect(status).not.toContainText("Searching semantic index");
    });

    test(`${surface.label}: a retry hop on a non-OPEN socket queues nothing and restarts after reconnect`, async ({
      page,
    }, testInfo) => {
      test.setTimeout(60_000);
      await installEmbeddedRoutes(page);
      await installKnowledgeSelectionBackend(page, {
        ...surface,
        theme: testInfo.project.name.includes("light") ? "light" : "dark",
        semanticRetry: true,
      });

      await page.goto(APP_URL);
      await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(
        SELECTION_ROW_COUNT,
      );

      await page
        .locator(".surface-knowledge .knowledge-search")
        .fill("  offline ladder  ");
      await page.waitForFunction(
        () => window.__kbFixture.searchRequests.length >= 1,
        undefined,
        { timeout: 10_000 },
      );
      await page.waitForFunction(
        () => window.__kbFixture.deliveredSearchResults >= 1,
        undefined,
        { timeout: 10_000 },
      );

      // The socket drops without delivering `close` yet — exactly the race
      // sendKnowledgeSemanticSearchNow owns. The 5 s hop fires into it.
      await page.evaluate(() => window.__kbFixture.closeSocketSilently());
      await page.waitForTimeout(5_600);
      const offline = await readSelectionFixtureTelemetry(page);
      expect(offline.sentWhileClosed).toBe(0);
      expect(offline.searchRequests).toBe(1);

      // AS-17.2: the visible close reconnects and restarts the ladder at 5 s.
      await page.evaluate(() => window.__kbFixture.dropSocket());
      // The reconnect flushes app.js's pending-message queue; a semantic
      // search must never be in it, so nothing may arrive with the flush.
      await page.waitForFunction(
        () => window.__kbFixture.socketOpenCount >= 2,
        undefined,
        { timeout: 10_000 },
      );
      expect(await page.evaluate(() => window.__kbFixture.searchRequests.length)).toBe(1);
      await page.waitForFunction(
        () => window.__kbFixture.searchRequests.length >= 2,
        undefined,
        { timeout: 20_000 },
      );
      const restarted = await readSelectionFixtureTelemetry(page);
      expect(restarted.searchRequests).toBe(2);
      expect(restarted.searchQueries).toEqual(["offline ladder", "offline ladder"]);
      expect(restarted.maxSearchOverlap).toBe(1);
      expect(restarted.sentWhileClosed).toBe(0);
      await expect(page.locator(".surface-knowledge .knowledge-status.error")).toHaveCount(0);
    });
  }
});

function percentile(values: number[], fraction: number): number {
  const sorted = [...values].sort((left, right) => left - right);
  const index = Math.min(
    sorted.length - 1,
    Math.max(0, Math.ceil(fraction * sorted.length) - 1),
  );
  return sorted[index];
}

// Frame-accurate in-page measurement: the click is dispatched from the page so
// the timestamp brackets exactly the click handler plus the frames until the
// detail pane paints the clicked identity. Playwright waits would measure
// protocol round-trips instead.
async function measureKnowledgeSelectionLatency(
  page,
  { warmupNumbers, measuredNumbers, holdMs, warmupHoldMs },
) {
  return await page.evaluate(
    async ({
      warmupNumbers: warmup,
      measuredNumbers: measured,
      holdMs: hold,
      warmupHoldMs: warmupHold,
    }) => {
      const probe = window.__knowledgeSelectionProbe();
      const clickAndObserve = async (number, observeMs) => {
        const row = probe.rowFor(number);
        if (!row) {
          throw new Error(`row ${number} is not rendered`);
        }
        const started = performance.now();
        row.click();
        const syncIdentity = probe.identity();
        const syncSelected = probe.selectedNumber();
        const syncLoadingPlaceholder = probe.loadingPlaceholder();
        let settleMs = -1;
        let mismatchedFrames = 0;
        let frames = 0;
        for (;;) {
          await probe.nextFrame();
          frames += 1;
          const identity = probe.identity();
          if (identity !== null && identity !== number) {
            mismatchedFrames += 1;
          }
          if (settleMs < 0 && identity === number) {
            settleMs = performance.now() - started;
          }
          const elapsed = performance.now() - started;
          if (settleMs >= 0 && elapsed >= observeMs) {
            break;
          }
          if (elapsed > 5000) {
            break;
          }
        }
        return {
          number,
          settleMs,
          frames,
          mismatchedFrames,
          syncIdentity,
          syncSelected,
          syncLoadingPlaceholder,
        };
      };

      for (const number of warmup) {
        await clickAndObserve(number, warmupHold);
      }
      const samples = [];
      for (const number of measured) {
        samples.push(await clickAndObserve(number, hold));
      }
      return samples;
    },
    { warmupNumbers, measuredNumbers, holdMs, warmupHoldMs },
  );
}

async function runReversedKnowledgeSelection(page, { first, second, observeMs }) {
  return await page.evaluate(
    async ({ first: a, second: b, observeMs: window_ }) => {
      const probe = window.__knowledgeSelectionProbe();
      const observer = { expected: null, mismatchedFrames: 0, staleBodyFrames: 0, frames: 0 };
      const rowA = probe.rowFor(a);
      const rowB = probe.rowFor(b);
      if (!rowA || !rowB) {
        throw new Error("reversed-order fixture rows are not rendered");
      }
      const started = performance.now();
      let secondSettleMs = -1;
      observer.expected = a;
      rowA.click();
      let clickedSecondAt = -1;
      for (;;) {
        await probe.nextFrame();
        observer.frames += 1;
        const identity = probe.identity();
        if (identity !== null && identity !== observer.expected) {
          observer.mismatchedFrames += 1;
        }
        if (observer.expected === b) {
          if (secondSettleMs < 0 && identity === b) {
            secondSettleMs = performance.now() - clickedSecondAt;
          }
          if (probe.bodyIncludes(`Detail body for #${a}`)) {
            observer.staleBodyFrames += 1;
          }
        }
        const elapsed = performance.now() - started;
        if (observer.expected === a && elapsed >= 32) {
          observer.expected = b;
          clickedSecondAt = performance.now();
          rowB.click();
          continue;
        }
        if (elapsed >= window_) {
          break;
        }
      }
      return {
        secondSettleMs,
        frames: observer.frames,
        mismatchedFrames: observer.mismatchedFrames,
        staleBodyFrames: observer.staleBodyFrames,
        finalIdentity: probe.identity(),
        finalSelected: probe.selectedNumber(),
        finalBodyHasSecond: probe.bodyIncludes(`Detail body for #${b}`),
        finalBodyHasFirst: probe.bodyIncludes(`Detail body for #${a}`),
        deliveredDetails: window.__kbFixture.deliveredDetails,
      };
    },
    { first, second, observeMs },
  );
}

async function clickKnowledgeRow(page, number) {
  return await page.evaluate((target) => {
    const probe = window.__knowledgeSelectionProbe();
    const row = probe.rowFor(target);
    if (!row) {
      throw new Error(`row ${target} is not rendered`);
    }
    row.click();
    return {
      identity: probe.identity(),
      selected: probe.selectedNumber(),
      loadingPlaceholder: probe.loadingPlaceholder(),
    };
  }, number);
}

async function readSelectionFixtureTelemetry(page) {
  return await page.evaluate(() => ({
    searchRequests: window.__kbFixture.searchRequests.length,
    searchQueries: window.__kbFixture.searchRequests.map((entry) => entry.query),
    detailRequests: window.__kbFixture.detailRequests.length,
    deliveredDetails: window.__kbFixture.deliveredDetails,
    deliveredSearchResults: window.__kbFixture.deliveredSearchResults,
    maxSearchOverlap: window.__kbFixture.maxSearchOverlap,
    sentWhileClosed: window.__kbFixture.sentWhileClosed,
    socketOpenCount: window.__kbFixture.socketOpenCount,
  }));
}

// Deterministic cache-backed backend for the SPEC #3170 selection scenarios.
// Detail replies are delayed (globally or per request sequence) so the local
// preview is the only thing that can satisfy the latency contract, and the
// socket exposes a silent-drop hook for the retry-ladder tests.
async function installKnowledgeSelectionBackend(
  page,
  {
    preset = "issue",
    windowId = "issue-selection",
    tabId = "tab-issue-selection",
    windowTitle = "Issue",
    theme = "dark",
    rowCount = SELECTION_ROW_COUNT,
    firstNumber = SELECTION_FIRST_NUMBER,
    detailDelayMs = 300,
    detailDelayPlan = [],
    searchDelayMs = 20,
    searchEntryCount = 16,
    semanticRetry = false,
    label: _label,
  } = {},
) {
  await page.addInitScript(
    ({
      preset: windowPreset,
      windowId: fixtureWindowId,
      tabId: fixtureTabId,
      windowTitle: fixtureWindowTitle,
      theme: selectedTheme,
      rowCount: fixtureRowCount,
      firstNumber: fixtureFirstNumber,
      detailDelayMs: fixtureDetailDelayMs,
      detailDelayPlan: fixtureDetailDelayPlan,
      searchDelayMs: fixtureSearchDelayMs,
      searchEntryCount: fixtureSearchEntryCount,
      semanticRetry: fixtureSemanticRetry,
    }) => {
      localStorage.setItem("gwt:ui:theme", selectedTheme);

      const entries = [];
      for (let index = 0; index < fixtureRowCount; index += 1) {
        const number = fixtureFirstNumber + index;
        entries.push({
          number,
          title: `Cached work item ${number}`,
          state: "open",
          meta: `Fixture row ${index + 1}`,
          labels: index % 2 === 0 ? ["bug"] : ["gwt-spec"],
          linked_branch_count: index % 3,
          match_score: 100 - (index % 40),
          phase: null,
          has_unknown_phase: false,
          is_spec: windowPreset === "spec",
        });
      }

      const detailFor = (number) => ({
        number,
        title: `Cached work item ${number}`,
        state: "open",
        // The authoritative detail keeps the preview's identity strings so a
        // frame can only ever be "the clicked row" or "some other row".
        subtitle: `#${number}`,
        labels: ["bug"],
        launch_issue_number: number,
        sections: [
          {
            title: "Description",
            body: `Detail body for #${number}`,
            body_html: `<p>Detail body for #${number}</p>`,
          },
        ],
      });

      const workspaceState = {
        kind: "workspace_state",
        workspace: {
          app_version: "playwright",
          tabs: [
            {
              id: fixtureTabId,
              title: "Fixture Project",
              project_root: "/fixture",
              kind: "git",
              workspace: {
                viewport: { x: 0, y: 0, zoom: 1 },
                windows: [
                  {
                    id: fixtureWindowId,
                    title: fixtureWindowTitle,
                    preset: windowPreset,
                    geometry: { x: 40, y: 60, width: 1320, height: 760 },
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
          active_tab_id: fixtureTabId,
          recent_projects: [],
        },
      };

      const fixture = {
        detailRequests: [],
        searchRequests: [],
        loadRequests: [],
        deliveredDetails: 0,
        deliveredSearchResults: 0,
        searchOverlap: 0,
        maxSearchOverlap: 0,
        sentWhileClosed: 0,
        socketOpenCount: 0,
        workspaceSent: false,
        activeSocket: null,
        closeSocketSilently() {
          // No `close` event: the frontend still believes it is online, so the
          // next retry hop has to detect the dead socket through readyState.
          if (fixture.activeSocket) {
            fixture.activeSocket.readyState = 3;
          }
        },
        dropSocket() {
          const socket = fixture.activeSocket;
          if (!socket) {
            return;
          }
          socket.readyState = 3;
          socket.dispatchEvent(new CloseEvent("close"));
        },
      };
      window.__kbFixture = fixture;

      // Shared page-side probe so every scenario reads the same identity,
      // selection and placeholder contract out of the rendered DOM.
      window.__knowledgeSelectionProbe = () => {
        const root = document.querySelector(".surface-knowledge");
        if (!root) {
          throw new Error("knowledge surface is not mounted");
        }
        const pane = () => root.querySelector(".knowledge-detail-pane");
        return {
          rowFor: (number) =>
            root.querySelector(
              `.knowledge-row[data-issue-number="${number}"] .knowledge-row-select`,
            ),
          nextFrame: () =>
            new Promise((resolve) => requestAnimationFrame(() => resolve(null))),
          identity: () => {
            const detailPane = pane();
            if (!detailPane) {
              return null;
            }
            const title =
              detailPane.querySelector(".knowledge-detail-title")?.textContent || "";
            const subtitle =
              detailPane.querySelector(".knowledge-detail-subtitle")?.textContent || "";
            const titleMatch = title.match(/(\d+)/);
            const subtitleMatch = subtitle.match(/#(\d+)/);
            if (!titleMatch || !subtitleMatch) {
              return null;
            }
            const titleNumber = Number(titleMatch[1]);
            const subtitleNumber = Number(subtitleMatch[1]);
            // -1 marks a torn render (title and subtitle disagree) so callers
            // count it as a mismatched frame instead of ignoring it.
            return titleNumber === subtitleNumber ? titleNumber : -1;
          },
          selectedNumber: () => {
            const selected = root.querySelector(
              ".knowledge-row.selected .knowledge-row-select[aria-current='true']",
            );
            return selected
              ? Number(selected.closest(".knowledge-row")?.dataset.issueNumber)
              : null;
          },
          loadingPlaceholder: () =>
            (pane()?.textContent || "").includes("Loading detail"),
          bodyIncludes: (text) => (pane()?.textContent || "").includes(text),
        };
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
          fixture.activeSocket = this;
          setTimeout(() => {
            this.readyState = FixtureWebSocket.OPEN;
            fixture.socketOpenCount += 1;
            this.dispatchEvent(new Event("open"));
          }, 0);
        }

        send(raw) {
          if (this.readyState !== FixtureWebSocket.OPEN) {
            fixture.sentWhileClosed += 1;
            return;
          }
          const message = JSON.parse(raw);
          if (message.kind === "frontend_ready") {
            // Reconnect keeps the same workspace: re-emitting it would remount
            // the window and hide whether the retry ladder itself restarted.
            if (!fixture.workspaceSent) {
              fixture.workspaceSent = true;
              this.emit(workspaceState);
            }
            return;
          }
          if (message.kind === "load_knowledge_bridge") {
            fixture.loadRequests.push({
              requestId: message.request_id,
              refresh: message.refresh === true,
            });
            this.emit({
              kind: "knowledge_entries",
              id: message.id,
              knowledge_kind: message.knowledge_kind,
              request_id: message.request_id,
              entries,
              selected_number: null,
              empty_message: null,
              refresh_enabled: true,
            });
            return;
          }
          if (message.kind === "select_knowledge_bridge_entry") {
            const sequence = fixture.detailRequests.length;
            fixture.detailRequests.push({
              number: message.number,
              requestId: message.request_id,
              at: performance.now(),
            });
            const delay = sequence < fixtureDetailDelayPlan.length
              ? fixtureDetailDelayPlan[sequence]
              : fixtureDetailDelayMs;
            this.emit(
              {
                kind: "knowledge_detail",
                id: message.id,
                knowledge_kind: message.knowledge_kind,
                request_id: message.request_id,
                detail: detailFor(message.number),
              },
              delay,
              () => {
                fixture.deliveredDetails += 1;
              },
            );
            return;
          }
          if (message.kind === "search_knowledge_bridge") {
            fixture.searchRequests.push({
              query: message.query,
              requestId: message.request_id,
              at: performance.now(),
            });
            fixture.searchOverlap += 1;
            fixture.maxSearchOverlap = Math.max(
              fixture.maxSearchOverlap,
              fixture.searchOverlap,
            );
            this.emit(
              {
                kind: "knowledge_search_results",
                id: message.id,
                knowledge_kind: message.knowledge_kind,
                request_id: message.request_id,
                query: message.query,
                entries: entries.slice(0, fixtureSearchEntryCount),
                selected_number: null,
                empty_message: null,
                refresh_enabled: true,
                semantic_retry: fixtureSemanticRetry
                  ? {
                      error_code: "INDEX_NOT_READY",
                      retryable: true,
                      retry_after_ms: 5000,
                    }
                  : null,
              },
              fixtureSearchDelayMs,
              () => {
                fixture.searchOverlap -= 1;
              },
              () => {
                fixture.deliveredSearchResults += 1;
              },
            );
          }
        }

        close() {
          this.readyState = FixtureWebSocket.CLOSED;
          this.dispatchEvent(new CloseEvent("close"));
        }

        emit(payload, delay = 0, beforeDispatch = null, afterDispatch = null) {
          setTimeout(() => {
            if (beforeDispatch) {
              beforeDispatch();
            }
            if (this.readyState !== FixtureWebSocket.OPEN) {
              return;
            }
            this.dispatchEvent(
              new MessageEvent("message", { data: JSON.stringify(payload) }),
            );
            if (afterDispatch) {
              afterDispatch();
            }
          }, delay);
        }
      }

      Object.defineProperty(window, "WebSocket", {
        configurable: true,
        value: FixtureWebSocket,
      });
    },
    {
      preset,
      windowId,
      tabId,
      windowTitle,
      theme,
      rowCount,
      firstNumber,
      detailDelayMs,
      detailDelayPlan,
      searchDelayMs,
      searchEntryCount,
      semanticRetry,
    },
  );
}

async function installSpecPresetBackend(page, { theme }) {
  await page.addInitScript(
    ({ theme: selectedTheme }) => {
      const entries = [
        {
          number: 2017,
          title: "SPEC Issue Kanban View",
          state: "open",
          meta: "Phase 4 visual coverage",
          labels: ["gwt-spec", "phase/implementation"],
          linked_branch_count: 2,
          match_score: 99,
          phase: "implementation",
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 1935,
          title: "Coordination hooks and Board reminders",
          state: "open",
          meta: "Planning refinement",
          labels: ["gwt-spec", "phase/planning"],
          linked_branch_count: 1,
          match_score: 88,
          phase: "planning",
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 2008,
          title: "Window host interaction model",
          state: "open",
          meta: "Review follow-up",
          labels: ["gwt-spec", "phase/review"],
          linked_branch_count: 3,
          match_score: 82,
          phase: "review",
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 2077,
          title: "Runtime daemon event transport",
          state: "open",
          meta: "Draft architecture",
          labels: ["gwt-spec", "phase/draft"],
          linked_branch_count: 0,
          match_score: 76,
          phase: "draft",
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 2359,
          title: "Work Kanban stabilization",
          state: "open",
          meta: "Unscheduled backlog",
          labels: ["gwt-spec"],
          linked_branch_count: 0,
          match_score: 71,
          phase: null,
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 2470,
          title: "Merge Kanban implementation bundle",
          state: "closed",
          meta: "Completed rollout",
          labels: ["gwt-spec", "phase/done"],
          linked_branch_count: 1,
          match_score: 100,
          phase: "done",
          has_unknown_phase: false,
          is_spec: true,
        },
      ];

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
                    id: "spec-kanban",
                    title: "SPEC Kanban",
                    preset: "spec",
                    geometry: { x: 96, y: 76, width: 3600, height: 820 },
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

      localStorage.setItem("gwt:ui:theme", selectedTheme);

      class FixtureWebSocket extends EventTarget {
        static CONNECTING = 0;
        static OPEN = 1;
        static CLOSING = 2;
        static CLOSED = 3;

        constructor(url) {
          super();
          this.url = url;
          this.readyState = FixtureWebSocket.CONNECTING;
          setTimeout(() => {
            this.readyState = FixtureWebSocket.OPEN;
            this.dispatchEvent(new Event("open"));
          }, 0);
        }

        send(raw) {
          const message = JSON.parse(raw);
          if (message.kind === "frontend_ready") {
            this.emit(workspaceState);
            return;
          }
          if (message.kind === "load_knowledge_bridge") {
            this.emit({
              kind: "knowledge_entries",
              id: message.id,
              knowledge_kind: message.knowledge_kind,
              request_id: message.request_id,
              entries,
              selected_number: 2017,
              empty_message: null,
              refresh_enabled: true,
            });
            return;
          }
          if (message.kind === "select_knowledge_bridge_entry") {
            this.emit({
              kind: "knowledge_detail",
              id: message.id,
              knowledge_kind: message.knowledge_kind,
              request_id: message.request_id,
              detail: {
                number: message.number,
                title: `SPEC #${message.number}`,
                state: "open",
                subtitle: "Deterministic fixture detail",
                labels: ["gwt-spec"],
                launch_issue_number: message.number,
                sections: [
                  {
                    title: "Acceptance",
                    body: "Kanban columns stay readable in dark and light themes.",
                  },
                ],
              },
            });
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
    },
    { theme },
  );
}

async function installIssueBridgeBackend(
  page,
  {
    errorOnForcedRefresh = false,
    ignoreFirstLoad = false,
    legacyPreset = false,
    staleDetailBeforeWorkspace = false,
    triggerAutoRefreshOnce = false,
  } = {},
) {
  await page.addInitScript(
    ({
      errorOnForcedRefresh: shouldErrorOnForcedRefresh,
      ignoreFirstLoad: shouldIgnoreFirstLoad,
      legacyPreset: shouldUseLegacyPreset,
      staleDetailBeforeWorkspace: shouldSeedStaleDetail,
      triggerAutoRefreshOnce: shouldTriggerAutoRefreshOnce,
    }) => {
      window.__knowledgeLoadMessages = [];
      if (shouldTriggerAutoRefreshOnce) {
        window.__knowledgeAutoRefreshCallbacks = [];
        window.__triggerKnowledgeAutoRefresh = () => {
          const callbacks = window.__knowledgeAutoRefreshCallbacks || [];
          for (const callback of callbacks) {
            callback();
          }
        };
        const originalSetInterval = window.setInterval.bind(window);
        window.setInterval = (callback, delay, ...args) => {
          if (delay === 60000) {
            window.__knowledgeAutoRefreshCallbacks.push(() => callback(...args));
          }
          return originalSetInterval(callback, delay, ...args);
        };
      }
      const entries = [
        {
          number: 3273,
          title: "Design-required work item shares Issue list",
          state: "open",
          meta: "gwt-spec tagged work item",
          labels: ["GWT-SPEC", "phase/implementation"],
          linked_branch_count: 0,
          match_score: 98,
          phase: "implementation",
          has_unknown_phase: false,
          is_spec: true,
          monitor_state: "queued",
          queue_position: 1,
          exclusion_reason: null,
        },
        {
          number: 3096,
          title: "Issue Bridge shows empty columns despite cached issues",
          state: "open",
          meta: "Regression fixture",
          labels: ["bug"],
          linked_branch_count: 0,
          match_score: 100,
          phase: null,
          has_unknown_phase: false,
          is_spec: false,
          monitor_state: "needs_human",
          queue_position: null,
          exclusion_reason: null,
        },
        {
          number: 3094,
          title: "Closed issue hidden by default",
          state: "closed",
          meta: "Cached closed issue",
          labels: ["bug"],
          linked_branch_count: 1,
          match_score: 87,
          phase: null,
          has_unknown_phase: false,
          is_spec: false,
          monitor_state: "queued",
          queue_position: 3,
          exclusion_reason: null,
        },
        {
          number: 3095,
          title: "Session TOML corruption on new agent session",
          state: "open",
          meta: "Cached plain issue",
          labels: ["bug"],
          linked_branch_count: 0,
          match_score: 96,
          phase: null,
          has_unknown_phase: false,
          is_spec: false,
          monitor_state: "queued",
          queue_position: 2,
          exclusion_reason: null,
        },
        {
          number: 3097,
          title: "Issue excluded from autonomous launch",
          state: "open",
          meta: "Excluded fixture",
          labels: ["hold"],
          linked_branch_count: 0,
          match_score: 93,
          phase: null,
          has_unknown_phase: false,
          is_spec: false,
          monitor_state: "hold_excluded",
          queue_position: null,
          exclusion_reason: "Excluded by label: hold",
        },
      ];

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
                    title: shouldUseLegacyPreset ? "Issue Monitor" : "Issue",
                    preset: shouldUseLegacyPreset ? "issue_monitor" : "issue",
                    geometry: { x: 40, y: 60, width: 1320, height: 760 },
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

        loadCount = 0;

        constructor(url) {
          super();
          this.url = url;
          this.readyState = FixtureWebSocket.CONNECTING;
          setTimeout(() => {
            this.readyState = FixtureWebSocket.OPEN;
            this.dispatchEvent(new Event("open"));
          }, 0);
        }

        send(raw) {
          const message = JSON.parse(raw);
          window.__knowledgeLoadMessages.push(message);
          if (message.kind === "frontend_ready") {
            if (shouldSeedStaleDetail) {
              this.emit({
                kind: "knowledge_detail",
                id: "issue-kanban",
                knowledge_kind: "issue",
                request_id: 0,
                detail: {
                  number: 3095,
                  title: "Stale cached detail",
                  state: "open",
                  subtitle: "Detail survived without entries",
                  labels: ["bug"],
                  launch_issue_number: 3095,
                  sections: [],
                },
              });
            }
            this.emit(workspaceState);
            return;
          }
          if (message.kind === "list_issue_monitor") {
            this.emit({
              kind: "issue_monitor_status",
              status: {
                enabled: false,
                state: "disabled",
                queue_len: 3,
                active_count: 1,
                total_candidates: 5,
                autonomous_mode: false,
                launch_profile_source: "saved",
                launch_profile_summary: "codex / host",
              },
            });
            return;
          }
          if (message.kind === "load_knowledge_bridge") {
            this.loadCount += 1;
            if (shouldIgnoreFirstLoad && this.loadCount === 1) {
              return;
            }
            if (shouldErrorOnForcedRefresh && message.refresh === true) {
              this.emit({
                kind: "knowledge_error",
                id: message.id,
                knowledge_kind: message.knowledge_kind,
                request_id: message.request_id,
                message: "gh issue list: HTTP 401: Requires authentication",
              });
              return;
            }
            this.emit({
              kind: "knowledge_entries",
              id: message.id,
              knowledge_kind: message.knowledge_kind,
              request_id: message.request_id,
              entries,
              selected_number: 3096,
              empty_message: null,
              refresh_enabled: true,
            });
            return;
          }
          if (message.kind === "select_knowledge_bridge_entry") {
            this.emit({
              kind: "knowledge_detail",
              id: message.id,
              knowledge_kind: message.knowledge_kind,
              request_id: message.request_id,
              detail: {
                number: message.number,
                title: `Issue #${message.number}`,
                state: message.number === 3094 ? "closed" : "open",
                subtitle: "Cached Issue detail",
                labels: ["bug"],
                launch_issue_number: message.number,
                sections: [
                  {
                    title: "Description",
                    body: "Issue Bridge detail body",
                    body_html: "<p>Issue Bridge detail body</p>",
                  },
                  {
                    title: "Linked branches",
                    body: message.number === 3094 ? "work/closed" : "None",
                    body_html: message.number === 3094 ? "<p>work/closed</p>" : "<p>None</p>",
                  },
                ],
              },
            });
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
    },
    {
      errorOnForcedRefresh,
      ignoreFirstLoad,
      legacyPreset,
      staleDetailBeforeWorkspace,
      triggerAutoRefreshOnce,
    },
  );
}
