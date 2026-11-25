import { spawnSync } from "node:child_process";

// An absent PID needs no expensive platform query. A live or inaccessible PID
// still needs the caller's original identity checks before any cleanup signal.
export function inspectLiveFixtureProcess(pid, inspect) {
  try {
    process.kill(pid, 0);
  } catch (error) {
    if (error.code === "ESRCH") return "";
    if (error.code !== "EPERM") throw error;
  }
  return inspect();
}

// PowerShell + WMI startup on a saturated Windows runner has taken the whole
// former 10 s budget (Issue #7270), so a slow query must not read as a failure.
export const WINDOWS_PROCESS_QUERY_TIMEOUT_MS = 30_000;

export function queryWindowsProcess(script, run = spawnSync) {
  const result = run("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script],
    { encoding: "utf8", timeout: WINDOWS_PROCESS_QUERY_TIMEOUT_MS, windowsHide: true });
  if (result.error) throw result.error;
  return result;
}
