/**
 * Issue #3813 — opt-in live resource-isolation benchmark.
 *
 * The browser-check launcher prepends fixtures/agent-cargo-load to PATH, so
 * each Codex preset becomes a real AgentBootstrap process tree running a
 * cold cargo build in an independent target directory. The benchmark stays
 * skipped in ordinary E2E runs because three cold builds are intentionally
 * expensive.
 */
import { availableParallelism } from "node:os";
import { readdir, readFile } from "node:fs/promises";
import { test, expect, type Page, type TestInfo } from "@playwright/test";
import {
  acquireLiveGwtBackendLock,
  gotoLiveGwt,
  openLiveGwtProject,
  sendLiveGwtEvent,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const BENCHMARK_DIR = process.env.GWT_AGENT_RESOURCE_BENCHMARK_DIR ?? "";
const SAMPLE_COUNT = 30;
const WARMUP_SAMPLE_COUNT = 5;
const MAX_ACTIVE = 3;
const MAX_RTT_P95_MULTIPLIER = 3;
const ISSUE_MONITOR_SCAN_TIMEOUT_MS = 70_000;
const ISSUE_MONITOR_CONTROL_TIMEOUT_MS = 15_000;

type LatencySummary = {
  samples: number[];
  p50_ms: number;
  p95_ms: number;
  max_ms: number;
  long_task_count: number;
  long_task_max_ms: number;
  raf_gap_p95_ms: number;
  raf_gap_max_ms: number;
};

type UiTrace = {
  session_id?: string;
  entries?: Array<Record<string, unknown>>;
};

function percentile(values: number[], quantile: number): number {
  const sorted = [...values].sort((left, right) => left - right);
  const index = Math.max(0, Math.ceil(sorted.length * quantile) - 1);
  return sorted[index] ?? 0;
}

async function measureWindowListRoundTrips(page: Page): Promise<LatencySummary> {
  const result = await page.evaluate(async ({ sampleCount, warmupSampleCount }) => {
    const samples: number[] = [];

    const endpoint = new URL(window.location.href);
    endpoint.protocol = endpoint.protocol === "https:" ? "wss:" : "ws:";
    endpoint.pathname = "/ws";
    endpoint.search = "";
    endpoint.hash = "";
    const measurementSocket = new WebSocket(endpoint);
    await new Promise<void>((resolve, reject) => {
      const timeout = window.setTimeout(
        () => reject(new Error("measurement WebSocket open timed out")),
        5_000,
      );
      measurementSocket.addEventListener("open", () => {
        window.clearTimeout(timeout);
        resolve();
      }, { once: true });
      measurementSocket.addEventListener("error", () => {
        window.clearTimeout(timeout);
        reject(new Error("measurement WebSocket failed to open"));
      }, { once: true });
    });
    const synchronized = new Promise<void>((resolve, reject) => {
      let timeout = 0;
      const onMessage = (event: MessageEvent) => {
        try {
          const payload = JSON.parse(String(event.data));
          if (payload?.kind !== "workspace_state") return;
          window.clearTimeout(timeout);
          measurementSocket.removeEventListener("message", onMessage);
          resolve();
        } catch {
          /* non-JSON backend frames are irrelevant */
        }
      };
      timeout = window.setTimeout(() => {
        measurementSocket.removeEventListener("message", onMessage);
        reject(new Error("measurement WebSocket frontend sync timed out"));
      }, 5_000);
      measurementSocket.addEventListener("message", onMessage);
    });
    measurementSocket.send(JSON.stringify({ kind: "frontend_ready" }));
    await synchronized;

    try {
      for (let sample = 0; sample < sampleCount + warmupSampleCount; sample += 1) {
        const startedAt = performance.now();
        const reply = new Promise<void>((resolve, reject) => {
          let timeout = 0;
          const onMessage = (event: MessageEvent) => {
            try {
              const payload = JSON.parse(String(event.data));
              if (payload?.kind !== "window_list") return;
              window.clearTimeout(timeout);
              measurementSocket.removeEventListener("message", onMessage);
              resolve();
            } catch {
              /* non-JSON backend frames are irrelevant */
            }
          };
          timeout = window.setTimeout(() => {
            measurementSocket.removeEventListener("message", onMessage);
            reject(new Error(`window_list round trip timed out at sample ${sample}`));
          }, 5_000);
          measurementSocket.addEventListener("message", onMessage);
        });
        measurementSocket.send(JSON.stringify({ kind: "list_windows" }));
        await reply;
        if (sample >= warmupSampleCount) {
          samples.push(performance.now() - startedAt);
        }
      }
    } finally {
      measurementSocket.close();
    }
    return { samples };
  }, { sampleCount: SAMPLE_COUNT, warmupSampleCount: WARMUP_SAMPLE_COUNT });

  return {
    samples: result.samples,
    p50_ms: percentile(result.samples, 0.5),
    p95_ms: percentile(result.samples, 0.95),
    max_ms: Math.max(0, ...result.samples),
    long_task_count: 0,
    long_task_max_ms: 0,
    raf_gap_p95_ms: 0,
    raf_gap_max_ms: 0,
  };
}

async function runPaletteCommand(page: Page, query: string): Promise<void> {
  await page.locator("#op-palette-button").click();
  const input = page.locator("#op-palette-input");
  await expect(input).toBeVisible();
  await input.fill(query);
  await page.keyboard.press("Enter");
  await expect(page.locator("#op-palette-backdrop")).not.toHaveAttribute(
    "data-open",
    "true",
  );
}

async function installUiTraceCapture(page: Page): Promise<void> {
  await page.evaluate(() => {
    if ((window as any).__gwtResourceTraceCaptureInstalled) return;
    (window as any).__gwtResourceTraceCaptureInstalled = true;
    (window as any).__gwtResourceTraces = [];
    const originalSend = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data) {
      try {
        const payload = typeof data === "string" ? JSON.parse(data) : null;
        if (payload?.kind === "save_ui_trace" && payload.trace) {
          (window as any).__gwtResourceTraces.push(payload.trace);
        }
      } catch {
        /* binary and non-JSON frames are unrelated to UI Trace */
      }
      return originalSend.call(this, data);
    };
  });
}

function summarizeUiTrace(trace: UiTrace): Pick<
  LatencySummary,
  "long_task_count" | "long_task_max_ms" | "raf_gap_p95_ms" | "raf_gap_max_ms"
> {
  const entries = Array.isArray(trace.entries) ? trace.entries : [];
  const longTasks = entries
    .filter((entry) => entry.kind === "long_task")
    .map((entry) => Number(entry.duration_ms) || 0);
  const rafGaps = entries
    .filter((entry) => entry.kind === "raf_gap")
    .map((entry) => Number(entry.gap_ms) || 0);
  return {
    long_task_count: longTasks.length,
    long_task_max_ms: Math.max(0, ...longTasks),
    raf_gap_p95_ms: percentile(rafGaps, 0.95),
    raf_gap_max_ms: Math.max(0, ...rafGaps),
  };
}

async function measurePhaseWithUiTrace(
  page: Page,
  testInfo: TestInfo,
  phase: string,
): Promise<LatencySummary> {
  const traceIndex = await page.evaluate(() =>
    ((window as any).__gwtResourceTraces as unknown[]).length
  );
  const replyCursor = await page.evaluate(() =>
    Number((window as any).__gwtPlaywrightMessageSequence) || 0
  );
  await runPaletteCommand(page, "Start UI Trace");
  let latency: LatencySummary;
  try {
    latency = await measureWindowListRoundTrips(page);
  } finally {
    await runPaletteCommand(page, "Stop UI Trace");
  }
  await page.waitForFunction(
    (index) => ((window as any).__gwtResourceTraces as unknown[]).length > index,
    traceIndex,
  );
  await page.waitForFunction(
    (cursor) => {
      const messages = (window as any).__gwtPlaywrightMessages;
      return Array.isArray(messages) && messages.some((entry: any) =>
        entry?.sequence > cursor && entry?.payload?.kind === "ui_trace_saved"
      );
    },
    replyCursor,
  );
  const trace = await page.evaluate((index) =>
    ((window as any).__gwtResourceTraces as UiTrace[])[index], traceIndex
  );
  await testInfo.attach(`agent-resource-ui-trace-${phase}.json`, {
    body: Buffer.from(JSON.stringify(trace, null, 2)),
    contentType: "application/json",
  });
  return { ...latency, ...summarizeUiTrace(trace) };
}

async function waitForWindowListReady(page: Page): Promise<void> {
  let consecutiveReplies = 0;
  for (let attempt = 0; attempt < 36; attempt += 1) {
    const cursor = await page.evaluate(() =>
      Number((window as any).__gwtPlaywrightMessageSequence) || 0
    );
    await sendLiveGwtEvent(page, { kind: "list_windows" });
    const ready = await page.waitForFunction(
      (cursor) => {
        const messages = (window as any).__gwtPlaywrightMessages;
        return Array.isArray(messages) && messages.some((entry: any) =>
          entry?.sequence > cursor
          && entry?.payload?.kind === "window_list"
        );
      },
      cursor,
      { timeout: 5_000 },
    ).then(() => true).catch(() => false);
    consecutiveReplies = ready ? consecutiveReplies + 1 : 0;
    if (consecutiveReplies >= 3) return;
  }
  throw new Error(
    "window_list did not produce three consecutive replies after app initialization",
  );
}

async function waitForMaxActiveStatus(
  page: Page,
  cursor: number,
  expected: number | null,
  timeout: number,
): Promise<number> {
  await page.waitForFunction(
    ({ cursor, expected }) => {
      const messages = (window as any).__gwtPlaywrightMessages;
      return Array.isArray(messages) && messages.some((entry: any) =>
        entry?.sequence > cursor
        && entry?.payload?.kind === "issue_monitor_status"
        && Number.isInteger(entry?.payload?.status?.max_active_agents)
        && (expected === null
          || entry.payload.status.max_active_agents === expected)
      );
    },
    { cursor, expected },
    { timeout },
  );
  return page.evaluate(({ cursor, expected }) => {
    const messages = (window as any).__gwtPlaywrightMessages as Array<any>;
    const event = [...messages].reverse().find((entry) =>
      entry?.sequence > cursor
      && entry?.payload?.kind === "issue_monitor_status"
      && Number.isInteger(entry?.payload?.status?.max_active_agents)
      && (expected === null
        || entry.payload.status.max_active_agents === expected)
    );
    return Number(event?.payload?.status?.max_active_agents);
  }, { cursor, expected });
}

async function currentMaxActive(page: Page): Promise<number> {
  const cursor = await page.evaluate(() =>
    Number((window as any).__gwtPlaywrightMessageSequence) || 0
  );
  await sendLiveGwtEvent(page, { kind: "list_issue_monitor" });
  // This request owns a real Issue Monitor scan with a 60-second backend
  // deadline. It is intentionally used only once to preserve the prior value.
  return waitForMaxActiveStatus(
    page,
    cursor,
    null,
    ISSUE_MONITOR_SCAN_TIMEOUT_MS,
  );
}

async function setMaxActive(page: Page, value: number): Promise<void> {
  const cursor = await page.evaluate(() =>
    Number((window as any).__gwtPlaywrightMessageSequence) || 0
  );
  await sendLiveGwtEvent(page, {
    kind: "set_issue_monitor_max_active_agents",
    max_active_agents: value,
  });
  const observed = await waitForMaxActiveStatus(
    page,
    cursor,
    value,
    ISSUE_MONITOR_CONTROL_TIMEOUT_MS,
  );
  expect(observed).toBe(value);
}

async function latestMaxActive(page: Page): Promise<number> {
  return page.evaluate(() => {
    const messages = (window as any).__gwtPlaywrightMessages as Array<any>;
    const event = [...messages].reverse().find((entry) =>
      entry?.payload?.kind === "issue_monitor_status"
      && Number.isInteger(entry?.payload?.status?.max_active_agents)
    );
    return Number(event?.payload?.status?.max_active_agents);
  });
}

async function markerNames(excluded = new Set<string>()): Promise<string[]> {
  return (await readdir(BENCHMARK_DIR))
    .filter((name) => name.startsWith("started-") && !excluded.has(name))
    .sort();
}

async function waitForMarkers(
  count: number,
  excluded: Set<string>,
): Promise<string[]> {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    const names = await markerNames(excluded);
    if (names.length >= count) return names;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(
    `Only ${(await markerNames(excluded)).length}/${count} cargo agents started`,
  );
}

function markerProcessIsAlive(marker: string): boolean {
  const pid = Number.parseInt(marker.slice("started-".length), 10);
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code !== "ESRCH";
  }
}

function expectMarkerProcessesAlive(markers: string[], count: number): void {
  const active = markers.slice(0, count).filter(markerProcessIsAlive);
  expect(active, `expected ${count} overlapping cold cargo builds`).toHaveLength(count);
}

async function waitForMarkerProcessesToExit(markers: string[]): Promise<void> {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    if (markers.every((marker) => !markerProcessIsAlive(marker))) return;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("cargo agent processes survived window cleanup");
}

async function codexWindowIds(page: Page): Promise<string[]> {
  return page.evaluate(() =>
    Array.from(document.querySelectorAll('.workspace-window[data-preset="codex"]'))
      .map((node) => (node as HTMLElement).dataset.id || "")
      .filter(Boolean)
  );
}

async function spawnCargoAgent(
  page: Page,
  ordinal: number,
  initialWindowCount: number,
): Promise<void> {
  await sendLiveGwtEvent(page, {
    kind: "create_window",
    preset: "codex",
    bounds: {
      x: 80 + ordinal * 24,
      y: 80 + ordinal * 24,
      width: 960,
      height: 640,
    },
  });
  await expect(page.locator('.workspace-window[data-preset="codex"]'))
    .toHaveCount(initialWindowCount + ordinal, { timeout: 10_000 });
}

test.describe.serial("Agent resource isolation (live cold cargo builds)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live benchmark skipped");
  test.skip(
    !BENCHMARK_DIR,
    "GWT_AGENT_RESOURCE_BENCHMARK_DIR is not set; live benchmark skipped",
  );
  test.skip(process.platform === "win32", "the cold cargo fixture requires a POSIX shell");

  let releaseBackendLock: (() => Promise<void>) | undefined;

  test.beforeEach(async ({}, testInfo) => {
    test.skip(
      testInfo.project.name !== "chromium-dark",
      "the live benchmark runs once against the shared backend",
    );
    releaseBackendLock = await acquireLiveGwtBackendLock(BASE, testInfo);
  });

  test.afterEach(async () => {
    if (!releaseBackendLock) return;
    await releaseBackendLock();
    releaseBackendLock = undefined;
  });

  test("keeps GUI event-loop latency bounded at N=0/1/3", async ({ page }, testInfo) => {
    test.setTimeout(300_000);
    const browserErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") browserErrors.push(message.text());
    });
    page.on("pageerror", (error) => browserErrors.push(error.message));
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    if (await page.locator(".project-tab").count() === 0) {
      await openLiveGwtProject(page);
    }
    await waitForWindowListReady(page);
    await installUiTraceCapture(page);

    const originalMaxActive = await currentMaxActive(page);
    const initialWindowIds = new Set(await codexWindowIds(page));
    const initialMarkers = new Set(await markerNames());
    let createdWindowIds: string[] = [];
    let startedMarkers: string[] = [];
    try {
      await setMaxActive(page, MAX_ACTIVE);
      const baseline = await measurePhaseWithUiTrace(page, testInfo, "n0");

      await spawnCargoAgent(page, 1, initialWindowIds.size);
      startedMarkers = await waitForMarkers(1, initialMarkers);
      expectMarkerProcessesAlive(startedMarkers, 1);
      const oneAgent = await measurePhaseWithUiTrace(page, testInfo, "n1");
      expectMarkerProcessesAlive(startedMarkers, 1);

      await spawnCargoAgent(page, 2, initialWindowIds.size);
      await spawnCargoAgent(page, 3, initialWindowIds.size);
      startedMarkers = await waitForMarkers(3, initialMarkers);
      expectMarkerProcessesAlive(startedMarkers, 3);
      const threeAgents = await measurePhaseWithUiTrace(page, testInfo, "n3");
      expectMarkerProcessesAlive(startedMarkers, 3);

      createdWindowIds = (await codexWindowIds(page))
        .filter((id) => !initialWindowIds.has(id));
      const expectedCargoJobs = Math.max(
        1,
        Math.floor(availableParallelism() / MAX_ACTIVE),
      );
      for (const marker of startedMarkers.slice(0, 3)) {
        const content = await readFile(`${BENCHMARK_DIR}/${marker}`, "utf8");
        expect(content).toContain(`cargo_build_jobs=${expectedCargoJobs}`);
        if (process.platform !== "win32") {
          expect(content).toContain("nice=10");
        }
      }

      const evidence = {
        baseline,
        one_agent: oneAgent,
        three_agents: threeAgents,
        n3_p95_multiplier_vs_n1: threeAgents.p95_ms / oneAgent.p95_ms,
      };
      await testInfo.attach("agent-resource-isolation.json", {
        body: Buffer.from(JSON.stringify(evidence, null, 2)),
        contentType: "application/json",
      });
      console.log(JSON.stringify(evidence));

      expect(threeAgents.p95_ms).toBeLessThanOrEqual(100);
      expect(threeAgents.p95_ms).toBeLessThanOrEqual(
        oneAgent.p95_ms * MAX_RTT_P95_MULTIPLIER,
      );
      expect(threeAgents.raf_gap_p95_ms).toBeLessThanOrEqual(100);
    } finally {
      try {
        const cleanupMarkers = new Set([
          ...startedMarkers,
          ...(await markerNames(initialMarkers)),
        ]);
        if (createdWindowIds.length === 0) {
          createdWindowIds = (await codexWindowIds(page))
            .filter((id) => !initialWindowIds.has(id));
        }
        for (const [index, id] of createdWindowIds.entries()) {
          await sendLiveGwtEvent(page, {
            kind: "close_window",
            id,
            request_id: `resource-benchmark-cleanup-${index}`,
          });
        }
        await expect(page.locator('.workspace-window[data-preset="codex"]'))
          .toHaveCount(initialWindowIds.size, { timeout: 20_000 });
        await new Promise((resolve) => setTimeout(resolve, 100));
        for (const marker of await markerNames(initialMarkers)) {
          cleanupMarkers.add(marker);
        }
        await waitForMarkerProcessesToExit([...cleanupMarkers]);
      } finally {
        await setMaxActive(page, originalMaxActive);
      }
    }
    expect(await latestMaxActive(page)).toBe(originalMaxActive);
    expect(browserErrors).toEqual([]);
  });
});
