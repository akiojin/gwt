/** Issue #5543: copied real cache, Work and Sessions over the actual backend. */
import { mkdir, readFile, readdir, realpath, rm, stat, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const REQUEST_TIMEOUT_MS = 60_000;
const ENRICHMENT_TIMEOUT_MS = 180_000;
const rounds = positiveInteger(process.env.GWT_KNOWLEDGE_LOAD_ROUNDS, 1);
const observeSeconds = positiveInteger(process.env.GWT_KNOWLEDGE_LOAD_OBSERVE_SECONDS, 0);
type Sample = {
  id: string; request_id: number; phase: string; sent_at_ms: number;
  status: "pending" | "success" | "error" | "timeout";
  wall_ms?: number; response_at_ms?: number; entry_count?: number; response_bytes?: number; message?: string;
};
type Control = { phase: string; sent_at_ms?: number; response_at_ms?: number; wall_ms?: number; pending_loads?: number };
type PhaseRecord = { window_id: string; request_id: number; stage: string; status: string;
  recorded_at_ms: number; elapsed_ms?: number; duration_ms?: number };
type RelatedRow = { number: number; works: number; sessions: number };
type EntryReceipt = { id: string; request_id: number; at_ms: number; entry_count: number;
  related_rows: RelatedRow[] };
const requiredStages = ["queue", "cache_read", "assembly", "dispatch_enqueue", "dispatch_wait", "send"];

test("phase timings exclude a preceding theme with the same request ID", async ({}, info) => {
  test.skip(!CHECK_HOME, "requires an isolated diagnostic HOME");
  const directory = join(CHECK_HOME, ".gwt/logs");
  await mkdir(directory, { recursive: true });
  const path = join(directory, `gwt.log.phase-regression-${info.project.name}`);
  const sample = { id: "phase-regression", request_id: 5543 } as Sample;
  const record = (timestamp: string, duration_ms: number) => JSON.stringify({
    timestamp, target: "gwt::knowledge_load", fields: {
      window_id: sample.id, request_id: sample.request_id,
      stage: "cache_read", status: "completed", elapsed_ms: duration_ms, duration_ms,
    },
  });
  try {
    await writeFile(path, [record("2020-01-01T00:00:00Z", 1), record("2020-01-03T00:00:00Z", 2)].join("\n"));
    const records = await loadPhaseRecords([sample], "diagnostic-only", Date.parse("2020-01-02T00:00:00Z"));
    expect(records.map(row => row.duration_ms)).toEqual([2]);
  } finally {
    await rm(path, { force: true });
  }
});

test.describe("Issue cache load (live backend, Issue #5543)", () => {
  test.skip(!BASE || !CHECK_HOME || process.platform !== "win32", "requires the isolated Windows browser-check fixture");
  test.setTimeout(420_000 + rounds * REQUEST_TIMEOUT_MS + observeSeconds * 1000);
  test.use({ viewport: { width: 1600, height: 1100 } });

  test("first, warm and twelve parallel loads preserve GUI responses", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const fixture = JSON.parse(await readFile(join(CHECK_HOME, "issue-5543-isolated.json"), "utf8"));
      const theme = info.project.name.includes("light") ? "light" : "dark";
      const effectiveObserveSeconds = theme === "light" ? Math.min(observeSeconds, 60) : observeSeconds;
      const errors: string[] = [];
      const samples: Sample[] = [];
      const pending = new Map<string, { sample: Sample; timer: ReturnType<typeof setTimeout> }>();
      const controls: Control[] = [];
      const frames: Array<Record<string, unknown>> = [];
      const eventKinds: Array<Record<string, unknown>> = [];
      const measured: Sample[] = [];
      const receipts: EntryReceipt[] = [];
      const gateDirectory = join(CHECK_HOME, ".gwt/issue-3777-active-work-rendezvous");
      const gateEvidence: Record<string, unknown> = {};
      let firstSample: Sample | undefined;
      let phaseRecords: PhaseRecord[] = [];
      let phase = "setup";
      let windowId: string | undefined;
      let activeControl: Control | undefined;
      let batchIds = new Set<number>();
      let requestId = theme === "dark" ? 554_300_000 : 554_400_000;
      let verified = false;
      let diagnostics: unknown;
      let timeoutLedgerCount: number | null = null;
      const logStartedAtMs = Date.now();
      const started = performance.now();
      const now = () => performance.now() - started;
      const key = (id: string, request_id: number) => `${id}:${request_id}`;
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      // Observe CDP frames from the real socket. The live helper's test bridge
      // forwards events to that socket; it does not synthesize backend replies.
      page.on("websocket", socket => {
        socket.on("framesent", ({ payload }) => {
          let message: any;
          try { message = JSON.parse(String(payload)); } catch { return; }
          eventKinds.push({ direction: "sent", kind: message.kind, preset: message.kind === "create_window" ? message.preset : undefined,
            phase, at_ms: now() });
          if (message.kind === "load_knowledge_bridge" && message.knowledge_kind === "issue") {
            const sample: Sample = { id: message.id, request_id: message.request_id,
              phase, sent_at_ms: now(), status: "pending" };
            samples.push(sample);
            frames.push({ direction: "sent", kind: message.kind, id: message.id,
              request_id: message.request_id, phase, at_ms: sample.sent_at_ms });
            const timer = setTimeout(() => {
              sample.status = "timeout";
              sample.wall_ms = now() - sample.sent_at_ms;
              pending.delete(key(sample.id, sample.request_id));
            }, REQUEST_TIMEOUT_MS);
            pending.set(key(sample.id, sample.request_id), { sample, timer });
          } else if (message.kind === "list_windows" && activeControl && activeControl.sent_at_ms === undefined) {
            activeControl.sent_at_ms = now();
            frames.push({ direction: "sent", kind: message.kind, phase, at_ms: now() });
          }
        });
        socket.on("framereceived", ({ payload }) => {
          let message: any;
          try { message = JSON.parse(String(payload)); } catch { return; }
          eventKinds.push({ direction: "received", kind: message.kind, phase, at_ms: now() });
          if (message.kind === "window_list" && activeControl?.sent_at_ms !== undefined && activeControl.response_at_ms === undefined) {
            activeControl.response_at_ms = now();
            activeControl.wall_ms = activeControl.response_at_ms - activeControl.sent_at_ms;
            activeControl.pending_loads = [...pending.values()].filter(({ sample }) => batchIds.has(sample.request_id)).length;
            frames.push({ direction: "received", kind: message.kind, phase, at_ms: now() });
          }
          if (!["knowledge_entries", "knowledge_error"].includes(message.kind)) return;
          if (message.kind === "knowledge_entries") receipts.push({ id: message.id,
            request_id: message.request_id, at_ms: now(), entry_count: message.entries?.length ?? 0,
            related_rows: (message.entries ?? []).filter((entry: any) => entry.related_work_count > 0 || entry.related_session_count > 0)
              .map((entry: any) => ({ number: entry.number, works: entry.related_work_count ?? 0, sessions: entry.related_session_count ?? 0 })) });
          const record = pending.get(key(message.id, message.request_id));
          frames.push({ direction: "received", kind: message.kind, id: message.id,
            request_id: message.request_id, phase, at_ms: now(), late_or_untracked: !record,
            entry_count: message.entries?.length, response_bytes: Buffer.byteLength(String(payload)) });
          if (!record) return;
          clearTimeout(record.timer);
          pending.delete(key(message.id, message.request_id));
          Object.assign(record.sample, { status: message.kind === "knowledge_entries" ? "success" : "error",
            response_at_ms: now(), wall_ms: now() - record.sample.sent_at_ms,
            entry_count: message.entries?.length, response_bytes: Buffer.byteLength(String(payload)), message: message.message });
        });
      });

      const awaitSamples = async (selected: Sample[]) => {
        await expect.poll(() => selected.every(sample => sample.status !== "pending"), {
          timeout: REQUEST_TIMEOUT_MS + 5000, intervals: [100, 250],
          message: "record every actual response or its sixty-second timeout",
        }).toBe(true);
      };
      const sendLoads = async (count: number, withControl = false) => {
        const ids = Array.from({ length: count }, () => ++requestId);
        batchIds = new Set(ids);
        if (withControl) { activeControl = { phase }; controls.push(activeControl); }
        const payloads: Array<Record<string, unknown>> = ids.map(request_id => ({
          kind: "load_knowledge_bridge", id: windowId, knowledge_kind: "issue",
          request_id, selected_number: null, refresh: false,
        }));
        if (withControl) payloads.push({ kind: "list_windows" });
        await page.evaluate(messages => {
          for (const detail of messages) window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail }));
        }, payloads);
        await expect.poll(() => samples.filter(sample => ids.includes(sample.request_id)).length, {
          timeout: 5000, intervals: [100], message: "all distinct requests are sent through the real WebSocket",
        }).toBe(count);
        const selected = samples.filter(sample => ids.includes(sample.request_id));
        measured.push(...selected);
        await awaitSamples(selected);
        // A missing control remains explicit evidence instead of hanging a baseline.
        if (withControl && activeControl?.response_at_ms === undefined) {
          await expect.poll(() => activeControl?.response_at_ms !== undefined, {
            timeout: 5000, intervals: [100],
          }).toBe(true).catch(() => undefined);
        }
        activeControl = undefined;
      };

      try {
        expect(["before", "after"]).toContain(fixture.label);
        expect(await realpath(CHECK_HOME), "fresh isolated HOME").not.toBe(await realpath(homedir()));
        expect(fixture.pid).toBe(Number(process.env.GWT_PLAYWRIGHT_PID));
        expect(await realpath(fixture.project)).toBe(await realpath(process.env.GWT_PLAYWRIGHT_PROJECT_ROOT!));
        const state = join(CHECK_HOME, ".gwt/projects", fixture.repo_hash, "project-state");
        expect(JSON.parse(await readFile(join(state, "pm.json"), "utf8")).settings.auto_start).toBe(false);
        expect(JSON.parse(await readFile(join(state, "issue-monitor.json"), "utf8")).enabled).toBe(false);
        const served = (await readFile(join(CHECK_HOME, "url.txt"), "utf8")).trim();
        expect(new URL(BASE).origin).toBe(new URL(served).origin);
        await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
        await page.addInitScript(() => {
          // Copied Sessions are measurement input, not permission to restart
          // their agents. Keep the native WebSocket and every load/reply real;
          // suppress only the unrelated startup restore acknowledgement.
          const nativeSend = WebSocket.prototype.send;
          WebSocket.prototype.send = function(data) {
            if (typeof data === "string") {
              try {
                if (JSON.parse(data).kind === "startup_auto_resume_ready") {
                  (window as any).__gwtKnowledgeSuppressedRestores = ((window as any).__gwtKnowledgeSuppressedRestores ?? 0) + 1;
                  return;
                }
              } catch { /* non-JSON frames pass through unchanged */ }
            }
            return nativeSend.call(this, data);
          };
        });
        await gotoLiveGwt(page, BASE, { enableTestBridge: true, projectKey: fixture.repo_hash });
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await expect(page.locator("#close-project-button")).toBeVisible();
        // The real corpus can still be hydrating after the static shell and
        // test bridge become visible. Wait for this Project's authoritative
        // workspace before invoking its Settings listener.
        await page.waitForFunction(({ project, hash }) => ((window as any).__gwtPlaywrightMessages ?? [])
          .some(({ payload }: any) => payload.kind === "workspace_state" && payload.workspace?.tabs?.some((tab: any) =>
            tab.project_key === hash || tab.project_root === project)),
        { project: fixture.project, hash: fixture.repo_hash }, { timeout: 180_000 });
        await mkdir(gateDirectory, { recursive: true });
        for (const name of ["started", "release", "resumed", "completed"]) await rm(join(gateDirectory, name), { force: true });
        await writeFile(join(gateDirectory, "work-items-path"), `${join(state, "works.json")}\n`);
        phase = "first";
        // Issue-family presets are canonicalized to Settings. Use the actual
        // Settings Issues entrypoint instead of looking for a retired window.
        await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "issues" } })));
        const issueWindow = page.locator('.workspace-window[data-preset="settings"]').last();
        await expect(issueWindow).toBeVisible();
        await expect(issueWindow.locator('[data-settings-tab="issues"]')).toHaveAttribute("aria-selected", "true");
        const issueSurface = issueWindow.locator('[data-settings-panel="issues"] .surface-knowledge');
        await expect(issueSurface).toBeVisible();
        windowId = await issueWindow.getAttribute("data-id") ?? undefined;
        expect(windowId).toBeTruthy();
        await expect.poll(() => samples.find(sample => sample.id === windowId && sample.phase === "first"), {
          timeout: 10_000, intervals: [100], message: "the real Issue surface initiates its first cache load",
        }).toBeDefined();
        const first = samples.find(sample => sample.id === windowId && sample.phase === "first")!;
        firstSample = first;
        measured.push(first);
        try {
          await expect.poll(() => readFile(join(gateDirectory, "started"), "utf8").then(value => value.trim()).catch(() => ""), {
            timeout: ENRICHMENT_TIMEOUT_MS, intervals: [100, 250], message: "the actual copied Work file decode enters its rendezvous after Sessions are read",
          }).toBe("started");
          gateEvidence.started_at_ms = now();
          if (fixture.label === "after") {
            await expect(issueSurface.locator(".knowledge-row").first(), "cache rows appear while Work decode is held").toBeVisible({ timeout: 20_000 });
            expect(first.status, "the first real reply does not wait for Work decode").toBe("success");
            const gateAge = Date.now() - (await stat(join(gateDirectory, "started"))).mtimeMs;
            expect(gateAge, "the first UI rows precede the rendezvous's automatic thirty-second release").toBeLessThan(28_000);
            gateEvidence.hold_age_at_first_rows_ms = gateAge;
            await issueWindow.evaluate(element => {
              (window as any).__gwtKnowledgeInitialRows = new Map([...element.querySelectorAll<HTMLElement>(".knowledge-row")]
                .map(row => [Number(row.dataset.issueNumber), row]));
            });
          } else {
            // Baseline preserves the observed dependency as a RED measurement.
            await new Promise(resolve => setTimeout(resolve, 1000));
          }
        } finally {
          gateEvidence.first_response_before_release = first.status === "success";
          gateEvidence.rows_visible_before_release = await issueSurface.locator(".knowledge-row").first().isVisible().catch(() => false);
          gateEvidence.independence_pass = gateEvidence.first_response_before_release && gateEvidence.rows_visible_before_release;
          gateEvidence.released_at_ms = now();
          await writeFile(join(gateDirectory, "release"), "release\n");
        }
        await awaitSamples([first]);
        if (fixture.label === "after") {
          phase = "enrichment";
          await expect.poll(() => receipts.filter(receipt => receipt.id === first.id && receipt.request_id === first.request_id).length, {
            timeout: ENRICHMENT_TIMEOUT_MS, intervals: [100, 500], message: "the same first request receives its later enrichment reply",
          }).toBeGreaterThanOrEqual(2);
          const enriched = receipts.filter(receipt => receipt.id === first.id && receipt.request_id === first.request_id).at(-1)!;
          expect(enriched.related_rows.length, "the copied Work and Sessions enrich at least one Issue").toBeGreaterThan(0);
          const initial = receipts.find(receipt => receipt.id === first.id && receipt.request_id === first.request_id)!;
          const changed = enriched.related_rows.filter(row => {
            const old = initial.related_rows.find(previous => previous.number === row.number);
            return !old || old.works !== row.works || old.sessions !== row.sessions;
          });
          await expect.poll(() => page.evaluate(({ id, rows, requireChange }) => {
            const host = [...document.querySelectorAll<HTMLElement>(".workspace-window")].find(element => element.dataset.id === id);
            for (const expected of rows) {
              const row = host?.querySelector(`.knowledge-row[data-issue-number="${expected.number}"]`);
              if (!row) continue;
              // Current Issue cards derive their visible state from Work;
              // related counts participate in the keyed row version. A changed
              // receipt must update a mounted row, even without count badges.
              const previous = (window as any).__gwtKnowledgeInitialRows?.get(expected.number);
              if (!requireChange || row !== previous) return expected.number;
            }
            return null;
          }, { id: first.id, rows: changed.length ? changed : enriched.related_rows, requireChange: changed.length > 0 }), {
            timeout: 10_000, intervals: [100, 500], message: "the later related counts reach the actual UI instead of being discarded",
          }).not.toBeNull();
          gateEvidence.enrichment_received_at_ms = enriched.at_ms;
          gateEvidence.enrichment_ui_confirmed = true;
        }
        phase = "warm";
        await sendLoads(1);
        const observationStarted = performance.now();
        let round = 0;
        do {
          phase = `parallel-${++round}`;
          await sendLoads(12, true);
          const remaining = effectiveObserveSeconds * 1000 - (performance.now() - observationStarted);
          // A long soak samples bounded bursts instead of continuously
          // submitting enrichment work faster than the real corpus can settle.
          if (remaining > 0) await new Promise(resolve => setTimeout(resolve, Math.min(60_000, remaining)));
        } while (round < rounds || performance.now() - observationStarted < effectiveObserveSeconds * 1000);
        phase = "assertions";
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        expect(errors, "zero console and page errors").toEqual([]);
        expect(new Set(measured.map(sample => key(sample.id, sample.request_id))).size,
          "every measured load has its own request_id").toBe(measured.length);
        if (fixture.label === "after") {
          expect(measured.filter(sample => sample.status === "timeout"), "no sixty-second load timeouts").toEqual([]);
          expect(measured.filter(sample => sample.status === "error"), "no backend load errors").toEqual([]);
          for (const sample of measured) {
            expect(sample.status).toBe("success");
            expect(sample.entry_count, "copied real Issue cache produces usable rows").toBeGreaterThan(0);
          }
          for (const control of controls) {
            expect(control.response_at_ms, "list_windows responds during each parallel batch").toBeDefined();
            expect(control.pending_loads, "the control reply precedes pending cache loads").toBeGreaterThan(0);
          }
          await expect.poll(async () => {
            phaseRecords = await loadPhaseRecords([first], fixture.repo_hash, logStartedAtMs);
            return [first].flatMap(sample => requiredStages.filter(stage => !phaseRecords.some(record =>
              record.window_id === sample.id && record.request_id === sample.request_id &&
              record.stage === stage && record.status === "completed" &&
              [record.elapsed_ms, record.duration_ms].every(value => typeof value === "number" && Number.isFinite(value) && value >= 0)))
              .map(stage => `${sample.id}:${sample.request_id}:${stage}`));
          }, { timeout: 10_000, intervals: [100, 500], message: "the first real request has elapsed and duration evidence for all six backend stages" }).toEqual([]);
          await expect(issueWindow.locator(".knowledge-row").first()).toBeVisible();
          await expect(issueWindow.getByRole("button", { name: /^Issue load failed/ })).toHaveCount(0);
          timeoutLedgerCount = await countTimeoutLedgerRows();
          expect(timeoutLedgerCount, "the isolated error ledger contains no Issue-window load timeouts").toBe(0);
        }
        const screenshot = info.outputPath(`knowledge-load-${fixture.label}-${theme}.png`);
        await issueWindow.screenshot({ path: screenshot });
        await info.attach(`knowledge-load-${theme}`, { path: screenshot, contentType: "image/png" });
        verified = true;
      } finally {
        await writeFile(join(gateDirectory, "release"), "release\n").catch(() => undefined);
        for (const record of pending.values()) clearTimeout(record.timer);
        if (fixture.label === "after") phaseRecords = await loadPhaseRecords(firstSample ? [firstSample] : [], fixture.repo_hash, logStartedAtMs);
        timeoutLedgerCount = await countTimeoutLedgerRows().catch(() => null);
        diagnostics = await page.evaluate(() => {
          const safeWindow = (item: any) => ({ id: item.id, preset: item.preset, status: item.status });
          const messages = ((window as any).__gwtPlaywrightMessages ?? []).map(({ sequence, payload }: any) => ({
            sequence, kind: payload.kind,
            workspace: payload.kind === "workspace_state" ? {
              active_tab_id: payload.workspace?.active_tab_id,
              tabs: payload.workspace?.tabs?.map((tab: any) => ({ id: tab.id, project_root: tab.project_root,
                project_key: tab.project_key, windows: tab.workspace?.windows?.map(safeWindow) })),
            } : undefined,
            windows: payload.kind === "window_list" ? payload.windows?.map(safeWindow) : undefined,
          }));
          return { pathname: location.pathname, bridge_installed: (window as any).__gwtPlaywrightTestBridgeInstalled,
            suppressed_startup_restore_acknowledgements: (window as any).__gwtKnowledgeSuppressedRestores ?? 0,
            settings_module_loaded: performance.getEntriesByType("resource").some(entry => new URL(entry.name).pathname === "/settings-surface.js"),
            sockets: ((window as any).__gwtPlaywrightSockets ?? []).map((socket: WebSocket) => ({
              pathname: new URL(socket.url).pathname, repo_hash: new URL(socket.url).searchParams.get("repo_hash"), ready_state: socket.readyState })),
            windows: [...document.querySelectorAll<HTMLElement>(".workspace-window")].map(element => ({
              id: element.dataset.id, preset: element.dataset.preset, settings_section: element.querySelector<HTMLElement>("[data-settings-section]")?.dataset.settingsSection })),
            settings_tabs: document.querySelectorAll("[data-settings-tab]").length, messages };
        }).catch(() => null);
        if (!verified) {
          const screenshot = info.outputPath(`knowledge-load-failure-${theme}.png`);
          await page.screenshot({ path: screenshot }).then(() => info.attach(`knowledge-load-failure-${theme}`,
            { path: screenshot, contentType: "image/png" })).catch(() => undefined);
        }
        const evidence = { issue: 5543, label: fixture.label, theme, owned_pid: fixture.pid,
          project: fixture.project, checkout: fixture.checkout, seed: fixture.seed,
          phase, verified, log_started_at_ms: logStartedAtMs,
          request_timeout_ms: REQUEST_TIMEOUT_MS, requested_rounds: rounds,
          requested_observation_seconds: observeSeconds, elapsed_ms: now(),
          effective_theme_observation_seconds: effectiveObserveSeconds,
          workspace_ready_ms: eventKinds.find(event => event.direction === "received" && event.kind === "workspace_state")?.at_ms,
          samples: measured, all_load_samples: samples, controls, frames, phase_records: phaseRecords,
          work_decode_gate: gateEvidence,
          first_request_receipts: receipts.filter(receipt => receipt.id === firstSample?.id && receipt.request_id === firstSample?.request_id),
          diagnostics,
          protocol_event_kinds: eventKinds,
          console_and_page_errors: errors,
          issue_window_timeout_ledger_count: timeoutLedgerCount,
          user_verification_result: "n/a (autonomous)",
          agent_visual_check: fixture.label === "after" ? (verified ? "pass" : "fail") : "measurement only" };
        const path = info.outputPath(`knowledge-load-${fixture.label}-${theme}.json`);
        await writeFile(path, JSON.stringify(evidence, null, 2));
        await info.attach(`knowledge-load-${theme}`, { path, contentType: "application/json" });
        console.log(JSON.stringify({ issue: 5543, label: fixture.label, theme, pass: verified, elapsed_ms: now(),
          load_status_counts: Object.fromEntries(["success", "error", "timeout", "pending"].map(status =>
            [status, measured.filter(sample => sample.status === status).length])),
          ledger_timeout_count: timeoutLedgerCount, artifact: path }));
        if (windowId) await sendLiveGwtEvent(page, { kind: "close_window", id: windowId }).catch(() => undefined);
      }
    });
  });
});

function positiveInteger(value: string | undefined, fallback: number): number {
  if (value === undefined) return fallback;
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 0) throw new Error("load observation settings must be nonnegative integers");
  return parsed;
}

async function loadPhaseRecords(samples: Sample[], repoHash: string, notBeforeMs: number): Promise<PhaseRecord[]> {
  const wanted = new Set(samples.map(sample => `${sample.id}:${sample.request_id}`));
  const records: PhaseRecord[] = [];
  for (const directory of [join(CHECK_HOME, ".gwt/logs"), join(CHECK_HOME, ".gwt/projects", repoHash, "logs")]) {
    const files = await readdir(directory, { withFileTypes: true }).catch(() => []);
    for (const file of files) {
      if (!file.isFile() || !file.name.startsWith("gwt.log")) continue;
      const log = await readFile(join(directory, file.name), "utf8").catch(() => "");
      for (const line of log.split(/\r?\n/)) {
        let row: any;
        try { row = JSON.parse(line); } catch { continue; }
        if (row.target !== "gwt::knowledge_load") continue;
        // Browser contexts restart request IDs; the preceding theme shares
        // this GUI's logs but cannot provide evidence for the current theme.
        const recorded_at_ms = Date.parse(row.timestamp);
        if (!(recorded_at_ms >= notBeforeMs)) continue;
        const identity = Object.assign({}, ...(row.spans ?? []), row.span ?? {}, row.fields ?? {});
        // tracing's optional u64 span field may be rendered as Some(N).
        const match = /^(?:Some\()?([0-9]+)\)?$/.exec(String(identity.request_id));
        if (!match) continue;
        const request_id = Number(match[1]);
        const window_id = String(identity.window_id ?? "");
        if (!wanted.has(`${window_id}:${request_id}`)) continue;
        const fields = row.fields ?? {};
        records.push({ window_id, request_id, recorded_at_ms, stage: String(fields.stage ?? ""), status: String(fields.status ?? ""),
          elapsed_ms: fields.elapsed_ms === undefined ? undefined : Number(fields.elapsed_ms),
          duration_ms: fields.duration_ms === undefined ? undefined : Number(fields.duration_ms) });
      }
    }
  }
  return records;
}

async function countTimeoutLedgerRows(): Promise<number> {
  const directory = join(CHECK_HOME, ".gwt/logs/errors");
  const files = await readdir(directory, { withFileTypes: true }).catch(error => {
    if (error.code === "ENOENT") return [];
    throw error;
  });
  let count = 0;
  for (const file of files) {
    if (!file.isFile() || !/^errors\..*\.jsonl$/.test(file.name)) continue;
    for (const line of (await readFile(join(directory, file.name), "utf8")).split(/\r?\n/)) {
      let row: any;
      try { row = JSON.parse(line); } catch { continue; }
      if (row.message === "Issue window load failed: Timed out loading cache-backed data") count += 1;
    }
  }
  return count;
}
