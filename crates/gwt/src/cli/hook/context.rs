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
//! worktrees). The Board reminder additionally preloads its audience and
//! canonical project projections here so one invocation never deserializes the
//! same projection separately for audience, title, stale-title, and progress
//! decisions.

use std::{path::Path, sync::Arc};

use gwt_agent::Session;
use gwt_core::workspace_projection::{load_workspace_projection, WorkspaceProjection};
use gwt_skills::LaneProfile;

use super::HookError;

/// Context shared across the handlers of a single hook invocation.
pub struct HookContext {
    /// The resolved lane profile for the worktree (deterministic; defaults to
    /// execution when no lane file / signal is present — FR-009).
    pub lane: &'static LaneProfile,
    audience_projection: Option<Arc<WorkspaceProjection>>,
    canonical_project_projection: Option<Arc<WorkspaceProjection>>,
}

impl HookContext {
    /// Resolve the context for a worktree. The lane comes from the worktree's
    /// lane file (source of truth), falling back to the `GWT_SESSION_KIND` env
    /// fast-path and then to execution.
    #[must_use]
    pub fn for_worktree(worktree: &Path) -> Self {
        Self {
            lane: gwt_skills::resolve_lane_for_worktree(worktree),
            audience_projection: None,
            canonical_project_projection: None,
        }
    }

    /// Resolve the Board-reminder context once for this invocation.
    ///
    /// The worktree projection determines Board audience. The canonical
    /// project projection drives title/stale/progress decisions. When both
    /// roots are identical, the same `Arc` is shared and only one load occurs.
    pub fn for_board_reminder(session: &Session) -> Result<Self, HookError> {
        Self::for_board_reminder_with_loader(session, load_workspace_projection)
    }

    fn for_board_reminder_with_loader<F>(session: &Session, mut load: F) -> Result<Self, HookError>
    where
        F: FnMut(&Path) -> gwt_core::Result<Option<WorkspaceProjection>>,
    {
        super::diagnostics::record_projection_load();
        let audience_projection = load(&session.worktree_path)?.map(Arc::new);
        let canonical_root = crate::agent_project_state::canonical_project_state_root_for_session(
            session,
            &session.worktree_path,
        );
        let normalized_audience_root = dunce::canonicalize(&session.worktree_path)
            .unwrap_or_else(|_| session.worktree_path.clone());
        let canonical_project_projection = if canonical_root == normalized_audience_root {
            audience_projection.clone()
        } else {
            super::diagnostics::record_projection_load();
            load(&canonical_root)?.map(Arc::new)
        };

        Ok(Self {
            lane: gwt_skills::resolve_lane_for_worktree(&session.worktree_path),
            audience_projection,
            canonical_project_projection,
        })
    }

    #[must_use]
    pub fn audience_projection(&self) -> Option<&WorkspaceProjection> {
        self.audience_projection.as_deref()
    }

    #[must_use]
    pub fn canonical_project_projection(&self) -> Option<&WorkspaceProjection> {
        self.canonical_project_projection.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_agent::AgentId;

    #[test]
    fn board_reminder_context_loads_each_projection_once() {
        let temp = tempfile::tempdir().expect("tempdir");
        let worktree = temp.path().join("worktree");
        let canonical = temp.path().join("canonical");
        std::fs::create_dir_all(&worktree).expect("worktree");
        std::fs::create_dir_all(&canonical).expect("canonical");
        let mut session = Session::new(&worktree, "work/context", AgentId::Codex);
        session.project_state_root = Some(canonical.clone());
        let mut loaded = Vec::new();

        let context = HookContext::for_board_reminder_with_loader(&session, |path| {
            loaded.push(path.to_path_buf());
            Ok(Some(WorkspaceProjection::default_for_project(path)))
        })
        .expect("context");

        assert_eq!(
            loaded,
            vec![
                worktree,
                dunce::canonicalize(&canonical).unwrap_or(canonical)
            ]
        );
        assert!(context.audience_projection().is_some());
        assert!(context.canonical_project_projection().is_some());
    }

    #[test]
    fn board_reminder_context_shares_projection_when_roots_match() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut session = Session::new(&repo, "work/context", AgentId::Codex);
        session.project_state_root = Some(repo.clone());
        let mut loaded = Vec::new();

        let context = HookContext::for_board_reminder_with_loader(&session, |path| {
            loaded.push(path.to_path_buf());
            Ok(Some(WorkspaceProjection::default_for_project(path)))
        })
        .expect("context");

        assert_eq!(loaded, vec![repo]);
        assert!(std::ptr::eq(
            context.audience_projection().expect("audience projection"),
            context
                .canonical_project_projection()
                .expect("canonical projection"),
        ));
    }
}
