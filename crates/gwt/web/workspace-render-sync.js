// Issue #3365 — render-key lifecycle + exception-isolated workspace sync.
//
// `renderWorkspace` used to commit `renderedWorkspaceWindowsKey` before the
// per-window sync loops and the telemetry recompute ran. Any exception in
// those steps escaped to the WebSocket dispatcher (which warns and continues),
// while the already-committed key made every following identical
// workspace_state short-circuit on the diff check — freezing the Fleet
// Minimap, the window list, and the telemetry counts until a full reload.
//
// This module owns that lifecycle with exception safety:
// - the key is committed only after a fully clean sync, so a degraded render
//   retries on the next workspace_state instead of being diff-skipped,
// - `isolate(label, items, run)` guards each item so one poisoned window
//   cannot block the remaining windows,
// - `recompute` (telemetry / minimap) and `afterSync` (focus) always run,
// - `render()` never throws, so callers such as `renderAppState` keep going
//   (`renderWindowList()` stays live even on a degraded render).

export function createWorkspaceRenderSync({ onDegraded } = {}) {
  let renderedKey = "";

  function render({ key, sync, recompute, afterSync }) {
    if (renderedKey === key) {
      return { skipped: true, failures: [] };
    }
    // Invalidate before syncing: a failure below must leave the key
    // uncommitted so the next workspace_state retries instead of skipping.
    renderedKey = "";
    const failures = [];
    const guard = (label, item, run) => {
      try {
        run();
      } catch (error) {
        failures.push({ label, item, error });
      }
    };
    const isolate = (label, items, run) => {
      for (const item of items) {
        guard(label, item, () => run(item));
      }
    };

    guard("sync", null, () => sync?.(isolate));
    if (failures.length === 0) {
      renderedKey = key;
    }
    guard("recompute", null, () => recompute?.());
    guard("after_sync", null, () => afterSync?.());
    if (failures.length > 0) {
      try {
        onDegraded?.(failures);
      } catch (_) {
        // Degradation reporting must never take down the render path.
      }
    }
    return { skipped: false, failures };
  }

  return { render };
}
