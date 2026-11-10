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
 * @typedef {{source: "runtime", state: "stopped" | "interrupted" | "error", windowData?: object,
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
  if (input.state === "interrupted") {
    return { kind: "agent_interrupted", title: "Agent interrupted", body: `${agentName} was interrupted in ${projectName}.` };
  }
  if (input.state === "error") {
    return { kind: "agent_error", title: "Agent error", body: `${agentName} hit an error in ${projectName}.` };
  }
  return null;
}

// Issue #6932: presentation identity only; the raw diagnostic ledger is unchanged.
export function monitorDiagnostic({ message, kind, stage, issue_number } = {}) {
  const raw = typeof message === "string" ? message.trim() : "";
  if (!raw) return null;
  const continuations = [...raw.matchAll(/\(continued_with_([\w]+)\)/g)];
  if (raw.startsWith("scan continued:") && !raw.includes("launch_suppressed") && continuations.length
    && continuations.every(([, value]) => ["stale_readback", "previous_candidates"].includes(value))) return null;
  const stalled = raw.startsWith("Issue Monitor scan stalled");
  const normalized = raw
    .replace(/\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})/g, "<time>")
    .replace(/\b\d+(?:\.\d+)?(?:ms|s)\b/g, "<duration>")
    .replace(/\s+/g, " ");
  const identity = stalled
    ? normalized.replace(/ for <duration> \(poll interval <duration>\)/, "").replace(/; last scan at <time>/, "")
    : normalized;
  const phase = (!stalled && stage) || raw.match(/scan failed at ([\w-]+) stage:/)?.[1] || "monitor";
  const source = stalled ? "scan_stall" : kind || (phase === "monitor" ? "monitor" : "scan_failure");
  return { key: `issue-monitor:${JSON.stringify([source, phase, identity, issue_number || null])}`,
    message: stalled ? identity : raw, stalled };
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
      || !["stopped", "interrupted", "error", "needs_human"].includes(notice.category)
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
// that wants the operator's eyes (needs_input / error), a
// quiet in-app toast appears even while the operator is present. Clicking a
// toast selects that window in its surface (frameWindow), so attention is one
// click from action. Pure controller: DOM rendering is injected via
// `showToast`. The mapping keys off the runtime wire state, not the Living
// Telemetry projection, so this stays independent of CSS state names.
const ATTENTION_STATES = Object.freeze({
  waiting: "needs_input",
  error: "error",
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
    default:
      return null;
  }
}

export function createAgentAttentionToaster({
  showToast = () => {},
  dismissToast = () => {},
  now = () => Date.now(),
} = {}) {
  // Track flavors per window and retire superseded Issue windows until catalog
  // removal. Late frames/reconnects from an old window cannot replace or clear
  // its successor's attention. Non-error flavors retain transition-only dedupe.
  const lastFlavor = new Map();
  const errors = new Map();
  // Page-lifetime high-water marks: closing a successful successor must not
  // revive a retained older diagnostic on the next replay.
  const latestRuntimes = new Map();

  function retireErrorWindow(key, successor) {
    const previous = errors.get(key)?.windowId;
    if (previous && previous !== successor) lastFlavor.set(previous, `superseded:${key}`);
  }

  function resolveError(key, successor) {
    if (!errors.has(key)) return;
    if (successor) retireErrorWindow(key, successor);
    errors.delete(key);
    dismissToast(key);
  }

  function handleRuntimeState({
    windowId,
    runtimeState,
    windowData = null,
    statusDetail = "",
    projectKey = "",
    source = "terminal_status",
    retryable = false,
  }) {
    if (!windowId) {
      return null;
    }
    const state = normalizeState(runtimeState);
    const flavor = ATTENTION_STATES[state] || null;
    const issue = windowData?.linked_issue_number;
    const groupedIssue = projectKey && Number.isSafeInteger(issue) && issue > 0;
    const errorKey = groupedIssue
      ? `attention-${projectKey}-issue-${issue}` : `attention-${windowId}`;
    // These timestamps are wire-only local runtime facts, including retained
    // diagnostic panes. Learn them from snapshots before diagnostic replay so
    // a full page reload cannot resurrect a provably older Issue window.
    const startedAt = windowData?.runtime_started_at_ms;
    if (groupedIssue && Number.isSafeInteger(startedAt) && startedAt > 0) {
      const latest = latestRuntimes.get(errorKey);
      if (latest && latest.startedAt > startedAt) return null;
      if (!latest || startedAt > latest.startedAt) {
        latestRuntimes.set(errorKey, { windowId, startedAt });
        lastFlavor.delete(windowId);
        const previousStart = errors.get(errorKey)?.startedAt;
        if (Number.isSafeInteger(previousStart) && previousStart > 0 && previousStart < startedAt) {
          resolveError(errorKey, windowId);
        }
      }
    }
    if (lastFlavor.get(windowId) === `superseded:${errorKey}`) return null;

    // Workspace/WindowState frames project persisted status, including errors
    // from other machines. Only the local runtime diagnostic (also replayed on
    // reconnect) can create error attention. Do not consume its dedupe early.
    if (flavor === "error" && source !== "terminal_status") return null;
    if (flavor !== "error" || retryable) {
      if (source === "terminal_status" || errors.get(errorKey)?.windowId === windowId) {
        resolveError(errorKey, windowId);
      }
    }
    if (flavor === "error") {
      if (retryable) {
        lastFlavor.delete(windowId);
        return null;
      }
      const previous = errors.get(errorKey);
      if (previous?.windowId === windowId && previous.detail === statusDetail) return null;
      retireErrorWindow(errorKey, windowId);
      errors.set(errorKey, { windowId, detail: statusDetail, startedAt });
    }

    if (!flavor) {
      lastFlavor.delete(windowId);
      return null;
    }

    if (flavor !== "error" && lastFlavor.get(windowId) === flavor) {
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
    notice.id = flavor === "error" ? errorKey : `attention-${windowId}`;
    notice.createdAt = now();
    showToast(notice);
    return notice;
  }

  function forgetWindow(windowId) {
    lastFlavor.delete(windowId);
    for (const [key, error] of errors) {
      if (error.windowId === windowId) resolveError(key);
    }
  }

  return {
    handleRuntimeState,
    forgetWindow,
  };
}
