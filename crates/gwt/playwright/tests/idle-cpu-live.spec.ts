import { expect, test, type Page } from "@playwright/test";
import { spawn } from "node:child_process";
import { readFile, readdir, writeFile } from "node:fs/promises";
import { isAbsolute, join, relative, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { clearLiveLaunchWizard, gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

type Thread = { id: number; name: string; created_file_time: string; kernel_seconds: number; user_seconds: number };
type Snapshot = {
  phase: "start" | "end"; process_id: number; process_started_at: string;
  binary: string; binary_sha256: string; timestamp_utc: string;
  elapsed_seconds: number; process_cpu_seconds: number; threads: Thread[];
};
type Traffic = { raw_chunks: number; raw_bytes: number; text: string };
type Agent = { id: string; session_id: string; status: string };
type ReadEvent = { kind: string; at: string; ended_at?: string; ok?: boolean; latency_ms?: number; reason?: string };
type Receipt = { session_id: string; pid: number; started_at: string; cwd: string; history_lines: number;
  padding_bytes: number; cli_binary: string; events: ReadEvent[] };
type Fleet = { gui_pid: number; gui_started_at: string; fixture_clone_sha: string; provider_sha256: string; agents: Agent[] };
type IdleReadiness = {
  requested: boolean; ready: boolean; checked_at: string; timeout_ms: number; latest_launch_utc: string;
  readiness_basis: string; perf_diagnostics: { lossy: boolean; used_for_readiness: boolean; limitation: string };
  latest_state_save_utc: string | null; latest_completion_utc: string | null; latest_progress_utc: string | null;
  index_path: string; format: number | null; projection_version: string | null; source_list_present: boolean;
  matched_worktree_groups: number; expected_worktree_groups: number; missing_worktree_roots: string[]; diagnostics: string[];
};
const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
const checkout = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT;
const buildProfile = process.env.GWT_IDLE_BUILD_PROFILE ?? "debug";
if (!["debug", "release"].includes(buildProfile)) throw new Error("Invalid checkout build profile");
const targetPid = Number(process.env.GWT_PLAYWRIGHT_PID);
const requestedMs = Number(process.env.GWT_IDLE_SAMPLE_MS ?? "30000");
const label = process.env.GWT_IDLE_CPU_LABEL ?? "after";
const functionalOnly = process.env.GWT_IDLE_FUNCTIONAL_ONLY === "1";
const assertCpuBudget = process.env.GWT_IDLE_ASSERT_BUDGET === "1";
const measurementScope = functionalOnly ? "functional (includes startup)" : "steady idle";
const paneCount = 12;
const historyLines = 600;
const paddingBytes = 80;
const agentId = "idle-cpu-workload";
const readyPeriodMs = 120000 / 11;
const intakeProjectionVersion = "source-context-v10-legacy-log-metadata-cache";
const idleReadinessTimeoutMs = 1_800_000;
test.use({ viewport: { width: 1720, height: 1080 }, headless: false });

test.describe("Issue #5658 idle GUI CPU", () => {
  test.skip(!base || !checkout || !targetPid || !process.env.GWT_PLAYWRIGHT_CHECK_HOME || !process.env.GWT_IDLE_PROJECT_STATE || process.platform !== "win32",
    "requires the owned Windows checkout and fresh HOME from issue-5658-browser-check.ps1");
  test.setTimeout(requestedMs + 2_700_000);

  test("twelve launched Agent PTYs stay quiet under authenticated Ready polling", async ({ page }, info) => {
    await withLiveGwtBackendLock(base!, info, async () => {
      expect(["before", "after"]).toContain(label);
      expect(requestedMs).toBeGreaterThanOrEqual(1000);
      expect(requestedMs).toBeLessThanOrEqual(1_800_000);
      expect(functionalOnly && assertCpuBudget, "functional startup verification cannot assert idle CPU acceptance").toBe(false);
      const theme = info.project.name.includes("light") ? "light" : "dark";
      const sampleMs = theme === "dark" ? requestedMs : Math.min(requestedMs, 30_000);
      const errors: string[] = [];
      const created: string[] = [];
      const agents: Agent[] = [];
      const traffic = new Map<string, Traffic>();
      const markers = new Map<string, { id: string; sent_at: number; raw_latency_ms?: number }>();
      const latency: Array<{ phase: string; id: string; raw_latency_ms: number; visible_latency_ms: number }> = [];
      const snapshots: Snapshot[] = [];
      const trafficSnapshots: Array<Record<string, { raw_chunks: number; raw_bytes: number }>> = [];
      const attachJson = async (name: string, value: unknown) => {
        const path = info.outputPath(`${label}-${theme}-${name}.json`);
        await writeFile(path, JSON.stringify(value, null, 2));
        await info.attach(name, { path, contentType: "application/json" });
      };
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      page.on("websocket", socket => socket.on("framereceived", ({ payload }) => {
        try {
          const event = JSON.parse(String(payload));
          if (event.kind !== "terminal_output") return;
          const pane = traffic.get(event.id) ?? { raw_chunks: 0, raw_bytes: 0, text: "" };
          const bytes = Buffer.from(event.data_base64, "base64");
          pane.raw_chunks += 1;
          pane.raw_bytes += bytes.length;
          pane.text = (pane.text + bytes.toString("utf8")).slice(-32_768);
          traffic.set(event.id, pane);
          for (const [marker, probe] of markers) {
            if (probe.id === event.id && probe.raw_latency_ms === undefined && pane.text.includes(marker)) {
              probe.raw_latency_ms = performance.now() - probe.sent_at;
            }
          }
        } catch (error) { errors.push(`terminal observation: ${String(error)}`); }
      }));
      const probeInput = async (phase: string) => {
        for (const [index, id] of created.entries()) {
          const marker = `GWT_IDLE_5658_${theme}_${phase}_${index}`;
          const sentAt = performance.now();
          const probe = { id, sent_at: sentAt, raw_latency_ms: undefined as number | undefined };
          markers.set(marker, probe);
          // The input contains only phase/index, so terminal input echo cannot
          // satisfy the complete marker produced by the launched provider.
          await sendLiveGwtEvent(page, { kind: "terminal_input", id,
            data: JSON.stringify({ kind: "probe", theme, phase, index }) + "\r" });
          await expect.poll(() => probe.raw_latency_ms, { timeout: 15_000 }).toBeDefined();
          await expect.poll(() => page.evaluate(id => window.__gwtTerminalTestApi.bufferText(id), id),
            { timeout: 15_000 }).toContain(marker);
          latency.push({ phase, id, raw_latency_ms: probe.raw_latency_ms!, visible_latency_ms: performance.now() - sentAt });
        }
      };
      let verified = false;
      let reusedFleet = false;
      const fleetPath = join(process.env.GWT_PLAYWRIGHT_CHECK_HOME!, "workload", "fleet.json");
      try {
        await gotoLiveGwt(page, base!, { enableTestBridge: true });
        const migration = page.locator("#migration-modal.open");
        if (await migration.isVisible()) {
          await migration.getByRole("button", { name: "Migrate", exact: true }).click();
          await expect(migration).toHaveCount(0, { timeout: 60_000 });
        }
        await clearLiveLaunchWizard(page);
        await retainWorkspaceState(page);
        await expect(page.locator("#close-project-button")).toBeVisible();
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        const savedFleet = await readSavedFleet(fleetPath);
        if (savedFleet) {
          expect(savedFleet.gui_pid, "same owned GUI for both themes").toBe(targetPid);
          expect(savedFleet.gui_started_at).toBe(process.env.GWT_PLAYWRIGHT_GUI_STARTED_AT);
          expect(savedFleet.fixture_clone_sha).toBe(process.env.GWT_IDLE_FIXTURE_SHA);
          expect(savedFleet.provider_sha256).toBe(process.env.GWT_IDLE_PROVIDER_SHA);
          expect(savedFleet.agents).toHaveLength(paneCount);
          const expected = savedFleet.agents.map(agent => agent.id + "/" + agent.session_id).sort();
          await expect.poll(() => page.evaluate(agentId => (window as any).__gwt5658Workspace?.tabs
            .flatMap((tab: any) => tab.workspace.windows).filter((pane: any) => pane.agent_id === agentId
              && ["running", "idle"].includes(pane.status))
            .map((pane: any) => pane.id + "/" + pane.session_id).sort() ?? [], agentId),
          { timeout: 30_000, message: "same twelve live Sessions belong to the current fixture workspace" }).toEqual(expected);
          agents.push(...savedFleet.agents);
          created.push(...agents.map(agent => agent.id));
          const previous = await readReceipts(agents);
          await expect.poll(async () => (await readReceipts(agents)).every((receipt, index) => receipt.events.filter(event => event.kind === "read" && event.ok).length
            > previous[index].events.filter(event => event.kind === "read" && event.ok).length),
          { timeout: 30_000, message: "every reused Agent authenticates another pane.read with its own capability" }).toBe(true);
          reusedFleet = true;
        } else for (let index = 0; index < paneCount; index += 1) {
          if (!functionalOnly && index === paneCount - 1) {
            // The prior Agent has completed seed/read readiness. Cross the
            // production 30s ingest-request throttle so the final normal
            // Launch queues a pass that includes all twelve worktrees.
            await page.waitForTimeout(31_000);
          }
          const agent = await launchAgent(page, theme, index, created);
          const id = agent.id;
          agents.push(agent);
          created.push(id);
          const marker = `GWT_IDLE_5658_SEEDED_${index}`;
          await expect.poll(async () => readReceipts([agent]).then(receipts => receipts.length).catch(() => 0),
            { timeout: 30_000, message: "launched Node provider publishes its own Session identity" }).toBe(1);
          await sendLiveGwtEvent(page, { kind: "terminal_input", id,
            data: JSON.stringify({ kind: "bind", id, index }) + "\r" });
          // Large initial history may use bounded transport snapshot repair;
          // the actual xterm screen is the authoritative completion fence.
          await expect.poll(() => page.evaluate(id => window.__gwtTerminalTestApi.bufferText(id), id),
            { timeout: 30_000 }).toContain(marker);
          await expect.poll(async () => (await readReceipts([agent]))[0].events.filter(event => event.kind === "read" && event.ok).length,
            { timeout: 40_000, message: "each launched Agent authenticates its own pane.read" }).toBeGreaterThan(0);
        }
        expect(new Set(agents.map(agent => agent.session_id)).size, "twelve distinct launch-issued Sessions").toBe(paneCount);
        await expect(page.locator(".workspace-window")).toHaveCount(paneCount);
        await probeInput("BEFORE");
        if (!reusedFleet) {
          const startAt = Date.now() + 1000;
          for (const [index, id] of created.entries()) {
            await sendLiveGwtEvent(page, { kind: "terminal_input", id,
              data: JSON.stringify({ kind: "start", at: startAt, index }) + "\r" });
          }
        }
        await expect.poll(async () => (await readReceipts(agents)).filter(receipt => receipt.events.some(event => event.kind === "started")).length,
          { timeout: 30_000, message: "all twelve Agent periodic timers have actually started" }).toBe(paneCount);
        const identities = await readReceipts(agents);
        const latestLaunch = identities.map(receipt => receipt.started_at)
          .reduce((latest, started) => Date.parse(started) > Date.parse(latest) ? started : latest);
        let idleReadiness: IdleReadiness | undefined;
        try {
          if (functionalOnly) {
            idleReadiness = await readIdleReadiness(identities.map(receipt => receipt.cwd), latestLaunch, false);
          } else await expect.poll(async () => {
            idleReadiness = await readIdleReadiness(identities.map(receipt => receipt.cwd), latestLaunch);
            return idleReadiness.ready;
          }, { timeout: idleReadinessTimeoutMs, intervals: [1000, 2000],
            message: "committed Work event intake covers all twelve newly materialized Agent worktrees" }).toBe(true);
        } finally { await attachJson("idle-readiness", idleReadiness); }
        // No user input or PTY producers run during sampling. The twelve
        // Agents continue their authenticated, phase-spread pane.read calls.
        await sampleCpu(sampleMs, snapshot => {
          snapshots.push(snapshot);
          if (snapshot.phase === "start") {
            console.log(`${label}/${theme}: ${measurementScope}; sampling owned GUI ${targetPid} for ${sampleMs / 1000}s from ${snapshot.timestamp_utc}`);
          }
          trafficSnapshots.push(Object.fromEntries(created.map(id => [id, {
            raw_chunks: traffic.get(id)?.raw_chunks ?? 0, raw_bytes: traffic.get(id)?.raw_bytes ?? 0,
          }])));
        });
        const [start, end] = snapshots;
        const elapsed = end.elapsed_seconds - start.elapsed_seconds;
        const cpuSeconds = end.process_cpu_seconds - start.process_cpu_seconds;
        const averageCores = cpuSeconds / elapsed;
        expect(end.process_id).toBe(start.process_id);
        expect(Date.parse(start.process_started_at), "owned GUI retains its launch creation time")
          .toBe(Date.parse(process.env.GWT_PLAYWRIGHT_GUI_STARTED_AT!));
        expect(end.process_started_at).toBe(start.process_started_at);
        expect(end.binary_sha256).toBe(start.binary_sha256);
        expect(elapsed).toBeGreaterThanOrEqual(sampleMs / 1000);
        const rawDeltas = created.map(id => ({ id,
          raw_bytes: trafficSnapshots[1][id].raw_bytes - trafficSnapshots[0][id].raw_bytes,
          raw_chunks: trafficSnapshots[1][id].raw_chunks - trafficSnapshots[0][id].raw_chunks,
        }));
        await probeInput("AFTER");
        const receipts = await readReceipts(agents);
        const windowStart = Date.parse(start.timestamp_utc);
        const windowEnd = Date.parse(end.timestamp_utc);
        const ipc = receipts.map(receipt => {
          const reads = receipt.events.filter(event => event.kind === "read"
            && Date.parse(event.at) >= windowStart && Date.parse(event.ended_at!) <= windowEnd);
          return { ...receipt, events: undefined,
            attempt_count: receipt.events.filter(event => event.kind === "attempt" && Date.parse(event.at) >= windowStart && Date.parse(event.at) <= windowEnd).length,
            success_count: reads.filter(event => event.ok).length,
            failures: reads.filter(event => !event.ok), latencies_ms: reads.filter(event => event.ok).map(event => event.latency_ms!) };
        });
        const successes = ipc.reduce((sum, receipt) => sum + receipt.success_count, 0);
        const attempts = ipc.reduce((sum, receipt) => sum + receipt.attempt_count, 0);
        await attachJson("agent-ipc-receipts", receipts);
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        const screenshot = info.outputPath(`${label}-${theme}-twelve-idle-agents.png`);
        await page.screenshot({ path: screenshot });
        await info.attach("twelve-idle-agents", { path: screenshot, contentType: "image/png" });
        const maxCores = Number(process.env.GWT_IDLE_MAX_CORES ?? "0.2");
        await attachJson("idle-cpu", {
          label, theme, measurement_scope: measurementScope, build_profile: buildProfile, gui_pid: targetPid, requested_sample_ms: sampleMs,
          elapsed_seconds: elapsed, process_cpu_seconds: cpuSeconds, average_cores: averageCores,
          idle_readiness: idleReadiness,
          workload: { kind: "regularly launched custom command Agent PTYs", panes: paneCount, history_lines_per_pane: historyLines,
            padding_bytes_per_line: paddingBytes, ansi_color: true, idle_input_bytes: 0,
            fixture_clone_sha: process.env.GWT_IDLE_FIXTURE_SHA, provider_sha256: process.env.GWT_IDLE_PROVIDER_SHA,
            ready_requests_per_minute: 66, ready_period_per_agent_ms: readyPeriodMs,
            reused_across_themes: reusedFleet, ipc_attempt_count: attempts,
            offered_ready_requests_per_minute: attempts * 60 / elapsed,
            ipc_success_count: successes, observed_ready_requests_per_minute: successes * 60 / elapsed, agents: ipc },
          budget: { asserted: !functionalOnly && assertCpuBudget, max_average_cores: maxCores },
          raw_deltas: rawDeltas, input_latency: latency, native_snapshots: snapshots,
          user_verification_result: "n/a (autonomous)",
          agent_visual_check: errors.length === 0 && rawDeltas.every(pane => pane.raw_bytes === 0)
            ? "pass" : "fail(console/page error or unexpected idle output)",
        });
        console.log(`${label}/${theme}: ${cpuSeconds.toFixed(6)} CPU s / ${elapsed.toFixed(3)} s = ${averageCores.toFixed(6)} cores`);
        for (const pane of rawDeltas) expect(pane.raw_bytes, `${pane.id}: quiet throughout CPU sampling`).toBe(0);
        for (const receipt of ipc) {
          expect(receipt.history_lines).toBe(historyLines);
          expect(receipt.padding_bytes).toBe(paddingBytes);
          expect(receipt.failures, `${receipt.session_id}: authenticated IPC failures`).toEqual([]);
          if (sampleMs >= readyPeriodMs) expect(receipt.success_count, "each Agent continues authenticated Ready polling")
            .toBeGreaterThanOrEqual(Math.max(1, Math.floor(elapsed * 1000 / readyPeriodMs) - 1));
        }
        // Each Agent can straddle one sample boundary. Long observations must
        // sustain the fixed rate; short smoke runs still prove launch/auth above.
        expect(successes, "phase-spread Ready throughput across twelve Agents")
          .toBeGreaterThanOrEqual(Math.max(0, Math.floor(66 * elapsed / 60) - paneCount));
        expect(errors, "console and page errors").toEqual([]);
        if (assertCpuBudget) expect(averageCores, "whole owned GUI idle CPU budget").toBeLessThanOrEqual(maxCores);
        if (theme === "dark") await writeFile(fleetPath, JSON.stringify({ gui_pid: targetPid,
          gui_started_at: process.env.GWT_PLAYWRIGHT_GUI_STARTED_AT,
          fixture_clone_sha: process.env.GWT_IDLE_FIXTURE_SHA, provider_sha256: process.env.GWT_IDLE_PROVIDER_SHA, agents }));
        verified = true;
      } catch (error) {
        await attachJson("failure-state", {
          error: String(error), markers: Object.fromEntries(markers),
          traffic: Object.fromEntries(traffic), input_latency: latency, agents,
          receipts: await readReceipts(agents).catch(() => []),
          visible: await page.evaluate(ids => Object.fromEntries(ids.map(id =>
            [id, window.__gwtTerminalTestApi.bufferText(id).slice(-4096)])), created).catch(String),
        });
        throw error;
      } finally {
        await attachJson("errors", errors);
        // A successful dark run keeps exactly this owned fleet for light.
        // The wrapper always owns final GUI/daemon/process-tree cleanup.
        for (const id of (theme === "dark" && verified ? [] : created.reverse())) {
          await sendLiveGwtEvent(page, { kind: "close_window", id });
          if (verified) await expect(page.locator(`.workspace-window[data-id="${id}"]`)).toHaveCount(0);
        }
      }
    });
  });
});

async function readIdleReadiness(worktreeRoots: string[], latestLaunch: string, requested = true): Promise<IdleReadiness> {
  const observation: IdleReadiness = {
    requested, ready: false, checked_at: new Date().toISOString(), timeout_ms: requested ? idleReadinessTimeoutMs : 0, latest_launch_utc: latestLaunch,
    readiness_basis: requested ? "committed intake index covering twelve newly materialized worktrees in the fresh single-branch fixture"
      : "not requested for functional startup verification",
    perf_diagnostics: { lossy: true, used_for_readiness: false,
      limitation: "SelfBudgetGovernor may omit phase/route records; their absence, timing, and subsequent progress do not gate readiness" },
    latest_state_save_utc: null, latest_completion_utc: null, latest_progress_utc: null,
    index_path: join(process.env.GWT_IDLE_PROJECT_STATE!, "work-events-intake", "index.json"),
    format: null, projection_version: null, source_list_present: false,
    matched_worktree_groups: 0, expected_worktree_groups: worktreeRoots.length, missing_worktree_roots: [...worktreeRoots], diagnostics: [],
  };
  try {
    // These twelve worktrees did not exist before normal Launch in this fresh
    // fixture. The index is written last, after intake and its group snapshots,
    // so their committed snapshots prove the cold pass included all twelve.
    // Later incremental polls remain part of the measured workload.
    const index = JSON.parse(await readFile(observation.index_path, "utf8"));
    observation.format = index.format ?? null;
    observation.projection_version = index.projection_version ?? null;
    observation.source_list_present = typeof index.ungrouped?.["source-list:v1"] === "string"
      && index.ungrouped["source-list:v1"].length > 0;
    const groups = new Set(Object.entries(index.groups ?? {})
      .filter(([key, group]) => key.startsWith("worktree:") && (group as { snapshot?: string | null }).snapshot != null)
      .map(([key]) => resolve(key.slice("worktree:".length)).toLowerCase()));
    observation.missing_worktree_roots = worktreeRoots.filter(root => !groups.has(resolve(root).toLowerCase()));
    observation.matched_worktree_groups = worktreeRoots.length - observation.missing_worktree_roots.length;
  } catch (error) { observation.diagnostics.push(`committed index: ${String(error)}`); }
  try {
    const directory = join(process.env.GWT_PLAYWRIGHT_CHECK_HOME!, ".gwt", "logs", "perf");
    const files = (await readdir(directory)).filter(file => /^perf-\d{4}-\d{2}-\d{2}\.jsonl$/.test(file)
      && file.slice(5, 15) >= latestLaunch.slice(0, 10));
    for (const file of files) {
      const content = await readFile(join(directory, file), "utf8");
      for (const line of content.slice(0, content.lastIndexOf("\n") + 1).split("\n").filter(Boolean)) {
        const record = JSON.parse(line);
        if (record.type !== "sample") continue;
        const later = (previous: string | null) => previous === null || Date.parse(record.timestamp) > Date.parse(previous);
        if (record.target === "route:work_events.ingest" && later(observation.latest_completion_utc)) observation.latest_completion_utc = record.timestamp;
        if (record.target === "phase:work_events.ingest.state_save" && later(observation.latest_state_save_utc)) observation.latest_state_save_utc = record.timestamp;
        if (record.target.startsWith("phase:work_events.ingest.") && later(observation.latest_progress_utc)) observation.latest_progress_utc = record.timestamp;
      }
    }
  } catch (error) { observation.diagnostics.push(`ingest completion: ${String(error)}`); }
  observation.ready = requested && observation.format === 2
    && observation.projection_version === intakeProjectionVersion && observation.source_list_present
    && observation.matched_worktree_groups === worktreeRoots.length;
  return observation;
}

async function readSavedFleet(file: string): Promise<Fleet | null> {
  try { return JSON.parse(await readFile(file, "utf8")); }
  catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") return null; throw error; }
}

async function readReceipts(agents: Agent[]): Promise<Receipt[]> {
  return Promise.all(agents.map(async agent => {
    const file = join(process.env.GWT_PLAYWRIGHT_CHECK_HOME!, "workload", `${agent.session_id}.json`);
    const identity = JSON.parse(await readFile(file, "utf8"));
    const ownedPath = relative(process.env.GWT_PLAYWRIGHT_CHECK_HOME!, identity.cwd);
    expect(isAbsolute(ownedPath) || /^\.\.(?:[\\/]|$)/.test(ownedPath), "Agent worktree belongs to the disposable HOME").toBe(false);
    expect(resolve(identity.cli_binary).toLowerCase(), "Agent CLI belongs to the checkout")
      .toBe(resolve(checkout!, `target/${buildProfile}/gwtd.exe`).toLowerCase());
    const records = await readFile(file + "l", "utf8");
    // Read only complete append records while the provider is running.
    const events = records.slice(0, records.lastIndexOf("\n") + 1).split("\n").filter(Boolean).map(line => JSON.parse(line));
    expect(identity.session_id).toBe(agent.session_id);
    return { ...identity, events };
  }));
}

async function retainWorkspaceState(page: Page): Promise<void> {
  await page.evaluate(() => {
    const state = window as any;
    state.__gwt5658Workspace = state.__gwtPlaywrightMessages?.findLast((entry: any) => entry.payload.kind === "workspace_state")?.payload.workspace;
    for (const socket of state.__gwtPlaywrightSockets as WebSocket[]) socket.addEventListener("message", (event: MessageEvent) => {
      const payload = JSON.parse(String(event.data));
      if (payload.kind === "workspace_state") state.__gwt5658Workspace = payload.workspace;
    });
  });
}

async function wizardRequest(page: Page, detail: unknown, requireOpen = false): Promise<any> {
  return page.evaluate(({ detail, requireOpen }) => {
    const socket = [...((window as any).__gwtPlaywrightSockets as WebSocket[])].reverse()
      .find(socket => socket.readyState === WebSocket.OPEN && new URL(socket.url).searchParams.has("repo_hash"));
    if (!socket) throw new Error("fixture project socket missing");
    return new Promise((resolve, reject) => {
      const cleanup = () => { clearTimeout(timer); socket.removeEventListener("message", reply); };
      const reply = (event: MessageEvent) => {
        const payload = JSON.parse(String(event.data));
        if (payload.kind === "launch_wizard_open_error") { cleanup(); reject(new Error(payload.message || "wizard refused")); return; }
        if (payload.kind !== "launch_wizard_state" || (requireOpen && !payload.wizard) || (payload.wizard && (payload.wizard.is_hydrating
          || payload.wizard.runtime_resolution_pending || payload.wizard.launch_materialization_pending))) return;
        cleanup(); resolve(payload);
      };
      const timer = setTimeout(() => { cleanup(); reject(new Error("fixture Launch Wizard timed out")); }, 90_000);
      socket.addEventListener("message", reply);
      window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail }));
    });
  }, { detail, requireOpen });
}

async function launchAgent(page: Page, theme: string, index: number, before: string[]): Promise<Agent> {
  expect(process.env.GWT_IDLE_FIXTURE_BRANCH, "shared clone source branch").toBeTruthy();
  if (index === 0) {
    // Normal Launch binds a fixed-tool member; provider registration alone
    // does not make this disposable command tool available in the roster.
    const rosterRequest = async (action: unknown, expectedRevision: number | null = null) => {
      const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
      await sendLiveGwtEvent(page, { kind: "agent_roster", action, expected_revision: expectedRevision });
      const roster = await page.waitForFunction(after => {
        const message = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
          entry.sequence > after && entry.payload.kind === "agent_roster");
        return message?.payload.roster || null;
      }, after, { timeout: 30_000 }).then(handle => handle.jsonValue());
      expect(roster.error, "disposable workload member preparation").toBeFalsy();
      return roster;
    };
    let roster = await rosterRequest({ kind: "read" });
    const available = () => roster.members.filter((member: any) =>
      member.agent_id === agentId && member.enabled && !member.assignment).length;
    for (let added = 0; available() < paneCount && added < paneCount; added += 1) {
      roster = await rosterRequest({ kind: "add", agent_id: agentId }, roster.revision);
    }
    expect(available(), "twelve normally allocated command-tool members").toBe(paneCount);
  }
  // Current Add Agent route: a regular Launch Wizard without a legacy Work
  // surface or an extra owner window. Launch materializes each new worktree.
  await wizardRequest(page, { kind: "open_active_work_launch_wizard",
    branch_name: process.env.GWT_IDLE_FIXTURE_BRANCH, linked_issue_number: null }, true);
  const action = (action: unknown) => wizardRequest(page, { kind: "launch_wizard_action", action,
    bounds: { x: 20 + index % 4 * 420, y: 60 + Math.floor(index / 4) * 320, width: 410, height: 310 } });
  await action({ kind: "set_launch_path", path: "manual_setup" });
  await action({ kind: "set_branch_mode", create_new: true });
  await action({ kind: "set_branch_name", value: `fixture/idle-cpu-${theme}-${index}` });
  let state = await action({ kind: "set_agent", agent_id: agentId });
  state = await action({ kind: "clear_linked_issue" });
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
  const agent = await page.waitForFunction(({ before, agentId }) => {
    const pane = (window as any).__gwt5658Workspace?.tabs.flatMap((tab: any) => tab.workspace.windows)
      .find((pane: any) => !before.includes(pane.id) && pane.agent_id === agentId && pane.session_id);
    if (pane?.status === "error") throw new Error("fixture Agent launch failed");
    return pane ? { id: pane.id, session_id: pane.session_id, status: pane.status } : null;
  }, { before, agentId }, { timeout: 90_000 }).then(handle => handle.jsonValue()) as Agent;
  await expect(page.locator(`.workspace-window[data-id="${agent.id}"]`)).toBeAttached();
  return agent;
}

async function sampleCpu(duration: number, onSnapshot: (snapshot: Snapshot) => void): Promise<void> {
  await new Promise<void>((resolveSample, reject) => {
    const env = { ...process.env };
    for (const key of Object.keys(env)) if (key.startsWith("GWT_") && !key.startsWith("GWT_PLAYWRIGHT_")) delete env[key];
    const child = spawn("pwsh", ["-NoProfile", "-NonInteractive", "-File",
      resolve(checkout!, "scripts/diagnostics/issue-5658-cpu.ps1"), "-TargetProcessId", String(targetPid),
      "-SampleMilliseconds", String(duration)], { env, windowsHide: true });
    let pending = "";
    let stderr = "";
    let count = 0;
    const timeout = setTimeout(() => { child.kill(); reject(new Error("CPU sampler timed out")); }, duration + 60_000);
    child.stdout.setEncoding("utf8").on("data", (data: string) => {
      pending += data;
      let boundary: number;
      while ((boundary = pending.indexOf("\n")) >= 0) {
        const line = pending.slice(0, boundary).trim();
        pending = pending.slice(boundary + 1);
        if (!line) continue;
        try {
          const snapshot = JSON.parse(line) as Snapshot;
          if (snapshot.phase !== (count === 0 ? "start" : "end") || count >= 2) throw new Error("Unexpected CPU snapshot order");
          onSnapshot(snapshot);
          count += 1;
        } catch (error) { child.kill(); reject(error); }
      }
    });
    child.stderr.setEncoding("utf8").on("data", (data: string) => { stderr += data; });
    child.on("error", reject);
    child.on("close", code => {
      clearTimeout(timeout);
      if (code === 0 && count === 2) resolveSample();
      else reject(new Error(`CPU sampler exit=${code}, snapshots=${count}: ${stderr}`));
    });
  });
}
