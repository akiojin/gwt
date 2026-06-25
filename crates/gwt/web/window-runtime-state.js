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
