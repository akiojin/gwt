/**
 * SPEC #1921 Phase 86 / Issue #3813 (T519) — agent process-tree resource
 * isolation live E2E.
 *
 * Runs against a real gwt browser-server backend launched from the checkout
 * under test (browser-check isolation). Two pieces of evidence:
 *
 * 1. Settings > System renders the resource controls with the shared Operator
 *    primitives in dark and light themes, persists an explicit value, and
 *    reconciles the automatic (empty) mode from the backend echo (AS-8).
 * 2. UI request/reply latency stays within budget while N=0 / N=1 / N=3 real
 *    AgentBootstrap full-build trees run through the gated launch path
 *    (AS-7 / FR-242 / SC-098). The build agent is a custom agent whose
 *    command is a cold `cargo build` in a private target directory; the pane
 *    text carries the injected `CARGO_BUILD_JOBS`, and the host process table
 *    carries the inherited priority class of every cargo / rustc descendant.
 *
 * Environment:
 * - `GWT_PLAYWRIGHT_BASE_URL`          fresh isolated gwt URL (required)
 * - `GWT_PLAYWRIGHT_BRANCH_NAME`       existing branch to launch on (required for the latency test)
 * - `GWT_PLAYWRIGHT_RESOURCE_AGENT_ID` custom agent id running the full build (required for the latency test)
 * - `GWT_PLAYWRIGHT_RESOURCE_EVIDENCE` optional JSON output path for the evidence bundle
 * - `GWT_PLAYWRIGHT_RESOURCE_MAX_ACTIVE` Issue Monitor max active agents to seed (default 3)
 * - `GWT_PLAYWRIGHT_RESOURCE_ROOT_EXE` executable path of the gwt process under test; the
 *   process evidence is scoped to its descendants so unrelated host builds do not leak in
 */
import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { availableParallelism } from "node:os";
import { dirname } from "node:path";
import {
  acquireLiveGwtBackendLock,
  clearLiveLaunchWizard,
  gotoLiveGwt,
  openLiveGwtProject,
  sendLiveGwtEvent,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const BRANCH_NAME = process.env.GWT_PLAYWRIGHT_BRANCH_NAME ?? "";
const AGENT_ID = process.env.GWT_PLAYWRIGHT_RESOURCE_AGENT_ID ?? "";
const EVIDENCE_PATH = process.env.GWT_PLAYWRIGHT_RESOURCE_EVIDENCE ?? "";
const MAX_ACTIVE = Number(process.env.GWT_PLAYWRIGHT_RESOURCE_MAX_ACTIVE ?? "3") || 3;
const ROOT_EXE = process.env.GWT_PLAYWRIGHT_RESOURCE_ROOT_EXE ?? "";

const WARMUP_SAMPLES = 5;
const MEASURED_SAMPLES = 30;
const ABSOLUTE_P95_BUDGET_MS = 100;
const RELATIVE_P95_BUDGET = 3.0;

type LatencySample = {
  label: string;
  rttMs: number[];
  rafGapMs: number[];
  longTaskMs: number[];
};

type Stats = { p50: number; p95: number; max: number; count: number };

type HostProcess = { pid: number; name: string; priority: string };

function percentile(values: number[], fraction: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const index = Math.min(sorted.length - 1, Math.ceil(fraction * sorted.length) - 1);
  return sorted[Math.max(0, index)];
}

function stats(values: number[]): Stats {
  return {
    p50: round(percentile(values, 0.5)),
    p95: round(percentile(values, 0.95)),
    max: round(values.length ? Math.max(...values) : 0),
    count: values.length,
  };
}

function round(value: number): number {
  return Math.round(value * 100) / 100;
}

test.describe.serial("Agent process-tree resource isolation (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");
  test.setTimeout(180_000);

  let releaseBackendLock: (() => Promise<void>) | undefined;
  let consoleErrors: string[] = [];
  let pageErrors: string[] = [];

  test.beforeEach(async ({ page }, testInfo) => {
    consoleErrors = [];
    pageErrors = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    releaseBackendLock = await acquireLiveGwtBackendLock(BASE, testInfo);
    // Keep a handle on the app's own backend socket so latency samples ride
    // the exact UI request/reply path instead of a second connection.
    await page.addInitScript(() => {
      const NativeWebSocket = window.WebSocket;
      (window as any).__gwtBenchSockets = [];
      window.WebSocket = new Proxy(NativeWebSocket, {
        construct(Target, args) {
          const socket = new Target(...(args as ConstructorParameters<typeof WebSocket>));
          (window as any).__gwtBenchSockets.push(socket);
          return socket;
        },
      });
    });
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await keepLaunchWizardModalVisible(page);
    await openLiveGwtProject(page);
    await clearLiveLaunchWizard(page);
  });

  test.afterEach(async ({ page }) => {
    if (!releaseBackendLock) return;
    try {
      await clearLiveLaunchWizard(page);
    } finally {
      await releaseBackendLock();
      releaseBackendLock = undefined;
    }
    expect(pageErrors, "page errors").toEqual([]);
    expect(consoleErrors, "console errors").toEqual([]);
  });

  test("Settings > System exposes and persists the agent resource controls", async ({
    page,
  }, testInfo) => {
    await openSystemSettings(page);
    const panel = page.locator("[data-settings-panel='system']").first();
    const enabled = panel.locator("#settings-system-agent-resource-enabled");
    const priority = panel.locator("#settings-system-agent-priority");
    const cpu = panel.locator("#settings-system-agent-cpu-limit");
    const cargo = panel.locator("#settings-system-agent-cargo-jobs");

    await expect(enabled).toBeVisible();
    await expect(enabled).toBeChecked();
    await expect(priority).toHaveClass(/settings-select/);
    await expect(priority).toHaveValue("below-normal");
    await expect(cpu).toHaveClass(/settings-input/);
    await expect(cpu).toHaveAttribute("placeholder", "Automatic");
    await expect(cargo).toHaveClass(/settings-input/);
    await expect(cargo).toHaveAttribute("placeholder", "Automatic");

    await testInfo.attach(`settings-system-${testInfo.project.name}`, {
      body: await page.screenshot({ fullPage: false }),
      contentType: "image/png",
    });

    // Explicit value round-trips through the backend echo.
    await cargo.fill("4");
    await cargo.dispatchEvent("change");
    await cargo.blur();
    await expect(panel.locator("[data-role='system-settings-status']")).toHaveText(
      "Saved system settings.",
      { timeout: 20_000 },
    );
    await expect(panel.locator("#settings-system-agent-cargo-jobs")).toHaveValue("4");

    // Empty input returns to automatic mode (null) rather than a magic zero.
    const cargoAgain = panel.locator("#settings-system-agent-cargo-jobs");
    await cargoAgain.fill("");
    await cargoAgain.dispatchEvent("change");
    await cargoAgain.blur();
    await expect(panel.locator("[data-role='system-settings-status']")).toHaveText(
      "Saved system settings.",
      { timeout: 20_000 },
    );
    await expect(panel.locator("#settings-system-agent-cargo-jobs")).toHaveValue("");

    // Out-of-range input is rejected locally and never persisted.
    const cpuInput = panel.locator("#settings-system-agent-cpu-limit");
    await cpuInput.fill("0");
    await cpuInput.dispatchEvent("change");
    await expect(panel.locator("[data-role='system-settings-status']")).toContainText(
      "CPU limit must be a whole number between 1 and 100",
    );
    await expect(panel.locator("#settings-system-agent-cpu-limit")).toHaveValue("");
  });

  test("N=0/1/3 agent full builds keep UI request/reply latency within budget", async ({
    page,
  }, testInfo) => {
    test.skip(
      testInfo.project.name !== "chromium-dark",
      "latency evidence runs once against the shared backend",
    );
    test.skip(
      !AGENT_ID || !BRANCH_NAME || !ROOT_EXE,
      "GWT_PLAYWRIGHT_RESOURCE_AGENT_ID / GWT_PLAYWRIGHT_BRANCH_NAME / GWT_PLAYWRIGHT_RESOURCE_ROOT_EXE are required",
    );
    test.setTimeout(1_800_000);

    const logicalCores = availableParallelism();
    const expectedCargoJobs = Math.max(1, Math.floor(logicalCores / Math.max(1, MAX_ACTIVE)));
    await sendLiveGwtEvent(page, {
      kind: "set_issue_monitor_max_active_agents",
      max_active_agents: MAX_ACTIVE,
    });
    await page.waitForTimeout(500);

    const launched: string[] = [];
    const samples: LatencySample[] = [];
    let processSnapshot: HostProcess[] = [];
    let afterSnapshot: HostProcess[] = [];
    const paneTexts: Record<string, string> = {};
    let evidenceWritten = false;
    const writeEvidence = async () => {
      const summary = samples.map((sample) => ({
        label: sample.label,
        rtt: stats(sample.rttMs),
        rafGap: stats(sample.rafGapMs),
        longTask: stats(sample.longTaskMs),
      }));
      const n1 = summary.find((entry) => entry.label === "N=1");
      const n3 = summary.find((entry) => entry.label === "N=3");
      const ratio = n1 && n3 && n1.rtt.p95 > 0 ? round(n3.rtt.p95 / n1.rtt.p95) : 0;
      const evidence = {
        recordedAt: new Date().toISOString(),
        platform: process.platform,
        logicalCores,
        maxActive: MAX_ACTIVE,
        expectedCargoJobs,
        warmupSamples: WARMUP_SAMPLES,
        measuredSamples: MEASURED_SAMPLES,
        absoluteP95BudgetMs: ABSOLUTE_P95_BUDGET_MS,
        relativeP95Budget: RELATIVE_P95_BUDGET,
        n3OverN1P95: ratio,
        samples: summary,
        buildProcessesDuringN3: processSnapshot,
        buildProcessesAfterN3: afterSnapshot,
        foreignBuildProcessesDuringN3: foreignBuildProcesses(processSnapshot),
        paneTexts,
      };
      const body = JSON.stringify(evidence, null, 2);
      await testInfo.attach("agent-resource-isolation-evidence", {
        body,
        contentType: "application/json",
      });
      if (EVIDENCE_PATH) {
        mkdirSync(dirname(EVIDENCE_PATH), { recursive: true });
        writeFileSync(EVIDENCE_PATH, body);
      }
      evidenceWritten = true;
      return { summary, n1, n3, ratio };
    };
    let result: Awaited<ReturnType<typeof writeEvidence>> | undefined;
    try {
      // Let the fresh backend finish its startup work before the baseline.
      await expect(async () => {
        await sendLiveGwtEvent(page, { kind: "get_system_settings" });
        await page.waitForFunction(
          () => ((window as any).__gwtPlaywrightMessages as Array<{ payload: any }>).some(
            (entry) => entry?.payload?.kind === "system_settings",
          ),
          undefined,
          { timeout: 10_000 },
        );
      }).toPass({ timeout: 120_000 });
      await settleBackend(page);
      samples.push(await measureLatency(page, "N=0"));

      launched.push(await launchBuildAgent(page));
      await waitForBuildProcesses(1, 300_000);
      samples.push(await measureLatency(page, "N=1"));

      launched.push(await launchBuildAgent(page));
      launched.push(await launchBuildAgent(page));
      await waitForBuildProcesses(3, 300_000);
      processSnapshot = hostBuildProcesses();
      samples.push(await measureLatency(page, "N=3"));
      afterSnapshot = hostBuildProcesses();
      for (const windowId of launched) {
        paneTexts[windowId] = await paneBufferText(page, windowId);
      }
      result = await writeEvidence();
    } finally {
      if (!evidenceWritten) {
        await writeEvidence().catch(() => undefined);
      }
      for (const windowId of launched) {
        await closeWorkspaceWindow(page, windowId).catch((error) => {
          console.warn(`close ${windowId} failed: ${error}`);
        });
      }
      await waitForBuildProcesses(0, 120_000, true).catch((error) => {
        console.warn(`build processes did not drain: ${error}`);
      });
    }

    expect(result, "evidence").toBeDefined();
    const { n1, n3, ratio } = result!;
    expect(n1, "N=1 sample").toBeDefined();
    expect(n3, "N=3 sample").toBeDefined();
    expect(
      afterSnapshot.filter((process) => process.name === "cargo").length,
      "all three cargo trees alive after the N=3 window",
    ).toBeGreaterThanOrEqual(3);
    for (const windowId of launched) {
      expect(paneTexts[windowId], `pane ${windowId} carries the injected cargo budget`).toContain(
        `GWT_BENCH_CARGO_BUILD_JOBS=${expectedCargoJobs}`,
      );
    }
    expect(
      processSnapshot.filter((process) => process.name === "cargo" || process.name === "rustc"),
      "build processes observed during N=3",
    ).not.toEqual([]);
    for (const process of processSnapshot) {
      expect(process.priority, `${process.name}[${process.pid}] priority`).toBe(
        expectedPriorityLabel(),
      );
    }
    expect(n3!.rtt.p95, "N=3 request/reply p95 within the absolute budget").toBeLessThanOrEqual(
      ABSOLUTE_P95_BUDGET_MS,
    );
    expect(ratio, "N=3 / N=1 request/reply p95 ratio").toBeLessThanOrEqual(RELATIVE_P95_BUDGET);
    expect(n3!.rafGap.p95, "N=3 rAF gap p95").toBeLessThanOrEqual(ABSOLUTE_P95_BUDGET_MS);
  });
});

function expectedPriorityLabel(): string {
  return process.platform === "win32" ? "BelowNormal" : "10";
}

/**
 * cargo / rustc processes that descend from the gwt process under test, with
 * their scheduling priority. Scoping to the process tree keeps builds run by
 * other sessions on the same host out of the evidence.
 */
function hostBuildProcesses(): HostProcess[] {
  if (process.platform === "win32") {
    const script = [
      "$rootExe = $env:GWT_PLAYWRIGHT_RESOURCE_ROOT_EXE",
      "$all = Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,ExecutablePath",
      "$set = New-Object 'System.Collections.Generic.HashSet[int]'",
      "foreach ($p in $all) { if ($p.ExecutablePath -eq $rootExe) { [void]$set.Add([int]$p.ProcessId) } }",
      "$changed = $true",
      "while ($changed) { $changed = $false; foreach ($p in $all) { if ($set.Contains([int]$p.ParentProcessId) -and -not $set.Contains([int]$p.ProcessId)) { [void]$set.Add([int]$p.ProcessId); $changed = $true } } }",
      "$result = @()",
      "foreach ($p in $all) { if ($set.Contains([int]$p.ProcessId) -and ($p.Name -eq 'cargo.exe' -or $p.Name -eq 'rustc.exe')) { $proc = Get-Process -Id $p.ProcessId -ErrorAction SilentlyContinue; if ($proc) { $result += [pscustomobject]@{ Id = [int]$p.ProcessId; ProcessName = $proc.ProcessName; Priority = [string]$proc.PriorityClass } } } }",
      "if ($result.Count -eq 0) { '[]' } else { ConvertTo-Json -Compress @($result) }",
    ].join("; ");
    const output = execFileSync("powershell", ["-NoProfile", "-Command", script], {
      encoding: "utf8",
      env: { ...process.env, GWT_PLAYWRIGHT_RESOURCE_ROOT_EXE: ROOT_EXE },
    }).trim();
    const parsed = JSON.parse(output || "[]") as Array<{
      Id: number;
      ProcessName: string;
      Priority: string;
    }>;
    return parsed.map((entry) => ({
      pid: entry.Id,
      name: entry.ProcessName,
      priority: entry.Priority,
    }));
  }
  const rows = execFileSync("ps", ["-axo", "pid=,ppid=,ni=,comm="], { encoding: "utf8" })
    .split("\n")
    .map((line) => line.trim().split(/\s+/))
    .filter((parts) => parts.length >= 4)
    .map((parts) => ({
      pid: Number(parts[0]),
      ppid: Number(parts[1]),
      nice: parts[2],
      comm: parts.slice(3).join(" "),
    }));
  const set = new Set<number>(rows.filter((row) => row.comm === ROOT_EXE).map((row) => row.pid));
  let changed = true;
  while (changed) {
    changed = false;
    for (const row of rows) {
      if (set.has(row.ppid) && !set.has(row.pid)) {
        set.add(row.pid);
        changed = true;
      }
    }
  }
  return rows
    .filter((row) => set.has(row.pid) && /(^|\/)(cargo|rustc)$/.test(row.comm))
    .map((row) => ({
      pid: row.pid,
      name: row.comm.replace(/^.*\//, ""),
      priority: row.nice,
    }));
}

/** Host-wide cargo / rustc processes that are NOT part of the gwt tree (noise record). */
function foreignBuildProcesses(scoped: HostProcess[]): HostProcess[] {
  const scopedIds = new Set(scoped.map((entry) => entry.pid));
  if (process.platform !== "win32") return [];
  const output = execFileSync(
    "powershell",
    [
      "-NoProfile",
      "-Command",
      "$p = Get-Process -Name cargo,rustc -ErrorAction SilentlyContinue | " +
        "Select-Object Id,ProcessName,@{Name='Priority';Expression={[string]$_.PriorityClass}}; " +
        "if ($null -eq $p) { '[]' } else { ConvertTo-Json -Compress @($p) }",
    ],
    { encoding: "utf8" },
  ).trim();
  const parsed = JSON.parse(output || "[]") as Array<{ Id: number; ProcessName: string; Priority: string }>;
  return parsed
    .filter((entry) => !scopedIds.has(entry.Id))
    .map((entry) => ({ pid: entry.Id, name: entry.ProcessName, priority: entry.Priority }));
}

async function waitForBuildProcesses(
  minimumCargo: number,
  timeoutMs: number,
  exact = false,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const processes = hostBuildProcesses();
    const cargo = processes.filter((entry) => entry.name === "cargo").length;
    const rustc = processes.filter((entry) => entry.name === "rustc").length;
    if (exact ? cargo === minimumCargo && rustc === 0 : cargo >= minimumCargo && rustc > 0) {
      return;
    }
    if (Date.now() >= deadline) {
      throw new Error(
        `build processes did not reach cargo>=${minimumCargo} (exact=${exact}): cargo=${cargo} rustc=${rustc}`,
      );
    }
    await new Promise((resolve) => setTimeout(resolve, 2_000));
  }
}

/**
 * Sample UI request/reply latency on the app's live backend socket while
 * recording rAF gaps and long tasks on the same document.
 */
async function measureLatency(
  page: Page,
  label: string,
  warmupSamples = WARMUP_SAMPLES,
  measuredSamples = MEASURED_SAMPLES,
): Promise<LatencySample> {
  const result = await page.evaluate(
    async ({ warmup, samples }) => {
      const sockets = ((window as any).__gwtBenchSockets as WebSocket[]) ?? [];
      const socket = [...sockets]
        .reverse()
        .find((candidate) => candidate.readyState === WebSocket.OPEN && /\/ws$/.test(candidate.url));
      if (!socket) throw new Error("no open backend socket to sample");
      const rttMs: number[] = [];
      const rafGapMs: number[] = [];
      const longTaskMs: number[] = [];
      let rafActive = true;
      let lastFrame = performance.now();
      const frame = (timestamp: number) => {
        rafGapMs.push(timestamp - lastFrame);
        lastFrame = timestamp;
        if (rafActive) requestAnimationFrame(frame);
      };
      requestAnimationFrame((timestamp) => {
        lastFrame = timestamp;
        requestAnimationFrame(frame);
      });
      let observer: PerformanceObserver | undefined;
      try {
        observer = new PerformanceObserver((list) => {
          for (const entry of list.getEntries()) longTaskMs.push(entry.duration);
        });
        observer.observe({ type: "longtask", buffered: false });
      } catch {
        observer = undefined;
      }
      for (let index = 0; index < warmup + samples; index += 1) {
        const started = performance.now();
        const reply = new Promise<void>((resolve, reject) => {
          const timer = setTimeout(() => {
            socket.removeEventListener("message", onMessage);
            reject(new Error("system_settings reply timed out"));
          }, 30_000);
          function onMessage(event: MessageEvent) {
            try {
              const payload = JSON.parse(String(event.data));
              if (
                payload.kind === "system_settings"
                || payload.kind === "system_settings_error"
              ) {
                clearTimeout(timer);
                socket.removeEventListener("message", onMessage);
                resolve();
              }
            } catch {
              /* non-JSON frame */
            }
          }
          socket.addEventListener("message", onMessage);
        });
        socket.send(JSON.stringify({ kind: "get_system_settings" }));
        await reply;
        if (index >= warmup) rttMs.push(performance.now() - started);
        await new Promise((resolve) => setTimeout(resolve, 50));
      }
      rafActive = false;
      observer?.disconnect();
      return { rttMs, rafGapMs: rafGapMs.slice(1), longTaskMs };
    },
    { warmup: warmupSamples, samples: measuredSamples },
  );
  return { label, ...result };
}

async function openSystemSettings(page: Page): Promise<void> {
  await page.evaluate(() => {
    document.dispatchEvent(
      new CustomEvent("settings:open", { detail: { target: "system" }, bubbles: true }),
    );
  });
  await expect(page.locator("[data-settings-panel='system']").first()).toBeVisible({
    timeout: 10_000,
  });
  await expect(
    page.locator("[data-settings-panel='system'] #settings-system-agent-resource-enabled").first(),
  ).toBeVisible({ timeout: 10_000 });
}

async function launchBuildAgent(page: Page): Promise<string> {
  const beforeIds = await workspaceWindowIds(page);
  await openLaunchWizardForCurrentBranch(page);
  const wizard = page.locator("#wizard-modal");
  // Opening resolves the branch / worktree through git first.
  await expect(wizard).toBeVisible({ timeout: 60_000 });
  await chooseConfigureAndStart(page);
  await selectWizardAgent(page, AGENT_ID);

  const submit = page.locator("#wizard-submit-button");
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const text = ((await submit.textContent()) ?? "").trim();
    if (text === "Launch" || text === "Create and launch") break;
    await submit.click();
    await page.waitForTimeout(400);
  }
  await expect(submit).toHaveText(/^(Launch|Create and launch)$/);
  await submit.click();

  const id = await page
    .waitForFunction(
      ({ beforeIds }) => {
        const seen = new Set(beforeIds);
        const node = Array.from(document.querySelectorAll(".workspace-window")).find(
          (candidate) => {
            const element = candidate as HTMLElement;
            if (seen.has(element.dataset.id || "")) return false;
            return Boolean(element.querySelector(".terminal-root"));
          },
        );
        return node ? (node as HTMLElement).dataset.id || "" : "";
      },
      { beforeIds },
      { timeout: 90_000 },
    )
    .then((handle) => handle.jsonValue());
  expect(id).toBeTruthy();
  await expect(async () => {
    expect(await paneBufferText(page, id)).toContain("GWT_BENCH_CARGO_BUILD_JOBS=");
  }).toPass({ timeout: 90_000 });
  return id;
}

async function paneBufferText(page: Page, windowId: string): Promise<string> {
  return page.evaluate(
    (id) => String((window as any).__gwtTerminalTestApi?.bufferText?.(id) ?? ""),
    windowId,
  );
}

/**
 * Wait until the backend answers quickly and consistently so the baseline is
 * not polluted by startup / project-open work still draining on the runtime.
 */
async function settleBackend(page: Page): Promise<void> {
  await expect(async () => {
    const sample = await measureLatency(page, "settle", 0, 10);
    expect(Math.max(...sample.rttMs)).toBeLessThan(50);
  }).toPass({ timeout: 180_000, intervals: [2_000] });
}

async function closeWorkspaceWindow(page: Page, windowId: string): Promise<void> {
  const window = page.locator(`.workspace-window[data-id="${windowId}"]`);
  if (!(await window.count())) return;
  await window.getByLabel("Close window").click();
  const closeConfirm = page.locator(
    '#window-close-confirm-modal [data-role="window-close-confirm"]',
  );
  // SPEC-3038 US-3 Close Guard: a live agent window always asks first.
  await closeConfirm.waitFor({ state: "visible", timeout: 10_000 }).catch(() => undefined);
  if (await closeConfirm.isVisible().catch(() => false)) {
    await closeConfirm.click();
  }
  await expect(window).toHaveCount(0, { timeout: 60_000 });
}

async function workspaceWindowIds(page: Page): Promise<string[]> {
  return page
    .locator(".workspace-window")
    .evaluateAll((nodes) => nodes.map((node) => (node as HTMLElement).dataset.id || ""));
}

async function keepLaunchWizardModalVisible(page: Page): Promise<void> {
  await page.addStyleTag({
    content: `
      #wizard-modal[aria-hidden="false"],
      #wizard-modal.open {
        display: flex !important;
        pointer-events: auto !important;
      }
      #wizard-modal[aria-hidden="true"] {
        display: none !important;
        pointer-events: none !important;
      }
    `,
  });
}

async function selectWizardAgent(page: Page, agentId: string): Promise<void> {
  const wizard = page.locator("#wizard-modal");
  const agentField = wizard.getByLabel("Agent", { exact: true });
  await expect(agentField).toBeVisible();
  const tag = await agentField.evaluate((node) => node.tagName.toLowerCase());
  if (tag === "select") {
    await agentField.selectOption(agentId);
    await expect(agentField).toHaveValue(agentId);
    await agentField.blur();
    return;
  }
  const option = wizard.locator(`.launch-segmented__option[data-value="${agentId}"]`);
  await option.click();
  await expect(option).toHaveAttribute("aria-checked", "true");
  await page.evaluate(() => {
    const active = document.activeElement;
    if (active instanceof HTMLElement) active.blur();
  });
}

async function openLaunchWizardForCurrentBranch(page: Page): Promise<void> {
  const beforeIds = await workspaceWindowIds(page);
  await sendLiveGwtEvent(page, {
    kind: "create_window",
    preset: "work",
    bounds: { x: 96, y: 96, width: 880, height: 520 },
  });
  const workWindowId = await page
    .waitForFunction(
      ({ beforeIds }) => {
        const seen = new Set(beforeIds);
        const node = Array.from(document.querySelectorAll(".workspace-window")).find(
          (candidate) => !seen.has((candidate as HTMLElement).dataset.id || ""),
        );
        return node ? (node as HTMLElement).dataset.id || "" : "";
      },
      { beforeIds },
    )
    .then((handle) => handle.jsonValue());
  expect(workWindowId).toBeTruthy();
  await sendLiveGwtEvent(page, {
    kind: "open_launch_wizard",
    id: workWindowId,
    branch_name: BRANCH_NAME,
  });
}

async function chooseConfigureAndStart(page: Page): Promise<void> {
  const wizard = page.locator("#wizard-modal");
  const agentSelect = wizard.getByLabel("Agent", { exact: true });
  if (await agentSelect.isVisible().catch(() => false)) {
    return;
  }
  await sendLiveGwtEvent(page, {
    kind: "launch_wizard_action",
    action: { kind: "set_launch_path", path: "manual_setup" },
    bounds: null,
  });
  await agentSelect.waitFor({ state: "visible", timeout: 10_000 });
}

