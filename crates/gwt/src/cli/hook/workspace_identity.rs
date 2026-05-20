use std::{fs, path::Path};

use chrono::{DateTime, Utc};
use gwt_agent::{Session, GWT_SESSION_ID_ENV};
use gwt_core::workspace_projection::{
    load_or_default_workspace_projection, load_workspace_projection, save_workspace_projection,
    update_workspace_projection_with_journal, WorkspaceAgentAffiliationStatus,
    WorkspaceAgentSummary, WorkspaceProjection, WorkspaceProjectionUpdate, WorkspaceStatusCategory,
};

use super::{HookError, HookOutput, IntentBoundaryEvent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspacePromptIdentity {
    pub title_summary: String,
    pub current_focus: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceIdentityHookResult {
    pub updated: bool,
    pub identity: Option<WorkspacePromptIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MissingIdentity {
    title_summary: bool,
    current_focus: bool,
}

impl MissingIdentity {
    fn complete(self) -> bool {
        !self.title_summary && !self.current_focus
    }
}

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
pub(crate) fn handle_session_start() -> Result<(), HookError> {
    let Some(session) = current_session_from_env()? else {
        return Ok(());
    };
    ensure_coordination_assets_for_session(&session);
    let mut projection = load_or_default_workspace_projection(&session.worktree_path)?;
    projection.project_root = session.worktree_path.clone();
    let now = Utc::now();
    let registered = register_session_in_projection(&mut projection, &session, now);
    let derived = derive_title_summary_from_owner(&mut projection, &session);
    if derived {
        projection.updated_at = now;
    }
    if registered || derived {
        save_workspace_projection(&session.worktree_path, &projection)?;
        crate::cli::workspace::publish_workspace_change(&session.worktree_path);
    }
    Ok(())
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

/// SPEC-2359 Phase U-9 (FR-173 / FR-174): auto-derive `title_summary`
/// from the workspace's first linked Issue/SPEC at SessionStart when the
/// agent has no explicit title yet. Existing explicit values are never
/// overwritten (US-43 non-destructive contract). Missing cache entries
/// are a silent no-op so owner-less sessions stay empty and fall back to
/// the existing empty-trigger reminder path.
fn derive_title_summary_from_owner(
    projection: &mut WorkspaceProjection,
    session: &Session,
) -> bool {
    let issue_cache_root =
        crate::issue_cache::issue_cache_root_for_repo_path_or_detached(&session.worktree_path);
    derive_title_summary_from_owner_with_cache(projection, &session.id, &issue_cache_root)
}

fn derive_title_summary_from_owner_with_cache(
    projection: &mut WorkspaceProjection,
    session_id: &str,
    issue_cache_root: &Path,
) -> bool {
    let Some(agent_idx) = projection
        .agents
        .iter()
        .position(|agent| agent.session_id == session_id)
    else {
        return false;
    };
    if projection.agents[agent_idx]
        .title_summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return false;
    }
    let Some(issue) = projection.linked_issues.first() else {
        return false;
    };
    let issue_number = issue.number;
    let title = match load_issue_title_from_cache(issue_cache_root, issue_number) {
        Some(value) => value,
        None => return false,
    };
    projection.agents[agent_idx].title_summary = Some(title);
    true
}

fn load_issue_title_from_cache(cache_root: &Path, issue_number: u64) -> Option<String> {
    let path = cache_root.join(issue_number.to_string()).join("meta.json");
    let bytes = fs::read(&path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let title = value.get("title")?.as_str()?.trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

/// Insert a stub `WorkspaceAgentSummary` for `session` if no agent with
/// the same `session_id` is present. Returns `true` when a new record
/// was inserted. Existing records are preserved as-is.
pub(crate) fn register_session_in_projection(
    projection: &mut WorkspaceProjection,
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

pub(crate) fn handle_user_prompt_submit(
    input: &str,
) -> Result<WorkspaceIdentityHookResult, HookError> {
    let Some(session) = current_session_from_env()? else {
        return Ok(WorkspaceIdentityHookResult {
            updated: false,
            identity: None,
        });
    };
    handle_user_prompt_submit_for_session(input, &session)
}

fn handle_user_prompt_submit_for_session(
    input: &str,
    session: &Session,
) -> Result<WorkspaceIdentityHookResult, HookError> {
    let Some(missing) = missing_identity_for_session(session)? else {
        return Ok(WorkspaceIdentityHookResult {
            updated: false,
            identity: None,
        });
    };
    if missing.complete() {
        return Ok(WorkspaceIdentityHookResult {
            updated: false,
            identity: None,
        });
    }

    let Some(prompt) = prompt_from_hook_input(input) else {
        return Ok(WorkspaceIdentityHookResult {
            updated: false,
            identity: None,
        });
    };
    let Some(identity) = derive_identity_from_prompt(&prompt) else {
        return Ok(WorkspaceIdentityHookResult {
            updated: false,
            identity: None,
        });
    };

    let Some(update) = workspace_projection_update_for_identity(&session.id, missing, &identity)
    else {
        return Ok(WorkspaceIdentityHookResult {
            updated: false,
            identity: Some(identity),
        });
    };

    update_workspace_projection_with_journal(&session.worktree_path, update)?;
    crate::cli::workspace::publish_workspace_change(&session.worktree_path);

    Ok(WorkspaceIdentityHookResult {
        updated: true,
        identity: Some(identity),
    })
}

pub(crate) fn append_identity_context(
    output: HookOutput,
    result: WorkspaceIdentityHookResult,
) -> HookOutput {
    if !result.updated {
        return output;
    };
    let Some(identity) = result.identity else {
        return output;
    };
    let text = format!(
        "# Workspace Identity Updated\n\nUserPromptSubmit has set this Agent window / Workspace identity from the prompt.\n\n- title-summary: `{}`\n- current-focus: {}\n\nIf this identity is inaccurate, correct it before continuing with `gwtd workspace update --agent-session \"$GWT_SESSION_ID\" --current-focus '<focus>' --title-summary '<short work name>'`.",
        identity.title_summary, identity.current_focus
    );
    append_user_prompt_context(output, text)
}

fn append_user_prompt_context(output: HookOutput, text: String) -> HookOutput {
    match output {
        HookOutput::HookSpecificAdditionalContext {
            event: IntentBoundaryEvent::UserPromptSubmit,
            text: existing,
        } => HookOutput::hook_specific_additional_context(
            IntentBoundaryEvent::UserPromptSubmit,
            format!("{text}\n\n{existing}"),
        ),
        HookOutput::Silent => HookOutput::hook_specific_additional_context(
            IntentBoundaryEvent::UserPromptSubmit,
            text,
        ),
        other => other,
    }
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

fn missing_identity_for_session(session: &Session) -> Result<Option<MissingIdentity>, HookError> {
    let Some(projection) = load_workspace_projection(&session.worktree_path)? else {
        return Ok(None);
    };
    let Some(agent) = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == session.id)
    else {
        return Ok(None);
    };
    if agent.is_unassigned() {
        return Ok(None);
    }
    Ok(Some(missing_identity_for_agent(agent)))
}

fn missing_identity_for_agent(
    agent: &gwt_core::workspace_projection::WorkspaceAgentSummary,
) -> MissingIdentity {
    MissingIdentity {
        title_summary: agent
            .title_summary
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none(),
        current_focus: agent
            .current_focus
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none(),
    }
}

fn workspace_projection_update_for_identity(
    session_id: &str,
    missing: MissingIdentity,
    identity: &WorkspacePromptIdentity,
) -> Option<WorkspaceProjectionUpdate> {
    let title_summary = missing
        .title_summary
        .then(|| identity.title_summary.clone());
    let current_focus = missing
        .current_focus
        .then(|| identity.current_focus.clone());
    if title_summary.is_none() && current_focus.is_none() {
        return None;
    }
    Some(WorkspaceProjectionUpdate {
        title: None,
        status_category: None,
        status_text: None,
        owner: None,
        next_action: None,
        summary: None,
        agent_session_id: Some(session_id.to_string()),
        agent_current_focus: current_focus,
        agent_title_summary: title_summary,
    })
}

pub(crate) fn derive_identity_from_prompt(prompt: &str) -> Option<WorkspacePromptIdentity> {
    let focus = sanitize_prompt_focus(prompt)?;
    let title_summary = derive_title_summary(&focus)?;
    if super::super::validate_title_summary_work_name("--title-summary", &title_summary).is_err() {
        return None;
    }
    Some(WorkspacePromptIdentity {
        title_summary,
        current_focus: truncate_chars(&focus, 160),
    })
}

fn derive_title_summary(focus: &str) -> Option<String> {
    let lower = focus.to_ascii_lowercase();
    if lower.contains("workspace")
        && (focus.contains("ウィンドウ")
            || lower.contains("window")
            || focus.contains("何をしている")
            || focus.contains("把握"))
        && (lower.contains("ux") || focus.contains("識別") || focus.contains("把握"))
    {
        return Some("Workspace識別UX不具合".to_string());
    }
    if (focus.contains("エージェントウィンドウ") || lower.contains("agent window"))
        && (focus.contains("更新") || focus.contains("タイトル") || lower.contains("title"))
        && (focus.contains("不具合")
            || focus.contains("されません")
            || focus.contains("直っていません")
            || lower.contains("bug"))
    {
        return Some("エージェントウィンドウ更新不具合".to_string());
    }

    let first = focus
        .split(['\n', '。', '.', '？', '?', '！', '!'])
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let first = trim_request_suffixes(first);
    let title = truncate_chars(&first, 30);
    (!title.trim().is_empty()).then_some(title)
}

fn sanitize_prompt_focus(prompt: &str) -> Option<String> {
    let without_blocks = strip_fenced_blocks(prompt);
    let mut lines = Vec::new();
    for raw in without_blocks.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('<') || line.starts_with("# ") {
            continue;
        }
        let line = line
            .strip_prefix("$gwt-discussion")
            .or_else(|| line.strip_prefix("$gwt-build-spec"))
            .or_else(|| line.strip_prefix("$gwt-fix-issue"))
            .unwrap_or(line)
            .trim();
        if !line.is_empty() {
            lines.push(line);
        }
    }
    let joined = lines.join(" ");
    let normalized = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

fn strip_fenced_blocks(value: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in value.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn trim_request_suffixes(value: &str) -> String {
    let suffixes = [
        "ちゃんと考えて対応してください",
        "対応してください",
        "修正してください",
        "実装してください",
        "調査してください",
        "お願いします",
        "してください",
    ];
    let mut out = value.trim().to_string();
    for suffix in suffixes {
        if out.ends_with(suffix) {
            out.truncate(out.len() - suffix.len());
            break;
        }
    }
    out.trim_matches(|ch: char| ch.is_whitespace() || matches!(ch, '。' | '.' | '、' | ','))
        .to_string()
}

fn truncate_chars(value: &str, max: usize) -> String {
    let value = value.trim();
    if value.chars().count() <= max {
        return value.to_string();
    }
    value.chars().take(max).collect::<String>()
}

fn prompt_from_hook_input(input: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(input).ok()?;
    string_at_any(
        &value,
        &[
            &["prompt"],
            &["user_prompt"],
            &["userPrompt"],
            &["message"],
            &["message", "content"],
            &["message", "text"],
            &["input"],
            &["input", "content"],
            &["input", "text"],
        ],
    )
    .or_else(|| latest_message_content(&value))
    .or_else(|| {
        string_at_any(&value, &[&["transcript_path"], &["transcriptPath"]])
            .and_then(|path| prompt_from_transcript_path(Path::new(&path)))
    })
}

fn latest_message_content(value: &serde_json::Value) -> Option<String> {
    let messages = value.get("messages")?.as_array()?;
    messages.iter().rev().find_map(|message| {
        let role = message.get("role").and_then(serde_json::Value::as_str);
        if role.is_some_and(|role| role != "user") {
            return None;
        }
        value_to_text(message.get("content")?)
    })
}

fn prompt_from_transcript_path(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    text.lines().rev().find_map(|line| {
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        if let Some(prompt) = string_at_any(&value, &[&["lastPrompt"], &["last_prompt"]]) {
            return Some(prompt);
        }
        if !is_transcript_user_record(&value) {
            return None;
        }
        value
            .get("message")
            .and_then(|message| value_to_text(message.get("content")?))
            .or_else(|| string_at_any(&value, &[&["text"]]))
    })
}

fn is_transcript_user_record(value: &serde_json::Value) -> bool {
    string_at_any(
        value,
        &[
            &["type"],
            &["role"],
            &["message", "role"],
            &["event", "role"],
        ],
    )
    .is_some_and(|role| role.eq_ignore_ascii_case("user"))
}

fn string_at_any(value: &serde_json::Value, paths: &[&[&str]]) -> Option<String> {
    paths.iter().find_map(|path| {
        let mut current = value;
        for key in *path {
            current = current.get(*key)?;
        }
        current
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn value_to_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.trim().to_string()),
        serde_json::Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(|item| {
                    item.as_str()
                        .map(str::to_string)
                        .or_else(|| string_at_any(item, &[&["text"], &["content"]]))
                })
                .map(|part| part.trim().to_string())
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        serde_json::Value::Object(_) => string_at_any(value, &[&["text"], &["content"]]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use gwt_agent::{AgentId, Session};
    use gwt_core::workspace_projection::{
        WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary, WorkspaceProjection,
        WorkspaceStatusCategory,
    };

    use super::*;

    fn projection_for(repo: &std::path::Path) -> WorkspaceProjection {
        let now = Utc::now();
        WorkspaceProjection {
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
    fn derive_identity_from_prompt_identifies_workspace_window_ux() {
        let prompt = "$gwt-discussion 単なるタイトル更新と思っているかもしれませんが、現在のWorkspace運用の場合、どのウィンドウで何をしているのか？を把握できないとUX的には最悪です。";

        let identity = derive_identity_from_prompt(prompt).expect("identity");

        assert_eq!(identity.title_summary, "Workspace識別UX不具合");
        assert!(
            identity
                .current_focus
                .contains("どのウィンドウで何をしているのか"),
            "{}",
            identity.current_focus
        );
    }

    #[test]
    fn user_prompt_submit_builds_update_for_missing_workspace_identity() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let agent = assigned_agent("session-1", &repo);

        let payload = serde_json::json!({
            "session_id": "codex-provider-session",
            "prompt": "$gwt-discussion エージェントウィンドウの更新がいまだにされません。今回の場合であれば「エージェントウィンドウの更新不具合」などが表示されるべきです。"
        });

        let prompt = prompt_from_hook_input(&payload.to_string()).expect("prompt");
        let identity = derive_identity_from_prompt(&prompt).expect("identity");
        let update = workspace_projection_update_for_identity(
            "session-1",
            missing_identity_for_agent(&agent),
            &identity,
        )
        .expect("projection update");

        assert_eq!(update.agent_session_id.as_deref(), Some("session-1"));
        assert_eq!(
            update.agent_title_summary.as_deref(),
            Some("エージェントウィンドウ更新不具合")
        );
        assert!(
            update
                .agent_current_focus
                .as_deref()
                .is_some_and(|focus| focus.contains("エージェントウィンドウの更新")),
            "{:?}",
            update.agent_current_focus
        );
    }

    #[test]
    fn user_prompt_submit_does_not_overwrite_existing_identity() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");

        let mut agent = assigned_agent("session-1", &repo);
        agent.title_summary = Some("明示タイトル".to_string());
        agent.current_focus = Some("明示 focus".to_string());

        let missing = missing_identity_for_agent(&agent);

        assert!(missing.complete(), "{missing:?}");
        let identity = WorkspacePromptIdentity {
            title_summary: "エージェントウィンドウ更新不具合".to_string(),
            current_focus: "エージェントウィンドウの更新がされません".to_string(),
        };
        assert!(
            workspace_projection_update_for_identity("session-1", missing, &identity).is_none()
        );
    }

    #[test]
    fn user_prompt_submit_uses_transcript_path_when_prompt_field_is_absent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let transcript = temp.path().join("transcript.jsonl");
        std::fs::write(
            &transcript,
            r#"{"type":"last-prompt","lastPrompt":"単なるタイトル更新と思っているかもしれませんが、現在のWorkspace運用の場合、どのウィンドウで何をしているのか？を把握できないとUX的には最悪です。"}"#,
        )
        .expect("transcript");

        let payload = serde_json::json!({
            "session_id": "codex-provider-session",
            "transcript_path": transcript
        });

        let prompt = prompt_from_hook_input(&payload.to_string()).expect("prompt");
        let identity = derive_identity_from_prompt(&prompt).expect("identity");

        assert_eq!(identity.title_summary, "Workspace識別UX不具合");
    }

    fn write_issue_cache_meta(cache_root: &std::path::Path, number: u64, title: &str) {
        let dir = cache_root.join(number.to_string());
        std::fs::create_dir_all(&dir).expect("issue dir");
        let meta = serde_json::json!({
            "number": number,
            "title": title,
            "labels": [],
            "state": "open",
            "updated_at": "2026-05-20T00:00:00Z",
            "comment_ids": [],
        });
        std::fs::write(dir.join("meta.json"), meta.to_string()).expect("meta");
    }

    fn projection_with_linked_issue(
        repo: &std::path::Path,
        session_id: &str,
        issue_number: u64,
    ) -> WorkspaceProjection {
        let mut projection = projection_for(repo);
        projection
            .linked_issues
            .push(gwt_core::workspace_projection::WorkspaceIssueLink {
                number: issue_number,
                title: None,
                url: None,
            });
        let mut agent = assigned_agent(session_id, repo);
        agent.title_summary = None;
        agent.current_focus = None;
        projection.agents.push(agent);
        projection
    }

    #[test]
    fn session_start_derives_title_summary_from_owner_issue_cache() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let cache_root = temp.path().join("cache");
        write_issue_cache_meta(&cache_root, 2359, "Workspace / Start Work");

        let session_id = "session-derive";
        let mut projection = projection_with_linked_issue(&repo, session_id, 2359);

        let updated =
            derive_title_summary_from_owner_with_cache(&mut projection, session_id, &cache_root);

        assert!(
            updated,
            "auto-derive must apply when title is empty and cache exists"
        );
        assert_eq!(
            projection.agents[0].title_summary.as_deref(),
            Some("Workspace / Start Work"),
            "title must be sourced from issue cache meta.json"
        );
    }

    #[test]
    fn session_start_does_not_overwrite_existing_title_summary() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let cache_root = temp.path().join("cache");
        write_issue_cache_meta(&cache_root, 2359, "Workspace / Start Work");

        let session_id = "session-explicit";
        let mut projection = projection_with_linked_issue(&repo, session_id, 2359);
        projection.agents[0].title_summary = Some("Custom title".to_string());

        let updated =
            derive_title_summary_from_owner_with_cache(&mut projection, session_id, &cache_root);

        assert!(!updated, "explicit title must be preserved");
        assert_eq!(
            projection.agents[0].title_summary.as_deref(),
            Some("Custom title")
        );
    }

    #[test]
    fn session_start_owner_absent_keeps_title_summary_empty() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let cache_root = temp.path().join("cache");

        let session_id = "session-no-owner";
        let mut projection = projection_for(&repo);
        let mut agent = assigned_agent(session_id, &repo);
        agent.title_summary = None;
        projection.agents.push(agent);

        let updated =
            derive_title_summary_from_owner_with_cache(&mut projection, session_id, &cache_root);

        assert!(!updated, "no linked_issues -> no-op");
        assert_eq!(projection.agents[0].title_summary, None);
    }

    #[test]
    fn session_start_owner_cache_missing_is_silent_noop() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let cache_root = temp.path().join("cache");
        // cache_root exists conceptually but contains no entry for issue 9999

        let session_id = "session-missing-cache";
        let mut projection = projection_with_linked_issue(&repo, session_id, 9999);

        let updated =
            derive_title_summary_from_owner_with_cache(&mut projection, session_id, &cache_root);

        assert!(!updated, "missing cache file must be a silent no-op");
        assert_eq!(projection.agents[0].title_summary, None);
    }

    #[test]
    fn session_start_owner_cache_with_empty_title_is_noop() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let cache_root = temp.path().join("cache");
        write_issue_cache_meta(&cache_root, 7, "   ");

        let session_id = "session-empty-title";
        let mut projection = projection_with_linked_issue(&repo, session_id, 7);

        let updated =
            derive_title_summary_from_owner_with_cache(&mut projection, session_id, &cache_root);

        assert!(!updated, "empty/whitespace title must not be applied");
        assert_eq!(projection.agents[0].title_summary, None);
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

    #[test]
    fn transcript_path_ignores_non_user_text_records() {
        let temp = tempfile::tempdir().expect("tempdir");
        let transcript = temp.path().join("transcript.jsonl");
        std::fs::write(
            &transcript,
            concat!(
                r#"{"type":"user","text":"エージェントウィンドウの更新がいまだにされません。"}"#,
                "\n",
                r#"{"type":"assistant","text":"実装方針を説明します。"}"#,
                "\n"
            ),
        )
        .expect("transcript");

        let prompt = prompt_from_transcript_path(&transcript).expect("prompt");

        assert!(prompt.contains("エージェントウィンドウの更新"), "{prompt}");
        assert!(!prompt.contains("実装方針"), "{prompt}");
    }
}
