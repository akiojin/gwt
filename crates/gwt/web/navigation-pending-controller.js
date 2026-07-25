const NAVIGATION_REQUESTS = Object.freeze({
  select_project_tab: {
    scope: "project_tab",
    targetField: "tab_id",
  },
  activate_window_tab: {
    scope: "window_tab",
    targetField: "id",
  },
  focus_window: {
    scope: "canvas",
    targetField: "id",
  },
});

function emptyState() {
  return {
    app_version: "",
    tabs: [],
    active_tab_id: null,
    recent_projects: [],
  };
}

function hasOwn(value, key) {
  return Object.prototype.hasOwnProperty.call(value || {}, key);
}

function mapWindows(state, update) {
  let stateChanged = false;
  const tabs = (state?.tabs || []).map((tab) => {
    let tabChanged = false;
    const windows = (tab.workspace?.windows || []).map((windowData) => {
      const nextWindow = update(windowData, tab);
      if (nextWindow !== windowData) {
        tabChanged = true;
      }
      return nextWindow;
    });
    if (!tabChanged) {
      return tab;
    }
    stateChanged = true;
    return {
      ...tab,
      workspace: {
        ...tab.workspace,
        windows,
      },
    };
  });
  return stateChanged ? { ...state, tabs } : state;
}

function applyCanonicalDelta(state, canonical) {
  if (!canonical || typeof canonical !== "object") {
    return state;
  }
  let nextState = state;
  if (hasOwn(canonical, "active_tab_id")) {
    nextState = {
      ...nextState,
      active_tab_id: canonical.active_tab_id ?? null,
    };
  }
  const updates = new Map(
    (canonical.window_updates || [])
      .filter((windowData) => typeof windowData?.id === "string")
      .map((windowData) => [windowData.id, windowData]),
  );
  if (updates.size === 0) {
    return nextState;
  }
  return mapWindows(nextState, (windowData) => {
    const update = updates.get(windowData.id);
    if (!update) {
      return windowData;
    }
    const nextWindow = { ...windowData };
    if (hasOwn(update, "z_index")) {
      nextWindow.z_index = update.z_index;
    }
    if (hasOwn(update, "tab_group_active")) {
      nextWindow.tab_group_active = update.tab_group_active;
    }
    return nextWindow;
  });
}

function projectTabOperation(state, targetId) {
  if (!(state?.tabs || []).some((tab) => tab.id === targetId)) {
    return state;
  }
  return {
    ...state,
    active_tab_id: targetId,
  };
}

function findWindowContext(state, targetId) {
  return (state?.tabs || [])
    .map((tab) => ({
      tab,
      target: (tab.workspace?.windows || []).find(
        (windowData) => windowData.id === targetId,
      ),
    }))
    .find((context) => context.target);
}

function maxZByTab(state) {
  const result = new Map();
  for (const tab of state?.tabs || []) {
    let maxZ = 0;
    for (const windowData of tab.workspace?.windows || []) {
      const zIndex = Number(windowData?.z_index);
      if (Number.isFinite(zIndex)) {
        maxZ = Math.max(maxZ, zIndex);
      }
    }
    result.set(tab.id, maxZ);
  }
  return result;
}

function windowOperation(state, operation) {
  const targetContext = findWindowContext(state, operation.targetId);
  if (!targetContext) {
    return state;
  }

  const target = targetContext.target;
  const groupId = target.tab_group_id || null;
  const targetZ = Number(target.z_index);
  const optimisticZ = Number.isFinite(operation.optimisticZ)
    ? operation.optimisticZ
    : Number.isFinite(targetZ)
      ? targetZ
      : 0;
  let nextState = {
    ...state,
    active_tab_id: targetContext.tab.id,
  };
  nextState = mapWindows(nextState, (windowData, tab) => {
    if (tab.id !== targetContext.tab.id) {
      return windowData;
    }
    const isTarget = windowData.id === operation.targetId;
    const isGroupMember =
      groupId !== null && windowData.tab_group_id === groupId;
    if (!isTarget && !isGroupMember) {
      return windowData;
    }
    const nextWindow = {
      ...windowData,
      z_index: optimisticZ,
    };
    if (isGroupMember) {
      nextWindow.tab_group_active = isTarget;
    }
    return nextWindow;
  });
  return nextState;
}

function applyPendingOperation(state, operation) {
  if (operation.scope === "project_tab") {
    return projectTabOperation(state, operation.targetId);
  }
  return windowOperation(state, operation);
}

function navigationTargetIndex(state) {
  const projectTabs = new Set();
  const windows = new Set();
  for (const tab of state?.tabs || []) {
    projectTabs.add(tab.id);
    for (const windowData of tab.workspace?.windows || []) {
      windows.add(windowData.id);
    }
  }
  return { projectTabs, windows };
}

function stateContainsTarget(targets, operation) {
  if (operation.scope === "project_tab") {
    return targets.projectTabs.has(operation.targetId);
  }
  return targets.windows.has(operation.targetId);
}

function safeRevision(value) {
  return Number.isSafeInteger(value) && value >= 0 ? value : null;
}

function defaultInteractionIdFactory() {
  const randomPart =
    globalThis.crypto?.randomUUID?.() ||
    `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  let sequence = 0;
  return () => `navigation-${randomPart}-${++sequence}`;
}

export function isNavigationRequest(message) {
  return Boolean(
    message &&
      typeof message.kind === "string" &&
      NAVIGATION_REQUESTS[message.kind],
  );
}

export function createNavigationPendingController({
  initialState = emptyState(),
  onState,
  createInteractionId = defaultInteractionIdFactory(),
} = {}) {
  let canonicalState = initialState || emptyState();
  let canonicalRevision = 0;
  let receivedVersionedState = false;
  let pending = [];
  let renderedState = canonicalState;
  let optimisticMaxZByTab = maxZByTab(canonicalState);

  const publish = () => {
    const latestOperation = pending.at(-1);
    renderedState = latestOperation
      ? applyPendingOperation(canonicalState, latestOperation)
      : canonicalState;
    onState?.(renderedState);
    return renderedState;
  };

  const rebuildOptimisticMaxZ = () => {
    optimisticMaxZByTab = maxZByTab(canonicalState);
    const latestOperation = pending.at(-1);
    if (latestOperation?.tabId) {
      latestOperation.optimisticZ =
        (optimisticMaxZByTab.get(latestOperation.tabId) || 0) + 1;
      optimisticMaxZByTab.set(
        latestOperation.tabId,
        latestOperation.optimisticZ,
      );
    }
  };

  return Object.freeze({
    begin(message) {
      const request = NAVIGATION_REQUESTS[message?.kind];
      if (!request) {
        return message;
      }
      const targetId = String(message?.[request.targetField] || "");
      const interactionId =
        typeof message.interaction_id === "string" && message.interaction_id
          ? message.interaction_id
          : createInteractionId();
      const operation = {
        interactionId,
        scope: request.scope,
        targetId,
      };
      if (request.scope !== "project_tab") {
        const context = findWindowContext(renderedState, targetId);
        if (context) {
          operation.tabId = context.tab.id;
          operation.optimisticZ =
            (optimisticMaxZByTab.get(context.tab.id) || 0) + 1;
          optimisticMaxZByTab.set(
            context.tab.id,
            operation.optimisticZ,
          );
        }
      }
      pending.push(operation);
      publish();
      return {
        ...message,
        interaction_id: interactionId,
      };
    },

    handleResult(event) {
      if (event?.kind !== "navigation_result") {
        return false;
      }
      const matchedIndex = pending.findIndex(
        (operation) => operation.interactionId === event.interaction_id,
      );
      if (matchedIndex < 0) {
        return false;
      }
      const revision = safeRevision(event.revision);
      // A result for a later interaction is also an ordering fence for every
      // earlier local intent. Keeping an older operation here would replay it
      // over the newly acknowledged final intent (A→B→A would flicker back to
      // B when A's result arrives first).
      pending = pending.filter((_, index) => index > matchedIndex);
      if (revision !== null && revision >= canonicalRevision) {
        receivedVersionedState = true;
        canonicalRevision = revision;
        canonicalState = applyCanonicalDelta(
          canonicalState,
          event.canonical,
        );
      }
      rebuildOptimisticMaxZ();
      publish();
      return true;
    },

    handleWorkspace(event) {
      const revisionPresent = hasOwn(event, "revision");
      const revision = safeRevision(event?.revision);
      if (revisionPresent && revision === null) {
        return false;
      }
      const versioned = revision !== null;
      if (
        (versioned && revision < canonicalRevision) ||
        (!versioned && receivedVersionedState)
      ) {
        return false;
      }
      if (versioned) {
        receivedVersionedState = true;
        canonicalRevision = revision;
      }
      canonicalState = event?.workspace || emptyState();
      const targets = navigationTargetIndex(canonicalState);
      pending = pending.filter((operation) => {
        if (!stateContainsTarget(targets, operation)) {
          return false;
        }
        if (!versioned) {
          return false;
        }
        // The matching navigation_result is ordered before the workspace
        // produced by this client's request. Until that result arrives, a
        // versioned snapshot may belong to another client and cannot retire
        // the local intent.
        return true;
      });
      rebuildOptimisticMaxZ();
      publish();
      return true;
    },

    clearPending() {
      pending = [];
      rebuildOptimisticMaxZ();
      publish();
    },

    resetConnection() {
      pending = [];
      canonicalRevision = 0;
      receivedVersionedState = false;
    },

    pendingCount() {
      return pending.length;
    },

    revision() {
      return canonicalRevision;
    },

    state() {
      return renderedState;
    },
  });
}
