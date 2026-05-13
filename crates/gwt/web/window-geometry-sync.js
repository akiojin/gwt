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
  state.localEdits.set(id, {
    baseRevision: normalizeRevision(baseRevision),
    phase: "active",
  });
}

export function commitLocalGeometryEdit(state, id, baseRevision) {
  if (!state || !id) {
    return;
  }
  const existing = state.localEdits.get(id);
  state.localEdits.set(id, {
    baseRevision: normalizeRevision(baseRevision ?? existing?.baseRevision ?? 0),
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
  if (incomingRevision > localEdit.baseRevision) {
    state.localEdits.delete(id);
    return true;
  }
  return false;
}
