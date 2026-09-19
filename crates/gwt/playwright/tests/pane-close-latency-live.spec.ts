/**
 * Issue #3752 — pane close → list round-trip latency (live backend).
 *
 * Issue #3705 fixed the regression budget at `< 400ms` for four consecutive
 * closes of live-PTY panes, and the Rust runtime test locks the in-process
 * handler to that budget. The symptom that stayed after PR #3719 lived only
 * in the real GUI dispatch path: the Work refresh continuations (events
 * ingest, tip subjects, PR titles, merge status) rebuilt the disk-backed
 * Active Work projection on the tao event loop, and every `close_window` /
 * `list_windows` queued behind a multi-second build. Unit and focused tests
 * cannot see that queueing, so this spec is the real-browser measurement the
 * Issue asks for (AC-1 / AC-3 / AC-5):
 *
 *   - open four Shell windows (each owns a live PTY) on the live backend;
 *   - wait until the Work refresh cycle that project-open triggers has
 *     started publishing projections, so the closes overlap the refresh
 *     continuations instead of landing in a quiet window;
 *   - close the four panes back-to-back with no pause between closes and
 *     measure `close_window` → `list_windows` → `window_list` inside the page;
 *   - keep probing `list_windows` for a while afterwards and report what the
 *     bridge answered, so a stall in the remaining continuations is visible.
 *
 * Only the four-close budget is asserted as a wall-clock number. The
 * post-burst probes assert *liveness* — every probe must get a `window_list`
 * back, and it must not list a closed pane — but their duration is reported,
 * not gated. Measured across six headed runs, the burst itself never exceeded
 * 111.5ms, while one post-burst probe reached 5674ms on a saturated host. The
 * `perf` log for that run named the cause: `route:work.hook_health` took
 * 24492ms and recorded its own 5000ms budget violation while `route:pane.close`
 * stayed at 1.5-2.9ms. That stall is real, but it belongs to the Work-rows
 * managed-hook surface read (Issue #4370) and the works.json growth behind it
 * (Issue #4508), not to the close path this spec guards. Asserting it here
 * would turn a 1-in-6 host-load excursion into a red CI run on unrelated PRs,
 * which is exactly the wall-clock flake class Issue #3882 just removed.
 *
 * The timing is taken with `performance.now()` inside the page so Playwright
 * RPC overhead is not part of the sample. The spec runs under both
 * `chromium-dark` and `chromium-light` projects and self-skips when
 * `GWT_PLAYWRIGHT_BASE_URL` is unset, like every other live spec.
 * Set `GWT_PLAYWRIGHT_HEADLESS=1` to run it headless (CI without a display).
 */
import { test, expect } from "@playwright/test";
import {
  gotoLiveGwt,
  openLiveGwtProject,
  sendLiveGwtEvent,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const PANE_COUNT = 4;
const CLOSE_BUDGET_MS = 400;
const PANE_READY_TIMEOUT_MS = 30_000;
// The events ingest that project-open forces runs on a worker and can take
// minutes on a large repository before the first projection lands.
const WORK_CYCLE_TIMEOUT_MS = 240_000;
// After the burst, the tip-subject / PR-title / merge-status continuations
// of the same cycle land one after another; probe across that window.
const POST_BURST_PROBE_WINDOW_MS = 20_000;
const POST_BURST_PROBE_INTERVAL_MS = 250;

// Headed by default: the Issue requires a real browser measurement. CI hosts
// without a display opt into headless explicitly.
test.use({ headless: process.env.GWT_PLAYWRIGHT_HEADLESS === "1" });

type CloseSample = {
  id: string;
  closeToListMs: number;
  domRemovedMs: number | null;
  listedIds: string[];
};

async function windowIds(page): Promise<string[]> {
  return page.evaluate(() =>
    Array.from(document.querySelectorAll(".workspace-window"))
      .map((node) => (node as HTMLElement).dataset.id || "")
      .filter(Boolean),
  );
}

async function createShellWindow(page, index: number): Promise<string> {
  const beforeIds = await windowIds(page);
  await sendLiveGwtEvent(page, {
    kind: "create_window",
    preset: "shell",
    bounds: { x: 64 + index * 48, y: 64 + index * 48, width: 720, height: 420 },
  });
  const handle = await page.waitForFunction(
    ({ beforeIds }) => {
      const seen = new Set(beforeIds);
      const node = Array.from(document.querySelectorAll(".workspace-window")).find(
        (candidate) => !seen.has((candidate as HTMLElement).dataset.id || ""),
      );
      return node ? (node as HTMLElement).dataset.id || "" : "";
    },
    { beforeIds },
    { timeout: PANE_READY_TIMEOUT_MS },
  );
  const id = await handle.jsonValue();
  expect(id, "create_window must add a workspace window").not.toBe("");
  return id;
}

async function installWindowStateRecorder(page): Promise<void> {
  // The shared test bridge keeps only the last 256 frames, and four live
  // shells flood it with terminal output. Record the latest status per
  // window id at push time so readiness never depends on the ring size.
  // A window's status arrives either as a `window_state` transition or
  // inside the `window_canvas_state` / `window_list` snapshots, whichever
  // the backend sends first for a freshly created pane.
  await page.evaluate(() => {
    const states: Record<string, string> = {};
    (window as any).__gwtPaneWindowStates = states;
    (window as any).__gwtActiveWorkProjectionCount = 0;
    const recordSnapshot = (windows: unknown) => {
      if (!Array.isArray(windows)) return;
      for (const window of windows as Array<{ id?: unknown; status?: unknown }>) {
        if (typeof window?.id === "string" && typeof window?.status === "string") {
          states[window.id] = window.status;
        }
      }
    };
    const messages = (window as any).__gwtPlaywrightMessages as unknown[];
    const originalPush = messages.push.bind(messages);
    messages.push = (...entries: any[]) => {
      for (const entry of entries) {
        const payload = entry?.payload;
        if (payload?.kind === "window_state" && typeof payload.window_id === "string") {
          states[payload.window_id] = String(payload.state);
        } else if (payload?.kind === "window_list") {
          recordSnapshot(payload.windows);
        } else if (payload?.kind === "window_canvas_state") {
          for (const tab of payload.workspace?.tabs ?? []) {
            recordSnapshot(tab?.workspace?.windows);
          }
        } else if (payload?.kind === "active_work_projection") {
          (window as any).__gwtActiveWorkProjectionCount += 1;
        }
      }
      return originalPush(...entries);
    };
  });
}

async function waitForRunningPty(page, id: string): Promise<void> {
  // `window_state` carries the process status of the PTY behind the window.
  // Waiting for `running` guarantees each close targets a live PTY, which is
  // the reproduction condition recorded on the Issue.
  await page.waitForFunction(
    ({ id }) => (window as any).__gwtPaneWindowStates?.[id] === "running",
    { id },
    { timeout: PANE_READY_TIMEOUT_MS },
  );
}

async function projectionBroadcastCount(page): Promise<number> {
  return page.evaluate(() => Number((window as any).__gwtActiveWorkProjectionCount) || 0);
}

async function waitForWorkRefreshCycle(page, seenBefore: number): Promise<void> {
  // `reopen_recent_project` forces the work-events ingest; its continuation
  // publishes a full `active_work_projection`. From that point the remaining
  // continuations of the cycle (tip subjects, PR titles, merge status) land
  // one after another, which is the window the Issue's closes must survive.
  await page.waitForFunction(
    ({ seenBefore }) => (Number((window as any).__gwtActiveWorkProjectionCount) || 0) > seenBefore,
    { seenBefore },
    { timeout: WORK_CYCLE_TIMEOUT_MS },
  );
}

async function closeAndList(page, id: string, close = true): Promise<CloseSample> {
  return page.evaluate(async ({ id, budget, close }) => {
    const send = (detail: unknown) =>
      window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail }));
    const messages = () =>
      ((window as any).__gwtPlaywrightMessages as Array<{ sequence: number; payload: any }>) ?? [];
    const cursor = Number((window as any).__gwtPlaywrightMessageSequence) || 0;
    const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

    const started = performance.now();
    if (close) send({ kind: "close_window", id });
    send({ kind: "list_windows" });

    let listedIds: string[] | null = null;
    let domRemovedMs: number | null = null;
    const kindsAfterCursor = () =>
      messages()
        .filter((entry) => entry.sequence > cursor)
        .map((entry) => String(entry.payload?.kind ?? "?"));
    // Give the backend well past the budget so a slow answer is reported as
    // a number instead of a timeout.
    const deadline = started + budget * 40;
    while (performance.now() < deadline) {
      if (domRemovedMs === null && !document.querySelector(`.workspace-window[data-id="${id}"]`)) {
        domRemovedMs = performance.now() - started;
      }
      const reply = messages().find(
        (entry) =>
          entry.sequence > cursor
          && entry.payload?.kind === "window_list"
          && Array.isArray(entry.payload.windows)
          && !entry.payload.windows.some((window: any) => window?.id === id),
      );
      if (reply) {
        listedIds = reply.payload.windows.map((window: any) => String(window?.id ?? ""));
        break;
      }
      await sleep(1);
    }
    const closeToListMs = performance.now() - started;
    if (listedIds === null) {
      // Name what the bridge did receive so a missing reply is diagnosable
      // (ring eviction, reconnect, or a genuinely silent backend).
      const kinds = kindsAfterCursor();
      const summary = kinds.reduce<Record<string, number>>((acc, kind) => {
        acc[kind] = (acc[kind] ?? 0) + 1;
        return acc;
      }, {});
      listedIds = [`<no window_list reply after ${kinds.length} frames: ${JSON.stringify(summary)}>`];
    }
    return { id, closeToListMs, domRemovedMs, listedIds };
  }, { id, budget: CLOSE_BUDGET_MS, close });
}

function expectWindowListReply(sample: CloseSample, label: string): void {
  const missing = sample.listedIds.find((id) => id.startsWith("<no window_list reply"));
  expect(missing, `${label}: ${missing ?? ""}`).toBeUndefined();
}

test.describe.serial("pane close latency (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  test(
    "four consecutive live-PTY closes each answer close→list under 400ms",
    async ({ page }, testInfo) => {
      test.setTimeout(WORK_CYCLE_TIMEOUT_MS + 120_000);
      // AGENTS.md headed E2E gate: no console errors and no page errors
      // while the closes and probes run.
      const consoleErrors: string[] = [];
      const pageErrors: string[] = [];
      page.on("console", (message) => {
        if (message.type() === "error") consoleErrors.push(message.text());
      });
      page.on("pageerror", (error) => pageErrors.push(String(error)));
      await withLiveGwtBackendLock(BASE, testInfo, async () => {
        await gotoLiveGwt(page, BASE, { enableTestBridge: true });
        await installWindowStateRecorder(page);
        const projectionsBeforeOpen = await projectionBroadcastCount(page);
        await openLiveGwtProject(page);

        const ids: string[] = [];
        for (let index = 0; index < PANE_COUNT; index += 1) {
          ids.push(await createShellWindow(page, index));
        }
        for (const id of ids) {
          await waitForRunningPty(page, id);
        }
        await waitForWorkRefreshCycle(page, projectionsBeforeOpen);

        const samples: CloseSample[] = [];
        for (const id of ids) {
          samples.push(await closeAndList(page, id));
        }

        // AC-3: the bridge must keep *answering* while the rest of the refresh
        // cycle lands. That is the symptom the Issue names —
        // `pane_backend_unresponsive` and a failed websocket handshake — and it
        // is what `expectWindowListReply` checks. How long each answer took is
        // reported below but deliberately not asserted; see the file header.
        const probes: number[] = [];
        const probeDeadline = Date.now() + POST_BURST_PROBE_WINDOW_MS;
        while (Date.now() < probeDeadline) {
          const probe = await closeAndList(page, ids[ids.length - 1], false);
          probes.push(probe.closeToListMs);
          expectWindowListReply(probe, "post-burst list_windows probe");
          for (const id of ids) {
            expect(probe.listedIds).not.toContain(id);
          }
          await page.waitForTimeout(POST_BURST_PROBE_INTERVAL_MS);
        }

        const report = samples
          .map((sample) =>
            `${sample.id}: close→list ${sample.closeToListMs.toFixed(1)}ms, `
            + `dom removed ${sample.domRemovedMs === null ? "n/a" : `${sample.domRemovedMs.toFixed(1)}ms`}`)
          .concat([
            `post-burst list_windows probes: n=${probes.length}, `
            + `max ${Math.max(...probes).toFixed(1)}ms`,
          ])
          .join("\n");
        testInfo.annotations.push({ type: "pane-close-latency", description: report });
        console.log(`[pane-close-latency] ${testInfo.project.name}\n${report}`);

        for (const sample of samples) {
          expectWindowListReply(sample, `window_list after closing ${sample.id}`);
          expect(
            sample.closeToListMs,
            `close→list for ${sample.id} exceeded the ${CLOSE_BUDGET_MS}ms budget:\n${report}`,
          ).toBeLessThan(CLOSE_BUDGET_MS);
        }
        expect(pageErrors, "page errors during the close burst").toEqual([]);
        expect(consoleErrors, "console errors during the close burst").toEqual([]);
      });
    },
  );
});
