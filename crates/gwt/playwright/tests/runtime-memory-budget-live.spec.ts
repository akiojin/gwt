/** #5381: real PTY and Chromium workload, without contacting model providers.
 * This bounds GUI memory under eight deterministic output producers; provider
 * inference, provider hooks and real implementation builds are not simulated.
 */
import { execFileSync } from "node:child_process";
import { readFile, readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { performance } from "node:perf_hooks";
import { expect, test, type Page } from "@playwright/test";
import {
  clearLiveLaunchWizard, gotoLiveGwt, sendLiveGwtEvent,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL || "";
const HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME || "";
const PID = Number(process.env.GWT_PLAYWRIGHT_MEMORY_PID);
const BRANCH = process.env.GWT_PLAYWRIGHT_MEMORY_BRANCH || "";
const REQUESTED_DURATION = Number(process.env.GWT_PLAYWRIGHT_MEMORY_SECONDS || "60");
const BUDGET = 2 * 1024 ** 3;
const AGENT = "runtime-memory-workload";
type Receipt = { pid: number; started_at: number; session: string; tick: number; observed_at: string };
type Sample = { timestamp: string; rss_bytes: number; private_bytes: number; cpu_seconds: number; elapsed_ms: number };
type Surface = { page: Page; id: string };
test.use({ headless: false });

test.describe("Issue #5381 runtime memory budget (live backend)", () => {
  test.skip(!BASE, "requires scripts/verify-runtime-memory-budget.ps1");
  test.setTimeout((REQUESTED_DURATION + 900) * 1000);
  test.use({ actionTimeout: 30_000, navigationTimeout: 60_000 });

  test("eight deterministic Agent panes remain below 2 GiB", async ({ page }, info) => {
    expect(process.platform).toBe("win32");
    expect(Number.isInteger(PID) && PID > 0, "owned GUI PID").toBe(true);
    expect(Number.isFinite(REQUESTED_DURATION) && REQUESTED_DURATION > 0).toBe(true);
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
    try {
      const projects: Array<{ path: string; project_key: string }> = JSON.parse(await readFile(join(HOME, "workload-projects.json"), "utf8"));
      expect(projects).toHaveLength(8);
      for (let index = 0; index < 8; index += 1) {
        const surfacePage = index === 0 ? page : await page.context().newPage();
        if (index > 0) {
          surfacePage.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
          surfacePage.on("pageerror", error => errors.push(String(error)));
        }
        await gotoLiveGwt(surfacePage, BASE, { enableTestBridge: true, projectKey: projects[index].project_key });
        await clearLiveLaunchWizard(surfacePage);
        await surfacePage.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
        await expect(surfacePage.locator("html")).toHaveAttribute("data-theme", theme);
        const id = await launch(surfacePage, []);
        panes.push(id);
        surfaces.push({ page: surfacePage, id });
        await assertOutput([surfaces.at(-1)!]);
        samples.push(sampleRss(0));
      }
      initial = await liveReceipts();
      expect(initial, "eight real workload processes").toHaveLength(8);
      await assertOutput(surfaces);
      observationStarted = Date.now();
      observationClock = performance.now();
      do {
        samples.push(sampleRss(performance.now() - observationClock));
        expect(samples.at(-1)!.rss_bytes, "owned GUI RSS budget").toBeLessThanOrEqual(BUDGET);
        final = await liveReceipts();
        expect(final.map(receipt => receipt.pid).sort(), "all eight producers survive")
          .toEqual(initial.map(receipt => receipt.pid).sort());
        await assertOutput(surfaces);
        expect(errors, "console and page errors").toEqual([]);
        const remaining = duration * 1000 - (performance.now() - observationClock);
        if (remaining > 0) await page.waitForTimeout(Math.min(5_000, remaining));
      } while (performance.now() - observationClock < duration * 1000);
      samples.push(sampleRss(performance.now() - observationClock));
      final = await liveReceipts();
      expect(samples.every(sample => sample.rss_bytes <= BUDGET), "every RSS sample, including launch").toBe(true);
      expect(samples.at(-1)!.elapsed_ms, "full observation interval").toBeGreaterThanOrEqual(duration * 1000);
      for (const receipt of initial) {
        const surviving = final.find(current => current.pid === receipt.pid);
        expect(surviving, `producer ${receipt.pid} survives`).toBeTruthy();
        expect(surviving!.tick, `producer ${receipt.pid} keeps producing`).toBeGreaterThan(receipt.tick);
      }
      expect(errors).toEqual([]);
      await info.attach(`workload-pane-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      passed = true;
    } finally {
      const evidence = {
        issue: 5381, theme, passed, gui_pid: PID,
        checkout_sha: process.env.GWT_PLAYWRIGHT_MEMORY_SHA,
        checkout: process.env.GWT_PLAYWRIGHT_PROJECT_ROOT,
        checkout_binary: join(process.env.GWT_PLAYWRIGHT_PROJECT_ROOT!, "target", "debug", "gwt.exe"),
        checkout_branch: BRANCH,
        source_worktree_count: Number(process.env.GWT_PLAYWRIGHT_MEMORY_WORKTREES),
        isolated_home: HOME, started_at: startedAt, ended_at: new Date().toISOString(),
        observation_started_at: observationStarted ? new Date(observationStarted).toISOString() : null,
        requested_duration_seconds: duration, budget_bytes: BUDGET, pane_ids: panes,
        workload: "8 custom command Agent panes; node heartbeat and 512-byte PTY output every second; no model-provider calls",
        limitations: "Eight deterministic PTYs in separate fresh folder Projects with isolated fixture Git metadata plus the real checkout source-history Project (nine Projects total). This covers #5381's fixture RSS budget; the actual implementation/model fleet 60-minute observation belongs to #5385. Provider inference, hooks and builds are not covered.",
        user_verification_result: "n/a (autonomous)", agent_visual_check: passed ? "pass" : "fail",
        initial_producers: initial, final_producers: final, rss_samples: samples, browser_errors: errors,
      };
      const body = JSON.stringify(evidence, null, 2);
      await writeFile(join(HOME, `runtime-memory-${theme}.json`), body);
      await info.attach(`runtime-memory-${theme}`, { body, contentType: "application/json" });
      for (const surface of surfaces) await sendLiveGwtEvent(surface.page, { kind: "close_window", id: surface.id }).catch(() => undefined);
      await expect.poll(async () => (await liveReceipts()).length,
        { message: "owned workload processes stop before the next theme", timeout: 60_000 }).toBe(0);
      for (const surface of surfaces) if (surface.page !== page) await surface.page.close();
    }
  });
});

function sampleRss(elapsed_ms: number): Sample {
  const command = `$p=Get-Process -Id ${PID} -ErrorAction Stop; [pscustomobject]@{timestamp=[DateTime]::UtcNow.ToString('o');rss_bytes=$p.WorkingSet64;private_bytes=$p.PrivateMemorySize64;cpu_seconds=$p.TotalProcessorTime.TotalSeconds}|ConvertTo-Json -Compress`;
  return { ...JSON.parse(execFileSync("pwsh", ["-NoProfile", "-Command", command], { encoding: "utf8", timeout: 30_000 })), elapsed_ms };
}

async function liveReceipts(): Promise<Receipt[]> {
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
  const command = `Get-Process -Id ${ids.join(",")} -ErrorAction SilentlyContinue | Select-Object Id,@{Name='started_at';Expression={([DateTimeOffset]$_.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()}} | ConvertTo-Json -AsArray -Compress; exit 0`;
  const processes: Array<{ Id: number; started_at: number }> = JSON.parse(execFileSync("pwsh",
    ["-NoProfile", "-Command", command], { encoding: "utf8", timeout: 30_000 }).trim() || "[]");
  // Windows eagerly recycles PIDs. Old theme receipts remain as evidence and
  // must not count a later unrelated process as a surviving workload.
  return receipts.filter(receipt => processes.some(process => process.Id === receipt.pid
    && Math.abs(process.started_at - receipt.started_at) < 1000));
}

async function assertOutput(surfaces: Surface[]): Promise<void> {
  for (const { page, id } of surfaces) {
    await expect(page.locator(`.workspace-window[data-id="${id}"]`)).toBeAttached();
    await expect.poll(() => page.evaluate(id => String((window as any).__gwtTerminalTestApi?.bufferText?.(id) || ""), id),
      { message: `PTY output from ${id}`, timeout: 30_000 }).toContain("GWT_MEMORY_TICK");
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

async function launch(page: Page, before: string[]): Promise<string> {
  if (before.length === 0) {
    await sendLiveGwtEvent(page, { kind: "create_window", preset: "work",
      bounds: { x: 0, y: 0, width: 800, height: 500 } });
  }
  const workId = await page.locator('.workspace-window[data-preset="work"]').first().getAttribute("data-id");
  expect(workId, "Work surface for existing checkout launch").toBeTruthy();
  await wizardRequest(page, { kind: "open_launch_wizard", id: workId, branch_name: "" }, true);
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
  return pane.id;
}
