// SPEC #3820 T-006a: provisional latency samples for inventory paths
// P07 (prompt submit -> next input reflected), P17 (PTY output arrival ->
// presented frame) and P18 (wheel input -> scrolled frame).
// This is a measurement driver, not a regression gate. It runs only against an
// isolated browser-check instance and writes raw samples to
// GWT_PERF_INVENTORY_OUT. See docs/perf/issue-3820/README.md.
import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { gotoLiveGwt, readLiveHubCatalog, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const SAMPLES = Number(process.env.GWT_PERF_INVENTORY_SAMPLES ?? "20");
// Hidden or occluded headed windows stop animation frames on macOS.
test.use({ launchOptions: { args: ["--window-position=-32000,-32000", "--disable-backgrounding-occluded-windows",
  "--disable-renderer-backgrounding", "--disable-background-timer-throttling"] } });

test.describe("SPEC #3820 terminal latency inventory", () => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME;
  const out = process.env.GWT_PERF_INVENTORY_OUT;
  test.skip(!base || !home || !out, "requires an isolated browser-check instance and GWT_PERF_INVENTORY_OUT");
  test.setTimeout(600_000);
  test.use({ viewport: { width: 1440, height: 900 } });

  test("P07 / P17 / P18 samples on a real Shell PTY", async ({ page }, info) => {
    await withLiveGwtBackendLock(base!, info, async () => {
      const theme = info.project.name.endsWith("light") ? "light" : "dark";
      const fixture = await realpath(await mkdtemp(join(home!, "perf-3820-")));
      const source = join(fixture, "source");
      const root = join(fixture, "project");
      const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith("GWT_")));
      Object.assign(env, { HOME: home!, USERPROFILE: home!, GIT_CONFIG_NOSYSTEM: "1" });
      execFileSync("git", ["init", "-q", source], { env });
      execFileSync("git", ["-C", source, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
        "commit", "--allow-empty", "-qm", "Perf fixture"], { env });
      execFileSync("git", ["-C", source, "worktree", "add", "--detach", "-q", root], { env });
      const gwtd = join(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT!, "target", "debug", "gwtd");
      const status = JSON.parse(execFileSync(gwtd, { cwd: root, env, encoding: "utf8", input: JSON.stringify({
        schema_version: 1, operation: "issue.monitor.status", params: {},
      }) + "\n" }));
      const key: string = status.project_store?.hash;
      expect(key).toMatch(/^[a-f0-9]{16}$/);
      const projectState = join(home!, ".gwt", "projects", key, "project-state");
      await mkdir(projectState, { recursive: true });
      await writeFile(join(projectState, "pm.json"), '{"settings":{"auto_start":false}}');
      await writeFile(join(projectState, "issue-monitor.json"), '{"enabled":false,"max_active_agents":1,"priority_order":[]}');

      const errors: string[] = [];
      const deliveryErrors: string[] = [];
      const created: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      page.on("websocket", socket => socket.on("framereceived", ({ payload }) => {
        const event = JSON.parse(String(payload));
        if (event.kind === "window_command_error" && created.includes(event.id)) deliveryErrors.push(event.message);
      }));
      await readLiveHubCatalog(page, base!, {
        send: { kind: "reopen_recent_project", path: root },
        until: (catalog, expected) => catalog.projects.some(project => project.project_key === expected), arg: key,
      });
      try {
        await page.bringToFront();
        await gotoLiveGwt(page, base!, { projectKey: key, enableTestBridge: true });
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        const readiness = await page.waitForFunction(() => (window as any).__gwtPlaywrightMessages
          .find(({ payload }: any) => payload.kind === "migration_detected" ||
            (payload.kind === "project_owner_health" && payload.health.state === "healthy"))?.payload.kind,
        undefined, { timeout: 30_000 }).then(value => value.jsonValue()).catch(() => "owner readiness timed out");
        expect(readiness, "GUI presence (Healthy project owner)").toBe("project_owner_health");
        const id = await createShell(page, created);
        // Let the 15 s owner attach grace pass so samples hit the resident owner.
        await new Promise(resolve => setTimeout(resolve, 16_000));
        await page.locator(`.workspace-window[data-id="${id}"] .xterm`).click();
        await installProbe(page, id);

        const p17 = await measureP17(page, id);
        const p07 = await measureP07(page, id);
        const p18 = await measureP18(page, id);
        expect(deliveryErrors, "Shell input delivery errors").toEqual([]);
        expect(errors, "console and page errors").toEqual([]);
        const record = {
          theme, samples_per_path: SAMPLES, viewport: page.viewportSize(),
          checkout: process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT, recorded_at: new Date().toISOString(),
          P07: p07, P17: p17, P18: p18,
          pane_message_kinds: await page.evaluate(() => (window as any).__perf3820.kinds), console_errors: errors, delivery_errors: deliveryErrors,
        };
        await mkdir(out!, { recursive: true });
        await writeFile(join(out!, `terminal-${theme}.json`), JSON.stringify(record, null, 2) + "\n");
        await info.attach(`terminal-latency-${theme}`, { body: JSON.stringify(record), contentType: "application/json" });
      } finally {
        for (const id of created) await sendLiveGwtEvent(page, { kind: "close_window", id }).catch(() => {});
        await readLiveHubCatalog(page, base!, {
          send: { kind: "preview_close_project", project_key: key, project_wide: true },
        }).catch(() => {});
      }
    });
  });
});

// Page-side probe. Start timestamps are DOM event timeStamps (same clock as
// performance.now()); the end is the first animation frame whose callback
// observes the condition, i.e. the frame that presents it.
async function installProbe(page: Page, id: string) {
  await page.evaluate(id => {
    const api = (window as any).__gwtTerminalTestApi;
    const probe: any = { lastKeydown: 0, lastWheel: 0, kinds: {}, stream: "", watch: null };
    document.addEventListener("keydown", event => { if (event.key === "Enter") probe.lastKeydown = event.timeStamp; }, true);
    document.addEventListener("wheel", event => { probe.lastWheel = event.timeStamp; }, { capture: true, passive: true });
    for (const socket of (window as any).__gwtPlaywrightSockets as WebSocket[]) {
      socket.addEventListener("message", (event: MessageEvent) => {
        try {
          const payload = JSON.parse(String(event.data));
          if (payload.id === id) probe.kinds[payload.kind] = (probe.kinds[payload.kind] ?? 0) + 1;
          // Output reaches the pane either as stream chunks (a marker may span
          // chunks and escape sequences) or as a full-screen snapshot.
          if ((payload.kind === "terminal_output" || payload.kind === "terminal_snapshot") && payload.id === id) {
            const text = atob(payload.data_base64).replace(/\x1b\[[0-9;?]*[ -\/]*[@-~]|\x1b[()][0-9A-Za-z]|\x1b[=>]/g, "");
            if (payload.kind === "terminal_output") probe.stream = (probe.stream + text).slice(-4096);
            const seen = payload.kind === "terminal_output" ? probe.stream : text;
            if (probe.watch && !probe.watch.arrival && seen.includes(probe.watch.marker)) {
              probe.watch.arrival = { t: event.timeStamp, kind: payload.kind };
            }
          }
        } catch { /* non-JSON frames are not terminal output */ }
      });
    }
    // Resolve with the timestamp of the first frame where predicate() holds.
    probe.untilFrame = (predicate: () => boolean, timeoutMs: number) => new Promise<number>((resolve, reject) => {
      const started = performance.now();
      const tick = (now: number) => {
        if (predicate()) return resolve(now);
        if (performance.now() - started > timeoutMs) return reject(new Error("frame condition timed out"));
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
    probe.text = () => api.bufferText(id, 60) as string;
    probe.metrics = () => api.metrics(id);
    (window as any).__perf3820 = probe;
  }, id);
}

function summarize(values: number[]) {
  const sorted = [...values].sort((a, b) => a - b);
  const rank = (q: number) => sorted[Math.max(0, Math.ceil(q * sorted.length) - 1)];
  const round = (value: number) => Math.round(value * 1000) / 1000;
  return { n: sorted.length, p50_ms: round(rank(0.5)), p95_ms: round(rank(0.95)), worst_ms: round(sorted.at(-1)!),
    samples_ms: values.map(round) };
}

async function measureP17(page: Page, id: string) {
  const values: number[] = [];
  const carriers: Record<string, number> = {};
  for (let index = 0; index < SAMPLES; index += 1) {
    // The echo shows "$((40+2))"; only the PTY output contains the marker.
    const marker = `P17M${index}X42`;
    const pending = page.evaluate(({ marker }) => {
      const probe = (window as any).__perf3820;
      probe.watch = { marker, arrival: null };
      return probe.untilFrame(() => probe.text().includes(marker), 10_000).then((frame: number) => {
        const arrival = probe.watch.arrival;
        if (!arrival) throw new Error(`no output frame carried ${marker}: ${JSON.stringify(probe.kinds)}`);
        return { ms: frame - arrival.t, kind: arrival.kind };
      });
    }, { marker });
    await sendLiveGwtEvent(page, { kind: "terminal_input", id, data: `echo P17M${index}X$((40+2))\r` });
    const { ms, kind } = await pending;
    values.push(ms);
    carriers[kind] = (carriers[kind] ?? 0) + 1;
    await page.waitForTimeout(150);
  }
  return { ...summarize(values), carriers };
}

async function measureP07(page: Page, id: string) {
  const values: number[] = [];
  for (let index = 0; index < SAMPLES; index += 1) {
    const command = `: P07_${index}`;
    await page.keyboard.type(command);
    const pending = page.evaluate(command => {
      const probe = (window as any).__perf3820;
      // The next input is reflected once the line after the submitted command ends with "Q".
      return probe.untilFrame(() => {
        const lines = probe.text().trimEnd().split("\n");
        return /Q$/.test(lines.at(-1)) && lines.at(-2)?.trimEnd().endsWith(command);
      }, 10_000).then((frame: number) => frame - probe.lastKeydown);
    }, command);
    await page.keyboard.press("Enter");
    await page.keyboard.type("Q");
    values.push(await pending);
    await page.keyboard.press("Backspace");
    await page.waitForTimeout(150);
  }
  return summarize(values);
}

async function measureP18(page: Page, id: string) {
  await sendLiveGwtEvent(page, { kind: "terminal_input", id, data: "seq 1 3000\r" });
  await expect.poll(() => page.evaluate(() => (window as any).__perf3820.metrics().baseY), { timeout: 15_000 })
    .toBeGreaterThan(1000);
  const box = (await page.locator(`.workspace-window[data-id="${id}"] .xterm-screen`).boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  const values: number[] = [];
  for (let index = 0; index < SAMPLES; index += 1) {
    const pending = page.evaluate(() => {
      const probe = (window as any).__perf3820;
      const before = probe.metrics().viewportY;
      probe.lastWheel = 0;
      return probe.untilFrame(() => probe.lastWheel > 0 && probe.metrics().viewportY !== before, 10_000)
        .then((frame: number) => frame - probe.lastWheel);
    });
    await page.mouse.wheel(0, index % 2 === 0 ? -400 : 400);
    values.push(await pending);
    await page.waitForTimeout(150);
  }
  return summarize(values);
}

async function projectWindows(page: Page): Promise<Array<{ id: string; preset: string; z_index?: number }>> {
  return page.evaluate(() => {
    const workspace = (window as any).__gwtPlaywrightMessages.findLast((entry: any) => entry.payload.kind === "workspace_state").payload.workspace;
    return workspace.tabs.flatMap((tab: any) => tab.workspace.windows);
  });
}

async function createShell(page: Page, created: string[]): Promise<string> {
  const before = new Set((await projectWindows(page)).map(window => window.id));
  await sendLiveGwtEvent(page, { kind: "create_window", preset: "shell" });
  await expect.poll(async () => (await projectWindows(page)).filter(window => window.preset === "shell" && !before.has(window.id)))
    .toHaveLength(1);
  const id = (await projectWindows(page)).find(window => window.preset === "shell" && !before.has(window.id))!.id;
  created.push(id);
  await sendLiveGwtEvent(page, { kind: "focus_window", id });
  // Reload only after the Shell is the top window; otherwise Settings may stay in front.
  await expect.poll(async () => (await projectWindows(page)).reduce((top, window) =>
    !top || (window.z_index || 0) > (top.z_index || 0) ? window : top, null as { id: string; z_index?: number } | null)?.id).toBe(id);
  await page.reload();
  await expect(page.locator(`.split-pane .workspace-window[data-id="${id}"]`)).toBeVisible();
  await expect.poll(() => page.evaluate(id =>
    (window as any).__gwtTerminalTestApi.bufferText(id).trim(), id), { timeout: 30_000 }).toMatch(/[^\n]+[>$#%]$/);
  return id;
}

