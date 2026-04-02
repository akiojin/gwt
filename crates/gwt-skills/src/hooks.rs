//! Hooks management — merge managed and user hooks while preserving ownership.

use serde::{Deserialize, Serialize};

/// Marker prefix that identifies gwt-managed hooks.
const GWT_MANAGED_MARKER: &str = "# gwt-managed";

/// A single hook definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hook {
    /// Event that triggers this hook (e.g. "pre-commit", "post-merge").
    pub event: String,
    /// Shell command to execute.
    pub command: String,
    /// Optional comment marker used to identify the hook's owner.
    pub comment_marker: Option<String>,
}

/// Configuration holding both managed and user hooks.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HooksConfig {
    /// Hooks managed by gwt (auto-generated, may be overwritten).
    pub managed_hooks: Vec<Hook>,
    /// Hooks added by the user (preserved across updates).
    pub user_hooks: Vec<Hook>,
}

/// Check whether a hook is gwt-managed based on its comment marker.
pub fn is_gwt_managed(hook: &Hook) -> bool {
    hook.comment_marker
        .as_deref()
        .is_some_and(|m| m.starts_with(GWT_MANAGED_MARKER))
}

/// Merge managed and user hooks into a single list.
///
/// Managed hooks come first, followed by user hooks. User hooks for the
/// same event are never overwritten.
pub fn merge_hooks(managed: &[Hook], user: &[Hook]) -> Vec<Hook> {
    let mut merged: Vec<Hook> = managed.to_vec();
    for uh in user {
        // Only add user hooks that don't duplicate a managed hook for the same event+command.
        let dominated = merged
            .iter()
            .any(|mh| mh.event == uh.event && mh.command == uh.command);
        if !dominated {
            merged.push(uh.clone());
        }
    }
    merged
}
