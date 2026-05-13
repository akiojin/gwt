function normalizeRevision(value) {
  if (!Number.isFinite(value) || value < 0) {
    return 0;
  }
  return Math.trunc(value);
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
