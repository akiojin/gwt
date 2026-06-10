// SPEC-2009 Phase 7 (FR-064..FR-067) unit tests for the Branches detail-check
// reconnect self-heal / last-known retention / stale-load guard. These cover
// the pure state transitions extracted into branch-list-state.js so the
// reconnect behavior is verifiable without a live WebSocket.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  BRANCH_DETAIL_CHECK_INTERRUPTED_NOTICE,
  indexHydratedCleanup,
  carryOverLastKnownCleanup,
  isStaleBranchLoad,
  markBranchDetailInterrupted,
  branchWindowNeedsResync,
  applyBranchEntriesEvent,
  branchLoadStatusSummary,
} from "../branch-list-state.js";

const hydratedEntry = (name, availability) => ({
  name,
  scope: "local",
  cleanup_ready: true,
  cleanup: { availability, blocked_reason: "", risks: [], merge_target: null },
});

const inventoryEntry = (name) => ({
  name,
  scope: "local",
  cleanup_ready: false,
  cleanup: { availability: "blocked", blocked_reason: "unknown", risks: [], merge_target: null },
});

test("indexHydratedCleanup maps only hydrated entries by name", () => {
  const byName = indexHydratedCleanup([
    hydratedEntry("work/a", "safe"),
    inventoryEntry("work/b"),
  ]);
  assert.equal(byName.size, 1);
  assert.equal(byName.get("work/a").availability, "safe");
  assert.equal(byName.has("work/b"), false);
});

test("carryOverLastKnownCleanup keeps last badge but does NOT mark fresh", () => {
  const last = indexHydratedCleanup([hydratedEntry("work/a", "safe")]);
  const [carried] = carryOverLastKnownCleanup([inventoryEntry("work/a")], last);
  // last-known badge is shown...
  assert.equal(carried.cleanup.availability, "safe");
  assert.equal(carried.cleanup_stale, true);
  // ...but selection stays gated because the data is not freshly verified.
  assert.equal(carried.cleanup_ready, false);
});

test("carryOverLastKnownCleanup is a no-op without prior hydration", () => {
  const entries = [inventoryEntry("work/a")];
  assert.deepEqual(carryOverLastKnownCleanup(entries, new Map()), entries);
});

test("isStaleBranchLoad drops out-of-order older load ids", () => {
  assert.equal(isStaleBranchLoad({ lastLoadId: 5 }, 4), true);
  assert.equal(isStaleBranchLoad({ lastLoadId: 5 }, 5), false);
  assert.equal(isStaleBranchLoad({ lastLoadId: 5 }, 6), false);
  assert.equal(isStaleBranchLoad({}, 1), false);
});

test("markBranchDetailInterrupted retains last-known cleanup instead of unknown", () => {
  const state = {
    loading: true,
    entries: [inventoryEntry("work/a")],
    lastHydratedByName: indexHydratedCleanup([hydratedEntry("work/a", "risky")]),
  };
  const changed = markBranchDetailInterrupted(state);
  assert.equal(changed, true);
  assert.equal(state.loading, false);
  // The row keeps a real badge — it must NOT collapse to "Safety unknown".
  assert.equal(state.entries[0].cleanup.availability, "risky");
  assert.equal(state.entries[0].cleanup_stale, true);
  assert.equal(state.entries[0].cleanup_ready, false);
  assert.equal(state.needsResync, true);
  assert.equal(state.notice, BRANCH_DETAIL_CHECK_INTERRUPTED_NOTICE);
});

test("markBranchDetailInterrupted with no rows reports a hard error, not interrupted", () => {
  const state = { loading: true, entries: [] };
  assert.equal(markBranchDetailInterrupted(state), true);
  assert.equal(state.error, "Connection lost while loading branches");
  assert.equal(state.notice, "");
  assert.equal(state.needsResync, true);
});

test("markBranchDetailInterrupted is a no-op when not loading", () => {
  const state = { loading: false, entries: [hydratedEntry("work/a", "safe")] };
  assert.equal(markBranchDetailInterrupted(state), false);
});

test("branchWindowNeedsResync detects interrupted/stale windows", () => {
  assert.equal(branchWindowNeedsResync({ needsResync: true }), true);
  assert.equal(branchWindowNeedsResync({ detailCheckStale: true }), true);
  assert.equal(
    branchWindowNeedsResync({ notice: BRANCH_DETAIL_CHECK_INTERRUPTED_NOTICE }),
    true,
  );
  assert.equal(branchWindowNeedsResync({ loading: false, notice: "" }), false);
});

test("applyBranchEntriesEvent hydrated phase records last-known and clears stale", () => {
  const state = { entries: [], detailCheckStale: true, needsResync: true };
  const res = applyBranchEntriesEvent(state, {
    phase: "hydrated",
    load_id: 2,
    entries: [hydratedEntry("work/a", "safe")],
  });
  assert.equal(res.applied, true);
  assert.equal(state.loading, false);
  assert.equal(state.detailCheckStale, false);
  assert.equal(state.needsResync, false);
  assert.equal(state.lastLoadId, 2);
  assert.equal(state.lastHydratedByName.get("work/a").availability, "safe");
});

test("applyBranchEntriesEvent inventory phase carries last-known badges", () => {
  const state = {
    entries: [],
    lastHydratedByName: indexHydratedCleanup([hydratedEntry("work/a", "safe")]),
  };
  const res = applyBranchEntriesEvent(state, {
    phase: "inventory",
    load_id: 3,
    entries: [inventoryEntry("work/a")],
  });
  assert.equal(res.applied, true);
  assert.equal(state.loading, true);
  assert.equal(state.entries[0].cleanup.availability, "safe");
  assert.equal(state.entries[0].cleanup_stale, true);
});

test("applyBranchEntriesEvent ignores a stale (older load_id) event", () => {
  const state = { entries: [hydratedEntry("work/a", "safe")], lastLoadId: 9 };
  const res = applyBranchEntriesEvent(state, {
    phase: "hydrated",
    load_id: 7,
    entries: [hydratedEntry("work/a", "blocked")],
  });
  assert.equal(res.applied, false);
  // unchanged
  assert.equal(state.entries[0].cleanup.availability, "safe");
});

test("branchLoadStatusSummary interrupted copy self-heals and never says 'Refresh to verify'", () => {
  const summary = branchLoadStatusSummary({
    loading: false,
    entries: [hydratedEntry("work/a", "safe")],
    notice: BRANCH_DETAIL_CHECK_INTERRUPTED_NOTICE,
  });
  assert.equal(summary.kind, "interrupted");
  assert.match(summary.title, /Reconnecting/i);
  assert.doesNotMatch(`${summary.detail} ${summary.hint}`, /Refresh to verify/i);
  assert.match(`${summary.hint}`, /automatically|no refresh/i);
});
