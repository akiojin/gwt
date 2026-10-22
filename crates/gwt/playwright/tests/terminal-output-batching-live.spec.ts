import { expect, test, type Page } from "@playwright/test";
import { spawn } from "node:child_process";
import { writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

type ThreadSample = {
  id: number;
  name: string;
  created_file_time: string;
  kernel_seconds: number;
  user_seconds: number;
};
type CpuSnapshot = { phase: "start" | "end"; process_id: number; timestamp_utc: string; threads: ThreadSample[] };
type PaneTraffic = { raw_chunks: number; raw_bytes: number; previews: number; last_raw_at: number; text: string; preview: string };

test.describe("terminal output batching (Issue #5501)", () => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME;
  const targetPid = Number(process.env.GWT_PLAYWRIGHT_PID);
  const label = process.env.GWT_BATCHING_CPU_LABEL ?? "after";
  const sampleMs = Number(process.env.GWT_BATCHING_SAMPLE_MS ?? "10000");
  test.skip(!base || !home || !targetPid || process.platform !== "win32",
    "requires an isolated Windows browser-check checkout, fresh HOME, and owned GUI PID");
  test.setTimeout(180_000);
  test.use({ viewport: { width: 1680, height: 1000 } });

  test("eight real PTYs stream continuously while previews converge at bounded cadence", async ({ page }, info) => {
    await withLiveGwtBackendLock(base!, info, async () => {
      expect(["before", "after"]).toContain(label);
      expect(sampleMs).toBeGreaterThanOrEqual(5000);
      expect(sampleMs).toBeLessThanOrEqual(120_000);
      const theme = info.project.name.includes("light") ? "light" : "dark";
      const errors: string[] = [];
      const traffic = new Map<string, PaneTraffic>();
      const relay = observeDaemonOutput(home!, targetPid);
      const created: string[] = [];
      const snapshots: Array<{ at: number; panes: ReturnType<typeof counters>; daemon: ReturnType<typeof relay.counters> }> = [];
      let sampleStart: CpuSnapshot | undefined;
      let sampleEnd: CpuSnapshot | undefined;
      let sampleTimer: ReturnType<typeof setInterval> | undefined;
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      page.on("websocket", socket => socket.on("framereceived", ({ payload }) => {
        const event = JSON.parse(String(payload));
        if (event.kind !== "terminal_output" && event.kind !== "terminal_preview") return;
        const pane = traffic.get(event.id) ?? { raw_chunks: 0, raw_bytes: 0, previews: 0,
          last_raw_at: 0, text: "", preview: "" };
        if (event.kind === "terminal_output") {
          const bytes = Buffer.from(event.data_base64, "base64");
          pane.raw_chunks += 1;
          pane.raw_bytes += bytes.length;
          pane.last_raw_at = Date.now();
          pane.text = (pane.text + bytes.toString("utf8")).slice(-32_768);
        } else {
          pane.previews += 1;
          pane.preview = event.text;
        }
        traffic.set(event.id, pane);
      }));
      const recordTraffic = () => snapshots.push({ at: Date.now(), panes: counters(created, traffic), daemon: relay.counters(created) });
      const attachJson = async (name: string, value: unknown) => {
        const path = info.outputPath(`${name}.json`);
        await writeFile(path, JSON.stringify(value, null, 2));
        await info.attach(name, { path, contentType: "application/json" });
      };
      let verified = false;
      try {
        expect(Number(process.env.GWT_BATCHING_DAEMON_PID), "owned live daemon").toBeGreaterThan(0);
        expect(process.env.GWT_BATCHING_DAEMON_STATUS).toMatch(/^running pid=\d+.*probe=ok/);
        await gotoLiveGwt(page, base!, { enableTestBridge: true });
        await expect(page.locator("#close-project-button")).toBeVisible();
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        for (let index = 0; index < 8; index += 1) await createShell(page, created, index);

        // Every pane runs exactly the same 20ms cursor redraw. Raw stdin lets
        // the test stop it with q and observe a real final screen, not an echo.
        const script = "let n=0;process.stdin.setRawMode(true);process.stdin.resume();"
          + "process.stdout.write('\\x1b[2J\\x1b[H');"
          + "const timer=setInterval(()=>process.stdout.write('\\x1b[HGWT_'+'BATCHING_5501_RUN:'"
          + "+String(n++).padStart(8,'0')+' '+('|/-'.charAt(n%3))+'x'.repeat(512)+String(n).padStart(8,'0')),20);"
          + "process.stdin.on('data',k=>{if(k.includes(113)){clearInterval(timer);"
          + "process.stdout.write('\\x1b[2J\\x1b[HGWT_'+'BATCHING_5501_DONE\\r\\n');process.exit(0)}})";
        for (const id of created) {
          await sendLiveGwtEvent(page, { kind: "terminal_input", id, data: `node -e "${script}"\r` });
        }
        for (const id of created) {
          await expect.poll(() => traffic.get(id)?.text ?? "", { timeout: 30_000 })
            .toContain("GWT_BATCHING_5501_RUN:");
          await expect.poll(() => relay.text(id), { timeout: 30_000 })
            .toContain("GWT_BATCHING_5501_RUN:");
        }

        await sampleThreadCpu(targetPid, sampleMs, snapshot => {
          if (snapshot.phase === "start") {
            sampleStart = snapshot;
            recordTraffic();
            sampleTimer = setInterval(recordTraffic, 1000);
          } else {
            sampleEnd = snapshot;
            clearInterval(sampleTimer);
            recordTraffic();
          }
        });
        await attachJson(`${label}-${theme}-native-thread-snapshots`, { start: sampleStart, end: sampleEnd });
        const seconds = (Date.parse(sampleEnd!.timestamp_utc) - Date.parse(sampleStart!.timestamp_utc)) / 1000;
        const first = snapshots[0];
        const last = snapshots.at(-1)!;
        const paneResults = created.map(id => ({ id,
          raw_chunks: last.panes[id].raw_chunks - first.panes[id].raw_chunks,
          raw_bytes: last.panes[id].raw_bytes - first.panes[id].raw_bytes,
          previews: last.panes[id].previews - first.panes[id].previews,
          last_raw_age_ms: last.at - last.panes[id].last_raw_at,
        }));
        const cpu = cpuDeltas(sampleStart!, sampleEnd!);
        const daemonPanes = created.map(id => ({ id,
          frames: last.daemon[id].frames - first.daemon[id].frames,
          bytes: last.daemon[id].bytes - first.daemon[id].bytes,
          previews: last.daemon[id].previews - first.daemon[id].previews,
        }));
        const artifact = { label, theme, process_id: targetPid, sample_seconds: seconds,
          daemon_pid: Number(process.env.GWT_BATCHING_DAEMON_PID),
          daemon_probe: process.env.GWT_BATCHING_DAEMON_STATUS, daemon_panes: daemonPanes,
          workload: { panes: 8, redraw_interval_ms: 20, payload_padding_bytes: 512,
            command: `node -e "${script}"`, stop_input: "q" },
          cpu, panes: paneResults, traffic_samples: snapshots,
          native_thread_snapshots: { start: sampleStart, end: sampleEnd } };
        await attachJson(`${label}-${theme}-terminal-output-cpu`, artifact);
        console.log(`${label}/${theme}: ${seconds.toFixed(3)}s, main=${cpu.main.cpu_seconds.toFixed(6)} CPU s, `
          + `publish=${cpu.publish_worker.cpu_seconds.toFixed(6)} CPU s; `
          + JSON.stringify(paneResults));
        for (const pane of paneResults) {
          expect(pane.raw_chunks, `${pane.id}: continuous raw output`).toBeGreaterThan(seconds * 10);
          expect(pane.last_raw_age_ms, `${pane.id}: output remains live at sample end`).toBeLessThan(2000);
          expect(pane.raw_bytes).toBeGreaterThan(10_000);
          for (let index = 1; index < snapshots.length; index += 1) {
            if (snapshots[index].at - snapshots[index - 1].at < 500) continue;
            expect(snapshots[index].panes[pane.id].raw_chunks,
              `${pane.id}: raw output in sample interval ${index}`)
              .toBeGreaterThan(snapshots[index - 1].panes[pane.id].raw_chunks);
          }
          if (label === "after") {
            expect(pane.previews, `${pane.id}: 150ms preview batching`).toBeLessThanOrEqual(Math.ceil(seconds * 10) + 2);
            expect(pane.previews, `${pane.id}: live preview publication`).toBeGreaterThan(0);
            expect(pane.raw_chunks, `${pane.id}: multiple raw chunks per preview`).toBeGreaterThan(pane.previews * 2);
          }
        }
        for (const pane of daemonPanes) {
          expect(pane.frames, `${pane.id}: daemon actually relays output during CPU sampling`).toBeGreaterThan(0);
          expect(pane.bytes).toBeGreaterThan(10_000);
          if (label === "after") expect(pane.previews).toBeLessThanOrEqual(Math.ceil(seconds * 10) + 2);
        }
        for (const id of created) {
          await sendLiveGwtEvent(page, { kind: "terminal_input", id, data: "q" });
        }
        for (const id of created) {
          await expect.poll(() => traffic.get(id)?.text ?? "", { timeout: 15_000 }).toContain("GWT_BATCHING_5501_DONE");
          await expect.poll(() => traffic.get(id)?.preview ?? "", { timeout: 15_000 }).toContain("GWT_BATCHING_5501_DONE");
          await expect.poll(() => relay.text(id), { timeout: 15_000 }).toContain("GWT_BATCHING_5501_DONE");
          await expect.poll(() => page.evaluate(id => window.__gwtTerminalTestApi.bufferText(id), id))
            .toContain("GWT_BATCHING_5501_DONE");
        }
        expect(errors, "console and page errors").toEqual([]);
        expect(relay.errors(), "daemon subscription errors").toEqual("");
        const screenshot = info.outputPath(`${label}-${theme}-eight-terminal-panes.png`);
        await page.screenshot({ path: screenshot });
        await info.attach(`${label}-${theme}-eight-terminal-panes`, { path: screenshot, contentType: "image/png" });
        verified = true;
      } finally {
        clearInterval(sampleTimer);
        relay.stop();
        await attachJson("terminal-output-errors", errors);
        for (const id of created.reverse()) {
          await sendLiveGwtEvent(page, { kind: "close_window", id });
          if (verified) await expect(page.locator(`.workspace-window[data-id="${id}"]`)).toHaveCount(0);
        }
      }
    });
  });
});

function observeDaemonOutput(home: string, sourcePid: number) {
  const root = process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd();
  const env = { ...process.env, HOME: home, USERPROFILE: home };
  delete env.GWT_BIN_PATH;
  const child = spawn(resolve(root, "target/debug/gwtd.exe"), [], { cwd: root, env, windowsHide: true });
  const panes = new Map<string, { frames: number; bytes: number; previews: number; text: string }>();
  let pending = "";
  let errors = "";
  child.on("error", error => { errors += error.message; });
  child.stderr.setEncoding("utf8").on("data", data => { errors += data; });
  child.stdout.setEncoding("utf8").on("data", data => {
    pending += data;
    let boundary: number;
    while ((boundary = pending.indexOf("\n")) >= 0) {
      const line = pending.slice(0, boundary).trim();
      pending = pending.slice(boundary + 1);
      if (!line) continue;
      try {
        const frame = JSON.parse(line);
        if (frame.type !== "event" || frame.channel !== "runtime_output" || frame.payload.source_pid !== sourcePid) continue;
        const payload = frame.payload;
        const pane = panes.get(payload.id) ?? { frames: 0, bytes: 0, previews: 0, text: "" };
        const bytes = Buffer.from(payload.data_base64, "base64");
        pane.frames += 1;
        pane.bytes += bytes.length;
        if (typeof payload.preview_text === "string") pane.previews += 1;
        pane.text = (pane.text + bytes.toString("utf8")).slice(-32_768);
        panes.set(payload.id, pane);
      } catch (error) { errors += String(error); }
    }
  });
  child.stdin.end(JSON.stringify({ schema_version: 1, operation: "daemon.subscribe",
    params: { channels: ["runtime_output"], timeout_seconds: 180 } }) + "\n");
  return {
    counters: (ids: string[]) => Object.fromEntries(ids.map(id => {
      const pane = panes.get(id) ?? { frames: 0, bytes: 0, previews: 0 };
      return [id, { frames: pane.frames, bytes: pane.bytes, previews: pane.previews }];
    })),
    text: (id: string) => panes.get(id)?.text ?? "",
    errors: () => errors,
    stop: () => { child.kill(); },
  };
}

function counters(ids: string[], traffic: Map<string, PaneTraffic>): Record<string, { raw_chunks: number; raw_bytes: number; previews: number; last_raw_at: number }> {
  return Object.fromEntries(ids.map(id => { const pane = traffic.get(id)!;
    return [id, { raw_chunks: pane.raw_chunks, raw_bytes: pane.raw_bytes, previews: pane.previews, last_raw_at: pane.last_raw_at }];
  }));
}

function cpuDeltas(start: CpuSnapshot, end: CpuSnapshot) {
  const main = start.threads[0]; // Native creation time identifies the process's initial thread.
  const publish = start.threads.find(thread => thread.name === "gwt-runtime-daemon-publish-worker");
  expect(main, "native main thread snapshot").toBeDefined();
  expect(publish, "named daemon publish worker snapshot").toBeDefined();
  const delta = (before: ThreadSample) => {
    const after = end.threads.find(thread => thread.id === before.id
      && thread.created_file_time === before.created_file_time);
    expect(after, `same live native thread ${before.id}`).toBeDefined();
    return { id: before.id, name: before.name,
      cpu_seconds: after!.kernel_seconds + after!.user_seconds - before.kernel_seconds - before.user_seconds };
  };
  return { main: delta(main), publish_worker: delta(publish!) };
}

async function sampleThreadCpu(pid: number, duration: number, onSnapshot: (snapshot: CpuSnapshot) => void): Promise<void> {
  const helper = resolve(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd(),
    "scripts/diagnostics/issue-5501-thread-cpu.ps1");
  await new Promise<void>((resolve, reject) => {
    const sampler = spawn("powershell.exe", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
      "-File", helper, "-TargetProcessId", String(pid), "-SampleMilliseconds", String(duration)], { windowsHide: true });
    let pending = "";
    let errors = "";
    let snapshots = 0;
    sampler.stdout.setEncoding("utf8").on("data", (data: string) => {
      pending += data;
      let boundary: number;
      while ((boundary = pending.indexOf("\n")) >= 0) {
        const line = pending.slice(0, boundary).trim();
        pending = pending.slice(boundary + 1);
        if (line) {
          try { onSnapshot(JSON.parse(line)); snapshots += 1; }
          catch (error) { sampler.kill(); reject(error); }
        }
      }
    });
    sampler.stderr.setEncoding("utf8").on("data", (data: string) => { errors += data; });
    sampler.on("error", reject);
    sampler.on("close", code => code === 0 && snapshots === 2
      ? resolve() : reject(new Error(`thread CPU sampler exit=${code}, snapshots=${snapshots}: ${errors}`)));
  });
}

async function createShell(page: Page, created: string[], index: number): Promise<void> {
  const before = new Set(await page.locator(".workspace-window").evaluateAll(nodes =>
    nodes.map(node => (node as HTMLElement).dataset.id)));
  await sendLiveGwtEvent(page, { kind: "create_window", preset: "shell",
    bounds: { x: 30 + index % 4 * 408, y: 60 + Math.floor(index / 4) * 430, width: 400, height: 420 } });
  const id = await page.waitForFunction(seen => {
    const shell = [...document.querySelectorAll<HTMLElement>('.workspace-window[data-preset="shell"]')]
      .find(node => !seen.includes(node.dataset.id!));
    return shell?.dataset.id;
  }, [...before], { timeout: 30_000 }).then(handle => handle.jsonValue());
  created.push(id);
  await expect.poll(() => page.evaluate(id => window.__gwtTerminalTestApi.bufferText(id).trim(), id),
    { timeout: 30_000 }).toMatch(/[^\n]+[>$#%]$/);
}
