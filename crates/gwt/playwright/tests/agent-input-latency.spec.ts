import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const ACTIVE_WINDOW_ID = "latency-active";
const BUSY_WINDOW_IDS = Array.from(
  { length: 8 },
  (_, index) => `latency-busy-${index + 1}`,
);
const SAMPLE_KEYS = "abcdefghijklmnopqrstuvwxyz012345678";
const WARMUP_SAMPLES = 5;
const MEASURED_SAMPLES = 30;
const PRELOAD_ROUNDS = 96;
const PRELOAD_CHUNK_BYTES = 256;
const PRIVACY_INPUT =
  "typed_text_LATENCY_SENTINEL credential_LATENCY_SENTINEL env_secret_LATENCY_SENTINEL";

type LatencySummary = {
  droppedEntries: number;
  exactFieldsOk: boolean;
  invalidInputCount: number;
  inputWhileBusyCount: number;
  longTaskOverBudgetCount: number;
  maxMs: number;
  orderingOk: boolean;
  p95Ms: number;
  privacyOk: boolean;
  rafGapOverBudgetCount: number;
  sampleCount: number;
  segmentP95Ms: Record<string, number>;
  sequenceIntegrityOk: boolean;
  stateBacklogEventCount: number;
  stageCounts: Record<string, number>;
  uniqueEchoCount: number;
};

type BusyLoadEvidence = {
  inputWhileBusyCount: number;
  measuredRounds: number;
  minBaseYDelta: number;
  producerRounds: number;
  progressedWindowCount: number;
  running: boolean;
};

test.setTimeout(90_000);
test.use({
  deviceScaleFactor: 1,
  // macOS headless Chromium throttles rAF to roughly 7.5 Hz even with no
  // long tasks. A headed context keeps this wall-clock acceptance comparable
  // to the hardware-accelerated WebView; Linux CI remains headless.
  headless: process.platform !== "darwin",
  // The measured terminal is the active foreground surface. Keep an
  // automation window that happens to be occluded by another app from being
  // renderer-throttled into a false 10–20 Hz result.
  launchOptions: {
    args: [
      "--disable-background-timer-throttling",
      "--disable-backgrounding-occluded-windows",
      "--disable-features=CalculateNativeWinOcclusion",
      "--disable-renderer-backgrounding",
    ],
  },
  screenshot: "off",
  trace: "off",
  video: "off",
  viewport: { width: 1600, height: 1000 },
});

test.describe("agent input latency", () => {
  test.describe.configure({ retries: 0 });

  test("prioritizes the active ninth shell without starving eight busy peers", async ({
    page,
  }, testInfo) => {
    test.skip(testInfo.project.name !== "chromium-dark");

    await installEmbeddedRoutes(page);
    await installLatencyBackend(page);
    await page.goto(APP_URL);
    await page.bringToFront();
    await expect
      .poll(() => maxAnimationFrameGap(page, 8), { timeout: 15_000 })
      .toBeLessThan(50);

    await runLatencyScenario(page, testInfo);
  });
});

async function installLatencyBackend(page: Page): Promise<void> {
  await page.addInitScript(
    ({
      activeWindowId,
      busyWindowIds,
      measuredSamples,
      preloadChunkBytes,
      preloadRounds,
      sampleKeys,
    }) => {
      window.__gwtPlaywrightTestBridge = true;

      const fixtureWindow = window as typeof window & {
        __gwtLatencyFixture?: Record<string, unknown>;
      };
      const privacyInput =
        "typed_text_LATENCY_SENTINEL credential_LATENCY_SENTINEL env_secret_LATENCY_SENTINEL";
      const privacyOutput =
        "output_bytes_LATENCY_SENTINEL data_base64_LATENCY_SENTINEL";
      const preloadFence = "latency-preload-fence";
      let grouped = false;
      let invalidInputCount = 0;
      let inputWhileBusyCount = 0;
      let privacyProbeCount = 0;
      let sampleEchoCount = 0;
      let stateBacklogEventCount = 0;
      let savedSummary: LatencySummary | null = null;
      const uniqueEchoes = new Set<string>();
      const forbiddenTraceValues = new Set([
        privacyInput,
        btoa(privacyInput),
        privacyOutput,
        btoa(privacyOutput),
        "credential_LATENCY_SENTINEL",
        "env_secret_LATENCY_SENTINEL",
      ]);
      const busyFramePayload = btoa(
        `${"x".repeat(Math.max(0, preloadChunkBytes - 2))}\r\n`,
      );
      let busyProducerFrame: number | null = null;
      let busyProducerRounds = 0;
      let busyProducerRunning = false;
      let measuredProducerStartRound = 0;
      let measuredBaseY = new Map<string, number>();

      function terminalWindow(id: string, index: number) {
        const column = index % 3;
        const row = Math.floor(index / 3);
        return {
          id,
          title:
            id === activeWindowId
              ? "Latency Active"
              : `Latency Busy ${index + 1}`,
          preset: "shell",
          geometry: {
            x: 70 + column * 500,
            y: 70 + row * 290,
            width: 460,
            height: 250,
          },
          geometry_revision: 0,
          z_index: index + 1,
          status: "running",
          minimized: false,
          maximized: false,
          pre_maximize_geometry: null,
          persist: true,
          purpose_title: null,
          dynamic_title: null,
          dynamic_title_detail: null,
          agent_id: null,
          agent_color: null,
          tab_group_id: grouped ? "latency-hidden-busy" : null,
          tab_group_active: grouped && id === activeWindowId,
        };
      }

      function workspaceState() {
        const windowIds = [...busyWindowIds, activeWindowId];
        return {
          kind: "workspace_state",
          workspace: {
            app_version: "playwright",
            tabs: [
              {
                id: "latency-tab",
                title: "Terminal Latency",
                project_root: "/fixture/terminal-latency",
                kind: "git",
                workspace: {
                  viewport: { x: 0, y: 0, zoom: 1 },
                  windows: windowIds.map(terminalWindow),
                },
              },
            ],
            active_tab_id: "latency-tab",
            recent_projects: [],
          },
        };
      }

      const activeWorkProjection = {
        kind: "active_work_projection",
        projection: {
          id: "latency-workspace",
          title: "Terminal latency fixture",
          status_category: "active",
          status_text: "Controlled terminal backlog",
          summary: "Controlled terminal backlog",
          owner: "SPEC-3170",
          branch: "work/issue-3170",
          workspaces: [],
          unassigned_agents: [],
          events: [],
        },
      };

      function terminalTestApi() {
        return (
          window as typeof window & {
            __gwtTerminalTestApi?: {
              enqueueSyntheticOutput(windowId: string, base64: string): void;
              metrics(windowId: string): { baseY: number };
            };
          }
        ).__gwtTerminalTestApi;
      }

      function injectStateBacklog() {
        for (let index = 0; index < 4; index += 1) {
          fixture.instance?.emitSync(workspaceState());
          fixture.instance?.emitSync(activeWorkProjection);
          stateBacklogEventCount += 2;
        }
      }

      function produceBusyOutput() {
        if (!busyProducerRunning) {
          return;
        }
        const terminalApi = terminalTestApi();
        if (!terminalApi) {
          invalidInputCount += 1;
          busyProducerRunning = false;
          busyProducerFrame = null;
          return;
        }
        busyProducerRounds += 1;
        for (const windowId of busyWindowIds) {
          terminalApi.enqueueSyntheticOutput(windowId, busyFramePayload);
        }
        busyProducerFrame = requestAnimationFrame(produceBusyOutput);
      }

      function summarizeTrace(trace: {
        dropped_entries?: number;
        entries?: Array<Record<string, unknown>>;
      }): LatencySummary {
        const entries = Array.isArray(trace?.entries) ? trace.entries : [];
        const expectedTraceSamples = sampleKeys.length + 1;
        const outputStages = [
          "terminal_output_ws_receive",
          "terminal_output_enqueue",
          "terminal_output_flush_start",
          "terminal_output_write_complete",
          "terminal_output_next_paint",
        ];
        const markerKinds = new Set([
          "terminal_input_enqueue",
          ...outputStages,
        ]);
        const markerEntries = entries.filter(
          (entry) =>
            markerKinds.has(String(entry.kind ?? "")) &&
            entry.window_id === activeWindowId,
        );
        const exactFieldsOk = markerEntries.every((entry) => {
          const keys = Object.keys(entry).sort();
          return (
            keys.length === 4 &&
            keys[0] === "kind" &&
            keys[1] === "sequence" &&
            keys[2] === "ts" &&
            keys[3] === "window_id"
          );
        });
        const stageCounts = Object.fromEntries(
          ["terminal_input_enqueue", ...outputStages].map((kind) => [
            kind,
            markerEntries.filter((entry) => entry.kind === kind).length,
          ]),
        );
        const inputEntries = markerEntries.filter(
          (entry) => entry.kind === "terminal_input_enqueue",
        );
        const inputSequences = inputEntries.map((entry) => entry.sequence);
        const inputSequenceIntegrity =
          inputEntries.length === expectedTraceSamples &&
          inputSequences.every(
            (sequence) =>
              typeof sequence === "number" &&
              Number.isSafeInteger(sequence) &&
              sequence > 0,
          ) &&
          new Set(inputSequences).size === inputSequences.length &&
          inputSequences.every(
            (sequence, index) =>
              index === 0 ||
              Number(sequence) > Number(inputSequences[index - 1]),
          );
        const outputBySequence = new Map<
          string,
          { sequence: unknown; stages: Map<string, number[]> }
        >();
        for (const entry of markerEntries) {
          const kind = String(entry.kind ?? "");
          if (!outputStages.includes(kind)) {
            continue;
          }
          const sequence = String(entry.sequence ?? "");
          const sample = outputBySequence.get(sequence) ?? {
            sequence: entry.sequence,
            stages: new Map<string, number[]>(),
          };
          const timestamps = sample.stages.get(kind) ?? [];
          timestamps.push(Number(entry.ts));
          sample.stages.set(kind, timestamps);
          outputBySequence.set(sequence, sample);
        }
        const orderedOutputSamples = [...outputBySequence.values()].sort(
          (left, right) =>
            Number(left.stages.get(outputStages[0])?.[0]) -
            Number(right.stages.get(outputStages[0])?.[0]),
        );
        const outputSequences = orderedOutputSamples.map(
          (sample) => sample.sequence,
        );
        const outputSequenceIntegrity =
          orderedOutputSamples.length === expectedTraceSamples &&
          orderedOutputSamples.every((sample) =>
            outputStages.every(
              (stage) => sample.stages.get(stage)?.length === 1,
            ),
          ) &&
          outputSequences.every(
            (sequence) =>
              typeof sequence === "number" &&
              Number.isSafeInteger(sequence) &&
              sequence > 0,
          ) &&
          new Set(outputSequences).size === outputSequences.length &&
          outputSequences.every(
            (sequence, index) =>
              index === 0 ||
              Number(sequence) > Number(outputSequences[index - 1]),
          );
        const sequenceIntegrityOk =
          inputSequenceIntegrity && outputSequenceIntegrity;
        const orderingOk = orderedOutputSamples.every((sample) => {
          const timestamps = outputStages.map(
            (stage) => sample.stages.get(stage)?.[0],
          );
          return (
            timestamps.every((timestamp) => Number.isFinite(timestamp)) &&
            timestamps.every(
              (timestamp, index) =>
                index === 0 ||
                Number(timestamp) >= Number(timestamps[index - 1]),
            )
          );
        });
        const measuredInputs = inputEntries.slice(-measuredSamples);
        const measuredOutputs = orderedOutputSamples.slice(-measuredSamples);
        const latencies = measuredInputs
          .map(
            (entry, index) =>
              Number(
                measuredOutputs[index]?.stages.get(
                  "terminal_output_next_paint",
                )?.[0],
              ) - Number(entry.ts),
          )
          .filter((latency) => Number.isFinite(latency) && latency >= 0);
        const sortedLatencies = latencies
          .slice()
          .sort((left, right) => left - right);
        const p95Index = Math.max(
          0,
          Math.ceil(sortedLatencies.length * 0.95) - 1,
        );
        const segmentSamples: Record<string, number[]> = {
          input_to_ws_receive: [],
          ws_receive_to_enqueue: [],
          enqueue_to_flush: [],
          flush_to_write_complete: [],
          write_complete_to_paint: [],
        };
        for (let index = 0; index < measuredInputs.length; index += 1) {
          const stages = measuredOutputs[index]?.stages;
          const points = [
            Number(measuredInputs[index]?.ts),
            Number(stages?.get("terminal_output_ws_receive")?.[0]),
            Number(stages?.get("terminal_output_enqueue")?.[0]),
            Number(stages?.get("terminal_output_flush_start")?.[0]),
            Number(stages?.get("terminal_output_write_complete")?.[0]),
            Number(stages?.get("terminal_output_next_paint")?.[0]),
          ];
          const names = Object.keys(segmentSamples);
          for (let pointIndex = 0; pointIndex < names.length; pointIndex += 1) {
            const duration = points[pointIndex + 1] - points[pointIndex];
            if (Number.isFinite(duration) && duration >= 0) {
              segmentSamples[names[pointIndex]].push(duration);
            }
          }
        }
        const segmentP95Ms = Object.fromEntries(
          Object.entries(segmentSamples).map(([name, values]) => {
            const sorted = values.slice().sort((left, right) => left - right);
            const index = Math.max(0, Math.ceil(sorted.length * 0.95) - 1);
            return [name, sorted[index] ?? Number.POSITIVE_INFINITY];
          }),
        );
        const serialized = JSON.stringify(trace);
        return {
          droppedEntries: Number(trace?.dropped_entries ?? -1),
          exactFieldsOk,
          invalidInputCount,
          inputWhileBusyCount,
          longTaskOverBudgetCount: entries.filter(
            (entry) =>
              entry.kind === "long_task" && Number(entry.duration_ms ?? 0) > 50,
          ).length,
          maxMs:
            sortedLatencies.length > 0
              ? sortedLatencies[sortedLatencies.length - 1]
              : Number.POSITIVE_INFINITY,
          orderingOk,
          p95Ms:
            sortedLatencies.length > 0
              ? sortedLatencies[p95Index]
              : Number.POSITIVE_INFINITY,
          privacyOk: [...forbiddenTraceValues].every(
            (value) => !serialized.includes(value),
          ),
          rafGapOverBudgetCount: entries.filter(
            (entry) =>
              entry.kind === "raf_gap" && Number(entry.gap_ms ?? 0) > 100,
          ).length,
          sampleCount: sortedLatencies.length,
          segmentP95Ms,
          sequenceIntegrityOk,
          stateBacklogEventCount,
          stageCounts,
          uniqueEchoCount: uniqueEchoes.size,
        };
      }

      class FixtureWebSocket extends EventTarget {
        static CONNECTING = 0;
        static OPEN = 1;
        static CLOSING = 2;
        static CLOSED = 3;

        readyState = FixtureWebSocket.CONNECTING;
        url: string;

        constructor(url: string) {
          super();
          this.url = url;
          fixture.instance = this;
          setTimeout(() => {
            this.readyState = FixtureWebSocket.OPEN;
            this.dispatchEvent(new Event("open"));
          }, 0);
        }

        send(raw: string) {
          let message: Record<string, unknown>;
          try {
            message = JSON.parse(raw);
          } catch {
            return;
          }
          if (message.kind === "frontend_ready") {
            this.emitSync(workspaceState());
            return;
          }
          if (message.kind === "save_ui_trace") {
            savedSummary = summarizeTrace(
              (message.trace ?? {}) as Parameters<typeof summarizeTrace>[0],
            );
            return;
          }
          if (message.kind !== "terminal_input") {
            return;
          }
          if (
            message.id !== activeWindowId ||
            typeof message.data !== "string"
          ) {
            invalidInputCount += 1;
            return;
          }
          injectStateBacklog();
          if (busyProducerRunning) {
            inputWhileBusyCount += 1;
          }
          const input = message.data;
          if (input.length === 1 && sampleKeys.includes(input)) {
            const expectedKey = sampleKeys[sampleEchoCount];
            if (input !== expectedKey) {
              invalidInputCount += 1;
            }
            const echo = `echo-${String(sampleEchoCount).padStart(2, "0")}-${input}`;
            const encodedEcho = btoa(echo);
            sampleEchoCount += 1;
            uniqueEchoes.add(echo);
            forbiddenTraceValues.add(echo);
            forbiddenTraceValues.add(encodedEcho);
            this.emitSync({
              kind: "terminal_output",
              id: activeWindowId,
              data_base64: encodedEcho,
            });
            return;
          }
          if (input === privacyInput) {
            privacyProbeCount += 1;
            this.emitSync({
              kind: "terminal_output",
              id: activeWindowId,
              data_base64: btoa(privacyOutput),
            });
            return;
          }
          invalidInputCount += 1;
        }

        close() {
          this.readyState = FixtureWebSocket.CLOSED;
          this.dispatchEvent(new CloseEvent("close"));
        }

        emitSync(payload: unknown) {
          this.dispatchEvent(
            new MessageEvent("message", { data: JSON.stringify(payload) }),
          );
        }
      }

      function busyLoadEvidence(): BusyLoadEvidence {
        const terminalApi = terminalTestApi();
        const baseYDeltas = busyWindowIds.map((windowId) => {
          const baseline = measuredBaseY.get(windowId);
          const current = terminalApi?.metrics(windowId)?.baseY;
          if (!Number.isFinite(baseline) || !Number.isFinite(current)) {
            return -1;
          }
          return Number(current) - Number(baseline);
        });
        return {
          inputWhileBusyCount,
          measuredRounds: busyProducerRounds - measuredProducerStartRound,
          minBaseYDelta: Math.min(...baseYDeltas),
          producerRounds: busyProducerRounds,
          progressedWindowCount: baseYDeltas.filter((delta) => delta > 0)
            .length,
          running: busyProducerRunning,
        };
      }

      const fixture = {
        instance: null as FixtureWebSocket | null,
        beginMeasuredBusyProgress() {
          const terminalApi = terminalTestApi();
          measuredProducerStartRound = busyProducerRounds;
          measuredBaseY = new Map(
            busyWindowIds.map((windowId) => [
              windowId,
              Number(terminalApi?.metrics(windowId)?.baseY),
            ]),
          );
        },
        busyLoadEvidence,
        collapseBusy() {
          grouped = true;
          this.instance?.emitSync(workspaceState());
        },
        echoCounts() {
          return { privacyProbeCount, sampleEchoCount };
        },
        preloadBusy() {
          const filler = btoa("x".repeat(preloadChunkBytes));
          for (let round = 0; round < preloadRounds; round += 1) {
            for (const windowId of busyWindowIds) {
              this.instance?.emitSync({
                kind: "terminal_output",
                id: windowId,
                data_base64: filler,
              });
            }
          }
          this.instance?.emitSync({
            kind: "terminal_status",
            id: busyWindowIds[busyWindowIds.length - 1],
            status: "running",
            detail: preloadFence,
          });
        },
        preloadFence,
        revealBusy() {
          grouped = false;
          this.instance?.emitSync(workspaceState());
        },
        startBusyOutput() {
          if (busyProducerRunning) {
            return;
          }
          if (!terminalTestApi()) {
            throw new Error("terminal test bridge is unavailable");
          }
          busyProducerRounds = 0;
          measuredProducerStartRound = 0;
          measuredBaseY = new Map(
            busyWindowIds.map((windowId) => [
              windowId,
              Number(terminalTestApi()?.metrics(windowId)?.baseY),
            ]),
          );
          busyProducerRunning = true;
          produceBusyOutput();
        },
        stopBusyOutput() {
          busyProducerRunning = false;
          if (busyProducerFrame !== null) {
            cancelAnimationFrame(busyProducerFrame);
          }
          busyProducerFrame = null;
        },
        summary() {
          return savedSummary;
        },
      };
      fixtureWindow.__gwtLatencyFixture = fixture;

      Object.defineProperty(window, "WebSocket", {
        configurable: true,
        value: FixtureWebSocket,
      });
    },
    {
      activeWindowId: ACTIVE_WINDOW_ID,
      busyWindowIds: BUSY_WINDOW_IDS,
      measuredSamples: MEASURED_SAMPLES,
      preloadChunkBytes: PRELOAD_CHUNK_BYTES,
      preloadRounds: PRELOAD_ROUNDS,
      sampleKeys: SAMPLE_KEYS,
    },
  );
}

async function runLatencyScenario(
  page: Page,
  testInfo: TestInfo,
): Promise<void> {
  expect(SAMPLE_KEYS).toHaveLength(WARMUP_SAMPLES + MEASURED_SAMPLES);
  expect(BUSY_WINDOW_IDS).toHaveLength(8);

  const allWindowIds = [...BUSY_WINDOW_IDS, ACTIVE_WINDOW_ID];
  await expect
    .poll(
      () =>
        page.evaluate((windowIds) => {
          const terminalApi = (
            window as typeof window & {
              __gwtTerminalTestApi?: {
                metrics(id: string): {
                  hasRuntime: boolean;
                  isReady: boolean | null;
                };
              };
            }
          ).__gwtTerminalTestApi;
          return windowIds.every((id) => {
            const metrics = terminalApi?.metrics(id);
            return metrics?.hasRuntime === true && metrics.isReady === true;
          });
        }, allWindowIds),
      { timeout: 20_000 },
    )
    .toBe(true);

  await page.evaluate(() => {
    (
      window as typeof window & {
        __gwtLatencyFixture?: { collapseBusy(): void };
      }
    ).__gwtLatencyFixture?.collapseBusy();
  });
  for (const windowId of BUSY_WINDOW_IDS) {
    await expect(
      page.locator(`.workspace-window[data-id='${windowId}']`),
    ).toBeHidden();
  }

  await page.evaluate(() => {
    (
      window as typeof window & {
        __gwtLatencyFixture?: { preloadBusy(): void };
      }
    ).__gwtLatencyFixture?.preloadBusy();
  });
  await expect(
    page.locator(
      `.workspace-window[data-id='${BUSY_WINDOW_IDS[BUSY_WINDOW_IDS.length - 1]}'] .status-chip`,
    ),
  ).toHaveAttribute("title", /latency-preload-fence/, { timeout: 20_000 });

  await runPaletteCommand(page, "Start UI Trace");
  await page.evaluate(() => {
    (
      window as typeof window & {
        __gwtLatencyFixture?: { startBusyOutput(): void };
      }
    ).__gwtLatencyFixture?.startBusyOutput();
  });
  await expect
    .poll(async () => (await readBusyLoadEvidence(page)).producerRounds, {
      timeout: 5_000,
    })
    .toBeGreaterThanOrEqual(2);
  await page.evaluate(() => {
    (
      window as typeof window & {
        __gwtLatencyFixture?: { revealBusy(): void };
      }
    ).__gwtLatencyFixture?.revealBusy();
  });
  for (const windowId of BUSY_WINDOW_IDS) {
    await expect(
      page.locator(`.workspace-window[data-id='${windowId}']`),
    ).toBeVisible();
  }

  const activeInput = page.locator(
    `.workspace-window[data-id='${ACTIVE_WINDOW_ID}'] .xterm-helper-textarea`,
  );
  await activeInput.focus();
  await page.keyboard.insertText(PRIVACY_INPUT);
  await expect
    .poll(() => fixtureEchoCounts(page), { timeout: 5_000 })
    .toMatchObject({ privacyProbeCount: 1 });

  for (let index = 0; index < SAMPLE_KEYS.length; index += 1) {
    await activeInput.focus();
    await page.keyboard.press(SAMPLE_KEYS[index]);
    await expect
      .poll(() => fixtureEchoCounts(page), { timeout: 5_000 })
      .toMatchObject({ sampleEchoCount: index + 1 });
    await expect
      .poll(
        () =>
          terminalContainsText(
            page,
            ACTIVE_WINDOW_ID,
            `echo-${String(index).padStart(2, "0")}-${SAMPLE_KEYS[index]}`,
          ),
        { timeout: 5_000 },
      )
      .toBe(true);
    await waitTwoAnimationFrames(page);
    if (index + 1 === WARMUP_SAMPLES) {
      await page.evaluate(() => {
        (
          window as typeof window & {
            __gwtLatencyFixture?: { beginMeasuredBusyProgress(): void };
          }
        ).__gwtLatencyFixture?.beginMeasuredBusyProgress();
      });
    }
  }

  const busyEvidence = await readBusyLoadEvidence(page);
  expect(busyEvidence.running).toBe(true);
  expect(busyEvidence.inputWhileBusyCount).toBe(SAMPLE_KEYS.length + 1);
  expect(busyEvidence.measuredRounds).toBeGreaterThanOrEqual(MEASURED_SAMPLES);
  expect(busyEvidence.progressedWindowCount).toBe(BUSY_WINDOW_IDS.length);
  expect(busyEvidence.minBaseYDelta).toBeGreaterThan(0);
  await page.evaluate(() => {
    (
      window as typeof window & {
        __gwtLatencyFixture?: { stopBusyOutput(): void };
      }
    ).__gwtLatencyFixture?.stopBusyOutput();
  });

  await runPaletteCommand(page, "Stop UI Trace");
  await expect
    .poll(() => latencySummary(page), { timeout: 5_000 })
    .not.toBeNull();
  const summary = await latencySummary(page);
  expect(summary).not.toBeNull();
  const evidence = summary as LatencySummary;
  expect(evidence.droppedEntries).toBe(0);
  expect(evidence.exactFieldsOk).toBe(true);
  expect(evidence.invalidInputCount).toBe(0);
  expect(evidence.inputWhileBusyCount).toBe(SAMPLE_KEYS.length + 1);
  expect(evidence.longTaskOverBudgetCount).toBe(0);
  expect(evidence.orderingOk).toBe(true);
  expect(evidence.privacyOk).toBe(true);
  expect(evidence.rafGapOverBudgetCount).toBe(0);
  expect(evidence.sampleCount).toBe(MEASURED_SAMPLES);
  expect(evidence.sequenceIntegrityOk).toBe(true);
  expect(evidence.stateBacklogEventCount).toBe((SAMPLE_KEYS.length + 1) * 8);
  expect(evidence.uniqueEchoCount).toBe(WARMUP_SAMPLES + MEASURED_SAMPLES);
  for (const count of Object.values(evidence.stageCounts)) {
    expect(count).toBe(WARMUP_SAMPLES + MEASURED_SAMPLES + 1);
  }
  console.log(
    `[agent-input-latency] segment_p95=${JSON.stringify(evidence.segmentP95Ms)}`,
  );
  expect(evidence.p95Ms).toBeLessThan(50);
  expect(evidence.maxMs).toBeLessThan(100);

  const measurement =
    `samples=${evidence.sampleCount} p95=${evidence.p95Ms.toFixed(1)}ms ` +
    `max=${evidence.maxMs.toFixed(1)}ms busy_served=${BUSY_WINDOW_IDS.length} ` +
    `busy_frames=${busyEvidence.measuredRounds} min_base_y_delta=${busyEvidence.minBaseYDelta}`;
  testInfo.annotations.push({ type: "measurement", description: measurement });
  console.log(`[agent-input-latency] ${measurement}`);
}

async function fixtureEchoCounts(page: Page): Promise<{
  privacyProbeCount: number;
  sampleEchoCount: number;
}> {
  return page.evaluate(() => {
    const fixture = (
      window as typeof window & {
        __gwtLatencyFixture?: {
          echoCounts(): { privacyProbeCount: number; sampleEchoCount: number };
        };
      }
    ).__gwtLatencyFixture;
    return (
      fixture?.echoCounts() ?? { privacyProbeCount: 0, sampleEchoCount: 0 }
    );
  });
}

async function readBusyLoadEvidence(page: Page): Promise<BusyLoadEvidence> {
  return page.evaluate(() => {
    const fixture = (
      window as typeof window & {
        __gwtLatencyFixture?: { busyLoadEvidence(): BusyLoadEvidence };
      }
    ).__gwtLatencyFixture;
    return (
      fixture?.busyLoadEvidence() ?? {
        inputWhileBusyCount: 0,
        measuredRounds: 0,
        minBaseYDelta: -1,
        producerRounds: 0,
        progressedWindowCount: 0,
        running: false,
      }
    );
  });
}

async function terminalContainsText(
  page: Page,
  windowId: string,
  expectedText: string,
): Promise<boolean> {
  return page.evaluate(
    ({ expected, id }) => {
      const rows = document.querySelector(
        `.workspace-window[data-id='${id}'] .xterm-rows`,
      );
      return rows?.textContent?.includes(expected) === true;
    },
    { expected: expectedText, id: windowId },
  );
}

async function latencySummary(page: Page): Promise<LatencySummary | null> {
  return page.evaluate(() => {
    const fixture = (
      window as typeof window & {
        __gwtLatencyFixture?: { summary(): LatencySummary | null };
      }
    ).__gwtLatencyFixture;
    return fixture?.summary() ?? null;
  });
}

async function runPaletteCommand(page: Page, query: string): Promise<void> {
  await page.locator("#op-palette-button").click();
  const input = page.locator("#op-palette-input");
  await expect(input).toBeVisible();
  await input.fill(query);
  await page.keyboard.press("Enter");
  await expect(page.locator("#op-palette-backdrop")).not.toHaveAttribute(
    "data-open",
    "true",
  );
}

async function waitTwoAnimationFrames(page: Page): Promise<void> {
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      }),
  );
}

async function maxAnimationFrameGap(
  page: Page,
  frameCount: number,
): Promise<number> {
  return page.evaluate(
    (count) =>
      new Promise<number>((resolve) => {
        const gaps: number[] = [];
        let previous: number | null = null;
        const sample = (timestamp: number) => {
          if (previous !== null) {
            gaps.push(timestamp - previous);
          }
          previous = timestamp;
          if (gaps.length >= count) {
            resolve(Math.max(...gaps));
            return;
          }
          requestAnimationFrame(sample);
        };
        requestAnimationFrame(sample);
      }),
    frameCount,
  );
}
