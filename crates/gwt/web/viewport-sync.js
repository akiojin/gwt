function finiteOrDefault(value, fallback) {
  return Number.isFinite(value) ? value : fallback;
}

function normalizeViewport(viewport) {
  return {
    x: finiteOrDefault(viewport?.x, 0),
    y: finiteOrDefault(viewport?.y, 0),
    zoom: finiteOrDefault(viewport?.zoom, 1),
  };
}

function cloneViewport(viewport) {
  return {
    x: viewport.x,
    y: viewport.y,
    zoom: viewport.zoom,
  };
}

function viewportEquals(left, right, epsilon) {
  return (
    Math.abs(left.x - right.x) <= epsilon &&
    Math.abs(left.y - right.y) <= epsilon &&
    Math.abs(left.zoom - right.zoom) <= epsilon
  );
}

export function createViewportSyncState({
  initialViewport = { x: 0, y: 0, zoom: 1 },
  epsilon = 0.0001,
} = {}) {
  let currentViewport = normalizeViewport(initialViewport);
  let pendingLocalViewport = null;
  let activeScopeKey = null;
  // SPEC-2008 camera-focus (FR-095): the camera is PER-VIEWER. We adopt the
  // server viewport exactly once per scope (the initial restore of the
  // persisted camera) and ignore every later server viewport so one client
  // panning / zooming / framing a window never drags another client's camera.
  let hasAppliedInitialServerViewport = false;

  function updateScope(scopeKey) {
    const normalizedScopeKey = scopeKey ?? null;
    if (activeScopeKey === normalizedScopeKey) {
      return;
    }
    activeScopeKey = normalizedScopeKey;
    pendingLocalViewport = null;
    hasAppliedInitialServerViewport = false;
  }

  return {
    applyLocalViewport(nextViewport, { scopeKey } = {}) {
      updateScope(scopeKey);
      currentViewport = normalizeViewport(nextViewport);
      pendingLocalViewport = cloneViewport(currentViewport);
      return cloneViewport(currentViewport);
    },

    applyServerViewport(nextViewport, { scopeKey } = {}) {
      updateScope(scopeKey);
      const incoming = normalizeViewport(nextViewport);
      // Preserve the initial-adopt reconciliation: if a local viewport was
      // applied before the server echo arrived, only adopt the echo once it
      // matches (so an in-flight local edit is not clobbered). Reaching this
      // match marks the initial server viewport as applied for this scope.
      if (pendingLocalViewport) {
        if (viewportEquals(incoming, pendingLocalViewport, epsilon)) {
          currentViewport = incoming;
          pendingLocalViewport = null;
          hasAppliedInitialServerViewport = true;
        }
        return cloneViewport(currentViewport);
      }
      // FR-095: adopt the persisted server camera exactly once per scope. Every
      // later server viewport is ignored so the camera stays local to this
      // viewer (pan / zoom / framing must not propagate between clients).
      if (hasAppliedInitialServerViewport) {
        return cloneViewport(currentViewport);
      }
      hasAppliedInitialServerViewport = true;
      currentViewport = incoming;
      return cloneViewport(currentViewport);
    },

    hasPendingLocalViewport() {
      return Boolean(pendingLocalViewport);
    },
  };
}
