// Startup observations share the existing workspace render and terminal fit paths.
export function createStartupMetrics({
  send,
  requestAnimationFrame = (callback) => globalThis.requestAnimationFrame(callback),
  now = () => performance.now(),
}) {
  let frameScheduled = false;
  const reportedRunningTerminals = new WeakSet();
  return {
    onWorkspaceRendered() {
      if (frameScheduled) return;
      frameScheduled = true;
      // The second callback follows a rendering opportunity for the workspace.
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          send({ kind: "startup_first_frame", navigation_ms: now() });
        });
      });
    },
    onTerminalReady(id, runtime) {
      if (runtime?.readOnly === true) return;
      send({ kind: "startup_terminal_ready", id });
    },
    onTerminalStatus(id, status, runtime) {
      // A restored process can reuse an already fitted placeholder terminal.
      if (status !== "running" || runtime?.isReady !== true || runtime.readOnly === true || reportedRunningTerminals.has(runtime)) return;
      reportedRunningTerminals.add(runtime);
      send({ kind: "startup_terminal_ready", id });
    },
  };
}
