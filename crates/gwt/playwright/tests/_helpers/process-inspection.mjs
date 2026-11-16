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
