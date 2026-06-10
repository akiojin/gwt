// GENERATED FILE — do not edit; regenerate with `cargo test -p gwt regenerate_protocol_enums_js -- --ignored`
//
// SPEC-3015: wire values of the Rust protocol enums shared with the
// frontend. Source of truth: crates/gwt/src/web_protocol_enums.rs
// (serde-serialized variants of WindowState, WorkspaceStatusCategory,
// WorkspaceLifecycleStage, WorkActiveLifecycleState). A stale copy
// fails `cargo test -p gwt protocol_enums_js_is_up_to_date`.

export const WINDOW_RUNTIME_STATES = Object.freeze([
  "running",
  "starting",
  "idle",
  "waiting",
  "stopped",
  "error",
]);

export const WORKSPACE_STATUS_CATEGORIES = Object.freeze([
  "active",
  "idle",
  "blocked",
  "done",
  "unknown",
]);

export const WORKSPACE_LIFECYCLE_STAGES = Object.freeze([
  "planning",
  "active",
  "in_review",
  "done",
  "archived",
]);

export const WORK_ACTIVE_LIFECYCLE_STATES = Object.freeze([
  "active",
  "paused",
  "done",
  "discarded",
]);
