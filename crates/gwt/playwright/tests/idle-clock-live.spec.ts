import { execFile } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { promisify } from "node:util";
import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const runFile = promisify(execFile);
const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
const repo = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT;
const phase = process.env.GWT_5504_PHASE ?? "after";
const paneCount = 8;
const settleMs = 5_000;
const measurementMs = 10_000;
test.use({ viewport: { width: 1440, height: 900 }, headless: false, reducedMotion: "no-preference" });

type ProcessCpu = { pid: number; process_name: string; cpu_seconds: number; captured_at: string };
type RendererCpu = { id: number; type: string; cpuTime: number };

test.describe("Issue #5504 idle panes and status clock", () => {
  test.skip(!base || !repo || !process.env.GWT_5504_GWT_PID,
    "requires scripts/verify-issue-5504.ps1 and its owned fresh-HOME process");
  test.setTimeout(180_000);

  test("eight idle Shells preserve clock behavior and record CPU", async ({ page, browser }, info) => {
    await withLiveGwtBackendLock(base!, info, async () => {
      const errors: string[] = [];
      const created: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await installClockCounter(page);
      await gotoLiveGwt(page, base!, { enableTestBridge: true });
      await page.bringToFront();
      const theme = info.project.name.includes("light") ? "light" : "dark";
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(page.locator("#op-strip-clock")).toHaveText(/^\d{2}:\d{2}:\d{2}$/);
      const cdp = await browser.newBrowserCDPSession();
      try {
        for (let index = 0; index < paneCount; index += 1) {
          created.push(await createIdleShell(page, index));
        }
        await expect(page.locator('.workspace-window[data-preset="shell"]')).toHaveCount(paneCount);
        expect(page.context().pages(), "one frontend page for renderer CPU attribution").toHaveLength(1);
        // No commands or polling are introduced into the observation interval.
        await new Promise(resolve => setTimeout(resolve, settleMs));
        const gwtBefore = await sampleGwtCpu();
        const renderersBefore = (await cdp.send("SystemInfo.getProcessInfo")).processInfo
          .filter((process: RendererCpu) => process.type === "renderer") as RendererCpu[];
        expect(renderersBefore.length).toBeGreaterThan(0);
        const clockBefore = await clockCounters(page);
        const started = performance.now();
        await new Promise(resolve => setTimeout(resolve, measurementMs));
        const elapsedMs = performance.now() - started;
        const clockAfter = await clockCounters(page);
        const renderersAfter = (await cdp.send("SystemInfo.getProcessInfo")).processInfo
          .filter((process: RendererCpu) => process.type === "renderer") as RendererCpu[];
        const gwtAfter = await sampleGwtCpu();
        expect(renderersAfter.map(process => process.id).sort())
          .toEqual(renderersBefore.map(process => process.id).sort());
        const rendererCpuSeconds = renderersAfter.reduce((sum, process) => sum + process.cpuTime, 0)
          - renderersBefore.reduce((sum, process) => sum + process.cpuTime, 0);
        const gwtCpuSeconds = gwtAfter.cpu_seconds - gwtBefore.cpu_seconds;
        const gwtElapsedMs = Date.parse(gwtAfter.captured_at) - Date.parse(gwtBefore.captured_at);
        const callbacks = {
          raf: clockAfter.raf - clockBefore.raf,
          timeout: clockAfter.timeout - clockBefore.timeout,
          text_changes: clockAfter.textChanges - clockBefore.textChanges,
        };
        const evidence = {
          issue: 5504, phase, theme, measured_at: new Date().toISOString(),
          source_sha: process.env.GWT_5504_SOURCE_SHA,
          gwt_binary_sha256: process.env.GWT_5504_BINARY_SHA256,
          operator_shell_source_sha256: process.env.GWT_5504_CLOCK_SHA256,
          conditions: { pane_count: paneCount, pane_preset: "shell", workload: "idle shell prompts",
            settle_ms: settleMs, requested_measurement_ms: measurementMs,
            viewport: page.viewportSize(), reduced_motion: "no-preference", build_profile: "debug", fresh_home: true,
            pm_auto_start: false, issue_monitor_enabled: false, automatic_background_index_enabled: false,
            frontend_pages: 1,
            cpu_percent_basis: "one logical core; CPU seconds / elapsed seconds * 100",
            renderer_scope: "sum of renderer processes in this owned Chromium browser",
            renderer_limitation: "Current checkout uses browser UI, not a native WebView. Chromium runs the same embedded frontend; values are not measurements of a native WebView process.",
            timing_limitation: "gwt snapshots bracket the renderer interval plus the process-sampler startup; each CPU percentage uses its own measured elapsed time.",
            instrumentation: "same callback-name counters and DOM MutationObserver for baseline and after; no stack capture" },
          gwt: { before: gwtBefore, after: gwtAfter, elapsed_ms: gwtElapsedMs,
            cpu_seconds: gwtCpuSeconds, cpu_percent_one_core: gwtCpuSeconds / gwtElapsedMs * 100_000 },
          renderer: { before: renderersBefore, after: renderersAfter, elapsed_ms: elapsedMs,
            cpu_seconds: rendererCpuSeconds, cpu_percent_one_core: rendererCpuSeconds / elapsedMs * 100_000 },
          clock: { before_text: clockBefore.text, after_text: clockAfter.text, callbacks }, errors,
        };
        const directory = process.env.GWT_5504_EVIDENCE_DIR ?? info.outputPath("cpu-evidence");
        await mkdir(directory, { recursive: true });
        const artifact = join(directory, `${phase}-${theme}.json`);
        await writeFile(artifact, JSON.stringify(evidence, null, 2) + "\n");
        await info.attach(`${phase}-${theme}-idle-cpu`, { path: artifact, contentType: "application/json" });
        console.log(`${phase}/${theme}: gwt=${gwtCpuSeconds.toFixed(6)} CPU s, renderer=${rendererCpuSeconds.toFixed(6)} CPU s, clock=${JSON.stringify(callbacks)}`);
        expect(callbacks.text_changes, "clock seconds continue to update").toBeGreaterThanOrEqual(5);
        expect(clockAfter.text).toMatch(/^\d{2}:\d{2}:\d{2}$/);
        if (phase !== "baseline") {
          expect(callbacks.raf, "status clock does not keep a RAF loop alive").toBe(0);
          expect(callbacks.timeout, "clock wakes about once per second").toBeGreaterThanOrEqual(5);
          expect(callbacks.timeout).toBeLessThanOrEqual(15);
          await assertClockVisibilityLifecycle(page);
        }
        expect(errors, "console and page errors").toEqual([]);
        const screenshot = info.outputPath(`${theme}-eight-idle-shells.png`);
        await page.screenshot({ path: screenshot });
        await info.attach(`${theme}-eight-idle-shells`, { path: screenshot, contentType: "image/png" });
      } finally {
        await cdp.detach();
        for (const id of created.reverse()) {
          await sendLiveGwtEvent(page, { kind: "close_window", id });
          await expect(page.locator(`.workspace-window[data-id="${id}"]`)).toHaveCount(0);
        }
      }
    });
  });
});

async function sampleGwtCpu(): Promise<ProcessCpu> {
  const { stdout } = await runFile("pwsh", ["-NoProfile", "-File",
    join(repo!, "scripts/diagnostics/issue-5504-process-cpu.ps1"),
    "-ProcessId", process.env.GWT_5504_GWT_PID!], { windowsHide: true });
  return JSON.parse(stdout);
}

async function createIdleShell(page: Page, index: number): Promise<string> {
  const seen = await page.locator(".workspace-window").evaluateAll(nodes =>
    nodes.map(node => (node as HTMLElement).dataset.id));
  await sendLiveGwtEvent(page, { kind: "create_window", preset: "shell",
    bounds: { x: 32 + (index % 4) * 300, y: 80 + Math.floor(index / 4) * 330, width: 500, height: 300 } });
  const id = await page.waitForFunction(seen => [...document.querySelectorAll<HTMLElement>(
    '.workspace-window[data-preset="shell"]')].find(node => !seen.includes(node.dataset.id))?.dataset.id,
  seen, { timeout: 30_000 }).then(handle => handle.jsonValue());
  await expect.poll(() => page.evaluate(id =>
    (window as any).__gwtTerminalTestApi.bufferText(id).trim(), id), { timeout: 30_000 })
    .toMatch(/[^\n]+[>$#%]$/);
  return id;
}

async function installClockCounter(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const counts = { raf: 0, timeout: 0, textChanges: 0 };
    (window as any).__gwt5504ClockCounts = counts;
    const raf = window.requestAnimationFrame.bind(window);
    const timeout = window.setTimeout.bind(window);
    window.requestAnimationFrame = callback => {
      // operator-shell.js owns the only RAF callback named loop in the
      // embedded frontend. Match the existing callback without stack capture.
      const clock = callback.name === "loop";
      return raf(timestamp => { if (clock) counts.raf += 1; callback(timestamp); });
    };
    window.setTimeout = ((callback, delay, ...args) => {
      // The other function named tick (ui-trace-profiler.js) uses RAF, not
      // setTimeout. The clock is the frontend's only timeout tick callback.
      const clock = typeof callback === "function" && callback.name === "tick";
      return timeout(typeof callback === "function" ? (...values) => {
        if (clock) counts.timeout += 1;
        callback(...values);
      } : callback, delay, ...args);
    }) as typeof window.setTimeout;
    document.addEventListener("DOMContentLoaded", () => {
      const clock = document.getElementById("op-strip-clock");
      if (!clock) return;
      let last = clock.textContent;
      new MutationObserver(() => {
        if (clock.textContent !== last) { counts.textChanges += 1; last = clock.textContent; }
      }).observe(clock, { subtree: true, childList: true, characterData: true });
    });
  });
}

async function clockCounters(page: Page) {
  return page.evaluate(() => ({ ...(window as any).__gwt5504ClockCounts,
    text: document.getElementById("op-strip-clock")?.textContent }));
}

async function assertClockVisibilityLifecycle(page: Page): Promise<void> {
  // Dispatch the browser visibility contract against the actual clock wiring.
  // A second page would change renderer CPU attribution; the override is used
  // only after measurement and is always removed before the test finishes.
  const before = await page.evaluate(() => {
    Object.defineProperty(document, "hidden", { configurable: true, value: true });
    document.dispatchEvent(new Event("visibilitychange"));
    return { ...(window as any).__gwt5504ClockCounts,
      text: document.getElementById("op-strip-clock")?.textContent };
  });
  try {
    await new Promise(resolve => setTimeout(resolve, 1_500));
    const hidden = await clockCounters(page);
    expect(hidden.timeout, "no hidden clock timeout callbacks").toBe(before.timeout);
    expect(hidden.text, "hidden clock DOM stays untouched").toBe(before.text);
  } finally {
    await page.evaluate(() => {
      delete (document as any).hidden;
      document.dispatchEvent(new Event("visibilitychange"));
    });
  }
  const resumed = await clockCounters(page);
  await expect.poll(async () => (await clockCounters(page)).text, { timeout: 5_000 }).not.toBe(resumed.text);
  await expect(page.locator("#op-strip-clock .op-strip-clock__colon")).toHaveCount(2);
}
