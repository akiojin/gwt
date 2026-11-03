function normalizeState(value) {
  return String(value || "").toLowerCase();
}

function windowLabel(windowData) {
  const candidates = [
    windowData?.dynamic_title,
    windowData?.purpose_title,
    windowData?.title,
    windowData?.agent_id,
  ];
  for (const candidate of candidates) {
    const label = String(candidate || "").trim();
    if (label) return label;
  }
  return "Agent";
}

function projectLabel(projectTab) {
  const title = String(projectTab?.title || "").trim();
  if (title) return title;
  const root = String(projectTab?.project_root || "").trim();
  if (root) return root;
  return "Project";
}

function defaultDesktopPermission(windowRef) {
  return windowRef?.Notification?.permission || "unsupported";
}

function defaultDesktopNotification(windowRef, notice, onActivate) {
  const notification = new windowRef.Notification(notice.title, {
    body: notice.body,
    tag: `gwt-notification-${notice.transition_id}`,
  });
  notification.onclick = () => {
    try {
      windowRef.focus?.();
      onActivate(notice);
    } finally {
      notification.close();
    }
  };
}

function appendStatusDetail(body, detail) {
  const trimmed = String(detail || "").trim();
  if (!trimmed) return body;
  const base = body.replace(/\.$/, "");
  return `${base}: ${trimmed}`;
}

/**
 * Source-tagged notification inputs. Runtime states come from WindowProcessStatus;
 * NeedsHuman comes only from the Monitor's typed transition, never Waiting/text.
 * @typedef {{source: "runtime", state: "stopped" | "error", windowData?: object,
 * projectTab?: object} | {source: "monitor", state: "needs_human", issueNumber: number}} NotificationTransition
 */
export function notificationForTransition(input) {
  if (input?.source === "monitor") {
    if (input.state !== "needs_human" || !Number.isSafeInteger(input.issueNumber) || input.issueNumber <= 0) return null;
    return { kind: "needs_human", title: "Needs human", body: `Issue #${input.issueNumber} needs human attention.` };
  }
  if (input?.source !== "runtime") return null;
  const agentName = windowLabel(input.windowData);
  const projectName = projectLabel(input.projectTab);
  if (input.state === "stopped") {
    return { kind: "agent_stopped", title: "Agent stopped", body: `${agentName} stopped in ${projectName}.` };
  }
  if (input.state === "error") {
    return { kind: "agent_error", title: "Agent error", body: `${agentName} hit an error in ${projectName}.` };
  }
  return null;
}

// The server owns runtime timing, attention suppression, and recipient election.
// This desktop-only sink never derives notifications from runtime snapshots.
export function createAgentCompletionNotifier({
  window: windowRef = typeof window !== "undefined" ? window : null,
  getDesktopNotificationPermission = () => defaultDesktopPermission(windowRef),
  onActivate = () => {},
  showDesktopNotification = (notice) => defaultDesktopNotification(windowRef, notice, onActivate),
} = {}) {
  // Page-lifetime dedupe survives socket reconnects and window removal.
  const seenTransitions = new Set();

  function handleIntent(notice) {
    if (!notice || typeof notice.transition_id !== "string" || !notice.transition_id.trim()
      || !["stopped", "error", "needs_human"].includes(notice.category)
      || seenTransitions.has(notice.transition_id)) return null;
    seenTransitions.add(notice.transition_id);
    if (getDesktopNotificationPermission() !== "granted") return null;
    try {
      showDesktopNotification(notice);
      return notice;
    } catch {
      // A browser can expose granted permission while its OS API is unavailable.
      windowRef?.console?.warn?.("gwt: desktop notification delivery failed");
      return null;
    }
  }

  return { handleIntent };
}

// SPEC-2356 Anshin Addendum (FR-040) — in-app attention toaster.
//
// Distinct from the server-intent desktop sink above. This is the
// in-app, always-on counterpart: the moment an agent transitions into a state
// that wants the operator's eyes (needs_input / blocked / error / done), a
// quiet in-app toast appears even while the operator is present. Clicking a
// toast selects that window in its surface (frameWindow), so attention is one
// click from action. Pure controller: DOM rendering is injected via
// `showToast`. The mapping keys off the runtime wire state, not the Living
// Telemetry projection, so this stays independent of CSS state names.
const ATTENTION_STATES = Object.freeze({
  waiting: "needs_input",
  error: "error",
  stopped: "done",
  exited: "done",
});

function attentionNoticeForFlavor({ flavor, windowData, windowId, statusDetail = "" }) {
  const agentName = windowLabel(windowData);
  switch (flavor) {
    case "needs_input":
      return {
        flavor,
        title: "Waiting for input",
        body: `${agentName} is waiting for your input.`,
        windowId,
      };
    case "error":
      return {
        flavor,
        title: "Agent error",
        body: appendStatusDetail(`${agentName} hit an error.`, statusDetail),
        windowId,
      };
    case "done":
      return {
        flavor,
        title: "Agent stopped",
        body: `${agentName} stopped.`,
        windowId,
      };
    default:
      return null;
  }
}

export function createAgentAttentionToaster({
  showToast = () => {},
  now = () => Date.now(),
} = {}) {
  // Last toasted flavor per window. We only fire when the flavor CHANGES, so a
  // string of waiting status frames does not spam the operator. Leaving the
  // attention set (e.g. back to running/idle) clears the entry so the next
  // entry into the same flavor toasts again.
  const lastFlavor = new Map();

  function handleRuntimeState({
    windowId,
    runtimeState,
    windowData = null,
    statusDetail = "",
  }) {
    if (!windowId) {
      return null;
    }
    const state = normalizeState(runtimeState);
    const flavor = ATTENTION_STATES[state] || null;

    if (!flavor) {
      lastFlavor.delete(windowId);
      return null;
    }

    if (lastFlavor.get(windowId) === flavor) {
      return null;
    }
    lastFlavor.set(windowId, flavor);

    const notice = attentionNoticeForFlavor({
      flavor,
      windowData,
      windowId,
      statusDetail,
    });
    if (!notice) {
      return null;
    }
    notice.createdAt = now();
    showToast(notice);
    return notice;
  }

  function forgetWindow(windowId) {
    lastFlavor.delete(windowId);
  }

  return {
    handleRuntimeState,
    forgetWindow,
  };
}
