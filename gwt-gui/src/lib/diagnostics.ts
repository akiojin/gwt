/**
 * Diagnostics collection for Issue Reports.
 *
 * Log collection delegates to the Tauri `read_recent_logs` command which only
 * returns normal logs (gwt.jsonl*). Profiling output (profile.json) is excluded
 * at the backend candidate-selection level. Privacy masking is applied on the
 * returned text before it is surfaced in the report.
 */
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

/**
 * Collect recent normal logs (gwt.jsonl*) via the backend command.
 *
 * Profiling output (profile.json) is never included — filtering is enforced
 * by `is_log_file_candidate` on the Rust side. The returned text is run
 * through `maskSensitiveData` to redact API keys, tokens, and passwords
 * before inclusion in a report.
 */
export async function collectRecentLogs(maxLines = 50): Promise<string> {
  try {
    const logs = await invoke<string>("read_recent_logs", { maxLines });
    return maskSensitiveData(logs);
  } catch {
    return "(Failed to collect logs)";
  }
}
