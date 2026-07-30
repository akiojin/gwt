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

// Issue #3364 — the guard must be decidable at processing time WITHOUT
// knowing the commit echo's revision: under load the receive queue holds
// broadcasts whose revisions already passed the client model, so revision
// arithmetic can neither suppress them nor recognise the echo. The guard
// therefore acks the echo by geometry CONTENT and bounds itself with expiry
// windows instead. A leaked active gesture stops suppressing after
// `ACTIVE_EDIT_EXPIRY_MS` (mirrors the resize staleness guard); a pending
// commit whose echo never arrives (lost socket) yields to server truth after
// `PENDING_EDIT_EXPIRY_MS`.
export const ACTIVE_EDIT_EXPIRY_MS = 30_000;
export const PENDING_EDIT_EXPIRY_MS = 15_000;
// Style strings and serde f64 round-trips can drift below a pixel; anything
// closer than this counts as "the same placement".
const ECHO_MATCH_EPSILON = 0.5;

export function createGeometrySyncState() {
  return {
    localEdits: new Map(),
  };
}

export function workspaceGeometryRevision(windowData) {
  return normalizeRevision(windowData?.geometry_revision ?? 0);
}

export function beginLocalGeometryEdit(state, id, baseRevision, now = Date.now()) {
  if (!state || !id) {
    return;
  }
  const existing = state.localEdits.get(id);
  const normalizedBaseRevision = normalizeRevision(baseRevision);
  state.localEdits.set(id, {
    baseRevision: normalizedBaseRevision,
    optimisticRevision: normalizedBaseRevision,
    phase: "active",
    committedGeometry: null,
    startedAt: finiteNumber(now),
    committedAt: null,
    previousPending:
      existing?.phase === "pending"
        ? clonePendingEdit(existing)
        : clonePendingEdit(existing?.previousPending),
  });
}

export function commitLocalGeometryEdit(
  state,
  id,
  baseRevision,
  geometry = null,
  now = Date.now(),
) {
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
    committedGeometry: geometry ? { ...geometry } : null,
    startedAt: existing?.startedAt ?? finiteNumber(now),
    committedAt: finiteNumber(now),
  });
}

export function clearLocalGeometryEdit(state, id) {
  if (!state || !id) {
    return;
  }
  state.localEdits.delete(id);
}

function clonePendingEdit(edit) {
  if (edit?.phase !== "pending") {
    return null;
  }
  return {
    ...edit,
    committedGeometry: edit.committedGeometry
      ? { ...edit.committedGeometry }
      : null,
    previousPending: null,
  };
}

// A pointerdown can start while the previous geometry commit is still
// waiting for its server echo. Cancelling that follow-up gesture (including
// a titlebar click with no movement) must restore the older pending guard;
// otherwise a queued stale workspace_state can snap the window back.
export function cancelLocalGeometryEdit(state, id) {
  if (!state || !id) {
    return;
  }
  const existing = state.localEdits.get(id);
  if (existing?.phase !== "active") {
    return;
  }
  const previousPending = clonePendingEdit(existing.previousPending);
  if (previousPending) {
    state.localEdits.set(id, previousPending);
  } else {
    state.localEdits.delete(id);
  }
}

function geometryMatches(incoming, committed) {
  if (!incoming || !committed) {
    return false;
  }
  return (
    Math.abs(finiteNumber(incoming.x) - finiteNumber(committed.x)) <= ECHO_MATCH_EPSILON &&
    Math.abs(finiteNumber(incoming.y) - finiteNumber(committed.y)) <= ECHO_MATCH_EPSILON &&
    Math.abs(finiteNumber(incoming.width) - finiteNumber(committed.width)) <=
      ECHO_MATCH_EPSILON &&
    Math.abs(finiteNumber(incoming.height) - finiteNumber(committed.height)) <=
      ECHO_MATCH_EPSILON
  );
}

// Decide what an incoming `workspace_state` window geometry means for a local
// gesture. Returns `{ apply, patchGeometry }`:
// - `apply: true` — no guard (or the guard was just released): render the
//   incoming geometry as-is.
// - `apply: false` — the incoming geometry is a backlogged stale broadcast;
//   the caller must keep the local truth. `patchGeometry` carries the
//   committed geometry for pending commits, or `null` while the gesture is
//   still active (the caller patches from the live DOM instead).
export function resolveIncomingGeometry(state, { id, geometry, now = Date.now() }) {
  const applied = { apply: true, patchGeometry: null };
  if (!state || !id) {
    return applied;
  }
  const localEdit = state.localEdits.get(id);
  if (!localEdit) {
    return applied;
  }
  const timestamp = finiteNumber(now);
  if (localEdit.phase === "active") {
    if (timestamp - finiteNumber(localEdit.startedAt) > ACTIVE_EDIT_EXPIRY_MS) {
      state.localEdits.delete(id);
      return applied;
    }
    return { apply: false, patchGeometry: null };
  }
  if (timestamp - finiteNumber(localEdit.committedAt) > PENDING_EDIT_EXPIRY_MS) {
    state.localEdits.delete(id);
    return applied;
  }
  if (geometryMatches(geometry, localEdit.committedGeometry)) {
    state.localEdits.delete(id);
    return applied;
  }
  return {
    apply: false,
    patchGeometry: localEdit.committedGeometry
      ? { ...localEdit.committedGeometry }
      : null,
  };
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
