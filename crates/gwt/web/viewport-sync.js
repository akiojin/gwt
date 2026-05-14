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

  function updateScope(scopeKey) {
    const normalizedScopeKey = scopeKey ?? null;
    if (activeScopeKey === normalizedScopeKey) {
      return;
    }
    activeScopeKey = normalizedScopeKey;
    pendingLocalViewport = null;
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
      if (pendingLocalViewport) {
        if (viewportEquals(incoming, pendingLocalViewport, epsilon)) {
          currentViewport = incoming;
          pendingLocalViewport = null;
        }
        return cloneViewport(currentViewport);
      }
      currentViewport = incoming;
      return cloneViewport(currentViewport);
    },

    hasPendingLocalViewport() {
      return Boolean(pendingLocalViewport);
    },
  };
}
