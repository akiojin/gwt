/** #5381: real PTY and Chromium workload, without contacting model providers.
 * This bounds GUI memory under deterministic output producers; provider
 * inference, provider hooks and real implementation builds are not simulated.
 */
import { spawn } from "node:child_process";
import { readFile, readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { performance } from "node:perf_hooks";
import { createInterface } from "node:readline";
import { expect, test, type Page } from "@playwright/test";
import {
  clearLiveLaunchWizard, gotoLiveGwt, sendLiveGwtEvent,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL || "";
const HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME || "";
const PID = Number(process.env.GWT_PLAYWRIGHT_MEMORY_PID);
const BRANCH = process.env.GWT_PLAYWRIGHT_MEMORY_BRANCH || "";
const REQUESTED_DURATION = Number(process.env.GWT_PLAYWRIGHT_MEMORY_SECONDS || "60");
const BUDGET = Number(process.env.GWT_PLAYWRIGHT_MEMORY_BUDGET_BYTES || 2 * 1024 ** 3);
const PANE_COUNT = Number(process.env.GWT_PLAYWRIGHT_MEMORY_PANES || "8");
const AGENT = "runtime-memory-workload";
type Receipt = { pid: number; started_at: number; session: string; tick: number; observed_at: string };
type Sample = { timestamp: string; rss_bytes: number; private_bytes: number; cpu_seconds: number; elapsed_ms: number };
type Surface = { page: Page; id: string };
test.use({ headless: false });

test.describe("runtime memory budget (live backend)", () => {
  test.skip(!BASE, "requires scripts/verify-runtime-memory-budget.ps1");
  test.setTimeout((REQUESTED_DURATION + 900) * 1000);
  test.use({ actionTimeout: 30_000, navigationTimeout: 60_000 });

  test(`${PANE_COUNT} deterministic Agent panes remain within the RSS budget`, async ({ page }, info) => {
    expect(process.platform).toBe("win32");
    expect(Number.isInteger(PID) && PID > 0, "owned GUI PID").toBe(true);
    expect(Number.isFinite(REQUESTED_DURATION) && REQUESTED_DURATION > 0).toBe(true);
    expect(Number.isInteger(PANE_COUNT) && PANE_COUNT >= 1 && PANE_COUNT <= 12).toBe(true);
    expect(Number.isFinite(BUDGET) && BUDGET > 0).toBe(true);
    expect(HOME).not.toBe("");
    expect(BRANCH).not.toBe("");
    const errors: string[] = [];
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("pageerror", error => errors.push(String(error)));
    const panes: string[] = [];
    const surfaces: Surface[] = [];
    const samples: Sample[] = [];
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const duration = theme === "light" ? Math.min(60, REQUESTED_DURATION) : REQUESTED_DURATION;
    let initial: Receipt[] = [];
    let final: Receipt[] = [];
    let observationStarted = 0;
    let observationClock = 0;
    const startedAt = new Date().toISOString();
    let passed = false;
    let failure: { message: string; stack?: string } | null = null;
    const sampler = createProcessSampler();
    try {
      const projects: Array<{ path: string; project_key: string; branch: string }> = JSON.parse(await readFile(join(HOME, "workload-projects.json"), "utf8"));
      expect(projects).toHaveLength(PANE_COUNT);
      for (let index = 0; index < PANE_COUNT; index += 1) {
        const surfacePage = index === 0 ? page : await page.context().newPage();
        if (index > 0) {
          surfacePage.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
          surfacePage.on("pageerror", error => errors.push(String(error)));
        }
        await gotoLiveGwt(surfacePage, BASE, { enableTestBridge: true, projectKey: projects[index].project_key });
        await clearLiveLaunchWizard(surfacePage);
        await surfacePage.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
        await expect(surfacePage.locator("html")).toHaveAttribute("data-theme", theme);
        const id = await launch(surfacePage, projects[index].branch);
        panes.push(id);
        surfaces.push({ page: surfacePage, id });
        // A materialized pane can precede its cold host process. Use the
        // launch wizard's startup budget here; steady observations stay short.
        await assertOutput([surfaces.at(-1)!], 90_000);
        samples.push(await sampleRss(sampler, 0));
      }
      initial = await liveReceipts(sampler);
      expect(initial, "all real workload processes").toHaveLength(PANE_COUNT);
      await assertOutput(surfaces);
      observationStarted = Date.now();
      observationClock = performance.now();
      do {
        samples.push(await sampleRss(sampler, performance.now() - observationClock));
        expect(samples.at(-1)!.rss_bytes, "owned GUI RSS budget").toBeLessThanOrEqual(BUDGET);
        final = await liveReceipts(sampler);
        expect(final.map(receipt => receipt.pid).sort(), "all producers survive")
          .toEqual(initial.map(receipt => receipt.pid).sort());
        await assertOutput(surfaces);
        expect(errors, "console and page errors").toEqual([]);
        const remaining = duration * 1000 - (performance.now() - observationClock);
        if (remaining > 0) await page.waitForTimeout(Math.min(5_000, remaining));
      } while (performance.now() - observationClock < duration * 1000);
      samples.push(await sampleRss(sampler, performance.now() - observationClock));
      final = await liveReceipts(sampler);
      expect(samples.every(sample => sample.rss_bytes <= BUDGET), "every RSS sample, including launch").toBe(true);
      expect(samples.at(-1)!.elapsed_ms, "full observation interval").toBeGreaterThanOrEqual(duration * 1000);
      for (const receipt of initial) {
        const surviving = final.find(current => current.pid === receipt.pid);
        expect(surviving, `producer ${receipt.pid} survives`).toBeTruthy();
        expect(surviving!.tick, `producer ${receipt.pid} keeps producing`).toBeGreaterThan(receipt.tick);
      }
      expect(errors).toEqual([]);
      const screenshot = await page.screenshot();
      await writeFile(join(HOME, `runtime-memory-${theme}.png`), screenshot);
      await info.attach(`workload-pane-${theme}`, { body: screenshot, contentType: "image/png" });
      passed = true;
    } catch (error) {
      failure = error instanceof Error
        ? { message: error.message, stack: error.stack }
        : { message: String(error) };
      console.error(`runtime memory ${theme}: ${failure.message}`);
      throw error;
    } finally {
      try {
        const evidence = {
          issue: Number(process.env.GWT_PLAYWRIGHT_MEMORY_ISSUE || "5381"), theme, passed, failure, gui_pid: PID,
          checkout_sha: process.env.GWT_PLAYWRIGHT_MEMORY_SHA,
          checkout: process.env.GWT_PLAYWRIGHT_PROJECT_ROOT,
          checkout_binary: join(process.env.GWT_PLAYWRIGHT_PROJECT_ROOT!, "target", "debug", "gwt.exe"),
          checkout_branch: BRANCH,
          source_worktree_count: Number(process.env.GWT_PLAYWRIGHT_MEMORY_WORKTREES),
          isolated_home: HOME, started_at: startedAt, ended_at: new Date().toISOString(),
          observation_started_at: observationStarted ? new Date(observationStarted).toISOString() : null,
          requested_duration_seconds: duration, budget_bytes: BUDGET, pane_ids: panes,
          workload: `${PANE_COUNT} custom command Agent panes; node heartbeat and 512-byte PTY output every second; no model-provider calls`,
          limitations: `${PANE_COUNT} deterministic PTYs in separate fresh folder Projects with isolated fixture Git metadata plus the real checkout source-history Project. Provider inference, provider hooks and implementation builds are not covered. This is not same-condition real model-agent fleet evidence.`,
          user_verification_result: "n/a (autonomous)", agent_visual_check: passed ? "pass" : "fail",
          initial_producers: initial, final_producers: final, rss_samples: samples, browser_errors: errors,
        };
        const body = JSON.stringify(evidence, (_key, value) => typeof value === "string"
          ? value.replace(/([a-z]:[\\/]+Users[\\/]+)[^\\/]+/gi, "$1<redacted>")
          : value, 2);
        await writeFile(join(HOME, `runtime-memory-${theme}.json`), body);
        await info.attach(`runtime-memory-${theme}`, { body, contentType: "application/json" });
        for (const surface of surfaces) await sendLiveGwtEvent(surface.page, { kind: "close_window", id: surface.id }).catch(() => undefined);
        await expect.poll(async () => (await liveReceipts(sampler)).length,
          { message: "owned workload processes stop before the next theme", timeout: 60_000 }).toBe(0);
        for (const surface of surfaces) if (surface.page !== page) await surface.page.close();
      } finally {
        await sampler.dispose();
      }
    }
  });
});

type ProcessSnapshot = {
  request_id: number;
  sample: Omit<Sample, "elapsed_ms"> | null;
  processes: Array<{ Id: number; started_at: number }>;
};
type ProcessSampler = ReturnType<typeof createProcessSampler>;

function createProcessSampler() {
  // A single warm process avoids thousands of cold pwsh starts under build load.
  const command = `
$ErrorActionPreference = 'Stop'
while ($null -ne ($line = [Console]::ReadLine())) {
  try {
    $query = $line | ConvertFrom-Json
    $sample = $null
    if ($null -ne $query.gui_pid) {
      $process = [Diagnostics.Process]::GetProcessById([int]$query.gui_pid)
      try {
        $process.Refresh()
        $sample = @{timestamp=[DateTime]::UtcNow.ToString('o');rss_bytes=$process.WorkingSet64;private_bytes=$process.PrivateMemorySize64;cpu_seconds=$process.TotalProcessorTime.TotalSeconds}
      } finally { $process.Dispose() }
    }
    $processes = @(foreach ($targetId in $query.ids) {
      try { $process = [Diagnostics.Process]::GetProcessById([int]$targetId) }
      catch [ArgumentException] { continue }
      try { @{Id=$process.Id;started_at=([DateTimeOffset]$process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()} }
      catch [InvalidOperationException], [ComponentModel.Win32Exception] { continue }
      finally { $process.Dispose() }
    })
    [Console]::WriteLine((@{request_id=$query.request_id;sample=$sample;processes=$processes} | ConvertTo-Json -Depth 4 -Compress))
    [Console]::Out.Flush()
  } catch { [Console]::Error.WriteLine($_.ToString()); exit 1 }
}`;
  const child = spawn("pwsh", ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", command], { windowsHide: true });
  const lines = createInterface({ input: child.stdout });
  let stderr = "";
  let sequence = 0;
  let failure: Error | null = null;
  let pending: { id: number; resolve: (snapshot: ProcessSnapshot) => void; reject: (error: Error) => void } | null = null;
  let disposed = false;
  let closed = false;
  const exited = new Promise<void>(resolve => child.once("close", () => { closed = true; resolve(); }));
  const fail = (error: Error) => {
    failure ??= error;
    pending?.reject(failure);
    pending = null;
    child.kill();
  };
  child.stderr.setEncoding("utf8").on("data", (data: string) => { stderr = (stderr + data).slice(-8192); });
  child.on("error", fail);
  child.stdin.on("error", fail);
  child.on("close", code => { if (!disposed) fail(new Error(`process sampler exited ${code}: ${stderr}`)); });
  lines.on("line", line => {
    try {
      const snapshot = JSON.parse(line) as ProcessSnapshot;
      if (!pending || snapshot.request_id !== pending.id || !Array.isArray(snapshot.processes)) {
        throw new Error("process sampler returned an unexpected or stale response");
      }
      pending.resolve(snapshot);
      pending = null;
    } catch (error) { fail(error instanceof Error ? error : new Error(String(error))); }
  });
  return {
    async query(gui_pid: number | null, ids: number[]): Promise<ProcessSnapshot> {
      if (failure) throw failure;
      if (disposed || pending) throw new Error("process sampler unavailable or already sampling");
      const request_id = ++sequence;
      let timer: ReturnType<typeof setTimeout>;
      try {
        return await new Promise<ProcessSnapshot>((resolve, reject) => {
          pending = { id: request_id, resolve, reject };
          // Only the cold startup uses the existing launch budget.
          timer = setTimeout(() => fail(new Error(`process sampler request ${request_id} timed out`)), request_id === 1 ? 90_000 : 30_000);
          child.stdin.write(JSON.stringify({ request_id, gui_pid, ids }) + "\n");
        });
      } finally { clearTimeout(timer!); }
    },
    async dispose(): Promise<void> {
      disposed = true;
      lines.close();
      if (closed) return;
      child.stdin.end();
      let timer: ReturnType<typeof setTimeout>;
      try {
        await Promise.race([exited, new Promise<never>((_, reject) => {
          timer = setTimeout(() => { child.kill(); reject(new Error("process sampler did not exit")); }, 10_000);
        })]);
      } finally { clearTimeout(timer!); }
    },
  };
}

async function sampleRss(sampler: ProcessSampler, elapsed_ms: number): Promise<Sample> {
  const { sample } = await sampler.query(PID, []);
  if (!sample || !Number.isFinite(sample.rss_bytes)) throw new Error("process sampler returned no GUI RSS");
  return { ...sample, elapsed_ms };
}

async function liveReceipts(sampler: ProcessSampler): Promise<Receipt[]> {
  const directory = join(HOME, "workload");
  const receipts = await Promise.all((await readdir(directory)).filter(name => name.endsWith(".json"))
    .map(async name => {
      const receipt = JSON.parse(await readFile(join(directory, name), "utf8"));
      const heartbeats = await readFile(join(directory, name + ".heartbeat"), "utf8");
      // Only a complete appended record is observable during a concurrent write.
      const latest = heartbeats.slice(0, heartbeats.lastIndexOf("\n")).split("\n").at(-1);
      return latest ? { ...receipt, ...JSON.parse(latest) } : receipt;
    })) as Receipt[];
  const ids = [...new Set(receipts.map(receipt => receipt.pid))];
  if (ids.length === 0) return [];
  expect(ids.every(id => Number.isInteger(id) && id > 0), "owned workload PID receipts").toBe(true);
  const { processes } = await sampler.query(null, ids);
  // Windows eagerly recycles PIDs. Old theme receipts remain as evidence and
  // must not count a later unrelated process as a surviving workload.
  return receipts.filter(receipt => processes.some(process => process.Id === receipt.pid
    && Math.abs(process.started_at - receipt.started_at) < 1000));
}

async function assertOutput(surfaces: Surface[], timeout = 30_000): Promise<void> {
  for (const { page, id } of surfaces) {
    await expect(page.locator(`.agent-tile[data-agent-id="${id}"]`)).toBeAttached();
    await expect.poll(() => page.evaluate(id => String((window as any).__gwtTerminalTestApi?.bufferText?.(id) || ""), id),
      { message: `PTY output from ${id}`, timeout }).toContain("GWT_MEMORY_TICK");
  }
}

async function wizardRequest(page: Page, detail: unknown, requireOpen = false): Promise<any> {
  return page.evaluate(({ detail, requireOpen }) => {
    const socket = [...((window as any).__gwtPlaywrightSockets as WebSocket[])].reverse()
      .find(socket => socket.readyState === WebSocket.OPEN && new URL(socket.url).searchParams.has("repo_hash"));
    if (!socket) throw new Error("project socket missing");
    return new Promise((resolve, reject) => {
      const cleanup = () => { clearTimeout(timer); socket.removeEventListener("message", reply); };
      const reply = (event: MessageEvent) => {
        let payload;
        try { payload = JSON.parse(String(event.data)); } catch { return; }
        if (payload.kind !== "launch_wizard_state" || (requireOpen && !payload.wizard) || (payload.wizard && (payload.wizard.is_hydrating
          || payload.wizard.runtime_resolution_pending || payload.wizard.launch_materialization_pending))) return;
        cleanup(); resolve(payload);
      };
      const timer = setTimeout(() => { cleanup(); reject(new Error("wizard timed out")); }, 90_000);
      socket.addEventListener("message", reply);
      window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail }));
    });
  }, { detail, requireOpen });
}

async function launch(page: Page, branch: string): Promise<string> {
  const before: string[] = await page.evaluate(() => {
    const message = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.payload.kind === "workspace_state");
    return message?.payload.workspace.tabs.flatMap((tab: any) => tab.workspace.windows.map((pane: any) => pane.id)) || [];
  });
  await wizardRequest(page, { kind: "open_active_work_launch_wizard", branch_name: branch }, true);
  const action = (action: unknown) => wizardRequest(page, { kind: "launch_wizard_action", action,
    bounds: { x: 40, y: 40, width: 650, height: 360 } });
  await action({ kind: "set_launch_path", path: "manual_setup" });
  let state = await action({ kind: "set_agent", agent_id: AGENT });
  for (let step = 0; step < 12 && state.wizard; step += 1) {
    expect(state.wizard.error).toBeFalsy();
    if (state.wizard.selected_runtime_target !== "host") {
      state = await action({ kind: "set_runtime_target", target: "Host" });
    } else {
      expect(state.wizard.primary_action_enabled, state.wizard.primary_action_disabled_reason).toBe(true);
      state = await action({ kind: "submit" });
    }
  }
  expect(state.wizard).toBeNull();
  const pane = await page.waitForFunction(({ before, agent }) => {
    const message = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.payload.kind === "workspace_state");
    return message?.payload.workspace.tabs.flatMap((tab: any) => tab.workspace.windows)
      .find((pane: any) => !before.includes(pane.id) && pane.agent_id === agent);
  }, { before, agent: AGENT }, { timeout: 90_000 }).then(handle => handle.jsonValue());
  expect(pane.status, "new Agent pane launch status").not.toBe("error");
  await page.locator('.op-rail__surface[data-surface="agents"]').click();
  return pane.id;
}
