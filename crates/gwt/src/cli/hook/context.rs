//! Prepared immutable state shared by one hook invocation.

use std::{path::Path, sync::Arc};

use gwt_agent::Session;
use gwt_core::workspace_projection::{load_workspace_projection_from_path, WorkspaceProjection};

use super::HookError;

pub struct HookContext {
    audience_projection: Option<Arc<WorkspaceProjection>>,
    canonical_project_projection: Option<Arc<WorkspaceProjection>>,
}

impl HookContext {
    pub fn for_board_reminder(session: &Session) -> Result<Self, HookError> {
        Self::for_board_reminder_with_loader(session, load_hook_workspace_projection)
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
        let audience_root = dunce::canonicalize(&session.worktree_path)
            .unwrap_or_else(|_| session.worktree_path.clone());
        let canonical_project_projection = if canonical_root == audience_root {
            audience_projection.clone()
        } else {
            super::diagnostics::record_projection_load();
            load(&canonical_root)?.map(Arc::new)
        };
        Ok(Self {
            audience_projection,
            canonical_project_projection,
        })
    }

    pub fn audience_projection(&self) -> Option<&WorkspaceProjection> {
        self.audience_projection.as_deref()
    }

    pub fn canonical_project_projection(&self) -> Option<&WorkspaceProjection> {
        self.canonical_project_projection.as_deref()
    }
}

fn load_hook_workspace_projection(
    repo_path: &Path,
) -> gwt_core::Result<Option<WorkspaceProjection>> {
    // UserPromptSubmit is a read-only hot path. The general loader performs a
    // legacy migration when current.json is absent, which acquires works.lock
    // and can collide with the repository-scale Active Work refresh that this
    // hook must remain responsive under. Startup owns migration; the hook
    // consumes only the canonical projection and treats absence as no scope.
    let path = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(repo_path);
    load_workspace_projection_from_path(&path)
}
