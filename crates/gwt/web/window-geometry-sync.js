function normalizeRevision(value) {
  if (!Number.isFinite(value) || value < 0) {
    return 0;
  }
  return Math.trunc(value);
}

function finiteNumber(value, fallback = 0) {
  return Number.isFinite(value) ? value : fallback;
}

function positiveFiniteNumber(value, fallback) {
  return Number.isFinite(value) && value > 0 ? value : fallback;
}

// SPEC-2008 camera-focus: maximize/minimize were replaced by a per-viewer
// camera that flies the canvas to frame a window in place, so the
// `maximizedGeometry` helper and its `MAXIMIZE_SCREEN_INSET` constant were
// removed. Windows always render at their own `geometry`; framing is a
// viewport concern owned by app.js (`frameWindow` / `enterOverview`).

export function createGeometrySyncState() {
  return {
    localEdits: new Map(),
  };
}

export function workspaceGeometryRevision(windowData) {
  return normalizeRevision(windowData?.geometry_revision ?? 0);
}

export function beginLocalGeometryEdit(state, id, baseRevision) {
  if (!state || !id) {
    return;
  }
  const normalizedBaseRevision = normalizeRevision(baseRevision);
  state.localEdits.set(id, {
    baseRevision: normalizedBaseRevision,
    optimisticRevision: normalizedBaseRevision,
    phase: "active",
  });
}

export function commitLocalGeometryEdit(state, id, baseRevision) {
  if (!state || !id) {
    return;
  }
  const existing = state.localEdits.get(id);
  const normalizedBaseRevision = normalizeRevision(
    baseRevision ?? existing?.baseRevision ?? 0,
  );
  state.localEdits.set(id, {
    baseRevision: normalizedBaseRevision,
    optimisticRevision: normalizedBaseRevision + 1,
    phase: "pending",
  });
}

export function clearLocalGeometryEdit(state, id) {
  if (!state || !id) {
    return;
  }
  state.localEdits.delete(id);
}

export function shouldApplyWorkspaceGeometry(state, { id, geometryRevision }) {
  if (!state || !id) {
    return true;
  }
  const localEdit = state.localEdits.get(id);
  if (!localEdit) {
    return true;
  }
  const incomingRevision = normalizeRevision(geometryRevision);
  const acceptedRevision =
    localEdit.phase === "pending"
      ? normalizeRevision(localEdit.optimisticRevision)
      : localEdit.baseRevision;
  if (
    incomingRevision > acceptedRevision ||
    (localEdit.phase === "pending" && incomingRevision === acceptedRevision)
  ) {
    state.localEdits.delete(id);
    return true;
  }
  return false;
}

export function localGeometryBaseRevision(state, id, windowData) {
  const workspaceRevision = workspaceGeometryRevision(windowData);
  if (!state || !id) {
    return workspaceRevision;
  }
  const localEdit = state.localEdits.get(id);
  if (!localEdit) {
    return workspaceRevision;
  }
  return Math.max(
    workspaceRevision,
    normalizeRevision(localEdit.optimisticRevision),
  );
}

export function syncResizeStatePointerEvent(state, event) {
  if (!state || !event) {
    return false;
  }
  if (!Number.isFinite(event.clientX) || !Number.isFinite(event.clientY)) {
    return false;
  }
  state.latestClientX = event.clientX;
  state.latestClientY = event.clientY;
  return true;
}

export function resizeGeometryFromPointerState(
  state,
  { zoom = 1, minWidth = 420, minHeight = 260 } = {},
) {
  const normalizedZoom = positiveFiniteNumber(zoom, 1);
  const minimumWidth = positiveFiniteNumber(minWidth, 420);
  const minimumHeight = positiveFiniteNumber(minHeight, 260);
  const startX = finiteNumber(state?.startX);
  const startY = finiteNumber(state?.startY);
  const clientX = finiteNumber(state?.latestClientX, startX);
  const clientY = finiteNumber(state?.latestClientY, startY);
  const baseWidth = finiteNumber(state?.width, minimumWidth);
  const baseHeight = finiteNumber(state?.height, minimumHeight);

  return {
    clientX,
    clientY,
    width: Math.max(minimumWidth, baseWidth + (clientX - startX) / normalizedZoom),
    height: Math.max(minimumHeight, baseHeight + (clientY - startY) / normalizedZoom),
  };
}
