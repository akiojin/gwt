import { expect, test } from "@playwright/test";
import { execFile } from "node:child_process";
import { writeFile } from "node:fs/promises";
import { promisify } from "node:util";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const run = promisify(execFile);
const query = "Search route responsiveness acceptance";
const documentTitle = "Issue 5539 search acceptance";
type SearchSample = { request_id: number; wall_ms: number; result_count: number };
type CpuSnapshot = { process_id: number; started_utc: string; timestamp_utc: string; cpu_seconds: number };

test.describe("real search route responsiveness (Issue #5539)", () => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const targetPid = Number(process.env.GWT_PLAYWRIGHT_PID);
  const label = process.env.GWT_SEARCH_CPU_LABEL ?? "after";
  test.skip(!base || !targetPid || process.platform !== "win32",
    "requires the isolated Windows browser-check backend and owned GUI PID");
  test.setTimeout(600_000);
  test.use({ viewport: { width: 1440, height: 900 } });

  test("warmed searches finish promptly and eight pending searches keep controls responsive", async ({ page }, info) => {
    await withLiveGwtBackendLock(base!, info, async () => {
      const theme = info.project.name.includes("light") ? "light" : "dark";
      const errors: string[] = [];
      const pending = new Map<number, {
        started: number;
        resolve: (sample: SearchSample) => void;
        reject: (error: Error) => void;
      }>();
      const controls: Array<{ kind: string; wall_ms: number; pending_searches: number }> = [];
      const failures: Array<{ request_id: number; wall_ms: number; message: string }> = [];
      const sent: Array<{ kind: string; request_id?: number; id?: string; at_ms: number; phase: string }> = [];
      const received: Array<{ kind: string; request_id?: number; at_ms: number; phase: string;
        wall_ms?: number; result_count?: number; fixture_returned?: boolean; message?: string }> = [];
      let phase = "setup";
      let concurrentStarted = 0;
      let batchTimer: ReturnType<typeof setTimeout> | undefined;
      let requestId = 553900;
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      page.on("pageerror", error => errors.push(error.message));
      page.on("websocket", socket => {
        socket.on("framesent", ({ payload }) => {
          let message: any;
          try { message = JSON.parse(String(payload)); } catch { return; }
          if (["search_project_index", "get_system_settings", "list_custom_agent_presets"].includes(message.kind)) {
            sent.push({ kind: message.kind, request_id: message.request_id, id: message.id,
              at_ms: performance.now(), phase });
          }
        });
        socket.on("framereceived", ({ payload }) => {
          let message: any;
          try { message = JSON.parse(String(payload)); } catch { return; }
          if (concurrentStarted && ["system_settings", "custom_agent_preset_list"].includes(message.kind)) {
            controls.push({ kind: message.kind, wall_ms: performance.now() - concurrentStarted,
              pending_searches: pending.size });
          }
          const request = pending.get(message.request_id);
          if (["project_index_search_results", "project_index_search_error", "system_settings", "custom_agent_preset_list"].includes(message.kind)) {
            received.push({ kind: message.kind, request_id: message.request_id, at_ms: performance.now(), phase,
              wall_ms: request ? performance.now() - request.started : undefined,
              result_count: message.results?.length, message: message.message,
              fixture_returned: message.results?.some((result: any) => result.scope === "memory" && result.title.includes(documentTitle)) });
          }
          if (!request) return;
          if (message.kind === "project_index_search_error") {
            pending.delete(message.request_id);
            failures.push({ request_id: message.request_id, wall_ms: performance.now() - request.started,
              message: message.message });
            request.reject(new Error(`real search failed: ${message.message}`));
          } else if (message.kind === "project_index_search_results") {
            pending.delete(message.request_id);
            if (!message.results?.some((result: any) => result.scope === "memory" && result.title.includes(documentTitle))) {
              request.reject(new Error(`real search did not return the seeded memory: ${JSON.stringify(message.results)}`));
            } else {
              request.resolve({ request_id: message.request_id, wall_ms: performance.now() - request.started,
                result_count: message.results.length });
            }
          }
        });
      });
      const attachJson = async (name: string, value: unknown) => {
        const path = info.outputPath(`${name}.json`);
        await writeFile(path, JSON.stringify(value, null, 2));
        await info.attach(name, { path, contentType: "application/json" });
      };
      try {
        expect(["baseline", "after"]).toContain(label);
        await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
        await gotoLiveGwt(page, base!, { enableTestBridge: true });
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await expect(page.locator("#close-project-button")).toBeVisible();
        await page.evaluate(() => document.dispatchEvent(
          new CustomEvent("settings:open", { detail: { target: "index" }, bubbles: true })));
        const indexWindow = page.locator('.workspace-window[data-preset="index"]').last();
        const root = indexWindow.locator(".index-search-root");
        await expect(root).toBeVisible();
        const id = await indexWindow.getAttribute("data-id");
        expect(id).toBeTruthy();
        const searchPayload = (request_id: number) => ({ kind: "search_project_index", id,
          query, request_id, scopes: ["memory"], worktree_hash: null, match_mode: "semantic" });
        const prepareSearch = () => {
          const request_id = ++requestId;
          const promise = new Promise<SearchSample>((resolve, reject) =>
            pending.set(request_id, { started: performance.now(), resolve, reject }));
          return { payload: searchPayload(request_id), promise };
        };
        const search = async () => {
          const request = prepareSearch();
          await sendLiveGwtEvent(page, request.payload);
          return request.promise;
        };

        // The first real request loads the worker/model and is excluded from p95.
        phase = "warmup";
        const warmup = await search();
        phase = "sequential";
        const sequentialStart = await processCpu(targetPid);
        const sequential: SearchSample[] = [];
        for (let sample = 0; sample < 20; sample += 1) sequential.push(await search());
        const sequentialEnd = await processCpu(targetPid);
        const sorted = sequential.map(sample => sample.wall_ms).sort((left, right) => left - right);
        const p95 = sorted[Math.ceil(sorted.length * 0.95) - 1];
        const sequentialEvidence = { label, theme, process_id: targetPid, query, scope: "memory", warmup,
          samples: sequential, p95_ms: p95, threshold_ms: 2000,
          cpu: cpuDelta(sequentialStart, sequentialEnd), snapshots: { start: sequentialStart, end: sequentialEnd } };
        await attachJson(`${label}-${theme}-search-sequential`, sequentialEvidence);
        console.log(JSON.stringify(sequentialEvidence));

        const concurrentStart = await processCpu(targetPid);
        phase = "concurrent";
        const requests = Array.from({ length: 8 }, prepareSearch);
        const batchIds = requests.map(request => request.payload.request_id);
        // AC-3 measures GUI responsiveness while the real requests are pending.
        // Preserve missing or rejected responses without requiring every query
        // to finish in this bounded observation window.
        const concurrentDone = Promise.allSettled(requests.map(request => request.promise));
        concurrentStarted = performance.now();
        const observation = Promise.race([
          concurrentDone.then(() => ({ timed_out: false })),
          new Promise<{ timed_out: boolean }>(resolve => {
            batchTimer = setTimeout(() => resolve({ timed_out: true }), 45_000);
          }),
        ]);
        await page.evaluate(payloads => {
          for (const detail of payloads) window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail }));
        }, [...requests.map(request => request.payload),
          { kind: "get_system_settings" }, { kind: "list_custom_agent_presets" }]);
        const interactionStarted = performance.now();
        await root.locator("[data-index-tab='health']").click({ timeout: 2000 });
        await expect(root.locator("[data-index-panel='health']")).toBeVisible();
        const interaction = { wall_ms: performance.now() - interactionStarted, pending_searches: pending.size };
        await expect.poll(() => sent.filter(frame => batchIds.includes(frame.request_id!)).length,
          { timeout: 2000 }).toBe(8);
        await expect.poll(() => controls.length, { timeout: 2000 }).toBeGreaterThanOrEqual(2);
        const observationResult = await observation;
        clearTimeout(batchTimer);
        const concurrent = received.filter(frame => batchIds.includes(frame.request_id!)
          && frame.kind === "project_index_search_results");
        const pendingAtCutoff = batchIds.filter(id => pending.has(id));
        concurrentStarted = 0;
        const concurrentEnd = await processCpu(targetPid);
        const artifact = { label, theme, process_id: targetPid, query, scope: "memory", warmup,
          sequential: { samples: sequential, p95_ms: p95, threshold_ms: 2000,
            cpu: cpuDelta(sequentialStart, sequentialEnd), snapshots: { start: sequentialStart, end: sequentialEnd } },
          concurrent: { count: 8, observation_limit_ms: 45_000, ...observationResult,
            sent: sent.filter(frame => batchIds.includes(frame.request_id!)),
            samples: concurrent, failures, pending_request_ids: pendingAtCutoff, controls, interaction,
            cpu: cpuDelta(concurrentStart, concurrentEnd), snapshots: { start: concurrentStart, end: concurrentEnd } } };
        await attachJson(`${label}-${theme}-search-route`, artifact);
        console.log(JSON.stringify(artifact));
        expect(sent.filter(frame => batchIds.includes(frame.request_id!)).map(frame => frame.request_id).sort(),
          "all eight search requests were sent through the real WebSocket").toEqual([...batchIds].sort());
        for (const kind of ["system_settings", "custom_agent_preset_list"]) {
          const response = controls.find(control => control.kind === kind);
          expect(response, `${kind} responds during the search batch`).toBeDefined();
          expect(response!.pending_searches, `${kind} arrives before all eight searches complete`).toBeGreaterThan(0);
          expect(response!.wall_ms, `${kind} remains responsive`).toBeLessThanOrEqual(2000);
        }
        expect(interaction.pending_searches, "real browser interaction finishes while search remains pending").toBeGreaterThan(0);
        expect(interaction.wall_ms).toBeLessThanOrEqual(2000);
        if (label !== "baseline" || process.env.GWT_SEARCH_ALLOW_BASELINE_LATENCY_FAILURE !== "1") {
          expect(p95, "20 sequential warmed real searches: p95 <= 2 seconds").toBeLessThanOrEqual(2000);
        }

        phase = "ui";
        await root.locator("[data-index-tab='search']").click();
        // Complete one search through the actual UI as well as the raw event route.
        const selectedScopes = await root.locator(".index-scope-button[aria-pressed='true']")
          .evaluateAll(nodes => nodes.map(node => (node as HTMLElement).dataset.scope!));
        for (const scope of selectedScopes) {
          if (scope !== "memory") await root.locator(`.index-scope-button[data-scope='${scope}']`).click();
        }
        const memory = root.locator(".index-scope-button[data-scope='memory']");
        if (await memory.getAttribute("aria-pressed") !== "true") await memory.click();
        await root.locator(".index-search-input").fill(query);
        await expect(root.locator(".index-result-row").first()).toContainText(documentTitle, { timeout: 30_000 });
        expect(errors, "zero console and page errors").toEqual([]);
        const screenshot = info.outputPath(`${label}-${theme}-search-route.png`);
        await page.screenshot({ path: screenshot });
        await info.attach(`${label}-${theme}-search-route`, { path: screenshot, contentType: "image/png" });
      } finally {
        clearTimeout(batchTimer);
        await attachJson(`${label}-${theme}-search-errors`, errors);
        await attachJson(`${label}-${theme}-search-failures`, failures);
        await attachJson(`${label}-${theme}-search-protocol`, { phase, sent, received, controls,
          pending_request_ids: [...pending.keys()], failures, console_and_page_errors: errors });
      }
    });
  });
});

async function processCpu(pid: number): Promise<CpuSnapshot> {
  if (!Number.isSafeInteger(pid) || pid <= 0) throw new Error("invalid owned GUI PID");
  const command = `$item = Get-Process -Id ${pid} -ErrorAction Stop; `
    + "@{process_id=$item.Id;started_utc=$item.StartTime.ToUniversalTime().ToString('o');"
    + "timestamp_utc=[DateTime]::UtcNow.ToString('o');cpu_seconds=$item.TotalProcessorTime.TotalSeconds} | ConvertTo-Json -Compress";
  const { stdout } = await run("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", command], { windowsHide: true });
  return JSON.parse(stdout.trim());
}

function cpuDelta(start: CpuSnapshot, end: CpuSnapshot) {
  expect(end.process_id).toBe(start.process_id);
  expect(end.started_utc, "the exact owned GUI process stayed alive").toBe(start.started_utc);
  return { cpu_seconds: end.cpu_seconds - start.cpu_seconds,
    wall_seconds: (Date.parse(end.timestamp_utc) - Date.parse(start.timestamp_utc)) / 1000 };
}
