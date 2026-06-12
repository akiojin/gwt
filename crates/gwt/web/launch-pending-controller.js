// SPEC-2359 W-17 (FR-398) — pending state for Resume / Launch requests.
//
// Single owner of "a Resume/Launch request is in flight". Entry points call
// begin() before sending the WebSocket message — a false return means the
// same request is already pending and the caller must not re-send
// (double-click guard, Issue #3034). The dispatcher settles entries on the
// backend ack (`workspace_resume_agent_started`) or error reply; a timeout
// clears stuck entries so the UI can never wedge permanently when the
// backend never answers (e.g. the socket dropped mid-flight).
//
// Keys are namespaced strings shared across surfaces:
//   `session:<gwt session id>` — Work/Session resume (kanban rows, picker)
//   `branch:<branch name>`     — Branches-row resume

export const LAUNCH_PENDING_TIMEOUT_MS = 20000;

export function createLaunchPendingController({
  onChange,
  setTimeoutFn = (callback, ms) => setTimeout(callback, ms),
  clearTimeoutFn = (timer) => clearTimeout(timer),
} = {}) {
  const pending = new Map();
  let timeoutNotice = "";

  function notify() {
    if (typeof onChange !== "function") return;
    try {
      onChange();
    } catch {
      // Listeners must never break the pending bookkeeping.
    }
  }

  function begin(key, label) {
    if (!key || pending.has(key)) return false;
    const timer = setTimeoutFn(() => {
      if (!pending.delete(key)) return;
      timeoutNotice = `${label || "Launch"} request timed out; check the connection and retry.`;
      notify();
    }, LAUNCH_PENDING_TIMEOUT_MS);
    pending.set(key, { label: label || "", timer });
    notify();
    return true;
  }

  function settle(key) {
    const entry = pending.get(key);
    if (!entry) return false;
    pending.delete(key);
    clearTimeoutFn(entry.timer);
    notify();
    return true;
  }

  /// Settle from a backend ack/error payload carrying `session_id` and/or
  /// `branch` — clears both key namespaces in one call.
  function settleAck(event) {
    let settled = false;
    if (event && event.session_id) {
      settled = settle(`session:${event.session_id}`) || settled;
    }
    if (event && event.branch) {
      settled = settle(`branch:${event.branch}`) || settled;
    }
    return settled;
  }

  function settleWhere(prefix) {
    for (const key of [...pending.keys()]) {
      if (key.startsWith(prefix)) settle(key);
    }
  }

  function isPending(key) {
    return pending.has(key);
  }

  function pendingCount() {
    return pending.size;
  }

  /// One-shot: returns the latest timeout notice and clears it.
  function consumeTimeoutNotice() {
    const value = timeoutNotice;
    timeoutNotice = "";
    return value;
  }

  return {
    begin,
    settle,
    settleAck,
    settleWhere,
    isPending,
    pendingCount,
    consumeTimeoutNotice,
  };
}
