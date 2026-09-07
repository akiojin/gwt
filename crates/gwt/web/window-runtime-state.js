// SPEC-3015 — window runtime state normalization and labeling, extracted
// from app.js (first extraction slice). Pure logic only: no DOM, no
// WebSocket, no app.js state. The wire-state list comes from the generated
// protocol enum contract so Rust-side renames propagate mechanically.

import { WINDOW_RUNTIME_STATES } from "./protocol-enums.js";

function capitalizeRuntimeState(state) {
  return state.charAt(0).toUpperCase() + state.slice(1);
}

// Display labels derived from the generated wire states (running →
// "Running", ...). Deriving instead of hand-writing guarantees every wire
// state has a label; `windowRuntimeLabel` falls back to the `running` label
// for anything unknown.
export const WINDOW_RUNTIME_STATE_LABELS = Object.freeze(
  Object.fromEntries(
    WINDOW_RUNTIME_STATES.map((state) => [state, capitalizeRuntimeState(state)]),
  ),
);

// US-69: the pre-lifecycle state is now `starting`. Legacy `not_started`
// spellings (and the older `starting`→running conflation) normalize to it.
//
// Hand-written on purpose: this is frontend display-compat normalization,
// not the serde wire contract. The Rust side keeps its own deserialization
// aliases (e.g. persistence maps legacy "ready" to Running), while the UI
// deliberately presents legacy "ready" as idle — so this table cannot be
// generated from the Rust enums.
export const LEGACY_WINDOW_RUNTIME_STATE_ALIASES = Object.freeze({
  not_started: "starting",
  notstarted: "starting",
  "not-started": "starting",
  ready: "idle",
  exited: "stopped",
});

export function presetSupportsWaitingStatus(preset) {
  return preset === "agent" || preset === "claude" || preset === "codex";
}

function isCanvasAgentFocusCandidate(windowData) {
  const placementKind = windowData?.placement?.kind;
  if (placementKind && placementKind !== "canvas") {
    return false;
  }
  const agentId = String(windowData?.agent_id || "").trim();
  return Boolean(
    windowData?.id &&
      (agentId || presetSupportsWaitingStatus(windowData?.preset)),
  );
}

function agentFocusPriority(windowData, runtimeStateForWindow) {
  const resolvedRuntimeState =
    typeof runtimeStateForWindow === "function"
      ? runtimeStateForWindow(windowData)
      : undefined;
  const rawState = String(
    resolvedRuntimeState ?? windowData?.status ?? "running",
  ).toLowerCase();
  // Display normalization intentionally falls back unknown states to running.
  // Focus ordering must instead fail closed: future states belong to the final
  // bucket until their interaction priority is explicitly designed.
  const runtimeState = LEGACY_WINDOW_RUNTIME_STATE_ALIASES[rawState] || rawState;
  if (runtimeState === "running" || runtimeState === "starting") {
    return 0;
  }
  if (runtimeState === "waiting" || runtimeState === "idle") {
    return 1;
  }
  return 2;
}

// Issue #3551 — focus navigation uses a deterministic Agent-only projection
// of the Canvas workspace. Hidden tab members stay in the projection so the
// caller can activate them before framing. Sorting by the original index after
// the runtime bucket makes the order independent of z-index and render state.
export function selectNextAgentFocusWindowId(
  windows,
  focusedId,
  direction = "forward",
  runtimeStateForWindow,
) {
  const candidates = (windows || [])
    .map((windowData, index) => ({ windowData, index }))
    .filter(({ windowData }) => isCanvasAgentFocusCandidate(windowData))
    .map((candidate) => ({
      ...candidate,
      priority: agentFocusPriority(
        candidate.windowData,
        runtimeStateForWindow,
      ),
    }))
    .sort(
      (left, right) =>
        left.priority - right.priority || left.index - right.index,
    )
    .map(({ windowData }) => windowData);

  if (candidates.length === 0) {
    return null;
  }

  const currentIndex = candidates.findIndex(
    (windowData) => windowData.id === focusedId,
  );
  if (currentIndex === -1) {
    return direction === "backward"
      ? candidates[candidates.length - 1].id
      : candidates[0].id;
  }

  const delta = direction === "backward" ? -1 : 1;
  const nextIndex =
    (currentIndex + delta + candidates.length) % candidates.length;
  return candidates[nextIndex].id;
}

export function normalizeWindowRuntimeState(status, preset) {
  const rawState = String(status || "running").toLowerCase();
  const normalizedState = LEGACY_WINDOW_RUNTIME_STATE_ALIASES[rawState] || rawState;
  if (!presetSupportsWaitingStatus(preset) && normalizedState === "waiting") {
    return "running";
  }
  if (!WINDOW_RUNTIME_STATE_LABELS[normalizedState]) {
    return "running";
  }
  return normalizedState;
}

export function windowRuntimeLabel(status) {
  return WINDOW_RUNTIME_STATE_LABELS[status] || WINDOW_RUNTIME_STATE_LABELS.running;
}

// SPEC-2356 — translate runtime state vocabulary to Operator telemetry states
// (`running|idle|waiting|error|done`). The mapping stays intentionally narrow
// so future runtime states surface as `idle` until the design language
// explicitly handles them.
//
// FR-039 (anshin): `waiting` (the agent is blocked on the operator's input)
// is its own LOUD state instead of collapsing into quiet `idle`. The wire
// `"waiting"` value is only emitted for agent/claude/codex presets (gated in
// normalizeWindowRuntimeState), so non-agent windows never reach here. The
// pre-lifecycle `starting` state aggregates into RUNNING for the Status Strip.
export function mapAgentTelemetryState(runtimeState) {
  switch (runtimeState) {
    case "running":
    case "starting":
      return "running";
    case "waiting":
      return "waiting";
    case "ready":
    case "idle":
      return "idle";
    case "stopped":
    case "exited":
      return "done";
    case "error":
      return "error";
    default:
      return "idle";
  }
}
