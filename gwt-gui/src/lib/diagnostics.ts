import { invoke } from "$lib/tauriInvoke";
import { maskSensitiveData } from "$lib/privacyMask";

export interface ReportSystemInfo {
  osName: string;
  osVersion: string;
  arch: string;
  gwtVersion: string;
}

export async function collectSystemInfo(): Promise<string> {
  try {
    const info = await invoke<ReportSystemInfo>("get_report_system_info");
    return [
      `- OS: ${info.osName} ${info.osVersion}`,
      `- Architecture: ${info.arch}`,
      `- gwt Version: ${info.gwtVersion}`,
    ].join("\n");
  } catch {
    return "(Failed to collect system info)";
  }
}

export async function collectRecentLogs(maxLines = 50): Promise<string> {
  try {
    const logs = await invoke<string>("read_recent_logs", { maxLines });
    return maskSensitiveData(logs);
  } catch {
    return "(Failed to collect logs)";
  }
}
