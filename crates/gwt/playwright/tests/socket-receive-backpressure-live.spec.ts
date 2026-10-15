import { expect, test, type CDPSession, type Page, type TestInfo } from "@playwright/test";
import type { WebSocket } from "ws";
import type { IncomingMessage } from "node:http";
import { execFileSync } from "node:child_process";
import { writeFile } from "node:fs/promises";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const THIRTY_MINUTES = 30 * 60 * 1000;
const RECOVERY_TIMEOUT_MS = 15_000;
const durationMs = Number(process.env.GWT_SOCKET_LOAD_DURATION_MS ?? THIRTY_MINUTES);
const label = process.env.GWT_SOCKET_LOAD_LABEL ?? "after";
const paneIds = Array.from({ length: 8 }, (_, index) => `socket-load-agent-${index + 1}`);

test.use({ viewport: { width: 1680, height: 1000 }, launchOptions: {
  // Exercise Chromium's native background policy, which automation disables.
  ignoreDefaultArgs: ["--disable-background-timer-throttling", "--disable-renderer-backgrounding"],
} });

test.describe("background receive backpressure (Issue #5531)", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL || !process.env.GWT_PLAYWRIGHT_CHECK_HOME,
    "requires the owned checkout and fresh HOME from issue-5531-browser-check.ps1");
  test.setTimeout(durationMs + 180_000);

  test("eight streaming agent panes recover their latest snapshot and input within two seconds", async ({ page, context }, info) => {
    expect(["before", "after"]).toContain(label);
    expect(durationMs).toBeGreaterThanOrEqual(1000);
    if (durationMs < THIRTY_MINUTES) {
      expect(process.env.GWT_SOCKET_LOAD_ALLOW_SMOKE,
        "short runs require explicit smoke opt-in and never prove the 30-minute AC").toBe("1");
    }
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const root = process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT!;
    const projectKey = process.env.GWT_SOCKET_LOAD_PROJECT_KEY!;
    expect(projectKey).toMatch(/^[0-9a-f]{16}$/);
    const baselineSha = process.env.GWT_SOCKET_LOAD_BASELINE_SHA
      ?? execFileSync("git", ["rev-parse", "origin/develop"], { cwd: root, encoding: "utf8" }).trim();
    const fixture = await installBackend(page, projectKey, root);
    const browserSession = await context.browser()!.newBrowserCDPSession();
    const browserProcess = (await browserSession.send("SystemInfo.getProcessInfo")).processInfo
      .find((process: { type: string }) => process.type === "browser");
    const browserPid = Number(browserProcess?.id);
    expect(Number.isSafeInteger(browserPid) && browserPid > 0).toBe(true);
    let cover: Page | undefined;
    let evidence: unknown;
    try {
      await installMeasurements(page);
      await page.route("**/socket-receive-dispatcher.js", async route => {
        const source = label === "before"
          ? execFileSync("git", ["show", `${baselineSha}:crates/gwt/web/socket-receive-dispatcher.js`],
            { cwd: root, encoding: "utf8" })
          : await (await route.fetch()).text();
        await attachJson(info, `${label}-${theme}-dispatcher-source`, { baseline_sha: baselineSha, source });
        // Both modes get identical passive queue instrumentation. The baseline
        // gets no-op resume/dispose adapters only because its public API predates
        // those methods; its scheduling, queue, parsing and receive remain intact.
        const measured = source.replace("export function createSocketReceiveDispatcher(",
          "function createMeasuredSocketReceiveDispatcher(");
        expect(measured).not.toBe(source);
        await route.fulfill({ contentType: "text/javascript", body: `${measured}\n
          export function createSocketReceiveDispatcher(options) {
            const metrics = window.__socketLoadMetrics;
            const dispatcher = createMeasuredSocketReceiveDispatcher({ ...options,
              onResync(reason) { metrics.resyncs++;
                metrics.resync_reasons[reason] = (metrics.resync_reasons[reason] || 0) + 1;
                options.onResync?.(reason); } });
            return { ...dispatcher,
              handle(event) { try { return dispatcher.handle(event); }
                finally { metrics.maxQueue = Math.max(metrics.maxQueue, dispatcher.pendingCount()); } },
              resume() { metrics.resumes++; return dispatcher.resume?.(); },
              dispose() { return dispatcher.dispose?.(); }
            };
          }` });
      });
      await page.bringToFront();
      await gotoLiveGwt(page, process.env.GWT_PLAYWRIGHT_BASE_URL!, {
        projectKey, suppressProjectSurfaces: true,
      });
      await page.bringToFront();
      showOwnedChromium(browserPid);
      await page.waitForFunction(() => document.visibilityState === "visible", undefined,
        { polling: 100, timeout: 10_000 });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(page.locator('.workspace-window[data-preset="agent"]')).toHaveCount(8);
      await expect.poll(() => terminalTexts(page)).toEqual(paneIds.map(id => snapshotText(0, id)));
      console.log(`${label}/${theme}: eight terminal snapshots rendered`);
      expect(await page.evaluate(() => PerformanceObserver.supportedEntryTypes.includes("longtask"))).toBe(true);
      const browserTimeOriginMs = await page.evaluate(() => performance.timeOrigin);

      // Playwright enables focus emulation by default. Disable it so the
      // second real Chromium tab genuinely backgrounds the measured document.
      const cdp = await context.newCDPSession(page);
      await cdp.send("Emulation.setFocusEmulationEnabled", { enabled: false });
      await page.bringToFront();
      await page.evaluate(() => { (window as any).__socketLoadMetrics.active = true; });
      const coverOpened = context.waitForEvent("page");
      // Open a tab from the measured page; context.newPage() may create a
      // separate window, which leaves the original document visible.
      await page.evaluate(() => window.open("about:blank", "socket-load-cover"));
      cover = await coverOpened;
      await cover.waitForLoadState();
      await cover.bringToFront();
      const measuredWindow = await cdp.send("Browser.getWindowForTarget");
      await cdp.send("Browser.setWindowBounds", { windowId: measuredWindow.windowId, bounds: { windowState: "minimized" } });
      // Poll from the controller: both rAF and page timers can stop in a
      // genuinely hidden document after prolonged background throttling.
      await expect.poll(() => page.evaluate(() => document.visibilityState), { timeout: 10_000 }).toBe("hidden");
      const backgroundStarted = Date.now();
      fixture.start();
      const samples: unknown[] = [];
      let lastBackgroundMetrics: any;
      while (Date.now() - backgroundStarted < durationMs) {
        await new Promise(resolve => setTimeout(resolve, Math.min(30_000,
          durationMs - (Date.now() - backgroundStarted))));
        const background = await measuredEvaluate(cdp, () => ({
          visibility: document.visibilityState,
          metrics: (window as any).__socketLoadCaptureMetrics(),
        }));
        lastBackgroundMetrics = background.metrics;
        samples.push({ at_ms: Date.now() - backgroundStarted, ...fixture.counters(), visibility: background.visibility,
          metrics: { messages: background.metrics.messages, maxQueue: background.metrics.maxQueue,
            max_long_task_ms: background.metrics.max_long_task_ms, resyncs: background.metrics.resyncs,
            resumes: background.metrics.resumes } });
        // Persist controller-owned evidence before foreground recovery can hang.
        await writeFile(info.outputPath(`${label}-${theme}-background-samples.json`),
          JSON.stringify({ label, theme, browser_performance_time_origin_ms: browserTimeOriginMs, samples }, null, 2));
        expect(fixture.errors).toEqual([]);
        expect(background.visibility).toBe("hidden");
        console.log(`${label}/${theme}: ${Math.round((Date.now() - backgroundStarted) / 1000)}s background; ${fixture.counters().frames} WS messages`);
      }
      const backgroundElapsedMs = Date.now() - backgroundStarted;
      fixture.stop();
      // A final hidden burst puts both implementations under the same immediate
      // recovery pressure even if Chromium's native timer policy changed.
      fixture.burst();
      // Native background policy may also delay browser message delivery.
      // Resume with those frames pending instead of demanding a hidden drain.
      const readyBeforeResume = fixture.counters().ready_requests;
      fixture.freezeSnapshot();
      const input = "GWT_INPUT_5531";
      let outcome: "complete" | "timeout" | "error" = "complete";
      let recoveryStage = "restore_window";
      let failedRecoveryStage: string | null = null;
      let recoveryError: string | null = null;
      let recoveryTimeoutMs: number | null = null;
      let snapshotMs: number | null = null;
      let inputMs: number | null = null;
      let foregroundMetricsComplete = false;
      let collectedForegroundMetrics: any;
      let metrics = { ...lastBackgroundMetrics, resume_max_long_task_ms: null, settled_max_long_task_ms: null };
      // This controller timestamp includes a blocked renderer delaying its
      // visibility handler. Never re-anchor it to a page-side callback.
      const restoreStarted = Date.now();
      const restorePerformanceStart = restoreStarted - browserTimeOriginMs;
      const recoveryTask = (async () => {
        await cdp.send("Browser.setWindowBounds", { windowId: measuredWindow.windowId, bounds: { windowState: "normal" } });
        recoveryStage = "bring_to_front";
        await cdp.send("Page.bringToFront");
        recoveryStage = "visible";
        await measuredWait(cdp, () => document.visibilityState === "visible");
        recoveryStage = "snapshots";
        await measuredWait(cdp, ({ ids, revision }) => ids.every(id =>
          (window as any).__gwtTerminalTestApi.bufferText(id).includes(`GWT_SNAPSHOT_5531_${revision}_${id}`)),
        { ids: paneIds, revision: fixture.revision() });
        const snapshotDuration = Date.now() - restoreStarted;
        // Echo after snapshots so replacement cannot erase a successful echo.
        recoveryStage = "input_focus";
        await measuredEvaluate(cdp, id => {
          const textarea = document.querySelector<HTMLTextAreaElement>(`.workspace-window[data-id="${id}"] .xterm-helper-textarea`);
          if (!textarea) throw new Error("terminal input is missing");
          textarea.focus();
          if (document.activeElement !== textarea) throw new Error("terminal input did not focus");
        }, paneIds[0]);
        recoveryStage = "input_type";
        // Pipeline ordered protocol messages rather than charging 26 controller
        // round trips to the renderer's input latency.
        await Promise.all([...input].flatMap(key => {
          const code = key === "_" ? "Minus" : /\d/.test(key) ? `Digit${key}` : `Key${key}`;
          const windowsVirtualKeyCode = key === "_" ? 189 : key.charCodeAt(0);
          return [
            cdp.send("Input.dispatchKeyEvent", { type: "keyDown", key, code, windowsVirtualKeyCode, text: key, unmodifiedText: key }),
            cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key, code, windowsVirtualKeyCode }),
          ];
        }));
        recoveryStage = "input_echo";
        await measuredWait(cdp, ({ id, value }) =>
          (window as any).__gwtTerminalTestApi.bufferText(id).includes(value),
        { id: paneIds[0], value: input });
        const inputDuration = Date.now() - restoreStarted;
        recoveryStage = "foreground_metrics";
        return { snapshotMs: snapshotDuration, inputMs: inputDuration };
      })();
      // Baseline snapshot recovery may never finish, even when its renderer
      // can answer measurements. Collect both task windows independently.
      const metricsOperation = (async () => {
        await new Promise(resolve => setTimeout(resolve, Math.max(0, 4200 - (Date.now() - restoreStarted))));
        const captured = await measuredEvaluate(cdp, () => (window as any).__socketLoadCaptureMetrics());
        const maximum = (from: number, to: number) => Math.max(0, ...captured.long_tasks
          .filter((entry: { start_ms: number; duration_ms: number }) =>
            entry.start_ms + entry.duration_ms >= restorePerformanceStart + from
            && entry.start_ms <= restorePerformanceStart + to)
          .map((entry: { duration_ms: number }) => entry.duration_ms));
        return { ...captured, resume_max_long_task_ms: maximum(0, 2000), settled_max_long_task_ms: maximum(2000, 4000) };
      })();
      const metricsTask = bounded(metricsOperation, Math.max(1, RECOVERY_TIMEOUT_MS - (Date.now() - restoreStarted)))
        .then(captured => { collectedForegroundMetrics = captured; return captured; });
      try {
        const [recovered, captured] = await bounded(Promise.all([recoveryTask, metricsTask]),
          Math.max(1, RECOVERY_TIMEOUT_MS - (Date.now() - restoreStarted)));
        snapshotMs = recovered.snapshotMs;
        inputMs = recovered.inputMs;
        metrics = captured;
        foregroundMetricsComplete = true;
        recoveryStage = "complete";
      } catch (error) {
        failedRecoveryStage = recoveryStage;
        outcome = error instanceof MeasurementTimeout || (error as Error)?.name === "TimeoutError" ? "timeout" : "error";
        recoveryError = String(error);
        if (outcome === "timeout") recoveryTimeoutMs = Date.now() - restoreStarted;
        snapshotMs = null;
        inputMs = null;
        foregroundMetricsComplete = Boolean(collectedForegroundMetrics);
        // Snapshot timeout is not proof of a blocked renderer. Keep independent
        // foreground measurements when available, otherwise report them unknown.
        metrics = collectedForegroundMetrics
          ?? { ...lastBackgroundMetrics, resume_max_long_task_ms: null, settled_max_long_task_ms: null };
        // The bounded race observes late rejections; killing only this recorded
        // browser PID also releases pending UI commands and Playwright teardown.
        stopOwnedChromium(browserPid);
        await bounded(Promise.allSettled([recoveryTask, metricsOperation, metricsTask]), 2000).catch(() => {});
      }
      evidence = {
        label, theme, baseline_sha: baselineSha,
        outcome, recovery_stage: failedRecoveryStage ?? recoveryStage, recovery_error: recoveryError,
        recovery_timeout_ms: recoveryTimeoutMs,
        foreground_metrics_complete: foregroundMetricsComplete,
        timing_origin: "controller timestamp immediately before native window restore request",
        controller_restore_started_at_ms: restoreStarted,
        browser_performance_time_origin_ms: browserTimeOriginMs,
        controller_restore_start_performance_ms: restorePerformanceStart,
        long_task_windows_ms: { recovery: [0, 2000], settled: [2000, 4000] },
        acceptance_duration: backgroundElapsedMs >= THIRTY_MINUTES,
        background_elapsed_ms: backgroundElapsedMs,
        workload: { transport: "native loopback WebSocket (ws@8.18.3); browser message events", panes: 8, tick_ms: 50, burst_events: 4096,
          burst_interval_ms: 30_000, events: ["terminal_output", "terminal_preview", "workspace_state"] },
        frontend: { base_url: process.env.GWT_PLAYWRIGHT_BASE_URL, checkout: root,
          owned_pid: process.env.GWT_PLAYWRIGHT_PID, isolated_home: process.env.GWT_PLAYWRIGHT_CHECK_HOME,
          browser_pid: browserPid, background: "native minimized Chromium window" },
        input_ms: inputMs,
        snapshot_ms: snapshotMs,
        ready_requests_after_resume: fixture.counters().ready_requests - readyBeforeResume,
        metrics, traffic: fixture.counters(), samples, errors, fixture_errors: fixture.errors,
      };
      await attachJson(info, `${label}-${theme}-background-recovery`, evidence);
      if (outcome === "complete") {
        await page.screenshot({ path: info.outputPath(`${label}-${theme}-recovered.png`), timeout: 2000 });
      }
      expect(errors, "console/page errors").toEqual([]);
      expect(fixture.errors, "WebSocket fixture errors").toEqual([]);
      expect(metrics.messages).toBeGreaterThan(4000);
      expect(Number.isFinite(metrics.max_long_task_ms), "last completed long-task maximum was recorded").toBe(true);
      if (label === "after") {
        expect(outcome, "foreground recovery completed").toBe("complete");
        expect(inputMs !== null && inputMs <= 2000, "terminal keyboard echo within two seconds").toBe(true);
        expect(snapshotMs !== null && snapshotMs <= 2000, "all eight authoritative terminal snapshots within two seconds").toBe(true);
        expect(fixture.counters().ready_requests - readyBeforeResume, "visible resume requests fresh backend state").toBeGreaterThan(0);
        // Whether native background delivery reaches the cap depends on the
        // browser. Unit tests force overflow; this run measures its real policy.
        expect(metrics.resumes, "real visible event resumes the dispatcher").toBeGreaterThan(0);
        expect(metrics.maxQueue, "dispatcher default queue bound").toBeLessThanOrEqual(2048);
        expect(foregroundMetricsComplete, "independent foreground metrics were captured").toBe(true);
        expect(Number.isFinite(metrics.resume_max_long_task_ms), "initial foreground long tasks were measured").toBe(true);
        expect(Number.isFinite(metrics.settled_max_long_task_ms), "foreground long tasks were measured").toBe(true);
        expect(metrics.settled_max_long_task_ms, "long tasks settle below 200ms within two seconds").toBeLessThan(200);
      } else {
        expect(outcome, "baseline completed or captured a bounded timeout").not.toBe("error");
      }
    } finally {
      await fixture.close();
      await bounded(cover?.close() ?? Promise.resolve(), 2000).catch(() => {});
      await attachJson(info, `${label}-${theme}-errors`, { errors, fixture: fixture.errors, evidence_recorded: Boolean(evidence) });
    }
  });
});

function snapshotText(revision: number, id: string) { return `GWT_SNAPSHOT_5531_${revision}_${id}`; }

// Keep tracing enabled without its before/after DOM snapshots running inside
// the timed interval. CDP sends are traced but do not trigger those snapshots.
// Input still uses Chromium's trusted keyboard path and the real backend echo.
async function measuredEvaluate<T, A = undefined>(cdp: CDPSession, callback: (argument: A) => T, argument?: A): Promise<Awaited<T>> {
  const result = await cdp.send("Runtime.evaluate", {
    expression: `(${callback.toString()})(${JSON.stringify(argument) ?? "undefined"})`,
    awaitPromise: true, returnByValue: true,
  });
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
  return result.result.value;
}

async function measuredWait<A = undefined>(cdp: CDPSession, predicate: (argument: A) => boolean, argument?: A) {
  const deadline = Date.now() + RECOVERY_TIMEOUT_MS;
  while (!await measuredEvaluate(cdp, predicate, argument)) {
    if (Date.now() >= deadline) throw new MeasurementTimeout("measured browser condition timed out");
    await new Promise(resolve => setTimeout(resolve, 100));
  }
}

class MeasurementTimeout extends Error {}

async function bounded<T>(operation: Promise<T>, milliseconds: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([operation, new Promise<never>((_resolve, reject) => {
      timer = setTimeout(() => reject(new MeasurementTimeout(`Measurement timed out after ${milliseconds}ms`)), milliseconds);
    })]);
  } finally {
    clearTimeout(timer);
  }
}

function stopOwnedChromium(pid: number) {
  if (process.platform === "win32") {
    try { execFileSync("taskkill.exe", ["/PID", String(pid), "/T", "/F"], { timeout: 5000, stdio: "ignore" }); }
    catch (error) { console.warn(`Cleanup of owned Chromium PID ${pid}: ${String(error)}`); }
  } else {
    try { process.kill(pid, "SIGKILL"); } catch { /* browser may already be closed */ }
  }
}

function showOwnedChromium(pid: number) {
  if (process.platform !== "win32") return;
  // Windows may launch an unattended browser occluded. Reveal only this
  // browser's native window before disabling focus emulation is measured.
  const shown = execFileSync("pwsh", ["-NoProfile", "-Command", `
    Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; using System.Text;
      public class OwnedChromiumWindow {
        public delegate bool Callback(IntPtr h, IntPtr value);
        [DllImport("user32.dll")] public static extern bool EnumWindows(Callback callback, IntPtr value);
        [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder value, int length);
        [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr h, int command);
        [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int width, int height, uint flags);
        public static int Reveal(uint pid) {
          int count=0;
          EnumWindows((h, value) => { uint owner; GetWindowThreadProcessId(h, out owner);
            var name=new StringBuilder(256); GetClassName(h, name, 256);
            if (owner==pid && name.ToString()=="Chrome_WidgetWin_1") {
              ShowWindowAsync(h, 9);
              if (!SetWindowPos(h, new IntPtr(-1), 0, 0, 0, 0, 3)) throw new Exception("Could not reveal owned window");
              count++;
            } return true;
          }, IntPtr.Zero); return count;
        }
      }';
    $shown=[OwnedChromiumWindow]::Reveal(${pid});
    if (!$shown) { throw 'Owned Chromium has no native window' }; Write-Output $shown;
  `], { encoding: "utf8" });
  console.log(`revealed ${shown.trim()} native window(s) owned by Chromium PID ${pid}`);
}

async function terminalTexts(page: Page): Promise<string[]> {
  return page.evaluate(ids => ids.map(id => (window as any).__gwtTerminalTestApi?.bufferText(id).trim() ?? ""), paneIds);
}

async function attachJson(info: TestInfo, name: string, value: unknown) {
  const path = info.outputPath(`${name}.json`);
  await writeFile(path, JSON.stringify(value, null, 2));
  await info.attach(name, { path, contentType: "application/json" });
}

async function installMeasurements(page: Page) {
  await page.addInitScript(() => {
    (window as any).__gwtPlaywrightTestBridge = true;
    const metrics = { active: false, maxQueue: 0, messages: 0, resyncs: 0, resumes: 0,
      resync_reasons: {} as Record<string, number>,
      visible_at: 0, max_long_task_ms: 0,
      long_tasks: [] as Array<{ start_ms: number; duration_ms: number }>, visibility: [] as unknown[] };
    (window as any).__socketLoadMetrics = metrics;
    document.addEventListener("visibilitychange", () => {
      metrics.visibility.push({ state: document.visibilityState, at_ms: performance.now() });
      if (metrics.active && !document.hidden) metrics.visible_at = performance.now();
    });
    const record = (entries: PerformanceEntryList) => {
      if (!metrics.active) return;
      for (const entry of entries) {
        metrics.max_long_task_ms = Math.max(metrics.max_long_task_ms, entry.duration);
        metrics.long_tasks.push({ start_ms: entry.startTime, duration_ms: entry.duration });
        if (metrics.long_tasks.length > 2000) metrics.long_tasks.splice(0, metrics.long_tasks.length - 2000);
      }
    };
    const observer = new PerformanceObserver(list => record(list.getEntries()));
    (window as any).__socketLoadCaptureMetrics = () => {
      // Include completed entries whose observer callback has not run yet.
      record(observer.takeRecords());
      return metrics;
    };
    observer.observe({ type: "longtask", buffered: false });
    const Native = window.WebSocket;
    window.WebSocket = new Proxy(Native, {
      construct(Target, args) {
        const socket = new Target(...args as ConstructorParameters<typeof WebSocket>);
        socket.addEventListener("message", () => { metrics.messages++; });
        return socket;
      },
    });
  });
}

/** Synthetic backend traffic reaches the live frontend's actual socket handler. */
async function installBackend(page: Page, projectKey: string, root: string) {
  const { WebSocketServer } = require(process.env.GWT_SOCKET_WS_PACKAGE!);
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 });
  await new Promise<void>((resolve, reject) => {
    server.once("listening", resolve);
    server.once("error", reject);
  });
  const address = server.address() as { port: number };
  await page.addInitScript(port => {
    const Native = window.WebSocket;
    window.WebSocket = new Proxy(Native, {
      construct(Target, args) {
        const url = new URL(String(args[0]), location.href);
        if (url.pathname === "/ws") { url.hostname = "127.0.0.1"; url.port = String(port); }
        return Reflect.construct(Target, [url.toString(), ...args.slice(1)]);
      },
    });
  }, address.port);
  const clients = new Map<WebSocket, boolean>();
  const errors: string[] = [];
  let revision = 0;
  let ticks = 0;
  let frames = 0;
  let readyRequests = 0;
  let timer: ReturnType<typeof setInterval> | undefined;
  const project = { id: "socket-load-project", project_key: projectKey, title: "Receive load", kind: "git" };
  const windows = paneIds.map((id, index) => ({
    id, title: `Agent ${index + 1}`, preset: "agent", agent_id: "codex", agent_name: `Load${index + 1}`,
    geometry: { x: 20 + index % 4 * 405, y: 70 + Math.floor(index / 4) * 420, width: 395, height: 390 },
    geometry_revision: 0, z_index: index + 1, status: "running", minimized: false, maximized: false,
    pre_maximize_geometry: null, persist: true, purpose_title: null, dynamic_title: null,
    dynamic_title_detail: null, agent_color: null, tab_group_id: null, tab_group_active: false,
    placement: { kind: "canvas" },
  }));
  const workspace = () => ({ kind: "workspace_state", workspace: { app_version: "playwright",
    tabs: [{ ...project, project_root: root, workspace: { viewport: { x: 0, y: 0, zoom: 1 }, windows } }],
    active_tab_id: project.id, recent_projects: [] } });
  function send(socket: WebSocket, messages: unknown[]) {
    frames += messages.length;
    try { for (const message of messages) socket.send(JSON.stringify(message)); }
    catch (error) { errors.push(String(error)); }
  }
  function snapshots(socket: WebSocket) {
    send(socket, [workspace(), ...paneIds.flatMap(id => [
      { kind: "terminal_snapshot", id, data_base64: Buffer.from(`\x1b[2J\x1b[H${snapshotText(revision, id)}\r\n`).toString("base64") },
      { kind: "terminal_status", id, status: "running", detail: null },
    ])]);
  }
  function burst() {
    const messages = Array.from({ length: 4096 }, (_, index) => ({
      kind: index % 2 ? "terminal_output" : "terminal_preview", id: paneIds[index % 8],
      ...(index % 2 ? { data_base64: Buffer.from(`\rBURST_${ticks}_${index} ${"x".repeat(128)}`).toString("base64") }
        : { text: `BURST_${ticks}_${index} ${"x".repeat(128)}` }),
    }));
    for (const [socket, scoped] of clients) if (scoped) send(socket, messages);
  }
  server.on("connection", (socket: WebSocket, request: IncomingMessage) => {
    const scoped = new URL(request.url!, "http://127.0.0.1").searchParams.has("repo_hash");
    clients.set(socket, scoped);
    socket.on("message", raw => {
      try {
        const message = JSON.parse(String(raw));
        if (message.kind === "frontend_ready") {
          if (scoped) { readyRequests++; snapshots(socket); }
          else send(socket, [{ kind: "hub_state", hub: { app_version: "playwright", projects: [project], recent_projects: [] } }]);
        } else if (message.kind === "terminal_input") {
          send(socket, [{ kind: "terminal_output", id: message.id, data_base64: Buffer.from(message.data).toString("base64") }]);
        }
      } catch (error) { errors.push(String(error)); }
    });
    socket.on("close", () => clients.delete(socket));
    socket.on("error", error => errors.push(String(error)));
  });
  return {
    errors,
    counters: () => ({ ticks, frames, ready_requests: readyRequests }),
    revision: () => revision,
    freezeSnapshot: () => { revision = ticks + 1; },
    start() {
      timer = setInterval(() => {
        ticks++;
        revision = ticks;
        const messages = paneIds.flatMap(id => [
          { kind: "terminal_output", id, data_base64: Buffer.from(`\rLIVE_${ticks} ${"x".repeat(128)}`).toString("base64") },
          { kind: "terminal_preview", id, text: `LIVE_${ticks}` },
        ]);
        for (const [socket, scoped] of clients) if (scoped) send(socket, ticks % 20 ? messages : [...messages, workspace()]);
        if (ticks === 1 || ticks % 600 === 0) burst();
      }, 50);
    },
    burst,
    stop() { clearInterval(timer); },
    async close() {
      clearInterval(timer);
      for (const socket of clients.keys()) socket.terminate();
      await new Promise<void>(resolve => server.close(() => resolve()));
      clients.clear();
    },
  };
}
