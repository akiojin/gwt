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

function defaultContinueWorkOperationId() {
  const random =
    typeof globalThis.crypto?.randomUUID === "function"
      ? globalThis.crypto.randomUUID()
      : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  return `continue-${random}`;
}

export function createLaunchPendingController({
  onChange,
  setTimeoutFn = (callback, ms) => setTimeout(callback, ms),
  clearTimeoutFn = (timer) => clearTimeout(timer),
} = {}) {
  const pending = new Map();
  const correlations = new Map();
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
    if (!key || pending.has(key) || correlations.has(key)) return false;
    const timer = setTimeoutFn(() => {
      if (!pending.delete(key)) return;
      timeoutNotice = `${label || "Launch"} request timed out; check the connection and retry.`;
      notify();
    }, LAUNCH_PENDING_TIMEOUT_MS);
    pending.set(key, { label: label || "", timer });
    notify();
    return true;
  }

  function armCorrelated(key, correlation) {
    if (pending.has(key)) return false;
    const timer = setTimeoutFn(() => {
      if (!pending.delete(key)) return;
      const retained = correlations.get(key);
      if (retained === correlation) {
        retained.timer = null;
      }
      timeoutNotice = `${correlation.label || "Continue work"} request timed out; check the connection and retry.`;
      notify();
    }, LAUNCH_PENDING_TIMEOUT_MS);
    correlation.timer = timer;
    pending.set(key, correlation);
    notify();
    return true;
  }

  function beginCorrelated(key, operationId, workId, label) {
    if (!key || !operationId || !workId || pending.has(key)) return false;
    const existing = correlations.get(key);
    if (existing) {
      if (
        existing.operationId !== operationId
        || existing.workId !== workId
      ) {
        return false;
      }
      return armCorrelated(key, existing);
    }
    const correlation = {
      operationId,
      workId,
      label: label || "",
      timer: null,
    };
    correlations.set(key, correlation);
    return armCorrelated(key, correlation);
  }

  function settle(key) {
    const entry = pending.get(key);
    if (!entry) return false;
    pending.delete(key);
    clearTimeoutFn(entry.timer);
    if (correlations.get(key) === entry) {
      correlations.delete(key);
    }
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

  function settleCorrelated(event) {
    const operationId = event?.operation_id;
    const workId = event?.work_id;
    if (!operationId || !workId) return false;
    for (const [key, correlation] of correlations) {
      if (
        correlation.operationId !== operationId
        || correlation.workId !== workId
      ) {
        continue;
      }
      correlations.delete(key);
      if (pending.delete(key) && correlation.timer !== null) {
        clearTimeoutFn(correlation.timer);
      }
      notify();
      return true;
    }
    return false;
  }

  function correlatedOperation(key) {
    return correlations.get(key)?.operationId || "";
  }

  function retryCorrelated(key) {
    const correlation = correlations.get(key);
    if (!correlation || pending.has(key)) return false;
    return armCorrelated(key, correlation);
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
    beginCorrelated,
    settle,
    settleAck,
    settleCorrelated,
    settleWhere,
    isPending,
    pendingCount,
    correlatedOperation,
    retryCorrelated,
    consumeTimeoutNotice,
  };
}

export function isStrongContinueWorkSuccess(event) {
  return [
    "focused_existing",
    "continued_conversation",
    "started_with_handoff",
  ].includes(event?.outcome);
}

export function continueWorkOutcomeNotice(event) {
  const message = typeof event?.message === "string" ? event.message.trim() : "";
  switch (event?.outcome) {
    case "focused_existing":
      return {
        level: "info",
        title: "Work already active",
        message: message || "Focused the existing active conversation.",
      };
    case "continued_conversation":
      return {
        level: "done",
        title: "Work continued",
        message: message || "The existing conversation is ready in a new execution generation.",
      };
    case "started_with_handoff":
      return {
        level: "info",
        title: "Work continued",
        message: message
          || "The previous conversation was unavailable, so a new conversation started with handoff context.",
      };
    case "conflict_unknown":
      return {
        level: "warn",
        title: "Continue work needs attention",
        message: message || "The current owner could not be verified. No state was changed.",
      };
    case "failed": {
      const base = message || "The continuation could not be committed.";
      return {
        level: "error",
        title: "Continue work failed",
        message: event?.retryable ? `${base} You can try again.` : base,
      };
    }
    default:
      return null;
  }
}

/**
 * Owns the public Continue work intent boundary.
 *
 * The browser is allowed to name only its idempotency key, the selected
 * opaque Work, and presentation bounds. Durable Session, conversation,
 * generation, binding, and Host authority are resolved by the backend.
 */
export function createContinueWorkDispatcher({
  launchPending,
  send,
  createOperationId = defaultContinueWorkOperationId,
} = {}) {
  function pendingKey(workId) {
    return `continue:${workId}`;
  }

  function dispatch(rawWorkId, bounds) {
    const workId = String(rawWorkId || "").trim();
    if (!workId || !bounds || !launchPending || typeof send !== "function") {
      return false;
    }
    const key = pendingKey(workId);
    let operationId = launchPending.correlatedOperation(key);
    const armed = operationId
      ? launchPending.retryCorrelated(key)
      : (() => {
          operationId = String(createOperationId() || "").trim();
          return Boolean(operationId) && launchPending.beginCorrelated(
            key,
            operationId,
            workId,
            "Continue work",
          );
        })();
    if (!armed) {
      return false;
    }
    send({
      kind: "continue_work",
      operation_id: operationId,
      work_id: workId,
      bounds,
    });
    return true;
  }

  function handleOutcome(event) {
    if (event?.kind !== "continue_work_outcome") {
      return null;
    }
    return launchPending?.settleCorrelated(event) ? event : null;
  }

  return {
    dispatch,
    handleOutcome,
  };
}
