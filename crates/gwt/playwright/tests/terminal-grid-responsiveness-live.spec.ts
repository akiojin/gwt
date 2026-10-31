import { expect, test, type Page } from "@playwright/test";
import { readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

test.describe("terminal-grid responsiveness (#5117)", () => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME;
  test.skip(!base || !home, "requires an isolated browser-check checkout and fresh HOME");
  test.setTimeout(180_000);
  test.use({ viewport: { width: 1440, height: 900 } });

  test("output advances without animation frames in an unfocused terminal window", async ({ page }, info) => {
    await withLiveGwtBackendLock(base!, info, async () => {
      const errors: string[] = [];
      const created: string[] = [];
      const theme = info.project.name.includes("light") ? "light" : "dark";
      let observation: Record<string, any> | undefined;
      let coldObservation: Record<string, any> | undefined;
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await gotoLiveGwt(page, base!, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      try {
        const id = await createShell(page, created);
        const target = page.locator(`.workspace-window[data-id="${id}"]`);
        const focusedControl = page.locator("#split-view-button");
        await focusedControl.focus();
        await expect(focusedControl).toBeFocused();
        await expect(target).toBeVisible();
        const token = `GWT_AC13_${Date.now()}`;
        const chunks = [`\r\n${token}_FI`, `RST\r\n${token}_SECOND\r\n`, `${token}_THIRD\r\n`];
        await page.evaluate(({ id, chunks }) => {
          const socket = [...((window as any).__gwtPlaywrightSockets as WebSocket[])].reverse()
            .find(candidate => candidate.readyState === WebSocket.OPEN
              && new URL(candidate.url).pathname === "/ws"
              && new URL(candidate.url).searchParams.has("repo_hash"));
          if (!socket) throw new Error("no live project socket");
          const target = document.querySelector(`.workspace-window[data-id="${id}"]`)!;
          const originalFrame = window.requestAnimationFrame;
          const originalCancel = window.cancelAnimationFrame;
          const captured = new Map<number, FrameRequestCallback>();
          let requestedFrames = 0;
          let focusEvents = 0;
          const onFocus = () => { focusEvents += 1; };
          target.addEventListener("focusin", onFocus);
          window.requestAnimationFrame = callback => {
            const handle = -(++requestedFrames);
            captured.set(handle, callback);
            return handle;
          };
          window.cancelAnimationFrame = handle => {
            if (!captured.delete(handle)) originalCancel.call(window, handle);
          };
          const expected = chunks.join("").replace(/\r/g, "").trim();
          const startedAt = performance.now();
          (window as any).__gwtAc13OutputFallback = {
            sample: () => ({
              elapsed_ms: performance.now() - startedAt,
              ordered_output: window.__gwtTerminalTestApi.bufferText(id).includes(expected),
              requested_frames: requestedFrames,
              captured_frames: captured.size,
              window_focused: target.contains(document.activeElement),
              focus_events: focusEvents,
              visibility_state: document.visibilityState,
              socket_url: socket.url,
              chunks: chunks.length,
            }),
            restore: () => {
              window.requestAnimationFrame = originalFrame;
              window.cancelAnimationFrame = originalCancel;
              target.removeEventListener("focusin", onFocus);
              for (const callback of captured.values()) originalFrame.call(window, callback);
              delete (window as any).__gwtAc13OutputFallback;
            },
          };
          // Controlled inbound frames use the real app's project transport and
          // output pipeline; this is not a real-PTY throughput measurement.
          for (const text of chunks) socket.dispatchEvent(new MessageEvent("message", {
            data: JSON.stringify({ kind: "terminal_output", id, data_base64: btoa(text) }),
          }));
        }, { id, chunks });
        // Poll from the controller because animation frames are withheld.
        await expect.poll(async () => {
          observation = await page.evaluate(() => (window as any).__gwtAc13OutputFallback.sample());
          return observation!.ordered_output;
        }, { timeout: 1_000, intervals: [25] }).toBe(true);
        expect(observation!.elapsed_ms, "ordered canonical output within one second").toBeLessThanOrEqual(1_000);
        expect(observation!.requested_frames, "animation callbacks were withheld").toBeGreaterThan(0);
        expect(observation!.window_focused).toBe(false);
        expect(observation!.focus_events, "no terminal focus restored output").toBe(0);
        await page.evaluate(() => (window as any).__gwtAc13OutputFallback.restore());

        // Reload the same real PTY with frames withheld before the app mounts
        // its runtime. The warm probe above cannot exercise first-fit readiness.
        await page.addInitScript(id => {
          const originalFrame = window.requestAnimationFrame;
          const originalCancel = window.cancelAnimationFrame;
          const captured = new Map<number, FrameRequestCallback>();
          let requests = 0;
          let focusEvents = 0;
          let first: Record<string, any> | undefined;
          let result: Record<string, any> | undefined;
          const onFocus = (event: FocusEvent) => {
            if ((event.target as Element).closest(`.workspace-window[data-id="${id}"]`)) focusEvents += 1;
          };
          document.addEventListener("focusin", onFocus);
          window.requestAnimationFrame = callback => {
            captured.set(--requests, callback);
            return requests;
          };
          window.cancelAnimationFrame = handle => {
            if (!captured.delete(handle)) originalCancel.call(window, handle);
          };
          const check = () => {
            if (!first || result) return;
            const sample = { first, elapsed_ms: performance.now() - first.mountedAt,
              ...window.__gwtTerminalTestApi.metrics(id), requested_frames: -requests,
              focus_events: focusEvents,
              has_prompt: /[^\n]+[>$#%]$/.test(window.__gwtTerminalTestApi.bufferText(id).trim()),
              window_focused: !!document.querySelector(`.workspace-window[data-id="${id}"]`)?.contains(document.activeElement) };
            if ((sample.isReady && sample.has_prompt) || sample.elapsed_ms >= 1_000) result = sample;
          };
          const observer = new MutationObserver(() => {
            const metrics = window.__gwtTerminalTestApi?.metrics(id);
            const host = document.querySelector(`.workspace-window[data-id="${id}"] .xterm`)?.parentElement;
            if (!metrics?.hasRuntime || !host?.clientWidth || !host.clientHeight) return;
            first = { ...metrics, mountedAt: performance.now(), width: host.clientWidth, height: host.clientHeight };
            observer.disconnect();
          });
          observer.observe(document, { childList: true, subtree: true, attributes: true });
          const timer = setInterval(check, 100);
          (window as any).__gwtAc13ColdFit = {
            get result() { return result; },
            restore: () => {
              clearInterval(timer);
              observer.disconnect();
              document.removeEventListener("focusin", onFocus);
              window.requestAnimationFrame = originalFrame;
              window.cancelAnimationFrame = originalCancel;
              for (const callback of captured.values()) originalFrame.call(window, callback);
              delete (window as any).__gwtAc13ColdFit;
            },
          };
        }, id);
        await page.reload();
        await expect(target).toBeVisible();
        await focusedControl.focus();
        await expect(focusedControl).toBeFocused();
        // Freeze completion on the page's clock; controller scheduling must
        // not turn an already completed fit into a false deadline failure.
        await page.waitForFunction(() => (window as any).__gwtAc13ColdFit.result,
          undefined, { polling: 100, timeout: 15_000 });
        coldObservation = await page.evaluate(() => (window as any).__gwtAc13ColdFit.result);
        expect(coldObservation!.isReady && coldObservation!.has_prompt).toBe(true);
        expect(coldObservation!.first.isReady, "the laid-out runtime was observed before its first fit").toBe(false);
        expect(coldObservation!.elapsed_ms, "cold readiness and buffered snapshot within one second").toBeLessThanOrEqual(1_000);
        expect({ cols: coldObservation!.cols, rows: coldObservation!.rows }).not.toEqual({
          cols: coldObservation!.first.cols, rows: coldObservation!.first.rows,
        });
        expect(coldObservation!.window_focused).toBe(false);
        expect(coldObservation!.focus_events).toBe(0);
        expect(coldObservation!.requested_frames).toBeGreaterThan(0);
      } finally {
        await page.evaluate(() => (window as any).__gwtAc13ColdFit?.restore());
        await page.evaluate(() => (window as any).__gwtAc13OutputFallback?.restore());
        // rAF is restored only after measuring buffer progress. The PNG records
        // the resumed renderer, not a paint guarantee while rAF was withheld.
        await page.evaluate(() => new Promise<void>(resolve =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
        const screenshot = info.outputPath(`${theme}-terminal-output-fallback.png`);
        await page.screenshot({ path: screenshot });
        await info.attach(`${theme}-terminal-output-fallback`, { path: screenshot, contentType: "image/png" });
        const evidence = info.outputPath(`${theme}-terminal-output-fallback.json`);
        await writeFile(evidence, JSON.stringify({ theme, observation, coldObservation, errors }));
        await info.attach("terminal-output-fallback-observation", { contentType: "application/json", path: evidence });
        // Park auxiliary terminals on the empty Works host before removal so
        // Settings does not create replacement windows during fixture teardown.
        await page.locator('.op-rail__surface[data-surface="works"]').click();
        if (await page.locator("#split-surfaces").getAttribute("data-split") === "true")
          await page.locator("#split-view-button").click();
        for (const id of created.reverse()) {
          await sendLiveGwtEvent(page, { kind: "close_window", id });
          await expect(page.locator(`.workspace-window[data-id="${id}"]`)).toHaveCount(0);
        }
      }
      expect(errors, "console and page errors").toEqual([]);
    });
  });

  test("snapshot preserves split UTF-8 output through the live terminal pipeline", async ({ page }, info) => {
    await withLiveGwtBackendLock(base!, info, async () => {
      const errors: string[] = [];
      const created: string[] = [];
      const theme = info.project.name.includes("light") ? "light" : "dark";
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await gotoLiveGwt(page, base!, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      try {
        const id = await createShell(page, created);
        const bufferText = () => page.evaluate(id => window.__gwtTerminalTestApi.bufferText(id), id);
        const emit = (kind: string, bytes: number[]) => page.evaluate(({ id, kind, bytes }) => {
          const socket = [...((window as any).__gwtPlaywrightSockets as WebSocket[])].reverse()
            .find(candidate => candidate.readyState === WebSocket.OPEN
              && new URL(candidate.url).pathname === "/ws"
              && new URL(candidate.url).searchParams.has("repo_hash"));
          if (!socket) throw new Error("no live project socket");
          socket.dispatchEvent(new MessageEvent("message", {
            data: JSON.stringify({ kind, id, data_base64: btoa(String.fromCharCode(...bytes)) }),
          }));
        }, { id, kind, bytes });
        // Controlled inbound packets exercise the real decoder, snapshot barrier
        // and xterm buffer. This is not a natural PTY throughput measurement.
        await emit("terminal_output", [...Buffer.from("\r\nold stream: "), 0xe3]);
        await expect.poll(bufferText).toContain("old stream: ");
        await emit("terminal_snapshot", [...Buffer.from("\x1b[2J\x1b[Hsnapshot: "), 0xf0, 0x9f, 0xa6]);
        await emit("terminal_output", [0x80, ...Buffer.from(" complete\r\n")]);
        await expect.poll(bufferText).toContain("snapshot: 🦀 complete");
        expect(await bufferText()).not.toContain("\uFFFD");
        expect(await bufferText()).not.toContain("old stream:");
        const screenshot = info.outputPath(`utf8-snapshot-${theme}.png`);
        await page.locator(`.workspace-window[data-id="${id}"]`).screenshot({ path: screenshot });
        await info.attach("UTF-8 snapshot continuation", { path: screenshot, contentType: "image/png" });
      } finally {
        await page.locator('.op-rail__surface[data-surface="works"]').click();
        for (const id of created.reverse()) {
          await sendLiveGwtEvent(page, { kind: "close_window", id });
          await expect(page.locator(`.workspace-window[data-id="${id}"]`)).toHaveCount(0);
        }
      }
      expect(errors, "console and page errors").toEqual([]);
    });
  });

  test("grid queue wait p95 stays within 500ms under rendering and frontend traffic", async ({ page }, info) => {
    await withLiveGwtBackendLock(base!, info, async () => {
      const errors: string[] = [];
      const outputs = new Map<string, string>();
      const created: string[] = [];
      const sent = new Map<string, number>();
      const received = new Map<string, number>();
      const socketEvents: string[] = [];
      const socketTraffic: Array<{ id: number; url: string; sent: Record<string, number>;
        received: Record<string, number>; lastSentAt: number | null; lastReceivedAt: number | null }> = [];
      const burstTrace: Array<Record<string, number | null>> = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      page.on("websocket", socket => {
        const traffic = { id: socketTraffic.length, url: socket.url(), sent: {}, received: {},
          lastSentAt: null, lastReceivedAt: null } as typeof socketTraffic[number];
        socketTraffic.push(traffic);
        socket.on("framesent", ({ payload }) => {
          const event = JSON.parse(String(payload));
          sent.set(event.kind, (sent.get(event.kind) ?? 0) + 1);
          traffic.sent[event.kind] = (traffic.sent[event.kind] ?? 0) + 1;
          traffic.lastSentAt = Date.now();
        });
        socket.on("close", () => socketEvents.push(`closed: ${socket.url()}`));
        socket.on("socketerror", error => socketEvents.push(`error: ${socket.url()}: ${error}`));
        socket.on("framereceived", ({ payload }) => {
          const event = JSON.parse(String(payload));
          received.set(event.kind, (received.get(event.kind) ?? 0) + 1);
          traffic.received[event.kind] = (traffic.received[event.kind] ?? 0) + 1;
          traffic.lastReceivedAt = Date.now();
          if (event.kind === "terminal_output") {
            outputs.set(event.id, (outputs.get(event.id) ?? "")
              + Buffer.from(event.data_base64, "base64").toString("utf8"));
          }

        });
      });
      await gotoLiveGwt(page, base!, { enableTestBridge: true });
      await expect(page.locator("#close-project-button")).toBeVisible();
      const theme = info.project.name.includes("light") ? "light" : "dark";
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      let verified = false;
      try {
        const foreground = await createShell(page, created);
        const background = await createShell(page, created);
        await sendLiveGwtEvent(page, { kind: "terminal_input", id: background, data:
          "node -e \"setInterval(()=>process.stdout.write('GWT_' + 'GRID_BACKGROUND_5117_' + 'x'.repeat(512) + '\\n'),100)\"\r" });
        await expect.poll(() => (outputs.get(background) ?? "").includes("GWT_GRID_BACKGROUND_5117_"),
          { timeout: 15_000 }).toBe(true);
        const outputBefore = (outputs.get(background) ?? "").length;
        const startedAt = Date.now();
        const samples = async (): Promise<number[]> => {
          const directory = join(home!, ".gwt", "logs");
          const names = (await readdir(directory)).filter(name => name.startsWith("gwt.log"));
          const logs = await Promise.all(names.map(name => readFile(join(directory, name), "utf8")));
          return logs.flatMap(log => log.split("\n").flatMap(line => {
            try {
              const row = JSON.parse(line);
              return row.target === "gwt.frontend.timing" && row.fields?.event === "UpdateTerminalGrid"
                && Date.parse(row.timestamp) >= startedAt && typeof row.fields.queue_wait_ms === "number"
                ? [row.fields.queue_wait_ms] : [];
            } catch { return []; }
          }));
        };

        // Observe an actual dispatch after each same-window burst. Coalescing
        // may discard intermediate grids; sent frames are not latency samples.
        const bursts = 120;
        const gridsPerBurst = 200;
        let processed = 0;
        for (let burst = 0; burst < bursts; burst += 1) {
          const sentBefore = sent.get("update_terminal_grid") ?? 0;
          const trace: Record<string, number | null> = { burst, startedAt: Date.now(), sentAt: null, sampledAt: null,
            samplesBefore: processed, samplesAfter: null,
            windowRepliesBefore: received.get("window_list") ?? 0, windowRepliesAfter: null };
          burstTrace.push(trace);
          await page.evaluate(({ id, count }) => {
            const send = (detail: unknown) => window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail }));
            send({ kind: "list_windows" });
            for (let index = 0; index < count; index += 1) {
              send({ kind: "update_terminal_grid", id, cols: 80 + index % 40, rows: 24 + index % 12 });
            }
            send({ kind: "update_terminal_grid", id, cols: 111, rows: 33 });
          }, { id: foreground, count: gridsPerBurst });
          // Exercise actual surface resize/layout alongside the streaming PTY.
          if (burst % 10 === 0) await page.setViewportSize({
            width: 1440 - burst % 20 * 4, height: 900 - burst % 20 * 2,
          });
          await expect.poll(() => sent.get("update_terminal_grid") ?? 0,
            { message: `burst ${burst} reaches the project WebSocket`, timeout: 15_000 })
            .toBeGreaterThanOrEqual(sentBefore + gridsPerBurst + 1);
          trace.sentAt = Date.now();
          await expect.poll(async () => (await samples()).length, { timeout: 15_000 }).toBeGreaterThan(processed);
          processed = (await samples()).length;
          trace.samplesAfter = processed;
          trace.sampledAt = Date.now();
          trace.windowRepliesAfter = received.get("window_list") ?? 0;
        }
        const waits = (await samples()).sort((left, right) => left - right);
        const p95 = waits[Math.ceil(waits.length * 0.95) - 1];
        const backgroundBytes = (outputs.get(background) ?? "").length - outputBefore;
        await info.attach("terminal-grid-queue-wait", {
          body: JSON.stringify({ theme, bursts, grids_sent: bursts * (gridsPerBurst + 1),
            processed_samples: waits.length, p95_ms: p95, max_ms: waits.at(-1), background_output_bytes: backgroundBytes,
            queue_wait_ms: waits }),
          contentType: "application/json",
        });
        console.log(`${theme}: UpdateTerminalGrid n=${waits.length}, p95=${p95}ms, background=${backgroundBytes} bytes`);
        expect(waits.length, "processed grid samples under load").toBeGreaterThanOrEqual(bursts);
        expect(backgroundBytes, "real PTY output while the grid workload runs").toBeGreaterThan(512);
        expect(p95).toBeLessThanOrEqual(500);
        await expectPtyGrid(page, outputs, foreground, "BURST", 111, 33);

        // The fixed surface follows the browser viewport; its fit sends the
        // real terminal grid. A later standalone grid must still replace it.
        const beforeFit = await page.evaluate(id => window.__gwtTerminalTestApi.metrics(id), foreground);
        await page.setViewportSize({ width: 1200, height: 760 });
        await expect.poll(async () => page.evaluate(id => {
          const metrics = window.__gwtTerminalTestApi.metrics(id);
          return { cols: metrics.cols, rows: metrics.rows };
        }, foreground)).not.toEqual({ cols: beforeFit.cols, rows: beforeFit.rows });
        const fitted = await page.evaluate(id => window.__gwtTerminalTestApi.metrics(id), foreground);
        expect(fitted.cols).toBeGreaterThan(0);
        expect(fitted.rows).toBeGreaterThan(0);
        await expectPtyGrid(page, outputs, foreground, "REFIT", fitted.cols, fitted.rows);
        processed = (await samples()).length;
        await sendLiveGwtEvent(page, { kind: "update_terminal_grid", id: foreground, cols: 103, rows: 31 });
        await expect.poll(async () => (await samples()).length).toBeGreaterThan(processed);
        await expectPtyGrid(page, outputs, foreground, "LATEST", 103, 31);
        expect(errors, "console and page errors").toEqual([]);
        const checkedAt = Date.now();
        for (const traffic of socketTraffic) {
          expect(checkedAt - (traffic.lastReceivedAt ?? 0), `receive liveness: ${traffic.url}`)
            .toBeLessThanOrEqual(15_000);
        }
        const screenshot = info.outputPath(`${theme}-terminal-grid.png`);
        await page.screenshot({ path: screenshot });
        await info.attach(`${theme}-terminal-grid`, { path: screenshot, contentType: "image/png" });
        verified = true;
      } finally {
        await info.attach("terminal-grid-transport", { contentType: "application/json",
          body: JSON.stringify({ sent: Object.fromEntries(sent), received: Object.fromEntries(received), socketEvents,
            socketTraffic, burstTrace,
            sockets: await page.evaluate(() => ((window as any).__gwtPlaywrightSockets ?? []).map((socket: WebSocket) =>
              ({ url: socket.url, readyState: socket.readyState, bufferedAmount: socket.bufferedAmount }))), errors }) });
        // Park auxiliary terminals on the empty Works host before removal so
        // Settings does not create replacement windows during fixture teardown.
        await page.locator('.op-rail__surface[data-surface="works"]').click();
        if (await page.locator("#split-surfaces").getAttribute("data-split") === "true")
          await page.locator("#split-view-button").click();
        for (const id of created.reverse()) {
          await sendLiveGwtEvent(page, { kind: "close_window", id });
          // Finish a successful repetition before the next page can discover
          // its old shells. Preserve the original error when the body failed.
          if (verified) await expect(page.locator(`.workspace-window[data-id="${id}"]`)).toHaveCount(0);
        }
        await page.setViewportSize({ width: 1440, height: 900 });
      }
    });
  });
});

async function projectWindows(page: Page): Promise<Array<{ id: string; preset: string; z_index?: number }>> {
  return page.evaluate(() => {
    const workspace = (window as any).__gwtPlaywrightMessages.findLast((entry: any) => entry.payload.kind === "workspace_state").payload.workspace;
    return workspace.tabs.flatMap((tab: any) => tab.workspace.windows);
  });
}

async function showShell(page: Page, id: string) {
  const windows = await projectWindows(page);
  const index = windows.findIndex(window => window.id === id);
  expect(index, "the exact Shell is in the project catalog").toBeGreaterThanOrEqual(0);
  await sendLiveGwtEvent(page, { kind: "focus_window", id });
  await expect.poll(async () => (await projectWindows(page)).reduce((top, window) => !top || (window.z_index || 0) > (top.z_index || 0) ? window : top, null as { id: string; preset: string; z_index?: number } | null)?.id).toBe(id);
  await page.reload();
  await expect(page.locator(`.split-pane .workspace-window[data-id="${id}"]`)).toBeVisible();
}

async function createShell(page: Page, created: string[]): Promise<string> {
  const before = new Set((await projectWindows(page)).map(window => window.id));
  await sendLiveGwtEvent(page, { kind: "create_window", preset: "shell" });
  await expect.poll(async () => (await projectWindows(page)).filter(window => window.preset === "shell" && !before.has(window.id)))
    .toHaveLength(1);
  const id = (await projectWindows(page)).find(window => window.preset === "shell" && !before.has(window.id))!.id;
  created.push(id);
  await showShell(page, id);
  if (created.length === 2) {
    // Shells are auxiliary Settings terminals. Keep both exact PTYs visible in
    // separate panes so focus, viewport fitting and queue measurements remain real.
    const beforeSplit = new Set((await projectWindows(page)).map(window => window.id));
    await page.locator("#split-view-button").click();
    const right = page.getByRole("region", { name: "Right pane", exact: true });
    await expect(right.locator('.workspace-window[data-preset="board"]')).toBeVisible();
    for (const window of await projectWindows(page)) if (!beforeSplit.has(window.id)) created.push(window.id);
    await right.locator(".split-pane__header").click({ position: { x: 8, y: 8 } });
    await showShell(page, created[0]);
    await expect(page.locator('.split-pane[data-surface="settings"]:visible')).toHaveCount(2);
    await expect(page.locator(`.split-pane .workspace-window[data-id="${id}"]`)).toBeVisible();
  }
  await expect.poll(() => page.evaluate(id =>
    window.__gwtTerminalTestApi.bufferText(id).trim(), id), { timeout: 30_000 }).toMatch(/[^\n]+[>$#%]$/);
  return id;
}

async function expectPtyGrid(page: Page, outputs: Map<string, string>, id: string, marker: string,
  cols: number, rows: number): Promise<void> {
  let attempt = 0;
  // PTY input has a fast path and can overtake an outstanding grid dispatch.
  // Retry observation with fresh markers rather than asserting on command echo.
  await expect(async () => {
    const token = `${marker}_${attempt++}`;
    await sendLiveGwtEvent(page, { kind: "terminal_input", id, data:
      `node -p "'GWT_' + 'GRID_5117_${token}:' + process.stdout.columns + 'x' + process.stdout.rows"\r` });
    const actual = () => {
      const match = (outputs.get(id) ?? "").match(new RegExp(`GWT_GRID_5117_${token}:(\\d+)x(\\d+)`));
      return match ? { cols: Number(match[1]), rows: Number(match[2]) } : null;
    };
    await expect.poll(actual, { timeout: 5_000 }).not.toBeNull();
    expect(actual(), `real PTY grid after ${marker}`).toEqual({ cols, rows });
  }).toPass({ timeout: 15_000, intervals: [250, 500] });
}
