use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use gwt_agent::{Session, GWT_SESSION_ID_ENV};
use gwt_core::work_projection::{
    load_or_default_workspace_projection, save_workspace_projection, WorkProjection,
    WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary, WorkspaceStatusCategory,
};

use super::HookError;

/// SessionStart hook: ensure the running agent session is present in the
/// Workspace projection's `agents[]` before any further coordination CLI
/// runs. Without this, `gwtd workspace update --agent-session ... --title-summary ...`
/// silently no-ops because `apply_update`'s session matcher finds nothing
/// to update (only a journal entry is written, and no `WorkspaceState`
/// broadcast fires).
///
/// Registration is idempotent: if the launch flow has already registered
/// the session (with `Assigned` affiliation, a workspace_id, etc.) we
/// leave that record untouched so the richer launch-time state survives.
///
/// SPEC-2359 Phase W-11 (US-58 / FR-341 / FR-342): the hook no longer
/// derives `title_summary` from the prompt or from the linked Issue. The
/// `title_summary` field now means "the purpose the agent authored". Empty
/// values are resolved at display time via the fallback chain (agent title
/// → linked Issue/SPEC title → neutral label) in the title sync layer.
pub(crate) fn handle_session_start() -> Result<(), HookError> {
    let Some(session) = current_session_from_env()? else {
        return Ok(());
    };
    ensure_coordination_assets_for_session(&session);
    let project_state_root = project_state_root_for_session(&session);
    let mut projection = load_or_default_workspace_projection(&project_state_root)?;
    projection.project_root = project_state_root.clone();
    let now = Utc::now();
    let registered = register_session_in_projection(&mut projection, &session, now);
    if registered {
        save_workspace_projection(&project_state_root, &projection)?;
        crate::cli::workspace::publish_workspace_change(&project_state_root);
    }
    Ok(())
}

fn project_state_root_for_session(session: &Session) -> PathBuf {
    crate::agent_project_state::canonical_project_state_root_for_session(
        session,
        &session.worktree_path,
    )
}

/// SPEC-2359 Phase U-9 (FR-177): re-materialize coordination skill + hook
/// assets on SessionStart when they are missing for an already-present
/// agent target. This closes the gap where a worktree created before the
/// generator was added (or partially cleaned) is missing
/// `.codex/skills/gwt-coordination/SKILL.md` or `.codex/hooks.json`
/// while other Codex assets are still present, leaving Codex without
/// canonical title-summary guidance. Best-effort: failures are swallowed
/// so the hook does not block agent start.
fn ensure_coordination_assets_for_session(session: &Session) {
    if !coordination_assets_need_refresh(&session.worktree_path) {
        return;
    }
    let _ = crate::managed_assets::refresh_existing_managed_gwt_assets_for_worktree(
        &session.worktree_path,
    );
}

fn coordination_assets_need_refresh(worktree: &Path) -> bool {
    let codex_dir = worktree.join(".codex");
    let codex_skill = worktree.join(".codex/skills/gwt-coordination/SKILL.md");
    let codex_hooks = worktree.join(".codex/hooks.json");
    let claude_dir = worktree.join(".claude");
    let claude_skill = worktree.join(".claude/skills/gwt-coordination/SKILL.md");
    let claude_settings = worktree.join(".claude/settings.local.json");

    let codex_needs = codex_dir.is_dir() && (!codex_skill.exists() || !codex_hooks.exists());
    let claude_needs = claude_dir.is_dir() && (!claude_skill.exists() || !claude_settings.exists());
    codex_needs || claude_needs
}

/// Insert a stub `WorkspaceAgentSummary` for `session` if no agent with
/// the same `session_id` is present. Returns `true` when a new record
/// was inserted. Existing records are preserved as-is.
pub(crate) fn register_session_in_projection(
    projection: &mut WorkProjection,
    session: &Session,
    now: DateTime<Utc>,
) -> bool {
    if projection
        .agents
        .iter()
        .any(|agent| agent.session_id == session.id)
    {
        return false;
    }
    projection
        .agents
        .push(workspace_agent_summary_from_session(session, now));
    projection.updated_at = now;
    true
}

fn workspace_agent_summary_from_session(
    session: &Session,
    now: DateTime<Utc>,
) -> WorkspaceAgentSummary {
    WorkspaceAgentSummary {
        session_id: session.id.clone(),
        window_id: None,
        agent_id: session.agent_id.command().to_string(),
        display_name: session.display_name.clone(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: Some(session.worktree_path.clone()),
        branch: Some(session.branch.clone()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Unassigned,
        workspace_id: None,
        updated_at: now,
    }
}

/// UserPromptSubmit hook.
///
/// SPEC-2359 Phase W-11 (US-58 / US-59 / FR-341): this hook no longer
/// derives `title_summary` / `current_focus` from the prompt. Writing the
/// raw prompt into the title produced titles like "あなたの目的は何ですか"
/// instead of the work purpose. The agent now authors the purpose via
/// `gwtd workspace update --title-summary` (provisional → confirmed), and
/// the title sync layer resolves empty values through the display fallback.
///
/// The hook still performs the Phase W-10 (US-57) canonical Project State
/// split repair so that a later `gwtd workspace update --agent-session`
/// from the agent reaches the projection record that owns the live window.
pub(crate) fn handle_user_prompt_submit(_input: &str) -> Result<(), HookError> {
    let Some(session) = current_session_from_env()? else {
        return Ok(());
    };
    handle_user_prompt_submit_for_session(&session)
}

fn handle_user_prompt_submit_for_session(session: &Session) -> Result<(), HookError> {
    let project_state_root = project_state_root_for_session(session);
    crate::agent_project_state::repair_split_agent_state_if_needed(
        &project_state_root,
        &session.worktree_path,
        &session.id,
    )?;
    Ok(())
}

fn current_session_from_env() -> Result<Option<Session>, HookError> {
    let Some(session_id) = std::env::var_os(GWT_SESSION_ID_ENV) else {
        return Ok(None);
    };
    let session_id = session_id.to_string_lossy();
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Ok(None);
    }
    let path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
    if !path.exists() {
        return Ok(None);
    }
    Session::load_and_migrate(&path)
        .map(Some)
        .map_err(HookError::Io)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use gwt_agent::{AgentId, Session};
    use gwt_core::work_projection::{
        load_workspace_projection, save_workspace_projection, WorkProjection,
        WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary, WorkspaceStatusCategory,
    };

    use super::*;

    fn projection_for(repo: &std::path::Path) -> WorkProjection {
        let now = Utc::now();
        WorkProjection {
            id: "ws-test".to_string(),
            project_root: repo.to_path_buf(),
            title: String::new(),
            status_category: WorkspaceStatusCategory::Unknown,
            status_text: String::new(),
            summary: None,
            owner: None,
            next_action: None,
            agents: Vec::new(),
            git_details: None,
            board_refs: Vec::new(),
            updated_at: now,
            created_at: now,
            creator: None,
            lifecycle_stage: Default::default(),
            blocked_reason: None,
            linked_issues: Vec::new(),
            linked_prs: Vec::new(),
            tags: Vec::new(),
            progress_pct: None,
        }
    }

    fn fresh_session(repo: &std::path::Path) -> Session {
        Session::new(repo.to_path_buf(), "work/test-session", AgentId::Codex)
    }

    fn assigned_agent(session_id: &str, repo: &std::path::Path) -> WorkspaceAgentSummary {
        WorkspaceAgentSummary {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: None,
            title_summary: None,
            worktree_path: Some(repo.to_path_buf()),
            branch: Some("work/identity".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn register_session_inserts_unassigned_agent_when_absent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let mut projection = projection_for(&repo);
        let session = fresh_session(&repo);
        let now = Utc::now();

        let inserted = register_session_in_projection(&mut projection, &session, now);

        assert!(inserted, "first registration should insert");
        assert_eq!(projection.agents.len(), 1);
        let agent = &projection.agents[0];
        assert_eq!(agent.session_id, session.id);
        assert_eq!(agent.agent_id, "codex");
        assert!(agent.is_unassigned());
        assert_eq!(agent.worktree_path.as_deref(), Some(repo.as_path()));
        assert_eq!(agent.branch.as_deref(), Some("work/test-session"));
        assert_eq!(agent.title_summary, None);
        assert_eq!(agent.window_id, None);
        assert_eq!(projection.updated_at, now);
    }

    #[test]
    fn register_session_is_idempotent_for_same_session() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let mut projection = projection_for(&repo);
        let session = fresh_session(&repo);

        assert!(register_session_in_projection(
            &mut projection,
            &session,
            Utc::now()
        ));
        let second = register_session_in_projection(&mut projection, &session, Utc::now());

        assert!(!second, "second registration should not re-insert");
        assert_eq!(projection.agents.len(), 1);
    }

    #[test]
    fn register_session_preserves_existing_agent_fields() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let mut projection = projection_for(&repo);
        let session = fresh_session(&repo);

        let mut existing = assigned_agent(&session.id, &repo);
        existing.title_summary = Some("preserved title".to_string());
        existing.current_focus = Some("preserved focus".to_string());
        existing.workspace_id = Some("ws-1".to_string());
        existing.window_id = Some("win-7".to_string());
        projection.agents.push(existing);

        let inserted = register_session_in_projection(&mut projection, &session, Utc::now());

        assert!(!inserted);
        assert_eq!(projection.agents.len(), 1);
        let agent = &projection.agents[0];
        assert_eq!(agent.title_summary.as_deref(), Some("preserved title"));
        assert_eq!(agent.current_focus.as_deref(), Some("preserved focus"));
        assert_eq!(agent.workspace_id.as_deref(), Some("ws-1"));
        assert_eq!(agent.window_id.as_deref(), Some("win-7"));
        assert!(agent.is_assigned());
    }

    /// SPEC-2359 Phase W-11 (US-58 / US-59 / SC-225): UserPromptSubmit must
    /// NOT write `title_summary` / `current_focus` from the prompt. A session
    /// whose agent has empty identity keeps it empty after the hook runs;
    /// the agent (not the hook) authors the purpose.
    #[test]
    fn user_prompt_submit_does_not_write_identity_from_prompt() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("20260602-0056");
        std::fs::create_dir_all(&worktree).expect("worktree");

        let mut session = fresh_session(&worktree);
        session.id = "session-no-derive".to_string();
        session.project_state_root = Some(project_root.clone());

        let mut projection = projection_for(&project_root);
        let mut agent = assigned_agent(&session.id, &worktree);
        agent.title_summary = None;
        agent.current_focus = None;
        projection.agents.push(agent);
        save_workspace_projection(&project_root, &projection).expect("save projection");

        handle_user_prompt_submit_for_session(&session).expect("handle prompt");

        let after = load_workspace_projection(&project_root)
            .expect("load projection")
            .expect("projection present");
        let agent = after
            .agents
            .iter()
            .find(|agent| agent.session_id == session.id)
            .expect("agent present");
        assert_eq!(
            agent.title_summary, None,
            "hook must not derive title_summary from prompt"
        );
        assert_eq!(
            agent.current_focus, None,
            "hook must not derive current_focus from prompt"
        );
    }

    #[test]
    fn coordination_assets_need_refresh_when_codex_skill_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let worktree = temp.path();
        // Pretend Codex has been set up but its coordination skill was lost.
        std::fs::create_dir_all(worktree.join(".codex/skills/gwt-other")).expect("codex other");
        std::fs::write(worktree.join(".codex/hooks.json"), "{}").expect("hooks");
        assert!(coordination_assets_need_refresh(worktree));
    }

    #[test]
    fn coordination_assets_need_refresh_when_codex_hooks_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let worktree = temp.path();
        std::fs::create_dir_all(worktree.join(".codex/skills/gwt-coordination"))
            .expect("coordination dir");
        std::fs::write(
            worktree.join(".codex/skills/gwt-coordination/SKILL.md"),
            "skill",
        )
        .expect("skill");
        // hooks.json absent
        assert!(coordination_assets_need_refresh(worktree));
    }

    #[test]
    fn coordination_assets_need_refresh_when_claude_skill_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let worktree = temp.path();
        std::fs::create_dir_all(worktree.join(".claude/skills/gwt-other")).expect("claude other");
        std::fs::write(worktree.join(".claude/settings.local.json"), "{}").expect("settings");
        assert!(coordination_assets_need_refresh(worktree));
    }

    #[test]
    fn coordination_assets_need_refresh_false_when_everything_present() {
        let temp = tempfile::tempdir().expect("tempdir");
        let worktree = temp.path();
        std::fs::create_dir_all(worktree.join(".codex/skills/gwt-coordination"))
            .expect("codex coord");
        std::fs::write(
            worktree.join(".codex/skills/gwt-coordination/SKILL.md"),
            "x",
        )
        .expect("codex skill");
        std::fs::write(worktree.join(".codex/hooks.json"), "{}").expect("hooks");
        std::fs::create_dir_all(worktree.join(".claude/skills/gwt-coordination"))
            .expect("claude coord");
        std::fs::write(
            worktree.join(".claude/skills/gwt-coordination/SKILL.md"),
            "x",
        )
        .expect("claude skill");
        std::fs::write(worktree.join(".claude/settings.local.json"), "{}").expect("settings");
        assert!(!coordination_assets_need_refresh(worktree));
    }

    #[test]
    fn coordination_assets_need_refresh_false_for_unmanaged_worktree() {
        let temp = tempfile::tempdir().expect("tempdir");
        // No .codex, no .claude at all — worktree is not managed.
        assert!(!coordination_assets_need_refresh(temp.path()));
    }
}
