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
