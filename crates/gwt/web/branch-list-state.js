// Pure state-transition helpers for the Branches surface detail-check lifecycle
// (SPEC-2009 Phase 7 / FR-064..FR-067). Extracted from app.js so the reconnect
// self-heal, last-known retention, and stale-load guard are unit testable
// without a live WebSocket. No DOM or closure dependencies.
//
// Background: the embedded WebSocket server evicts a client when its bounded
// outbound queue overflows (backpressure protection). If that happens while a
// Branches detail-check hydration is in flight, the old model dropped every row
// to "Safety unknown" and parked an interrupted banner until a manual Refresh,
// because branch entries were not part of the reconnect replay. These helpers
// make the Branches surface recover automatically and keep showing the last
// verified cleanup safety in the meantime.

// Internal sentinel kept stable for state bookkeeping. The user-facing title is
// produced by branchLoadStatusSummary() and is intentionally reassuring, since
// the detail check now recovers automatically on reconnect.
export const BRANCH_DETAIL_CHECK_INTERRUPTED_NOTICE = "Branch detail check interrupted";

// Build a name -> cleanup index from hydrated entries so a later inventory /
// interruption can re-display the last verified badges.
export function indexHydratedCleanup(entries) {
  const byName = new Map();
  for (const entry of entries || []) {
    if (entry && entry.cleanup_ready && entry.cleanup) {
      byName.set(entry.name, entry.cleanup);
    }
  }
  return byName;
}

// FR-065: carry last-known cleanup into not-yet-hydrated entries so the list
// keeps showing real cleanup badges during re-hydration / after interruption
// instead of collapsing every row to "Safety unknown". Carried entries are
// flagged cleanup_stale (for a subtle "last verified" hint) and deliberately
// keep cleanup_ready === false so destructive selection stays gated on fresh
// verification.
export function carryOverLastKnownCleanup(entries, lastKnownByName) {
  if (!Array.isArray(entries) || !lastKnownByName || lastKnownByName.size === 0) {
    return Array.isArray(entries) ? entries : [];
  }
  return entries.map((entry) => {
    if (!entry || entry.cleanup_ready) {
      return entry;
    }
    const known = lastKnownByName.get(entry.name);
    if (!known) {
      return entry;
    }
    return { ...entry, cleanup: known, cleanup_stale: true };
  });
}

// FR-067: ignore a branch_entries event whose load_id is older than the newest
// load already applied to this window (stale, out-of-order delivery after an
// evict/reconnect when a previous load's background thread is still running).
export function isStaleBranchLoad(state, loadId) {
  if (!state || typeof loadId !== "number") {
    return false;
  }
  return typeof state.lastLoadId === "number" && loadId < state.lastLoadId;
}

// FR-064/FR-065: connection loss while a detail-check is in flight. Keep the
// rows and their last-known cleanup, flag the window as needing a re-sync, and
// stop the loading spinner — but do NOT wipe cleanup metadata to unknown.
export function markBranchDetailInterrupted(state) {
  if (!state || !state.loading) {
    return false;
  }
  state.loading = false;
  state.receivedFreshEntries = false;
  if (!Array.isArray(state.entries) || state.entries.length === 0) {
    state.error = "Connection lost while loading branches";
    state.notice = "";
    state.detailCheckStale = false;
    state.needsResync = true;
    return true;
  }
  state.error = "";
  state.entries = carryOverLastKnownCleanup(state.entries, state.lastHydratedByName);
  state.detailCheckStale = true;
  state.needsResync = true;
  state.notice = BRANCH_DETAIL_CHECK_INTERRUPTED_NOTICE;
  return true;
}

// FR-064: which open Branches windows should auto re-hydrate on reconnect.
export function branchWindowNeedsResync(state) {
  if (!state) {
    return false;
  }
  return Boolean(
    state.needsResync ||
      state.detailCheckStale ||
      state.notice === BRANCH_DETAIL_CHECK_INTERRUPTED_NOTICE,
  );
}

// Centralizes the FR-065/FR-067 ingest logic so the app.js branch_entries
// handler and the unit tests share one implementation. Mutates `state` and
// returns { applied } so the caller can skip DOM/telemetry work on stale drops.
export function applyBranchEntriesEvent(state, event) {
  if (!state || !event) {
    return { applied: false };
  }
  if (isStaleBranchLoad(state, event.load_id)) {
    return { applied: false };
  }
  if (typeof event.load_id === "number") {
    state.lastLoadId = event.load_id;
  }
  const phase = String(event.phase || "hydrated").toLowerCase();
  let entries = Array.isArray(event.entries) ? event.entries : [];
  if (phase === "hydrated") {
    state.lastHydratedByName = indexHydratedCleanup(entries);
    state.detailCheckStale = false;
    state.needsResync = false;
  } else {
    entries = carryOverLastKnownCleanup(entries, state.lastHydratedByName);
  }
  state.entries = entries;
  state.phase = phase;
  state.loading = phase !== "hydrated";
  state.receivedFreshEntries = true;
  state.error = "";
  state.notice = "";
  return { applied: true };
}

// Top-of-list status summary. FR-066: the interrupted state is now a small,
// non-blocking, reassuring inline notice (it self-heals on reconnect) rather
// than an alarming manual-refresh band.
export function branchLoadStatusSummary(state) {
  if (!state) {
    return null;
  }
  if (state.error) {
    return {
      kind: "error",
      title: "Branches unavailable",
      detail: state.error,
      hint: "Refresh to try again.",
    };
  }
  if (state.loading && Array.isArray(state.entries) && state.entries.length > 0) {
    return {
      kind: "checking",
      title: "Checking branch details",
      detail: "Loading branch details while cleanup safety is checked.",
      hint: "Cleanup selection unlocks after verification.",
    };
  }
  if (state.notice === BRANCH_DETAIL_CHECK_INTERRUPTED_NOTICE) {
    return {
      kind: "interrupted",
      title: "Reconnecting branch details",
      detail: "Showing the last verified cleanup safety while the connection recovers.",
      hint: "Recovering automatically — no refresh needed.",
    };
  }
  if (state.notice) {
    return {
      kind: "notice",
      title: "Branch notice",
      detail: state.notice,
      hint: "",
    };
  }
  return null;
}
