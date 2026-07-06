//! Shared per-invocation hook context (SPEC-3248 hooks v2, P1).
//!
//! Hooks currently re-derive the session lane (and, elsewhere, the session /
//! workspace projection / Board scope) independently in each handler. hooks v2
//! consolidates that into a single [`HookContext`] resolved once per hook event
//! and passed to handlers, so behavior branches on a shared, deterministic
//! lane profile instead of ad-hoc `SessionKind::from_env()` calls.
//!
//! P1 seeds the context with the lane resolution (consuming the P0 lane file as
//! the source of truth, with an env fast-path fallback for pre-hooks-v2
//! worktrees). Later phases fold the duplicated session / projection / Board
//! loading into this same struct.

use std::path::Path;

use gwt_skills::LaneProfile;

/// Context shared across the handlers of a single hook invocation.
pub struct HookContext {
    /// The resolved lane profile for the worktree (deterministic; defaults to
    /// execution when no lane file / signal is present — FR-009).
    pub lane: &'static LaneProfile,
}

impl HookContext {
    /// Resolve the context for a worktree. The lane comes from the worktree's
    /// lane file (source of truth), falling back to the `GWT_SESSION_KIND` env
    /// fast-path and then to execution.
    #[must_use]
    pub fn for_worktree(worktree: &Path) -> Self {
        Self {
            lane: gwt_skills::resolve_lane_for_worktree(worktree),
        }
    }
}
