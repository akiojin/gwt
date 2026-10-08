/** #5015: exercise real LaunchComplete deliveries while the canvas has 60 windows. */
import { readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import {
  acquireLiveGwtBackendLock,
  clearLiveLaunchWizard,
  gotoLiveGwt,
  openLiveGwtProject,
  sendLiveGwtEvent,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const PROJECT = process.env.GWT_PLAYWRIGHT_LAUNCH_PROJECT ?? "";
const PROBES = process.env.GWT_PLAYWRIGHT_LAUNCH_PROBES ?? "";
const AGENT = "launch-loop-probe";

test.describe("launch-loop responsiveness (live backend)", () => {
  test.skip(!BASE, "requires the isolated browser-check instance");
  // Twenty real Git materializations and launches can exceed six minutes on a
  // busy Windows host. Keep the operation/latency bounds below unchanged.
  test.setTimeout(600_000);
  test.use({ actionTimeout: 30_000, navigationTimeout: 30_000 });

  test("pane observations and PM requests survive repeated launches with 60 windows", async ({ page }, info) => {
    expect(PROJECT, "isolated fixture repository").not.toBe("");
    expect(PROBES, "fixture agent evidence directory").not.toBe("");
    // Separate fixture processes keep Work and Console history out of the
    // next theme's trace without changing the measured launch workload.
    const project = info.project.name.includes("light")
      ? process.env.GWT_PLAYWRIGHT_LAUNCH_LIGHT_PROJECT || PROJECT : PROJECT;
    const base = info.project.name.includes("light")
      ? process.env.GWT_PLAYWRIGHT_LAUNCH_LIGHT_BASE_URL || BASE : BASE;
    const probes = info.project.name.includes("light")
      ? process.env.GWT_PLAYWRIGHT_LAUNCH_LIGHT_PROBES || PROBES : PROBES;
    const release = await acquireLiveGwtBackendLock(base, info);
    const errors: string[] = [];
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("pageerror", error => errors.push(String(error)));
    const samples: number[] = [];
    const launched: string[] = [];
    const startedAt = new Date().toISOString();
    try {
      await rm(join(probes, "leader.json"), { force: true });
      await rm(join(probes, "pane-samples.json"), { force: true });
      await gotoLiveGwt(page, base, { enableTestBridge: true });
      await openLiveGwtProject(page, project);
      const migration = page.locator("#migration-modal.open");
      if (project !== PROJECT) await expect(migration).toBeVisible({ timeout: 60_000 });
      if (await migration.count()) {
        // Exercise the regular migration route only on this disposable repository.
        await migration.getByRole("button", { name: "Migrate", exact: true }).click();
        await expect(migration).toHaveCount(0, { timeout: 60_000 });
      }
      await clearLiveLaunchWizard(page);
      await closeWindows(page);
      await page.evaluate(theme => document.documentElement.dataset.theme = theme,
        info.project.name.includes("light") ? "light" : "dark");
      const initial = await ids(page);
      const windows: unknown[] = [];
      for (let index = initial.length; index < 60; index += 1) {
        windows.push({
          kind: "create_window", preset: "console",
          bounds: { x: index * 8, y: index * 8, width: 480, height: 280 },
        });
      }
      await sendBurst(page, windows);
      await expect(page.locator(".workspace-window")).toHaveCount(60, { timeout: 60_000 });
      const canvas = await ids(page);
      const workBefore = await ids(page);
      await sendLiveGwtEvent(page, {
        kind: "create_window", preset: "work",
        bounds: { x: 32, y: 32, width: 880, height: 520 },
      });
      const workId = await newWindow(page, workBefore);
      // The probe agent itself runs the authenticated gwtd pane.list operation
      // 20 times at 3.5 second intervals using its own injected capability.
      // No credential or production pane endpoint is copied by this test.
      for (let iteration = 0; iteration < 20; iteration += 1) {
        launched.push(await launch(page, workId, `feature/launch-loop-${info.project.name}-${iteration}`));
        await sendBurst(page, [{
          kind: "arrange_windows", mode: "tile",
          bounds: { x: 0, y: 0, width: 1440, height: 900 },
        }, ...canvas.map(id => ({
          kind: "update_window_geometry", id,
          geometry: { x: iteration, y: iteration, width: 480, height: 280 },
          cols: 80, rows: 24,
        }))]);
        samples.push(await pmRoundtrip(page, canvas[0]));
        // Keep launching across the full pane.list observation interval.
        await page.waitForTimeout(3_500);
      }
      await expect(async () => {
        const record = JSON.parse(await readFile(join(probes, "pane-samples.json"), "utf8"));
        expect(record.samples).toHaveLength(20);
        expect(record.failures).toEqual([]);
      }).toPass({ timeout: 90_000 });
      const sorted = [...samples].sort((a, b) => a - b);
      expect(sorted[Math.floor(sorted.length / 2)], "PM receive-to-reply p50 bounds queue wait").toBeLessThanOrEqual(200);
      expect(Math.max(...samples), "PM receive-to-reply maximum bounds queue wait").toBeLessThanOrEqual(2_000);
      expect(launched).toHaveLength(20);
      expect(errors, "console and page errors").toEqual([]);
      await info.attach("launch-loop-measurements", {
        body: Buffer.from(JSON.stringify({ windows: (await ids(page)).length, pmRoundtripMs: samples,
          pane: JSON.parse(await readFile(join(probes, "pane-samples.json"), "utf8")) })),
        contentType: "application/json",
      });
      await info.attach("canvas", { body: await page.screenshot(), contentType: "image/png" });
    } finally {
      await page.screenshot({ path: join(probes, `canvas-${info.project.name}.png`) }).catch(() => undefined);
      await writeFile(join(probes, `measurements-${info.project.name}.json`), JSON.stringify({
        theme: info.project.name, startedAt, endedAt: new Date().toISOString(),
        windows: (await ids(page).catch(() => [])).length, launchedCount: launched.length,
        pmRoundtripMs: samples, errors,
        pane: await readFile(join(probes, "pane-samples.json"), "utf8").then(JSON.parse).catch(() => null),
      }));
      await closeWindows(page).catch(() => undefined);
      await release();
    }
  });
});

async function ids(page: Page): Promise<string[]> {
  return page.locator(".workspace-window").evaluateAll(nodes =>
    nodes.map(node => (node as HTMLElement).dataset.id || ""));
}

// Keep every ordered WebSocket event, while tracing one browser evaluation
// rather than before/after DOM snapshots for each message in the burst.
async function sendBurst(page: Page, payloads: unknown[]): Promise<void> {
  await page.evaluate(events => {
    for (const detail of events) window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail }));
  }, payloads);
}

async function closeWindows(page: Page): Promise<void> {
  await sendBurst(page, (await ids(page)).map(id => ({ kind: "close_window", id })));
  await expect(page.locator(".workspace-window")).toHaveCount(0, { timeout: 30_000 });
}

async function newWindow(page: Page, before: string[]): Promise<string> {
  return page.waitForFunction(seen => {
    const node = [...document.querySelectorAll<HTMLElement>(".workspace-window")]
      .find(window => !seen.includes(window.dataset.id || ""));
    return node?.dataset.id;
  }, before, { timeout: 60_000 }).then(handle => handle.jsonValue());
}

async function launch(page: Page, workId: string, branch: string): Promise<string> {
  const { before } = await wizardRequest(page,
    { kind: "open_launch_wizard", id: workId, branch_name: "main" }, true);
  const wizard = page.locator("#wizard-modal");
  await expect(wizard).toBeVisible({ timeout: 60_000 });
  await wizardAction(page, { kind: "use_start_method", method: "configure_and_start" });
  await wizardAction(page, { kind: "set_branch_mode", create_new: true });
  await wizardAction(page, { kind: "set_branch_name", value: branch });
  let state = await wizardAction(page, { kind: "set_agent", agent_id: AGENT });
  for (let step = 0; step < 12 && state.wizard; step += 1) {
    expect(state.wizard.error).toBeFalsy();
    expect(state.wizard.primary_action_enabled, state.wizard.primary_action_disabled_reason).toBe(true);
    state = await wizardAction(page, { kind: "submit" });
  }
  expect(state.wizard).toBeNull();
  const id = await newWindow(page, before);
  await expect(async () => {
    const text = await page.evaluate(id => String((window as any).__gwtTerminalTestApi?.bufferText?.(id) ?? ""), id);
    expect(text).toContain("LAUNCH_LOOP_PROBE_READY");
  }).toPass({ timeout: 90_000 });
  return id;
}

async function wizardRequest(page: Page, detail: unknown, requireOpen = false): Promise<any> {
  return page.evaluate(({ detail, requireOpen }) => {
    const before = [...document.querySelectorAll<HTMLElement>(".workspace-window")]
      .map(node => node.dataset.id || "");
    const after = (window as any).__gwtPlaywrightMessageSequence || 0;
    const socket = [...((window as any).__gwtPlaywrightSockets as WebSocket[])]
      .reverse().find(socket => socket.readyState === WebSocket.OPEN
        && new URL(socket.url).pathname === "/ws" && new URL(socket.url).searchParams.has("repo_hash"));
    if (!socket) throw new Error("no project socket");
    // Capture the reply before sending: Console traffic can evict it from the
    // test bridge's 256-message history while Playwright collects DOM snapshots.
    return new Promise<{ before: string[]; wizard: unknown }>((resolve, reject) => {
      const cleanup = () => { clearTimeout(timer); socket.removeEventListener("message", reply); };
      const reply = (event: MessageEvent) => {
        let payload;
        try { payload = JSON.parse(String(event.data)); } catch { return; }
        const sequence = (window as any).__gwtPlaywrightMessageSequence || 0;
        if (sequence <= after || payload?.kind !== "launch_wizard_state"
          || (requireOpen && !payload.wizard)
          || (payload.wizard && (payload.wizard.is_hydrating
            || payload.wizard.runtime_resolution_pending
            || payload.wizard.launch_materialization_pending))) return;
        cleanup();
        resolve({ before, wizard: payload.wizard });
      };
      const timer = setTimeout(() => { cleanup(); reject(new Error("Wizard reply timed out")); }, 30_000);
      socket.addEventListener("message", reply);
      window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail }));
    });
  }, { detail, requireOpen });
}

async function wizardAction(page: Page, action: unknown): Promise<any> {
  return wizardRequest(page, { kind: "launch_wizard_action", action,
    bounds: { x: 32, y: 32, width: 880, height: 520 } });
}

async function pmRoundtrip(page: Page, id: string): Promise<number> {
  return page.evaluate(id => new Promise<number>((resolve, reject) => {
    const socket = [...((window as any).__gwtPlaywrightSockets as WebSocket[])]
      .reverse().find(socket => socket.readyState === WebSocket.OPEN
        && new URL(socket.url).pathname === "/ws" && new URL(socket.url).searchParams.has("repo_hash"));
    if (!socket) { reject(new Error("no project socket")); return; }
    const started = performance.now();
    const timeout = setTimeout(() => { socket.removeEventListener("message", reply); reject(new Error("PM reply timed out")); }, 15_000);
    const reply = (message: MessageEvent) => {
      const value = JSON.parse(String(message.data));
      if (value.kind !== "pm_conversation" || value.id !== id) return;
      clearTimeout(timeout);
      socket.removeEventListener("message", reply);
      resolve(performance.now() - started);
    };
    socket.addEventListener("message", reply);
    socket.send(JSON.stringify({ kind: "load_pm_conversation", id }));
  }), id);
}
