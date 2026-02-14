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

export function createSystemMonitor() {
  let cpuUsage = $state(0);
  let memUsed = $state(0);
  let memTotal = $state(0);
  let gpuInfo: GpuInfo | null = $state(null);
  let intervalId: ReturnType<typeof setInterval> | null = null;

  async function poll() {
    try {
      const info: SystemInfo = await invoke("get_system_info");
      cpuUsage = info.cpu_usage_percent;
      memUsed = info.memory_used_bytes;
      memTotal = info.memory_total_bytes;
      gpuInfo = info.gpu;
    } catch (e) {
      console.warn("Failed to get system info:", e);
    }
  }

  async function init() {
    // First call warms up sysinfo (returns 0% CPU)
    await invoke("get_system_info").catch(() => {});
    // Second call gets real values
    await poll();
  }

  function start() {
    if (intervalId) return;
    init();
    intervalId = setInterval(poll, 1000);
  }

  function stop() {
    if (intervalId) {
      clearInterval(intervalId);
      intervalId = null;
    }
  }

  function destroy() {
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
