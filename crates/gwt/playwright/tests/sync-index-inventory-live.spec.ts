// SPEC #3820 T-006e: provisional latency samples for inventory paths
// P12 (GitHub sync: refresh request -> refreshed Issue list delivered) and
// P11 (index build: rebuild request -> requested cell reported healthy).
// P10 (Issues scan) runs only with GWT_PERF_INVENTORY_P10_ONLY=1 against the
// read-only GitHub loopback replay: the enabled Monitor's own poll timer is the
// trigger (no agent Session), and no launch profile exists, so nothing launches.
// Never run P10 against a real repository: settlement can propose GitHub writes.
// This is a measurement driver, not a regression gate. It runs only against an
// isolated browser-check instance with live or replayed GitHub reads and writes raw
// samples to GWT_PERF_INVENTORY_OUT. See docs/perf/issue-3820/README.md.
import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readdir, readFile, realpath, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { gotoLiveGwt, readLiveHubCatalog, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const SAMPLES = Number(process.env.GWT_PERF_INVENTORY_SAMPLES ?? "5");
const REMOTE = process.env.GWT_PERF_INVENTORY_REMOTE ?? "https://github.com/akiojin/gwt.git";
const P12_ONLY = process.env.GWT_PERF_INVENTORY_P12_ONLY === "1";
const P10_ONLY = process.env.GWT_PERF_INVENTORY_P10_ONLY === "1";
test.use({ launchOptions: { args: ["--window-position=-32000,-32000", "--disable-backgrounding-occluded-windows",
  "--disable-renderer-backgrounding", "--disable-background-timer-throttling"] } });

test.describe("SPEC #3820 sync / index inventory", () => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME;
  const out = process.env.GWT_PERF_INVENTORY_OUT;
  test.skip(!base || !home || !out, "requires an isolated browser-check instance and GWT_PERF_INVENTORY_OUT");
  test.setTimeout(10_800_000);

  test("P12 / P11 samples on a GitHub-backed project", async ({ page }, info) => {
    await withLiveGwtBackendLock(base!, info, async () => {
      const fixture = await realpath(await mkdtemp(join(home!, "perf-3820-sync-")));
      const source = join(fixture, "source");
      const root = join(fixture, "project");
      const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith("GWT_")));
      Object.assign(env, { HOME: home!, USERPROFILE: home!, GIT_CONFIG_NOSYSTEM: "1" });
      execFileSync("git", ["init", "-q", source], { env });
      execFileSync("git", ["-C", source, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
        "commit", "--allow-empty", "-qm", "Perf fixture"], { env });
      // Only the remote URL is needed: gwt resolves owner/repo from it. Nothing is fetched or pushed.
      execFileSync("git", ["-C", source, "remote", "add", "origin", REMOTE], { env });
      execFileSync("git", ["-C", source, "worktree", "add", "--detach", "-q", root], { env });
      const gwtd = join(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT!, "target", "debug", "gwtd");
      const runGwtd = (operation: string) => JSON.parse(execFileSync(gwtd, { cwd: root, env, encoding: "utf8",
        input: JSON.stringify({ schema_version: 1, operation, params: {} }) + "\n" }));
      const key: string = runGwtd("issue.monitor.status").project_store?.hash;
      expect(key).toMatch(/^[a-f0-9]{16}$/);
      const projectState = join(home!, ".gwt", "projects", key, "project-state");
      await mkdir(projectState, { recursive: true });
      await writeFile(join(projectState, "pm.json"), '{"settings":{"auto_start":false}}');
      // Disabled Monitor: no periodic scan, no claim, no agent launch. P10 enables
      // it without a launch profile so the poll timer scans but cannot launch.
      await writeFile(join(projectState, "issue-monitor.json"), JSON.stringify({ enabled: P10_ONLY,
        autonomous_mode: false, max_active_agents: 1, priority_order: [] }));

      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await readLiveHubCatalog(page, base!, {
        send: { kind: "reopen_recent_project", path: root },
        until: (catalog, expected) => catalog.projects.some(project => project.project_key === expected), arg: key,
      });
      let issueWindow: string | null = null;
      try {
        await page.bringToFront();
        await gotoLiveGwt(page, base!, { projectKey: key, enableTestBridge: true });
        const readiness = await page.waitForFunction(() => (window as any).__gwtPlaywrightMessages
          .find(({ payload }: any) => payload.kind === "migration_detected" ||
            (payload.kind === "project_owner_health" && payload.health.state === "healthy"))?.payload.kind,
        undefined, { timeout: 30_000 }).then(value => value.jsonValue()).catch(() => "owner readiness timed out");
        expect(readiness, "GUI presence (Healthy project owner)").toBe("project_owner_health");
        await installRecorder(page);

        if (P10_ONLY) {
          const p10 = await measureP10(home!, [root, source], SAMPLES);
          expect(errors, "console and page errors").toEqual([]);
          const record = { samples_per_path: SAMPLES, remote: REMOTE, checkout: process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT,
            recorded_at: new Date().toISOString(), P10: p10, console_errors: errors };
          await mkdir(out!, { recursive: true });
          await writeFile(join(out!, `scan-${info.project.name}.json`), JSON.stringify(record, null, 2) + "\n");
          return;
        }
        issueWindow = await createIssueWindow(page);
        if (P12_ONLY) {
          const warmup = await measureP12(page, issueWindow, home!, 1);
          expect(warmup.samples[0].outcome, "P12 cache warmup").toBe("refreshed");
        }
        const p12 = await measureP12(page, issueWindow, home!, SAMPLES);
        if (P12_ONLY) {
          for (const sample of p12.samples) {
            expect(sample.outcome, sample.error ?? "P12 refresh").toBe("refreshed");
            expect(sample.entries).toBeGreaterThan(0);
          }
        }
        const p11 = P12_ONLY ? null : await measureP11(page, root);
        expect(errors, "console and page errors").toEqual([]);
        const record = {
          samples_per_path: SAMPLES, remote: REMOTE, checkout: process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT,
          recorded_at: new Date().toISOString(), P12: p12, P11: p11, console_errors: errors,
        };
        await mkdir(out!, { recursive: true });
        const filename = P12_ONLY ? `sync-${info.project.name}.json` : "sync-index.json";
        await writeFile(join(out!, filename), JSON.stringify(record, null, 2) + "\n");
        await info.attach("sync-index", { body: JSON.stringify(record), contentType: "application/json" });
      } finally {
        if (issueWindow) await sendLiveGwtEvent(page, { kind: "close_window", id: issueWindow }).catch(() => {});
        await readLiveHubCatalog(page, base!, {
          send: { kind: "preview_close_project", project_key: key, project_wide: true },
        }).catch(() => {});
      }
    });
  });
});

// Every received frame keeps its arrival time (performance.now() clock), so a
// sample is end-frame arrival minus the moment the request was handed to the socket.
async function installRecorder(page: Page) {
  await page.evaluate(() => {
    const frames: Array<{ t: number; payload: any }> = [];
    const sent: Array<{ t: number; payload: any }> = [];
    // The app assigns its own request ids, so read the one that reached the socket.
    const originalSend = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data: any) {
      try {
        const payload = JSON.parse(String(data));
        if (payload.kind === "load_knowledge_bridge") sent.push({ t: performance.now(), payload });
      } catch { /* non-JSON frames are not measured */ }
      return originalSend.call(this, data);
    };
    for (const socket of (window as any).__gwtPlaywrightSockets as WebSocket[]) {
      socket.addEventListener("message", (event: MessageEvent) => {
        try {
          const payload = JSON.parse(String(event.data));
          if (["knowledge_entries", "knowledge_error", "project_index_status", "window_command_error"]
            .includes(payload.kind)) frames.push({ t: event.timeStamp, payload });
        } catch { /* non-JSON frames are not measured */ }
      });
    }
    (window as any).__perf3820sync = { frames, sent, mark: () => performance.now() };
  });
}

async function send(page: Page, payload: unknown): Promise<number> {
  const started = await page.evaluate(() => (window as any).__perf3820sync.mark());
  await sendLiveGwtEvent(page, payload);
  return started;
}

async function waitFrame(page: Page, after: number, predicate: string, timeoutMs: number): Promise<{ t: number; payload: any }> {
  const handle = await page.waitForFunction(({ after, predicate }) => {
    const test = new Function("payload", `return (${predicate});`);
    return (window as any).__perf3820sync.frames.find((frame: any) => frame.t >= after && test(frame.payload)) ?? false;
  }, { after, predicate }, { timeout: timeoutMs, polling: 50 });
  return handle.jsonValue();
}

async function projectWindows(page: Page): Promise<Array<{ id: string; preset: string }>> {
  return page.evaluate(() => {
    const workspace = (window as any).__gwtPlaywrightMessages.findLast((entry: any) => entry.payload.kind === "workspace_state").payload.workspace;
    return workspace.tabs.flatMap((tab: any) => tab.workspace.windows);
  });
}

async function createIssueWindow(page: Page): Promise<string> {
  const before = new Set((await projectWindows(page)).map(window => window.id));
  await sendLiveGwtEvent(page, { kind: "create_window", preset: "issue" });
  await expect.poll(async () => (await projectWindows(page)).filter(window => window.preset === "issue" && !before.has(window.id)))
    .toHaveLength(1);
  return (await projectWindows(page)).find(window => window.preset === "issue" && !before.has(window.id))!.id;
}

// P12: Refresh forces the whole Issue cache sync; the end is the refreshed list
// answering that request.
async function measureP12(page: Page, id: string, home: string, count: number) {
  const samples: Array<{ ms: number; outcome: string; error: string | null; entries: number | null; gh_starts: number }> = [];
  for (let index = 0; index < count; index += 1) {
    const requestId = 3_820_000 + index;
    const wallStart = new Date();
    const started = await send(page, { kind: "load_knowledge_bridge", id, knowledge_kind: "issue",
      request_id: requestId, selected_number: null, refresh: true });
    const outgoing = await page.waitForFunction(({ started, id }) => (window as any).__perf3820sync.sent
      .find((entry: any) => entry.t >= started && entry.payload.id === id && entry.payload.refresh === true) ?? false,
    { started, id }, { timeout: 30_000, polling: 50 }).then(handle => handle.jsonValue()) as { t: number; payload: any };
    const sentId = outgoing.payload.request_id;
    const frame = await waitFrame(page, outgoing.t,
      `(["knowledge_entries", "knowledge_error"].includes(payload.kind) && payload.id === ${JSON.stringify(id)} && payload.request_id === ${JSON.stringify(sentId)})` +
      ` || (payload.kind === "window_command_error" && payload.id === ${JSON.stringify(id)})`, 2_700_000);
    expect(frame.payload.kind, JSON.stringify(frame.payload).slice(0, 300)).not.toBe("window_command_error");
    // A refused refresh is a sample too: it is the answer the user waited for.
    samples.push({ ms: frame.t - outgoing.t, outcome: frame.payload.kind === "knowledge_entries" ? "refreshed" : "error",
      error: frame.payload.message ?? null, entries: frame.payload.entries?.length ?? null,
      gh_starts: await countGhStarts(home, wallStart, new Date()) });
    await page.waitForTimeout(2_000);
  }
  return { ...summarize(samples.map(sample => sample.ms)), samples };
}

// P11: the request is accepted when the optimistic repairing status arrives;
// the end is the first status after it that reports the Issues cell healthy.
async function measureP11(page: Page, root: string) {
  const samples: Array<{ ms: number; accepted_ms: number }> = [];
  const forRoot = `payload.kind === "project_index_status" && payload.project_root === ${JSON.stringify(root)}`;
  for (let index = 0; index < SAMPLES; index += 1) {
    const started = await send(page, { kind: "rebuild_index_cell", project_root: root, scope: "issues", worktree_hash: null });
    const accepted = await waitFrame(page, started, `${forRoot} && Boolean(payload.status.repair_started_at)`, 120_000);
    const healthy = await waitFrame(page, accepted.t + 0.001,
      `${forRoot} && !payload.status.repair_started_at && payload.status.scopes?.issues?.healthy === true`, 1_800_000);
    samples.push({ ms: healthy.t - started, accepted_ms: accepted.t - started });
    await page.waitForTimeout(2_000);
  }
  return { ...summarize(samples.map(sample => sample.ms)), samples };
}

// P10: one sample is one daemon scan pass of this project (span root: worktree or repository), from its first stage
// (prefs-load started) to the scan's own `total` (the outer timer drops last, after
// nested timers). The durable commit and launch decision run after the pass and are
// listed per instance, not per project, because they log no project root.
async function measureP10(home: string, roots: string[], count: number) {
  const from = Date.now();
  let parsed = parseScanLog([], roots, from);
  await expect.poll(async () => {
    parsed = parseScanLog(await readGwtLog(home), roots, from);
    return parsed.scans.length;
  }, { timeout: 3_600_000, intervals: [5_000] }).toBeGreaterThanOrEqual(count + 1);
  // The first pass after project open also loads the cold candidate cache.
  const [warmup, ...scans] = parsed.scans;
  const measured = scans.slice(0, count);
  return { ...summarize(measured.map(scan => scan.ms)), warmup, samples: measured,
    durable_commit_ms: parsed.commits, launch_decision_ms: parsed.decisions };
}

async function readGwtLog(home: string): Promise<string[]> {
  const dir = join(home, ".gwt", "logs");
  const lines: string[] = [];
  for (const name of (await readdir(dir).catch(() => [] as string[])).filter(name => name.startsWith("gwt.log")).sort()) {
    lines.push(...(await readFile(join(dir, name), "utf8")).split("\n"));
  }
  return lines;
}

function parseScanLog(lines: string[], roots: string[], from: number) {
  const scans: Array<{ ms: number; started_at: string; stages: Record<string, number> }> = [];
  const commits: number[] = [];
  const decisions: number[] = [];
  let current: { started_at: string; stages: Record<string, number>; total?: number } | null = null;
  const close = () => { if (current?.total !== undefined) scans.push({ ms: current.total, started_at: current.started_at, stages: current.stages }); };
  for (const line of lines) {
    if (!line.includes("gwt.issue_monitor.scan")) continue;
    let entry: any;
    try { entry = JSON.parse(line); } catch { continue; }
    if (entry.target !== "gwt.issue_monitor.scan" || Date.parse(entry.timestamp ?? "") < from) continue;
    const { stage, state, duration_ms: ms } = entry.fields ?? {};
    const inScan = entry.span?.name === "issue_monitor_scan" && roots.includes(entry.span?.project_root);
    if (inScan && stage === "prefs-load" && state === "started") {
      close();
      current = { started_at: entry.timestamp, stages: {} };
    } else if (inScan && current && state === "finished" && typeof ms === "number") {
      if (stage === "total") current.total = ms; else current.stages[stage] = ms;
    } else if (!inScan && state === "finished" && typeof ms === "number") {
      // Both follow-up timers have a single stage, so its finished line is the whole timer.
      if (stage === "durable-commit") commits.push(ms);
      else if (stage === "launch-decision") decisions.push(ms);
    }
  }
  close();
  return { scans, commits, decisions };
}

// `gh` starts the isolated instance logged inside the sample window.
async function countGhStarts(home: string, from: Date, to: Date): Promise<number> {
  const dir = join(home, ".gwt", "logs");
  let count = 0;
  for (const name of (await readdir(dir).catch(() => [] as string[])).filter(name => name.startsWith("gwt.log"))) {
    for (const line of (await readFile(join(dir, name), "utf8")).split("\n")) {
      if (!line.includes("gwt.process.summary") || !line.includes('"kind":"gh"') || !line.includes('"phase":"start"')) continue;
      const at = Date.parse(JSON.parse(line).timestamp ?? "");
      if (at >= from.getTime() && at <= to.getTime()) count += 1;
    }
  }
  return count;
}

function summarize(values: number[]) {
  const sorted = [...values].sort((a, b) => a - b);
  const rank = (q: number) => sorted[Math.max(0, Math.ceil(q * sorted.length) - 1)];
  return { n: values.length, p50_ms: rank(0.5), p95_ms: rank(0.95), max_ms: sorted[sorted.length - 1], samples_ms: values };
}
