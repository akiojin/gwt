//! SPEC-3015: single source for the protocol enum contract shared with the
//! frontend.
//!
//! Generates `crates/gwt/web/protocol-enums.js` from the Rust serde enums so
//! the JS side never hand-copies wire values. Wire strings are derived by
//! serde-serializing each variant, so `#[serde(rename_all = ...)]` and
//! variant renames propagate automatically.
//!
//! Drift protection (FR-002):
//! - `protocol_enums_js_is_up_to_date` fails when the generated file on disk
//!   no longer matches the Rust definitions (stale detection).
//! - Each `all_*` variant list is guarded by an exhaustiveness unit test that
//!   `match`es every variant with NO wildcard arm — adding a Rust variant
//!   breaks compilation of that test and forces the list (and the
//!   regenerated JS) to be updated.
//!
//! Regenerate with:
//! `cargo test -p gwt regenerate_protocol_enums_js -- --ignored`

use serde::Serialize;

use crate::persistence::WindowState;
use gwt_core::work_projection::{
    WorkActiveLifecycleState, WorkspaceLifecycleStage, WorkspaceStatusCategory,
};

/// Every [`WindowState`] variant in wire order.
/// Guarded by `all_window_runtime_states_is_exhaustive`.
fn all_window_runtime_states() -> [WindowState; 6] {
    [
        WindowState::Running,
        WindowState::Starting,
        WindowState::Idle,
        WindowState::Waiting,
        WindowState::Stopped,
        WindowState::Error,
    ]
}

/// Every [`WorkspaceStatusCategory`] variant in wire order.
/// Guarded by `all_workspace_status_categories_is_exhaustive`.
fn all_workspace_status_categories() -> [WorkspaceStatusCategory; 5] {
    [
        WorkspaceStatusCategory::Active,
        WorkspaceStatusCategory::Idle,
        WorkspaceStatusCategory::Blocked,
        WorkspaceStatusCategory::Done,
        WorkspaceStatusCategory::Unknown,
    ]
}

/// Every [`WorkspaceLifecycleStage`] variant in wire order.
/// Guarded by `all_workspace_lifecycle_stages_is_exhaustive`.
fn all_workspace_lifecycle_stages() -> [WorkspaceLifecycleStage; 5] {
    [
        WorkspaceLifecycleStage::Planning,
        WorkspaceLifecycleStage::Active,
        WorkspaceLifecycleStage::InReview,
        WorkspaceLifecycleStage::Done,
        WorkspaceLifecycleStage::Archived,
    ]
}

/// Every [`WorkActiveLifecycleState`] variant in wire order.
/// Guarded by `all_work_active_lifecycle_states_is_exhaustive`.
fn all_work_active_lifecycle_states() -> [WorkActiveLifecycleState; 4] {
    [
        WorkActiveLifecycleState::Active,
        WorkActiveLifecycleState::Paused,
        WorkActiveLifecycleState::Done,
        WorkActiveLifecycleState::Discarded,
    ]
}

/// Serde-derived wire string of one enum variant (e.g. `InReview` →
/// `"in_review"`). Panics on non-string serialization, which would mean the
/// enum is no longer a plain unit-variant wire enum.
fn wire_value<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value).expect("protocol enum serializes to JSON") {
        serde_json::Value::String(text) => text,
        other => panic!("protocol enum serialized to a non-string wire value: {other}"),
    }
}

fn wire_values<T: Serialize>(variants: &[T]) -> Vec<String> {
    variants.iter().map(wire_value).collect()
}

fn push_js_export(out: &mut String, name: &str, values: &[String]) {
    out.push_str(&format!("export const {name} = Object.freeze([\n"));
    for value in values {
        out.push_str(&format!("  \"{value}\",\n"));
    }
    out.push_str("]);\n");
}

/// Full text of `crates/gwt/web/protocol-enums.js` (FR-001).
pub fn protocol_enums_js_source() -> String {
    let mut out = String::new();
    out.push_str(
        "// GENERATED FILE — do not edit; regenerate with \
         `cargo test -p gwt regenerate_protocol_enums_js -- --ignored`\n",
    );
    out.push_str(
        "//\n\
         // SPEC-3015: wire values of the Rust protocol enums shared with the\n\
         // frontend. Source of truth: crates/gwt/src/web_protocol_enums.rs\n\
         // (serde-serialized variants of WindowState, WorkspaceStatusCategory,\n\
         // WorkspaceLifecycleStage, WorkActiveLifecycleState). A stale copy\n\
         // fails `cargo test -p gwt protocol_enums_js_is_up_to_date`.\n\n",
    );
    push_js_export(
        &mut out,
        "WINDOW_RUNTIME_STATES",
        &wire_values(&all_window_runtime_states()),
    );
    out.push('\n');
    push_js_export(
        &mut out,
        "WORKSPACE_STATUS_CATEGORIES",
        &wire_values(&all_workspace_status_categories()),
    );
    out.push('\n');
    push_js_export(
        &mut out,
        "WORKSPACE_LIFECYCLE_STAGES",
        &wire_values(&all_workspace_lifecycle_stages()),
    );
    out.push('\n');
    push_js_export(
        &mut out,
        "WORK_ACTIVE_LIFECYCLE_STATES",
        &wire_values(&all_work_active_lifecycle_states()),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn assert_unique(values: &[String]) {
        let unique: BTreeSet<&String> = values.iter().collect();
        assert_eq!(
            unique.len(),
            values.len(),
            "duplicate wire values: {values:?}"
        );
    }

    /// Exhaustiveness guard: when a variant is added to [`WindowState`] this
    /// `match` (no wildcard arm) stops compiling, forcing
    /// `all_window_runtime_states` and the regenerated JS to be updated.
    #[test]
    fn all_window_runtime_states_is_exhaustive() {
        for state in all_window_runtime_states() {
            match state {
                WindowState::Running
                | WindowState::Starting
                | WindowState::Idle
                | WindowState::Waiting
                | WindowState::Stopped
                | WindowState::Error => {}
            }
        }
        assert_unique(&wire_values(&all_window_runtime_states()));
    }

    /// Exhaustiveness guard — see `all_window_runtime_states_is_exhaustive`.
    #[test]
    fn all_workspace_status_categories_is_exhaustive() {
        for category in all_workspace_status_categories() {
            match category {
                WorkspaceStatusCategory::Active
                | WorkspaceStatusCategory::Idle
                | WorkspaceStatusCategory::Blocked
                | WorkspaceStatusCategory::Done
                | WorkspaceStatusCategory::Unknown => {}
            }
        }
        assert_unique(&wire_values(&all_workspace_status_categories()));
    }

    /// Exhaustiveness guard — see `all_window_runtime_states_is_exhaustive`.
    #[test]
    fn all_workspace_lifecycle_stages_is_exhaustive() {
        for stage in all_workspace_lifecycle_stages() {
            match stage {
                WorkspaceLifecycleStage::Planning
                | WorkspaceLifecycleStage::Active
                | WorkspaceLifecycleStage::InReview
                | WorkspaceLifecycleStage::Done
                | WorkspaceLifecycleStage::Archived => {}
            }
        }
        assert_unique(&wire_values(&all_workspace_lifecycle_stages()));
    }

    /// Exhaustiveness guard — see `all_window_runtime_states_is_exhaustive`.
    #[test]
    fn all_work_active_lifecycle_states_is_exhaustive() {
        for state in all_work_active_lifecycle_states() {
            match state {
                WorkActiveLifecycleState::Active
                | WorkActiveLifecycleState::Paused
                | WorkActiveLifecycleState::Done
                | WorkActiveLifecycleState::Discarded => {}
            }
        }
        assert_unique(&wire_values(&all_work_active_lifecycle_states()));
    }

    #[test]
    fn generated_source_contains_expected_wire_values() {
        let source = protocol_enums_js_source();
        for expected in [
            "export const WINDOW_RUNTIME_STATES",
            "export const WORKSPACE_STATUS_CATEGORIES",
            "export const WORKSPACE_LIFECYCLE_STAGES",
            "export const WORK_ACTIVE_LIFECYCLE_STATES",
            "\"running\"",
            "\"starting\"",
            "\"in_review\"",
            "\"discarded\"",
        ] {
            assert!(
                source.contains(expected),
                "missing {expected:?} in:\n{source}"
            );
        }
    }

    /// FR-002 stale detection: the committed generated file must match the
    /// Rust definitions exactly.
    #[test]
    fn protocol_enums_js_is_up_to_date() {
        let generated = protocol_enums_js_source();
        let on_disk = include_str!("../web/protocol-enums.js");
        assert_eq!(
            on_disk, generated,
            "crates/gwt/web/protocol-enums.js is stale; regenerate with \
             `cargo test -p gwt regenerate_protocol_enums_js -- --ignored`"
        );
    }

    /// Regeneration entrypoint (writes the generated file in place).
    #[test]
    #[ignore = "writes crates/gwt/web/protocol-enums.js; run explicitly to regenerate"]
    fn regenerate_protocol_enums_js() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/protocol-enums.js");
        std::fs::write(&path, protocol_enums_js_source())
            .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
    }
}
