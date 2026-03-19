/**
 * Profiling state store (Svelte 5 runes).
 *
 * Manages profiling on/off, invoke round-trip ring buffer,
 * heartbeat interval, and freeze-detected event listener.
 */

import { invoke } from "./tauriInvoke";

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let profilingEnabled = $state(false);

export function isProfilingEnabled(): boolean {
  return profilingEnabled;
}

// ---------------------------------------------------------------------------
// Invoke metric ring buffer
// ---------------------------------------------------------------------------

export interface InvokeMetric {
  command: string;
  durationMs: number;
  timestamp: number;
}

const RING_BUFFER_MAX = 100;
let metrics: InvokeMetric[] = $state([]);

export function recordInvokeMetric(metric: InvokeMetric): void {
  if (!profilingEnabled) return;
  metrics.push(metric);
  if (metrics.length > RING_BUFFER_MAX) {
    metrics.splice(0, metrics.length - RING_BUFFER_MAX);
  }
}

// ---------------------------------------------------------------------------
// Heartbeat & metric reporter
// ---------------------------------------------------------------------------

let heartbeatTimer: ReturnType<typeof setInterval> | null = null;
let reportTimer: ReturnType<typeof setInterval> | null = null;
let freezeUnlisten: (() => void) | null = null;

function startHeartbeat(): void {
  if (heartbeatTimer) return;
  heartbeatTimer = setInterval(() => {
    invoke("heartbeat").catch(() => {
      /* heartbeat command may not exist yet */
    });
  }, 1000);
}

function stopHeartbeat(): void {
  if (heartbeatTimer) {
    clearInterval(heartbeatTimer);
    heartbeatTimer = null;
  }
}

function startMetricReporter(): void {
  if (reportTimer) return;
  reportTimer = setInterval(() => {
    if (metrics.length === 0) return;
    const batch = metrics.splice(0, metrics.length);
    invoke("report_frontend_metrics", { metrics: batch }).catch(() => {
      /* command may not exist yet */
    });
  }, 10_000);
}

function stopMetricReporter(): void {
  if (reportTimer) {
    clearInterval(reportTimer);
    reportTimer = null;
  }
}

async function startFreezeListener(): Promise<void> {
  if (freezeUnlisten) return;
  try {
    const { listen } = await import("@tauri-apps/api/event");
    const unlisten = await listen<string>("freeze-detected", (event) => {
      console.warn("[profiling] freeze detected:", event.payload);
    });
    freezeUnlisten = unlisten;
  } catch {
    /* event API unavailable in test */
  }
}

function stopFreezeListener(): void {
  if (freezeUnlisten) {
    freezeUnlisten();
    freezeUnlisten = null;
  }
}

// ---------------------------------------------------------------------------
// Init / teardown
// ---------------------------------------------------------------------------

export async function initProfiling(): Promise<void> {
  try {
    const settings = await invoke<{ profiling?: boolean }>("get_settings");
    profilingEnabled = settings.profiling ?? false;
  } catch {
    profilingEnabled = false;
  }

  if (profilingEnabled) {
    startHeartbeat();
    startMetricReporter();
    await startFreezeListener();
  }
}

export function teardownProfiling(): void {
  stopHeartbeat();
  stopMetricReporter();
  stopFreezeListener();
}

export function setProfilingEnabled(enabled: boolean): void {
  profilingEnabled = enabled;
  if (enabled) {
    startHeartbeat();
    startMetricReporter();
    startFreezeListener();
  } else {
    teardownProfiling();
  }
}
