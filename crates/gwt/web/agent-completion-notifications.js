const COMPLETION_STATES = new Set(["idle", "waiting"]);
const STOP_STATES = new Set(["stopped", "exited"]);

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

function defaultAttentionAway(documentRef) {
  const hidden = documentRef?.visibilityState === "hidden";
  const focused =
    typeof documentRef?.hasFocus === "function" ? documentRef.hasFocus() : false;
  return hidden || !focused;
}

function defaultDesktopPermission(windowRef) {
  const notificationApi =
    windowRef?.Notification ||
    (typeof globalThis !== "undefined" ? globalThis.Notification : null);
  return notificationApi?.permission || "unsupported";
}

function defaultDesktopNotification(windowRef, notice) {
  const notificationApi =
    windowRef?.Notification ||
    (typeof globalThis !== "undefined" ? globalThis.Notification : null);
  if (!notificationApi) {
    return;
  }
  new notificationApi(notice.title, {
    body: notice.body,
    tag: `gwt-agent-${notice.windowId}`,
  });
}

function noticeForState({ state, windowData, projectTab, windowId }) {
  const agentName = windowLabel(windowData);
  const projectName = projectLabel(projectTab);
  if (COMPLETION_STATES.has(state)) {
    return {
      kind: "turn_complete",
      title: "Turn complete",
      body: `${agentName} completed a turn in ${projectName}.`,
    };
  }
  if (STOP_STATES.has(state)) {
    return {
      kind: "agent_stopped",
      title: "Agent stopped",
      body: `${agentName} stopped in ${projectName}.`,
    };
  }
  if (state === "error") {
    return {
      kind: "agent_error",
      title: "Agent error",
      body: `${agentName} hit an error in ${projectName}.`,
    };
  }
  return null;
}

export function createAgentCompletionNotifier({
  document: documentRef = typeof document !== "undefined" ? document : null,
  window: windowRef = typeof window !== "undefined" ? window : null,
  now = () => Date.now(),
  minRunningMs = 5 * 60 * 1000,
  isAttentionAway = () => defaultAttentionAway(documentRef),
  getDesktopNotificationPermission = () => defaultDesktopPermission(windowRef),
  showDesktopNotification = (notice) =>
    defaultDesktopNotification(windowRef, notice),
  showToast = () => {},
  onProjectUnread = () => {},
} = {}) {
  const entries = new Map();

  function publish(notice) {
    showToast(notice);
    if (getDesktopNotificationPermission() === "granted") {
      showDesktopNotification(notice);
    }
    if (notice.projectId) {
      onProjectUnread(notice.projectId, notice);
    }
  }

  function handleRuntimeState({
    windowId,
    runtimeState,
    windowData = null,
    projectTab = null,
  }) {
    if (!windowId) {
      return null;
    }
    const state = normalizeState(runtimeState);
    const previous = entries.get(windowId);
    const timestamp = now();

    if (state === "running") {
      entries.set(windowId, {
        state,
        runningStartedAt:
          previous?.state === "running" && Number.isFinite(previous.runningStartedAt)
            ? previous.runningStartedAt
            : timestamp,
        windowData,
        projectTab,
      });
      return null;
    }

    entries.set(windowId, {
      state,
      runningStartedAt: null,
      windowData,
      projectTab,
    });

    if (previous?.state !== "running") {
      return null;
    }

    const elapsedMs = timestamp - previous.runningStartedAt;
    if (!Number.isFinite(elapsedMs) || elapsedMs < minRunningMs) {
      return null;
    }
    if (!isAttentionAway()) {
      return null;
    }

    const noticeBase = noticeForState({
      state,
      windowData: windowData || previous.windowData,
      projectTab: projectTab || previous.projectTab,
      windowId,
    });
    if (!noticeBase) {
      return null;
    }
    const project = projectTab || previous.projectTab || null;
    const notice = {
      ...noticeBase,
      windowId,
      projectId: project?.id || null,
      projectTitle: projectLabel(project),
      createdAt: timestamp,
    };
    publish(notice);
    return notice;
  }

  function forgetWindow(windowId) {
    entries.delete(windowId);
  }

  return {
    handleRuntimeState,
    forgetWindow,
  };
}

// SPEC-2356 Anshin Addendum (FR-040) — in-app attention toaster.
//
// Distinct from createAgentCompletionNotifier above, which only fires DESKTOP
// notifications when the operator is AWAY for a sustained run. This is the
// in-app, always-on counterpart: the moment an agent transitions into a state
// that wants the operator's eyes (needs_input / blocked / error / done), a
// quiet in-app toast appears even while the operator is present. Clicking a
// toast flies the camera to that window (frameWindow), so attention is one
// click from action. Pure controller: DOM rendering is injected via
// `showToast`. The mapping keys off the runtime wire state, not the Living
// Telemetry projection, so this stays independent of CSS state names.
const ATTENTION_STATES = Object.freeze({
  waiting: "needs_input",
  error: "error",
  stopped: "done",
  exited: "done",
});

function attentionNoticeForFlavor({ flavor, windowData, windowId }) {
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
        body: `${agentName} hit an error.`,
        windowId,
      };
    case "done":
      return {
        flavor,
        title: "Agent finished",
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

  function handleRuntimeState({ windowId, runtimeState, windowData = null }) {
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

    const notice = attentionNoticeForFlavor({ flavor, windowData, windowId });
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
