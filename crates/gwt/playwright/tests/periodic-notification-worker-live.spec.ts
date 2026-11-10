/** Issue #5597: real periodic collection leaves only cheap GUI notification work. */
import { readFile, readdir, realpath, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const HANDLERS = ["ProviderUsageSnapshot", "IssueMonitorCapacityChanged"];
type Fixture = { project: string; repo_hash: string; pid: number; started_at: string; git_head: string; sha256: Record<string, string> };
type LogRow = { file: string; raw: string; record: any };

test.describe("Periodic notification worker (live backend)", () => {
  test.skip(!BASE || !HOME || process.platform !== "win32", "requires the isolated Windows checkout harness");
  test.setTimeout(180_000);
  test.use({ viewport: { width: 1440, height: 1000 } });

  test("usage refreshes and capacity sampling keep the GUI responsive", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const fixture = await requireFixture();
      const theme = info.project.name.includes("light") ? "light" : "dark";
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const requests: { elapsed_ms: number; reply_ms: number; forced_usage: boolean }[] = [];
      const capacityObservations = new Set<number>();
      let startedAt: string | undefined;
      let endedAt: string | undefined;
      let windowId: string | undefined;
      let passed = false;
      try {
        await page.bringToFront();
        await gotoLiveGwt(page, BASE, { enableTestBridge: true });
        await expect(page.locator("#close-project-button")).toBeVisible();
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await page.locator(".op-rail").getByRole("button", { name: "Issues", exact: true }).click();
        const monitor = page.locator('.split-pane[data-active="true"][data-surface="issues"] .workspace-window[data-preset="issue"]');
        await expect(monitor).toBeVisible();
        windowId = await monitor.getAttribute("data-id") ?? undefined;
        await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
          .some(({ payload }: any) => payload.kind === "provider_usage"));
        // Retain every usage observation: the helper's general frame buffer is
        // bounded, while this evidence spans more than two normal 30s ticks.
        await page.evaluate(() => {
          const state = window as any;
          state.__gwt5597Usage = [];
          for (const socket of state.__gwtPlaywrightSockets as WebSocket[]) socket.addEventListener("message", event => {
            const payload = JSON.parse(String(event.data));
            if (payload.kind === "provider_usage") state.__gwt5597Usage.push({ at: Date.now(), accounts: payload.accounts });
          });
        });
        const clock = performance.now();
        startedAt = new Date().toISOString();
        // Three unchanged requests exercise forced collection. The remainder
        // observes the unmodified 30s usage and 5s capacity worker cadences.
        do {
          const forcedUsage = requests.length < 3;
          const replyMs = await requestReply(page, forcedUsage);
          requests.push({ elapsed_ms: performance.now() - clock, reply_ms: replyMs, forced_usage: forcedUsage });
          const snapshot = await readFile(join(HOME, ".gwt/machine-state/agent-capacity.json"), "utf8")
            .then(JSON.parse).catch(error => { if (error.code === "ENOENT") return null; throw error; });
          if (snapshot) capacityObservations.add(snapshot.observed_at);
          await expect(monitor).toBeVisible();
          await page.waitForTimeout(5_000);
        } while (performance.now() - clock < 65_000);
        endedAt = new Date().toISOString();
        const usage = await usageRows(page);
        expect(usage.length, "forced and ordinary usage collection both returned").toBeGreaterThanOrEqual(5);
        expect(new Set(usage.map(row => JSON.stringify(semanticAccounts(row.accounts)))).size,
          "fresh HOME keeps quota-relevant account usage unchanged").toBe(1);
        expect(capacityObservations.size, "real capacity snapshots renewed during observation").toBeGreaterThanOrEqual(2);
        const rows = await timingRows(fixture);
        for (const handler of HANDLERS) {
          const samples = dispatchSamples(rows, handler);
          expect(samples.length, `${handler} timing instrumentation is live`).toBeGreaterThan(0);
          assertBudget(samples, handler);
        }
        assertBudget(rows, "combined periodic handlers");
        expect(dispatchSamples(rows, HANDLERS[0]).filter(row => Date.parse(row.record.timestamp) >= Date.parse(startedAt!)),
          "unchanged forced and ordinary usage do not enqueue GUI quota work").toHaveLength(0);
        const summaries = await dispatchSummaries(fixture.pid);
        expect(summaries.length, "minute collector emitted measured evidence").toBeGreaterThan(0);
        expect(summaries.every(row => row.record.dispatch.dropped_windows === 0)).toBe(true);
        expect(errors, "zero console/page errors").toEqual([]);
        await info.attach(`periodic-notification-${theme}`, { body: await page.screenshot({ animations: "disabled" }), contentType: "image/png" });
        passed = true;
      } finally {
        endedAt ??= new Date().toISOString();
        const rows = await timingRows(fixture);
        const summaries = await dispatchSummaries(fixture.pid);
        const evidence = {
          issue: 5597, theme, passed, fixture, started_at: startedAt, ended_at: endedAt,
          observation_ms: startedAt ? Date.parse(endedAt) - Date.parse(startedAt) : 0,
          requests, capacity_observed_at: [...capacityObservations], usage: await usageRows(page).catch(() => []), errors,
          handlers: HANDLERS.map(handler => ({ handler, ...budget(dispatchSamples(rows, handler)) })),
          combined_handlers: budget(rows),
          timing_log_rows: rows, gui_dispatch_summary_rows: summaries,
          limitations: "Dispatch summaries include only handlers over 50ms; their absence for these handlers is not a 16ms proof. Debug rows include every dispatch. elapsed_ms is truncated; +1ms per sample gives a conservative upper bound. All captured handler rows since owned GUI startup are included in the budgets.",
          user_verification_result: "n/a (autonomous)", agent_visual_check: passed ? "pass" : "fail",
        };
        const path = info.outputPath(`periodic-notification-${theme}.json`);
        await writeFile(path, JSON.stringify(evidence, null, 2) + "\n");
        await info.attach(`periodic-notification-evidence-${theme}`, { path, contentType: "application/json" });
        if (windowId) await sendLiveGwtEvent(page, { kind: "close_window", id: windowId }).catch(() => undefined);
      }
    });
  });
});

async function requireFixture(): Promise<Fixture> {
  expect(await realpath(HOME), "fresh isolated HOME").not.toBe(await realpath(homedir()));
  const fixture: Fixture = JSON.parse(await readFile(join(HOME, "issue-5597-isolated.json"), "utf8"));
  expect(await realpath(fixture.project)).toBe(await realpath(process.env.GWT_PLAYWRIGHT_PROJECT_ROOT!));
  expect(fixture.pid).toBe(Number(process.env.GWT_PLAYWRIGHT_PID));
  const state = join(HOME, ".gwt/projects", fixture.repo_hash, "project-state");
  expect(JSON.parse(await readFile(join(state, "pm.json"), "utf8")).settings.auto_start).toBe(false);
  expect(JSON.parse(await readFile(join(state, "issue-monitor.json"), "utf8")).enabled).toBe(false);
  expect(new URL(BASE).origin).toBe(new URL((await readFile(join(HOME, "url.txt"), "utf8")).trim()).origin);
  return fixture;
}

async function requestReply(page: Page, refresh: boolean): Promise<number> {
  return page.evaluate(refresh => {
    const socket = [...((window as any).__gwtPlaywrightSockets as WebSocket[])].reverse()
      .find(socket => socket.readyState === WebSocket.OPEN && new URL(socket.url).searchParams.has("repo_hash"));
    if (!socket) throw new Error("project WebSocket missing");
    return new Promise<number>((resolve, reject) => {
      const started = performance.now();
      const cleanup = () => { clearTimeout(timer); socket.removeEventListener("message", reply); };
      const reply = (event: MessageEvent) => {
        if (JSON.parse(String(event.data)).kind !== "window_list") return;
        cleanup(); resolve(performance.now() - started);
      };
      const timer = setTimeout(() => { cleanup(); reject(new Error("GUI reply timed out")); }, 10_000);
      socket.addEventListener("message", reply);
      if (refresh) window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail: { kind: "refresh_usage" } }));
      window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail: { kind: "list_windows" } }));
    });
  }, refresh);
}

async function usageRows(page: Page): Promise<{ at: number; accounts: unknown[] }[]> {
  return page.evaluate(() => (window as any).__gwt5597Usage ?? []);
}

function semanticAccounts(accounts: unknown[]) {
  return accounts.map(account => {
    const current = structuredClone(account) as any;
    delete current.fetched_at;
    if (current.state?.kind === "stale") current.state.age_secs = 0;
    return current;
  });
}

async function readLogs(directory: string, accepts: (file: string) => boolean): Promise<LogRow[]> {
  const rows: LogRow[] = [];
  for (const file of (await readdir(directory).catch(() => [])).filter(accepts)) {
    for (const raw of (await readFile(join(directory, file), "utf8")).split("\n")) {
      if (!raw.trim()) continue;
      let record;
      try { record = JSON.parse(raw); } catch { continue; } // writer may have an incomplete final line
      rows.push({ file: join(directory, file), raw, record });
    }
  }
  return rows;
}

async function timingRows(fixture: Fixture): Promise<LogRow[]> {
  const roots = [join(HOME, ".gwt/logs"), join(HOME, ".gwt/projects", fixture.repo_hash, "logs")];
  const rows = (await Promise.all(roots.map(root => readLogs(root, file => file.startsWith("gwt.log."))))).flat();
  return rows.filter(({ record }) => record.target === "gwt.frontend.timing" && HANDLERS.includes(record.fields?.event)
    && Number.isFinite(record.fields?.elapsed_ms)).sort((a, b) => Date.parse(a.record.timestamp) - Date.parse(b.record.timestamp));
}

function dispatchSamples(rows: LogRow[], handler: string): LogRow[] {
  return rows.filter(row => row.record.fields.event === handler);
}

function budget(rows: LogRow[]) {
  let maxMinute = 0;
  let maxMinuteUpper = 0;
  for (let start = 0; start < rows.length; start++) {
    const window = rows.slice(start).filter(row => Date.parse(row.record.timestamp) - Date.parse(rows[start].record.timestamp) < 60_000);
    const sum = window.reduce((sum, row) => sum + row.record.fields.elapsed_ms, 0);
    maxMinute = Math.max(maxMinute, sum);
    maxMinuteUpper = Math.max(maxMinuteUpper, sum + window.length);
  }
  return { count: rows.length, max_ms: Math.max(0, ...rows.map(row => row.record.fields.elapsed_ms)),
    max_upper_bound_ms: Math.max(0, ...rows.map(row => row.record.fields.elapsed_ms + 1)),
    max_rolling_minute_total_ms: maxMinute, max_rolling_minute_upper_bound_ms: maxMinuteUpper };
}

function assertBudget(rows: LogRow[], handler: string): void {
  const measured = budget(rows);
  expect(measured.max_upper_bound_ms, `${handler} maximum including integer rounding`).toBeLessThanOrEqual(16);
  expect(measured.max_rolling_minute_upper_bound_ms, `${handler} every rolling minute including integer rounding`).toBeLessThanOrEqual(100);
}

async function dispatchSummaries(pid: number): Promise<LogRow[]> {
  return (await readLogs(join(HOME, ".gwt/logs/perf"), file => /^perf-.*\.jsonl$/.test(file)))
    .filter(row => row.record.type === "gui_dispatch_summary" && row.record.dispatch?.pid === pid);
}
