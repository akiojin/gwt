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

// Screen-space inset (px) between a maximized window and the visible viewport
// edges. A "complete maximize" fills the entire canvas work area edge-to-edge
// (between the project bar and the status strip), so this inset is 0. The
// division-by-zoom logic below is kept intact so any future non-zero inset
// would still render as a constant SCREEN-space gap regardless of zoom.
const MAXIMIZE_SCREEN_INSET = 0;

/**
 * Compute a maximized window's geometry from the visible viewport bounds.
 *
 * `bounds` is in WORLD space (the viewport size already divided by zoom), and
 * the window is positioned inside `#canvas-stage`, which applies
 * `scale(zoom)`. The inset is a constant SCREEN-space gap, so it must be
 * divided by zoom: `screenInset = worldInset * zoom`, hence
 * `worldInset = MAXIMIZE_SCREEN_INSET / zoom`. With a zero inset the geometry
 * equals `bounds` verbatim at every zoom, so the maximized window spans the
 * full canvas work area and never drifts off the viewport.
 */
export function maximizedGeometry(bounds, zoom = 1) {
  const normalizedZoom = positiveFiniteNumber(zoom, 1);
  const inset = MAXIMIZE_SCREEN_INSET / normalizedZoom;
  return {
    x: finiteNumber(bounds?.x) + inset,
    y: finiteNumber(bounds?.y) + inset,
    width: Math.max(finiteNumber(bounds?.width) - inset * 2, 0),
    height: Math.max(finiteNumber(bounds?.height) - inset * 2, 0),
  };
}

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
