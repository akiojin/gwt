import { invoke } from "@tauri-apps/api/core";

interface SystemInfo {
  cpu_usage_percent: number;
  memory_used_bytes: number;
  memory_total_bytes: number;
  gpu: GpuInfo | null;
}

export interface GpuInfo {
  name: string;
  vram_total_bytes: number | null;
  vram_used_bytes: number | null;
  usage_percent: number | null;
}

const POLL_INTERVAL_MS = 5000;

export function createSystemMonitor() {
  let cpuUsage = $state(0);
  let memUsed = $state(0);
  let memTotal = $state(0);
  let gpuInfo: GpuInfo | null = $state(null);
  let timerId: ReturnType<typeof setTimeout> | null = null;
  let running = false;
  let destroyed = false;
  let warmingUp = false;
  let warmedUp = false;
  let polling = false;

  async function pollOnce() {
    if (polling) return;
    polling = true;
    try {
      const info: SystemInfo = await invoke("get_system_info");
      cpuUsage = info.cpu_usage_percent;
      memUsed = info.memory_used_bytes;
      memTotal = info.memory_total_bytes;
      gpuInfo = info.gpu;
    } catch (e) {
      console.warn("Failed to get system info:", e);
    } finally {
      polling = false;
    }
  }

  async function warmupIfNeeded() {
    if (warmedUp || warmingUp) return;
    warmingUp = true;
    // First call warms up sysinfo (may return 0% CPU).
    await invoke("get_system_info").catch(() => {});
    warmedUp = true;
    warmingUp = false;
  }

  function clearTimer() {
    if (timerId) {
      clearTimeout(timerId);
      timerId = null;
    }
  }

  function scheduleNext() {
    if (!running || destroyed || timerId) return;
    timerId = setTimeout(() => {
      timerId = null;
      void runCycle();
    }, POLL_INTERVAL_MS);
  }

  async function runCycle() {
    if (!running || destroyed) return;
    await warmupIfNeeded();
    if (!running || destroyed) return;
    await pollOnce();
    if (!running || destroyed) return;
    scheduleNext();
  }

  function start() {
    if (running || destroyed) return;
    running = true;
    clearTimer();
    void runCycle();
  }

  function stop() {
    running = false;
    clearTimer();
  }

  function destroy() {
    destroyed = true;
    stop();
    document.removeEventListener("visibilitychange", handleVisibility);
  }

  function handleVisibility() {
    if (document.hidden) stop();
    else start();
  }

  document.addEventListener("visibilitychange", handleVisibility);

  return {
    get cpuUsage() { return cpuUsage; },
    get memUsed() { return memUsed; },
    get memTotal() { return memTotal; },
    get gpuInfo() { return gpuInfo; },
    start,
    stop,
    destroy,
  };
}
