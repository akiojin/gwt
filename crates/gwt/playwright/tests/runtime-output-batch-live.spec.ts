/** #5379: five real Windows custom Agent PTYs; fixed output, no model calls. */
import { createReadStream } from "node:fs";
import { readFile, readdir, rename, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { expect, test, type Page } from "@playwright/test";
import { clearLiveLaunchWizard, gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL || "";
const HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME || "";
const AGENT = "runtime-output-5379";
type Surface = { page: Page; id: string };
type Dispatch = { event: string; count: number; stalls: number; max_ms: number };
type NativeTiming = { samples: number; stalls: number; max_ms: number; events: Dispatch[]; slow_rows: unknown[]; summaries: any[];
  output_batches: { count: number; stalls: number; max_ms: number } };

test.use({ headless: false, viewport: { width: 1280, height: 900 }, actionTimeout: 30_000, navigationTimeout: 60_000 });
test.describe("Runtime output batching (live backend)", () => {
  test.skip(!BASE || !HOME || process.platform !== "win32", "requires scripts/diagnostics/issue-5379-browser-check.ps1");
  test("five Agent PTYs complete fixed output while native dispatch is measured", async ({ page }, info) => {
    const fixture = JSON.parse(await readFile(join(HOME, "fixture.json"), "utf8"));
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const seconds = theme === "light" ? Math.min(10, fixture.duration_seconds) : fixture.duration_seconds;
    test.setTimeout((seconds + 900) * 1000);
    expect(fixture.projects).toHaveLength(5);
    expect(fixture.gui_pid).toBeGreaterThan(0);
    expect(fixture.binary_sha256).toMatch(/^[A-Fa-f0-9]{64}$/);
    const errors: string[] = [];
    const surfaces: Surface[] = [];
    const runId = fixture.arm + "-" + theme;
    const barrier = join(HOME, "workload", "barrier.json");
    let startedAt: string | undefined;
    let endedAt: string | undefined;
    let receipts: any[] = [];
    const wire: any[] = [];
    let timing: NativeTiming | undefined;
    let comparison: unknown;
    let passed = false;
    try {
      await rm(barrier, { force: true });
      for (let index = 0; index < 5; index++) {
        const surfacePage = index === 0 ? page : await page.context().newPage();
        surfacePage.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
        surfacePage.on("pageerror", error => errors.push(error.message));
        await gotoLiveGwt(surfacePage, BASE, { enableTestBridge: true, projectKey: fixture.projects[index].project_key });
        await clearLiveLaunchWizard(surfacePage);
        await surfacePage.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
        await expect(surfacePage.locator("html")).toHaveAttribute("data-theme", theme);
        const migration = surfacePage.locator("#migration-modal.open");
        if (await migration.isVisible()) {
          await migration.getByRole("button", { name: "Migrate", exact: true }).click();
          await expect(migration).toHaveCount(0, { timeout: 60_000 });
        }
        const launched = await launch(surfacePage, index, fixture.projects[index].branch_name);
        const id = launched.id;
        surfaces.push({ page: surfacePage, id });
        // The initial workspace row is a launch placeholder. The receipt
        // proves that the custom Agent process has actually started.
        await expect.poll(async () => (await producerReceipts()).some(receipt => receipt.session === launched.session_id),
          { message: "custom Agent process has started", timeout: 60_000 }).toBe(true);
        await expect.poll(() => surfacePage.evaluate(({ id, session }) => {
          const message = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.payload.kind === "workspace_state");
          const pane = message?.payload.workspace.tabs.flatMap((tab: any) => tab.workspace.windows).find((pane: any) => pane.id === id);
          return pane?.session_id === session && !["stopped", "error"].includes(pane.status);
        }, { id, session: launched.session_id }), { message: "same live Agent Session remains in the workspace", timeout: 30_000 }).toBe(true);
        await surfacePage.locator('.op-rail__surface[data-surface="agents"]').click();
        await expect(surfacePage.locator('.agents-surface:not(.agents-surface--preview) .agent-tile[data-agent-id="' + id + '"] .xterm-rows'))
          .toBeVisible({ timeout: 30_000 });
        await expect.poll(() => surfacePage.evaluate(id => String((window as any).__gwtTerminalTestApi?.bufferText?.(id) || ""), id),
          { timeout: 60_000 }).toContain("GWT5379_READY " + index);
        await installOutputProbe(surfacePage, id, index);
      }
      await expect.poll(async () => (await producerReceipts()).filter(receipt => !receipt.run_id).length,
        { timeout: 30_000 }).toBe(5);
      const startAt = Date.now() + 1000;
      startedAt = new Date(startAt).toISOString();
      // Producers observe file existence synchronously. Publish complete JSON
      // atomically so no process can read a partially written barrier.
      await writeFile(barrier + ".tmp", JSON.stringify({ start_at: startAt, seconds, run_id: runId }));
      await rename(barrier + ".tmp", barrier);
      await expect.poll(async () => {
        receipts = (await producerReceipts()).filter(receipt => receipt.run_id === runId);
        return receipts.filter(receipt => receipt.complete && receipt.sequence === seconds && receipt.bytes === seconds * 8192).length;
      }, { timeout: (seconds + 60) * 1000, intervals: [1000] }).toBe(5);
      for (const [index, surface] of surfaces.entries()) {
        await expect.poll(() => surface.page.evaluate(() => (window as any).__gwt5379Output.sequences.size),
          { timeout: 30_000 }).toBe(seconds);
        await expect.poll(() => surface.page.evaluate(id => String((window as any).__gwtTerminalTestApi?.bufferText?.(id) || ""), surface.id),
          { message: "final sequence reaches the terminal buffer", timeout: 30_000 })
          .toContain("GWT5379_SEQ_" + index + "_" + String(seconds).padStart(4, "0"));
        wire.push(await surface.page.evaluate(() => {
          const state = (window as any).__gwt5379Output;
          return { index: state.index, output_bytes: state.output_bytes, snapshot_bytes: state.snapshot_bytes,
            output_frames: state.output_frames, snapshot_frames: state.snapshot_frames, sequences: [...state.sequences].sort((a: number, b: number) => a - b) };
        }));
      }
      endedAt = new Date().toISOString();
      expect(Date.parse(endedAt) - startAt, "full continuous observation interval").toBeGreaterThanOrEqual(seconds * 1000);
      expect(receipts.map(receipt => receipt.index).sort()).toEqual([0, 1, 2, 3, 4]);
      expect(new Set(receipts.map(receipt => receipt.pid)).size).toBe(5);
      expect(new Set(receipts.map(receipt => receipt.session)).size).toBe(5);
      for (const observed of wire) {
        expect(observed.output_frames, "real PTY WebSocket output is observed").toBeGreaterThan(0);
        expect(observed.output_bytes, "PTY output includes the fixed payload bytes").toBeGreaterThanOrEqual(seconds * 8192);
        expect(observed.sequences).toEqual(Array.from({ length: seconds }, (_, index) => index + 1));
      }
      timing = await nativeTimings(fixture, startedAt, endedAt);
      expect(timing.samples, "native dispatch instrumentation is active").toBeGreaterThan(0);
      const output = timing.events.find(event => event.event === "RuntimeOutput");
      expect(output?.count || 0, "zero RuntimeOutput samples cannot pass").toBeGreaterThan(0);
      if (fixture.arm === "after") {
        expect(output!.max_ms, "RuntimeOutput dispatch stays below 100ms").toBeLessThan(100);
        expect(timing.output_batches.count, "screen-derived batch work is measured independently of callback labels").toBeGreaterThan(0);
        expect(timing.output_batches.max_ms, "each GUI output batch stays below 100ms").toBeLessThan(100);
      }
      expect(timing.summaries.every(row => row.dispatch.dropped_windows === 0), "no dispatch summary windows were dropped").toBe(true);
      if (seconds === 1800) expect(timing.summaries.length, "full run observes native minute summaries").toBeGreaterThan(0);
      if (process.env.GWT5379_BASELINE && theme === "dark") {
        const before = JSON.parse(await readFile(process.env.GWT5379_BASELINE, "utf8"));
        expect(before.passed).toBe(true);
        expect(before.fixture.arm).toBe("before");
        expect(before.theme).toBe(theme);
        expect(before.duration_seconds).toBe(seconds);
        expect(before.workload).toBe("5 custom Agent PTYs; 8192 ASCII bytes per second per Agent");
        comparison = { before_binary_sha256: before.fixture.binary_sha256, after_binary_sha256: fixture.binary_sha256,
          before: { stalls: before.native_dispatch.stalls, max_ms: before.native_dispatch.max_ms },
          after: { stalls: timing.stalls, max_ms: timing.max_ms },
          stall_count_delta: timing.stalls - before.native_dispatch.stalls, max_ms_delta: timing.max_ms - before.native_dispatch.max_ms };
      }
      expect(errors, "zero console/page errors").toEqual([]);
      await info.attach("runtime-output-" + theme, { body: await page.screenshot(), contentType: "image/png" });
      passed = true;
    } finally {
      endedAt ??= new Date().toISOString();
      if (startedAt) timing ??= await nativeTimings(fixture, startedAt, endedAt);
      if (!passed) await page.screenshot({ path: info.outputPath("runtime-output-failure.png") }).catch(() => undefined);
      const dom = await Promise.all(surfaces.map(async surface => ({ pane_id: surface.id,
        agent_tiles: await surface.page.locator(".agents-surface:not(.agents-surface--preview) .agent-tile").evaluateAll(nodes => nodes.map(node => ({
          id: (node as HTMLElement).dataset.agentId,
          terminal_rows: node.querySelectorAll(".xterm-rows").length,
        }))).catch(() => []) })));
      const evidence = { issue: 5379, fixture, theme, passed, duration_seconds: seconds,
        full_acceptance_observation: seconds === 1800 && theme === "dark", started_at: startedAt, ended_at: endedAt,
        actual_elapsed_ms: startedAt ? Date.parse(endedAt) - Date.parse(startedAt) : 0,
        workload: "5 custom Agent PTYs; 8192 ASCII bytes per second per Agent",
        producer_receipts: receipts, wire, native_dispatch: timing, comparison, browser_errors: errors, dom_snapshot: dom,
        limitations: "Deterministic custom PTYs, no provider inference/hooks/builds. Native outer RuntimeOutput dispatch remains the event label; batching flushes at the beginning of every native callback and can be attributed to any event label. All event counts and maxima are retained rather than treating RuntimeOutput alone as the flush budget. Native timing uses integer milliseconds truncated. Slow raw rows are retained; minute summaries cover >50ms events and may straddle interval boundaries. Light is a separate 10-second functional check. Preserved binary source identity must be established by the owner separately from the current checkout HEAD.",
        user_verification_result: "n/a (autonomous)", agent_visual_check: passed ? "pass" : "fail" };
      const body = JSON.stringify(evidence, null, 2) + "\n";
      await writeFile(join(HOME, fixture.arm + "-" + theme + ".json"), body);
      await info.attach("runtime-output-evidence-" + theme, { body, contentType: "application/json" });
      for (const surface of surfaces) await sendLiveGwtEvent(surface.page, { kind: "close_window", id: surface.id }).catch(() => undefined);
      for (const surface of surfaces) await expect(surface.page.locator('.agents-surface:not(.agents-surface--preview) .agent-tile[data-agent-id="' + surface.id + '"]'))
        .toHaveCount(0, { timeout: 60_000 });
      // Keep each theme's five output receipts; the next theme uses new PIDs.
      await rm(barrier, { force: true });
      for (const surface of surfaces) if (surface.page !== page) await surface.page.close();
    }
  });
});

async function producerReceipts(): Promise<any[]> {
  const directory = join(HOME, "workload");
  return Promise.all((await readdir(directory)).filter(name => /^\d+\.json$/.test(name)).map(async name => {
    const identity = JSON.parse(await readFile(join(directory, name), "utf8"));
    const progress = await readFile(join(directory, name + ".progress"), "utf8").catch(error => {
      if (error.code === "ENOENT") return ""; throw error;
    });
    const complete = progress.slice(0, progress.lastIndexOf("\n")).split("\n").at(-1);
    return complete ? { ...identity, ...JSON.parse(complete) } : identity;
  }));
}

async function installOutputProbe(page: Page, id: string, index: number): Promise<void> {
  await page.evaluate(({ id, index }) => {
    const state = { index, output_bytes: 0, snapshot_bytes: 0, output_frames: 0, snapshot_frames: 0, sequences: new Set<number>(), tail: "" };
    (window as any).__gwt5379Output = state;
    for (const socket of (window as any).__gwtPlaywrightSockets as WebSocket[]) {
      socket.addEventListener("message", event => {
        let payload;
        try { payload = JSON.parse(String(event.data)); } catch { return; }
        if (payload.id !== id || !["terminal_output", "terminal_snapshot"].includes(payload.kind)) return;
        const decoded = atob(payload.data_base64);
        const snapshot = payload.kind === "terminal_snapshot";
        if (snapshot) { state.snapshot_bytes += decoded.length; state.snapshot_frames++; }
        else { state.output_bytes += decoded.length; state.output_frames++; }
        const text = (snapshot ? "" : state.tail) + decoded;
        for (const marker of text.matchAll(/GWT5379_SEQ_([0-4])_(\d{4})/g)) {
          if (Number(marker[1]) === index) state.sequences.add(Number(marker[2]));
        }
        if (!snapshot) state.tail = text.slice(-32);
      });
    }
  }, { id, index });
}

async function nativeTimings(fixture: any, start: string, end: string): Promise<NativeTiming> {
  const roots = [join(HOME, ".gwt/logs"), ...fixture.projects.map((project: any) => join(HOME, ".gwt/projects", project.project_key, "logs"))];
  const groups = new Map<string, Dispatch>();
  const batches = { count: 0, stalls: 0, max_ms: 0 };
  const slow: unknown[] = [];
  const from = Date.parse(start), until = Date.parse(end);
  for (const root of roots) for (const name of (await readdir(root).catch(() => [])).filter(name => name.startsWith("gwt.log."))) {
    const lines = createInterface({ input: createReadStream(join(root, name)), crlfDelay: Infinity });
    for await (const raw of lines) {
      let record;
      try { record = JSON.parse(raw); } catch { continue; } // concurrent final write may be incomplete
      if (record.target !== "gwt.frontend.timing" || Date.parse(record.timestamp) < from || Date.parse(record.timestamp) > until) continue;
      const fields = record.fields;
      if (fields?.marker === "issue_5379_runtime_output_batch" && Number.isFinite(fields.elapsed_ms)) {
        batches.count++; batches.max_ms = Math.max(batches.max_ms, fields.elapsed_ms);
        if (fields.elapsed_ms >= 100) batches.stalls++;
        continue;
      }
      if (!fields || !Number.isFinite(fields.elapsed_ms) ||
        !(fields.message === "GUI event loop dispatch handled" || /^[A-Za-z]+ blocked the GUI event loop for \d+ms$/.test(fields.message || ""))) continue;
      const group = groups.get(fields.event) || { event: fields.event, count: 0, stalls: 0, max_ms: 0 };
      group.count++; group.max_ms = Math.max(group.max_ms, fields.elapsed_ms);
      if (fields.elapsed_ms >= 100) { group.stalls++; slow.push({ file: join(root, name), ...record }); }
      groups.set(group.event, group);
    }
  }
  const summaries: any[] = [];
  const directory = join(HOME, ".gwt/logs/perf");
  for (const name of (await readdir(directory).catch(() => [])).filter(name => /^perf-.*\.jsonl$/.test(name))) {
    const lines = createInterface({ input: createReadStream(join(directory, name)), crlfDelay: Infinity });
    for await (const line of lines) {
      let record;
      try { record = JSON.parse(line); } catch { continue; }
      const ended = Date.parse(record.dispatch?.window_ended_at);
      if (record.type === "gui_dispatch_summary" && record.dispatch?.pid === fixture.gui_pid && ended >= from && ended <= until) summaries.push(record);
    }
  }
  const events = [...groups.values()].sort((a, b) => a.event.localeCompare(b.event));
  return { samples: events.reduce((sum, event) => sum + event.count, 0), stalls: events.reduce((sum, event) => sum + event.stalls, 0),
    max_ms: Math.max(0, ...events.map(event => event.max_ms)), events, slow_rows: slow, summaries, output_batches: batches };
}

// Existing Project Add Agent entrypoint; the Work legacy preset now canonicalizes to Settings.
async function launch(page: Page, index: number, branch: string): Promise<{ id: string; session_id: string }> {
  await sendLiveGwtEvent(page, { kind: "create_window", preset: "work", bounds: { x: 0, y: 0, width: 800, height: 500 } });
  await expect(page.locator('.workspace-window[data-preset="settings"]').first()).toBeAttached();
  const request = (detail: unknown, requireOpen = false) => page.evaluate(({ detail, requireOpen }) => {
    const socket = [...((window as any).__gwtPlaywrightSockets as WebSocket[])].reverse()
      .find(socket => socket.readyState === WebSocket.OPEN && new URL(socket.url).searchParams.has("repo_hash"));
    if (!socket) throw new Error("project socket missing");
    return new Promise<any>((resolve, reject) => {
      const cleanup = () => { clearTimeout(timer); socket.removeEventListener("message", reply); };
      const reply = (event: MessageEvent) => {
        let payload;
        try { payload = JSON.parse(String(event.data)); } catch { return; }
        if (payload.kind === "launch_wizard_open_error") { cleanup(); reject(new Error(payload.message || "wizard open refused")); return; }
        if (payload.kind !== "launch_wizard_state" || (requireOpen && !payload.wizard) || (payload.wizard && (payload.wizard.is_hydrating ||
          payload.wizard.runtime_resolution_pending || payload.wizard.launch_materialization_pending))) return;
        cleanup(); resolve(payload);
      };
      const timer = setTimeout(() => { cleanup(); reject(new Error("wizard timed out")); }, 90_000);
      socket.addEventListener("message", reply);
      window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail }));
    });
  }, { detail, requireOpen });
  await request({ kind: "open_active_work_launch_wizard", branch_name: branch, linked_issue_number: null }, true);
  const action = (action: unknown) => request({ kind: "launch_wizard_action", action, bounds: { x: 20, y: 30, width: 900, height: 600 } });
  await action({ kind: "set_launch_path", path: "manual_setup" });
  let state = await action({ kind: "set_agent", agent_id: AGENT + "-" + index });
  for (let step = 0; step < 12 && state.wizard; step++) {
    expect(state.wizard.error).toBeFalsy();
    if (state.wizard.selected_runtime_target !== "host") state = await action({ kind: "set_runtime_target", target: "Host" });
    else {
      expect(state.wizard.primary_action_enabled, state.wizard.primary_action_disabled_reason).toBe(true);
      state = await action({ kind: "submit" });
    }
  }
  expect(state.wizard).toBeNull();
  const pane = await page.waitForFunction(agent => {
    const message = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.payload.kind === "workspace_state");
    return message?.payload.workspace.tabs.flatMap((tab: any) => tab.workspace.windows)
      .find((pane: any) => pane.agent_id === agent && pane.session_id);
  }, AGENT + "-" + index, { timeout: 90_000 }).then(handle => handle.jsonValue());
  expect(pane.status).not.toBe("error");
  return { id: pane.id, session_id: pane.session_id };
}
