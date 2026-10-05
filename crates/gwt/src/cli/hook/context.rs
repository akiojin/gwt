//! Prepared immutable state shared by one hook invocation.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use gwt_agent::Session;
use gwt_core::workspace_projection::{load_workspace_projection_from_path, WorkspaceProjection};

use super::HookError;

pub struct HookContext {
    audience_root: PathBuf,
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
            audience_root,
            audience_projection,
            canonical_project_projection,
        })
    }

    pub fn audience_root(&self) -> &Path {
        &self.audience_root
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
    // UserPromptSubmit is a read-only hot path: read canonical state without
    // acquiring works.lock so an Active Work refresh cannot block the hook.
    let path = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(repo_path);
    load_workspace_projection_from_path(&path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_agent::AgentId;

    #[test]
    fn issue_3777_legacy_projection_is_refused_without_mutation() {
        let home = tempfile::tempdir().expect("isolated home");
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let root = home.path().join("repo");
        std::fs::create_dir_all(&root).expect("project root");
        let mut session = Session::new(&root, "work/issue-3777", AgentId::Codex);
        session.project_state_root = Some(root.clone());
        let context = HookContext::for_board_reminder(&session).expect("fresh project context");
        assert!(context.audience_projection().is_none());
        assert!(context.canonical_project_projection().is_none());

        let projection = WorkspaceProjection::default_for_project(&root);
        let bytes = serde_json::to_vec(&projection).expect("serialize");
        let legacy =
            gwt_core::paths::gwt_project_dir_for_repo_path(&root).join("workspace/current.json");
        std::fs::create_dir_all(legacy.parent().expect("legacy directory"))
            .expect("create legacy directory");
        std::fs::write(&legacy, &bytes).expect("write legacy projection");

        let error = HookContext::for_board_reminder(&session)
            .err()
            .expect("legacy layout must be refused");

        assert!(error.to_string().contains("v9.106.0"));
        assert_eq!(std::fs::read(&legacy).unwrap(), bytes);
        let canonical = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&root);
        assert!(!canonical.exists());
        assert!(!canonical.with_file_name("works.json").exists());
    }

    #[test]
    fn issue_3777_same_audience_and_canonical_root_loads_projection_once() {
        let root = tempfile::tempdir().expect("project root");
        let mut session = Session::new(root.path(), "work/issue-3777", AgentId::Codex);
        session.project_state_root = Some(root.path().to_path_buf());
        let mut loaded = Vec::new();

        let context = HookContext::for_board_reminder_with_loader(&session, |path| {
            loaded.push(path.to_path_buf());
            Ok(Some(WorkspaceProjection::default_for_project(path)))
        })
        .expect("prepare hook context");

        assert_eq!(loaded.len(), 1);
        assert!(Arc::ptr_eq(
            context.audience_projection.as_ref().expect("audience"),
            context
                .canonical_project_projection
                .as_ref()
                .expect("canonical"),
        ));
    }

    #[test]
    fn issue_3777_distinct_audience_and_canonical_roots_load_each_once() {
        let audience = tempfile::tempdir().expect("audience root");
        let canonical = tempfile::tempdir().expect("canonical root");
        let mut session = Session::new(audience.path(), "work/issue-3777", AgentId::Codex);
        session.project_state_root = Some(canonical.path().to_path_buf());
        let mut loaded = Vec::new();

        let context = HookContext::for_board_reminder_with_loader(&session, |path| {
            loaded.push(path.to_path_buf());
            Ok(Some(WorkspaceProjection::default_for_project(path)))
        })
        .expect("prepare hook context");

        assert_eq!(loaded.len(), 2);
        assert_ne!(loaded[0], loaded[1]);
        assert!(!Arc::ptr_eq(
            context.audience_projection.as_ref().expect("audience"),
            context
                .canonical_project_projection
                .as_ref()
                .expect("canonical"),
        ));
    }
}
