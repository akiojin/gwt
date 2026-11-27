//! Session launch preparation and persistence without a GUI dependency.

pub mod recovery;

use std::{
    fs, io,
    path::{Path, PathBuf},
};

const DURABLE_LAUNCH_RECOVERY_DIR: &str = "execution-launch-recovery";

/// Directory containing pending launch-recovery receipts.
pub fn durable_launch_recovery_dir(sessions_dir: &Path) -> PathBuf {
    sessions_dir.join(DURABLE_LAUNCH_RECOVERY_DIR)
}

/// Resolve an exact Session receipt after validating its path component.
pub fn durable_launch_recovery_path(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<PathBuf, String> {
    gwt_agent::validate_session_id_path_component(session_id)?;
    Ok(durable_launch_recovery_dir(sessions_dir).join(format!("{session_id}.json")))
}

/// Whether an exact Session has a pending launch-recovery receipt.
pub fn durable_launch_recovery_exists(sessions_dir: &Path, session_id: &str) -> io::Result<bool> {
    let path = durable_launch_recovery_path(sessions_dir, session_id)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Why a restored Session cannot be handed to the agent CLI as `--resume`
/// (SPEC-3966 AC-4).
///
/// `None` means the Session does carry a usable handle. Every other answer is
/// the reason the restore falls back to a brand-new conversation, so the
/// fallback is never silent.
pub fn resume_handle_unavailable_reason(session: &gwt_agent::Session) -> Option<&'static str> {
    if session.exact_resume_session_id().is_some() {
        return None;
    }
    match session.agent_session_id.as_deref().map(str::trim) {
        None => Some(
            "no agent_session_id was ever persisted for this Session \
             (the SessionStart managed hook never reached persist_agent_session_id)",
        ),
        Some("") => Some("the persisted agent_session_id is empty"),
        Some(_) => Some(
            "the persisted agent_session_id is the Codex placeholder, not a conversation handle",
        ),
    }
}

/// Carry a restored launch's conversation identity onto the successor Session
/// record (SPEC-3966 AC-5).
///
/// Without this the successor is written with no `agent_session_id`, so the
/// *next* restore has nothing to resume from either and every restart leaves
/// one more handle-less Session behind.
pub fn launch_config_from_persisted_session(
    session: &gwt_agent::Session,
) -> gwt_agent::LaunchConfig {
    let agent_id = session.agent_id.clone();
    let pm_scratch = crate::pm_registry::pm_scratch_dir_for_pm_worktree(&session.worktree_path);
    let mut builder = gwt_agent::AgentLaunchBuilder::new(agent_id);
    if let gwt_agent::AgentId::Custom(id) = &session.agent_id {
        if let Some(agent) =
            gwt_agent::load_custom_agents_from_path(&gwt_core::paths::gwt_config_path())
                .unwrap_or_default()
                .into_iter()
                .find(|agent| agent.id == *id)
        {
            builder = builder.custom_agent(agent);
        }
    }
    builder = builder.working_dir(session.worktree_path.clone());
    if !session.branch.is_empty() {
        builder = builder.branch(session.branch.clone());
    }
    if let Some(model) = session.model.clone() {
        builder = builder.model(model);
    }
    if let Some(level) = session.reasoning_level.clone() {
        builder = builder.reasoning_level(level);
    }
    // FR-028 belongs to the PM role, including older Sessions whose saved
    // skip_permissions field defaults to false.
    if session.skip_permissions || pm_scratch.is_some() {
        builder = builder.skip_permissions(true);
    }
    builder = builder.codex_speed(session.codex_speed());
    if session.fast_mode_enabled() {
        builder = builder.fast_mode(true);
    }
    builder = builder.runtime_target(session.runtime_target);
    if let Some(service) = session.docker_service.clone() {
        builder = builder.docker_service(service);
    }
    builder = builder.docker_lifecycle_intent(session.docker_lifecycle_intent);
    if let Some(shell) = session.windows_shell {
        builder = builder.windows_shell(shell);
    }
    if let Some(linked) = session.linked_issue_number {
        builder = builder.linked_issue_number(linked);
    }
    // Issue #4217 FR-002: a rebuilt config keeps the route the launcher
    // originally stamped, so a restored or resumed monitor launch does not
    // quietly become a human-driven one.
    builder = builder.launch_route(session.launch_route);

    // Issue #3965: the PM bootstrap prompt is a property of the PM role, not of
    // any structured `Session` field, so rebuilding a config here used to drop
    // it and restore the PM as a plain agent under no PM contract. Re-apply it
    // from the same PM detection that already restores the scratch dir below —
    // unconditionally, because a persisted `launch_args` that still records the
    // prompt is not consulted by the builder and would otherwise lose it too.
    if pm_scratch.is_some()
        && crate::pm_registry::pm_autonomous_bootstrap_allowed(&session.worktree_path)
    {
        builder = builder.extra_arg(crate::pm_registry::PM_BOOTSTRAP_PROMPT);
    }

    if let Some(resume_id) = session.exact_resume_session_id() {
        builder = builder
            .session_mode(gwt_agent::SessionMode::Resume)
            .resume_session_id(resume_id.to_string())
            .predecessor_session_id(session.id.clone());
    } else {
        // SPEC-3966 AC-4: restoring without a resume handle silently started a
        // brand-new conversation, which is how the resident PM lost its whole
        // decision history on every gwt restart without anyone noticing. Say so
        // instead, with the reason, so the next reader does not have to
        // reverse-engineer it from an empty Session TOML.
        tracing::warn!(
            session_id = %session.id,
            worktree = %session.worktree_path.display(),
            agent = ?session.agent_id,
            reason = resume_handle_unavailable_reason(session).unwrap_or("unknown"),
            "restored Session has no resume handle; launching a new conversation instead of --resume"
        );
        builder = builder.session_mode(gwt_agent::SessionMode::Normal);
    }

    let mut config = builder.build();
    // Retained records keep their original bytes. A proven migration alias
    // also identifies Sessions that predate the member-ID field.
    let member_id = crate::agent_names::registered_session_member_id(session)
        .map_err(|error| {
            tracing::warn!(session_id = %session.id, %error,
                "restored Agent identity could not be resolved")
        })
        .ok()
        .flatten()
        // Keep an unresolved recorded ID so reservation fails closed.
        .or_else(|| session.agent_member_id.clone());
    if let Some(member_id) = member_id {
        config
            .env_vars
            .insert("GWT_AGENT_MEMBER_ID".into(), member_id);
    }
    if !session.display_name.is_empty() {
        config.display_name = session.display_name.clone();
    }
    if let Some(scratch) = pm_scratch {
        config.env_vars.insert(
            crate::pm_registry::GWT_PM_SCRATCH_DIR_ENV.to_string(),
            scratch.to_string_lossy().into_owned(),
        );
        config.suppress_execution_control = true;
    }
    if session.review_dispatch {
        // Restore the observation role without rematerializing its existing
        // detached checkout or arming producing execution control.
        config
            .env_vars
            .insert("GWT_REVIEW_DISPATCH".to_string(), "1".to_string());
        config.suppress_execution_control = true;
        config.is_ephemeral = true;
        // The persisted branch can be the detached checkout's display fallback.
        // Matching it would join a different review that shares that label.
        config.branch = None;
    }
    config
}

#[cfg(test)]
mod persisted_review_launch_tests {
    use super::*;

    #[test]
    fn restored_custom_session_retains_configured_command_args_env_and_resume() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let config_path = gwt_core::paths::gwt_config_path();
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(
            config_path,
            r#"[tools.customCodingAgents.fixture]
id = "fixture"
displayName = "Fixture"
agentType = "command"
command = "fixture-cli"
defaultArgs = ["--flag"]
[tools.customCodingAgents.fixture.modeArgs]
resume = ["--resume"]
[tools.customCodingAgents.fixture.env]
FIXTURE_ENV = "configured"
"#,
        )
        .unwrap();
        let mut session = gwt_agent::Session::new(
            home.path(),
            "main",
            gwt_agent::AgentId::Custom("fixture".into()),
        );
        session.agent_session_id = Some("saved-conversation".into());

        let config = launch_config_from_persisted_session(&session);
        assert_eq!(config.command, "fixture-cli");
        assert_eq!(config.args, ["--flag", "--resume"]);
        assert_eq!(
            config.env_vars.get("FIXTURE_ENV").map(String::as_str),
            Some("configured")
        );
        assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
        assert_eq!(
            config.resume_session_id.as_deref(),
            Some("saved-conversation")
        );
    }

    #[test]
    fn restored_review_retains_observation_contract_and_isolated_worktree() {
        let mut session = gwt_agent::Session::new(
            "/tmp/isolated-review-checkout",
            "work",
            gwt_agent::AgentId::Codex,
        );
        session.review_dispatch = true;
        session.linked_issue_number = Some(5478);
        session.agent_session_id = Some("review-provider-conversation".to_string());
        let config = launch_config_from_persisted_session(&session);

        assert_eq!(
            config
                .env_vars
                .get("GWT_REVIEW_DISPATCH")
                .map(String::as_str),
            Some("1"),
            "a restored reviewer must retain its read-only hook identity"
        );
        assert!(config.suppress_execution_control);
        assert!(config.is_ephemeral);
        assert!(
            config.branch.is_none(),
            "a detached review must not join another review via the shared fallback branch"
        );
        assert_eq!(config.working_dir.as_ref(), Some(&session.worktree_path));
        assert!(config.ephemeral_base_ref.is_none());
        assert_eq!(config.linked_issue_number, Some(5478));
        assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
    }
}

#[derive(Debug, Clone, Copy)]
pub enum NamedIssueActivity {
    PrCreated { number: u64 },
    Completed,
}

const NAMED_ACTIVITY_PREFIX: &str = "<!-- gwt-agent-work ";

#[derive(serde::Serialize, serde::Deserialize)]
struct NamedIssueRecord {
    name: String,
    provider: String,
    session_id: String,
    phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pr_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    role: Option<String>,
}

/// Who is writing a GitHub Issue or PR body. Issue #7707.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubWriter {
    pub name: String,
    pub detail: String,
    pub role: GithubWriterRole,
    pub session_id: String,
    pub phase: String,
}

/// Visible signature role. Agents show a provider; a PM shows the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubWriterRole {
    Agent,
    Pm,
    Issues,
}

impl GithubWriter {
    fn visible_line(&self) -> String {
        match self.role {
            GithubWriterRole::Agent => format!("— {}（{}）", self.name, self.detail),
            GithubWriterRole::Pm => format!("— PM {}（{}）", self.name, self.detail),
            GithubWriterRole::Issues => "— gwt Issues".to_string(),
        }
    }

    fn role_token(&self) -> &'static str {
        match self.role {
            GithubWriterRole::Agent => "agent",
            GithubWriterRole::Pm => "pm",
            GithubWriterRole::Issues => "issues",
        }
    }
}

/// Automatic settlement comments are written by gwt Issues, not by a pane.
pub fn issues_github_writer() -> GithubWriter {
    GithubWriter {
        name: "gwt Issues".to_string(),
        detail: "gwt".to_string(),
        role: GithubWriterRole::Issues,
        session_id: String::new(),
        phase: "settlement".to_string(),
    }
}

/// Build a writer from a named Session. `pm_host` selects the PM signature.
pub fn github_writer_for_session(
    session: &gwt_agent::Session,
    pm_host: Option<&str>,
) -> Option<GithubWriter> {
    let name = session.agent_name.as_deref()?.trim();
    if name.is_empty() {
        return None;
    }
    if let Some(host) = pm_host.map(str::trim).filter(|host| !host.is_empty()) {
        return Some(GithubWriter {
            name: name.to_string(),
            detail: host.to_string(),
            role: GithubWriterRole::Pm,
            session_id: session.id.clone(),
            phase: "write".to_string(),
        });
    }
    let provider = session.display_name.trim();
    if provider.is_empty() {
        return None;
    }
    Some(GithubWriter {
        name: name.to_string(),
        detail: provider.to_string(),
        role: GithubWriterRole::Agent,
        session_id: session.id.clone(),
        phase: "write".to_string(),
    })
}

/// Resolve the ambient Session's writer. A missing name leaves the body unchanged.
pub fn current_github_writer(repo: &Path) -> Option<GithubWriter> {
    let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).ok()?;
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return None;
    }
    let session = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml")),
    )
    .ok()?;
    let pm_host = pm_host_for_repo(repo, &session.id);
    github_writer_for_session(&session, pm_host.as_deref())
}

fn pm_host_for_repo(repo: &Path, session_id: &str) -> Option<String> {
    if session_id.is_empty() {
        return None;
    }
    let registered = crate::pm_registry::session_is_registered_pm(
        &crate::pm_registry::pm_prefs_path_for_repo_path(repo),
        session_id,
    ) || crate::pm_registry::pm_repository_key(repo).is_some_and(|key| {
        crate::pm_registry::pm_registrations_for_repository(&key)
            .iter()
            .any(|record| record.registration.session_id == session_id)
    });
    if !registered {
        return None;
    }
    let host = crate::process::current_hostname();
    (!host.is_empty()).then_some(host)
}

/// Append one writer signature. The same writer is not added twice.
pub fn stamp_github_body(body: &str, writer: Option<&GithubWriter>) -> String {
    let Some(writer) = writer else {
        return body.to_string();
    };
    if writer.name.trim().is_empty() || writer.detail.trim().is_empty() {
        return body.to_string();
    }
    let visible = writer.visible_line();
    let already_signed = body.lines().any(|line| line.trim() == visible)
        || named_records_in(body)
            .iter()
            .any(|record| record.name == writer.name && record.provider == writer.detail);
    if already_signed {
        return body.to_string();
    }
    let record = NamedIssueRecord {
        name: writer.name.clone(),
        provider: writer.detail.clone(),
        session_id: writer.session_id.clone(),
        phase: writer.phase.clone(),
        pr_number: None,
        role: Some(writer.role_token().to_string()),
    };
    let metadata = serde_json::to_string(&record).expect("github writer metadata serializes");
    let trimmed = body.trim_end();
    if trimmed.is_empty() {
        format!("{NAMED_ACTIVITY_PREFIX}{metadata} -->\n{visible}\n")
    } else {
        format!("{trimmed}\n\n{NAMED_ACTIVITY_PREFIX}{metadata} -->\n{visible}\n")
    }
}

fn named_record(
    session: &gwt_agent::Session,
    phase: &str,
    pr_number: Option<u64>,
) -> Option<NamedIssueRecord> {
    if session.review_dispatch {
        return None;
    }
    Some(NamedIssueRecord {
        name: session.agent_name.as_deref()?.trim().to_string(),
        provider: session.display_name.clone(),
        session_id: session.id.clone(),
        phase: phase.to_string(),
        pr_number,
        role: None,
    })
    .filter(|record| !record.name.is_empty())
}

fn named_records_in(body: &str) -> Vec<NamedIssueRecord> {
    body.lines()
        .filter_map(|line| {
            let json = line
                .trim()
                .strip_prefix(NAMED_ACTIVITY_PREFIX)?
                .strip_suffix(" -->")?;
            serde_json::from_str(json).ok()
        })
        .collect()
}

fn named_record_from_body(body: &str) -> Option<NamedIssueRecord> {
    named_records_in(body).into_iter().next_back()
}

pub(crate) fn github_body_is_stamped_by_pm(body: &str, session_id: &str) -> bool {
    named_record_from_body(body).is_some_and(|record| {
        record.session_id == session_id && record.role.as_deref() == Some("pm")
    })
}

fn writer_from_record(record: &NamedIssueRecord) -> GithubWriter {
    let role = match record.role.as_deref() {
        Some("pm") => GithubWriterRole::Pm,
        Some("issues") => GithubWriterRole::Issues,
        _ => GithubWriterRole::Agent,
    };
    GithubWriter {
        name: record.name.clone(),
        detail: record.provider.clone(),
        role,
        session_id: record.session_id.clone(),
        phase: record.phase.clone(),
    }
}

fn record_named_comment<C: gwt_github::IssueClient + ?Sized>(
    client: &C,
    session: &gwt_agent::Session,
    record: NamedIssueRecord,
    narrative: &str,
) -> Result<bool, gwt_github::ApiError> {
    use gwt_github::{FetchResult, IssueNumber};
    let Some(number) = session.linked_issue_number else {
        return Ok(false);
    };
    let snapshot = match client.fetch(IssueNumber(number), None)? {
        FetchResult::Updated(snapshot) => snapshot,
        FetchResult::NotModified => {
            return Err(gwt_github::ApiError::Unexpected(
                "unconditional named Issue history fetch reported not modified".into(),
            ))
        }
    };
    if snapshot
        .comments
        .iter()
        .filter_map(|comment| named_record_from_body(&comment.body))
        .any(|existing| {
            existing.name == record.name
                && existing.phase == record.phase
                && existing.pr_number == record.pr_number
        })
    {
        return Ok(false);
    }
    let metadata = serde_json::to_string(&record).expect("named Issue metadata serializes");
    client.create_comment(
        IssueNumber(number),
        &format!("{NAMED_ACTIVITY_PREFIX}{metadata} -->\n\n{narrative}\n"),
    )?;
    Ok(true)
}

/// Append a durable assignment after the named Session has successfully launched.
/// Call from the launch worker, never from the GUI event loop.
pub fn record_named_issue_assignment<C: gwt_github::IssueClient + ?Sized>(
    client: &C,
    session: &gwt_agent::Session,
    previous_name: Option<&str>,
) -> Result<bool, gwt_github::ApiError> {
    let Some(record) = named_record(session, "assignment", None) else {
        return Ok(false);
    };
    let previous_name = previous_name.map(str::trim).filter(|name| !name.is_empty());
    if previous_name == Some(record.name.as_str()) {
        return Ok(false);
    }
    let narrative = match previous_name {
        Some(previous) => format!(
            "{previous} から {} に引き継ぎ（{}）。",
            record.name, record.provider
        ),
        None => format!("{}（{}）が担当を開始。", record.name, record.provider),
    };
    record_named_comment(client, session, record, &narrative)
}

/// Name the producing agent in the owner's append-only GitHub history.
pub fn record_named_issue_activity<C: gwt_github::IssueClient + ?Sized>(
    client: &C,
    session: &gwt_agent::Session,
    activity: NamedIssueActivity,
) -> Result<bool, gwt_github::ApiError> {
    let (phase, pr_number) = match activity {
        NamedIssueActivity::PrCreated { number } => ("pr_created", Some(number)),
        NamedIssueActivity::Completed => ("completed", None),
    };
    let Some(record) = named_record(session, phase, pr_number) else {
        return Ok(false);
    };
    let narrative = match activity {
        NamedIssueActivity::PrCreated { number } => format!(
            "{} が対応し、PR #{number} を作成（{}）。",
            record.name, record.provider
        ),
        NamedIssueActivity::Completed => {
            format!("{} が対応を完了（{}）。", record.name, record.provider)
        }
    };
    record_named_comment(client, session, record, &narrative)
}

/// Keep an immutable name/provider snapshot with the PR, independent of a live pane.
pub fn with_named_pr_author(body: &str, session: Option<&gwt_agent::Session>) -> String {
    let Some(session) = session else {
        return body.to_string();
    };
    if session.review_dispatch {
        return body.to_string();
    }
    let repo = session
        .project_state_root
        .as_deref()
        .unwrap_or(session.worktree_path.as_path());
    let pm_host = pm_host_for_repo(repo, &session.id);
    let Some(mut writer) = github_writer_for_session(session, pm_host.as_deref()) else {
        return body.to_string();
    };
    writer.phase = "pr_author".to_string();
    stamp_github_body(body, Some(&writer))
}

pub fn preserve_named_pr_author(body: &str, previous: &str) -> String {
    let Some(record) = named_records_in(previous)
        .into_iter()
        .find(|record| record.phase == "pr_author")
    else {
        return body.to_string();
    };
    stamp_github_body(body, Some(&writer_from_record(&record)))
}

pub fn initialize_launch_session(
    worktree: &Path,
    project_root: &Path,
    config: &gwt_agent::LaunchConfig,
) -> gwt_agent::Session {
    let branch_name = config.branch.clone().unwrap_or_else(|| "work".to_string());
    let mut session = gwt_agent::Session::new(worktree, branch_name, config.agent_id.clone());
    session.project_state_root = Some(gwt_core::paths::normalize_windows_child_process_path(
        project_root,
    ));
    session.display_name = config.display_name.clone();
    session.tool_version = config.tool_version.clone();
    session.model = config.model.clone();
    session.reasoning_level = config.reasoning_level.clone();
    session.session_mode = config.session_mode;
    session.skip_permissions = config.skip_permissions;
    session.fast_mode = config.fast_mode;
    session.codex_fast_mode = config.codex_fast_mode;
    session.runtime_target = config.runtime_target;
    session.docker_service = config.docker_service.clone();
    session.docker_lifecycle_intent = config.docker_lifecycle_intent;
    session.linked_issue_number = config.linked_issue_number;
    session.review_dispatch =
        crate::issue_monitor_review::review_dispatch_session_from_env(|name| {
            config.env_vars.get(name).cloned()
        });
    session.launch_route = config.launch_route;
    session.launch_command = config.command.clone();
    session.launch_args = config.args.clone();
    session.codex_auth_root = config.validated_codex_auth_root_for_cwd(worktree);
    session.windows_shell = config.windows_shell;
    apply_resume_identity_to_session(&mut session, config);
    if crate::pm_registry::is_canonical_pm_worktree(worktree) {
        session.restore_source_session_id = config.predecessor_session_id.clone();
    }
    session.update_status(gwt_agent::AgentStatus::Running);
    session
}

pub fn apply_resume_identity_to_session(
    session: &mut gwt_agent::Session,
    config: &gwt_agent::LaunchConfig,
) {
    if session.agent_member_id.is_none() {
        session.agent_member_id = config
            .env_vars
            .get("GWT_AGENT_MEMBER_ID")
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty());
    }
    if session.agent_name.is_none() {
        session.agent_name = config
            .env_vars
            .get("GWT_AGENT_NAME")
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());
    }
    if config.session_mode == gwt_agent::SessionMode::Resume {
        session.agent_session_id = config.resume_session_id.clone();
    }
}

#[cfg(any(test, feature = "test-gh-guard"))]
type FinalizedSessionPostSaveHook =
    Box<dyn FnOnce(&gwt_agent::Session) -> std::io::Result<()> + 'static>;

#[cfg(any(test, feature = "test-gh-guard"))]
thread_local! {
    static FINALIZED_SESSION_POST_SAVE_HOOK:
        std::cell::RefCell<Option<FinalizedSessionPostSaveHook>> =
            std::cell::RefCell::new(None);
}

#[cfg(any(test, feature = "test-gh-guard"))]
pub(crate) fn set_finalized_session_post_save_hook_for_test(hook: FinalizedSessionPostSaveHook) {
    FINALIZED_SESSION_POST_SAVE_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(any(test, feature = "test-gh-guard"))]
fn invoke_finalized_session_post_save_hook(session: &gwt_agent::Session) -> std::io::Result<()> {
    FINALIZED_SESSION_POST_SAVE_HOOK.with(|slot| {
        slot.borrow_mut()
            .take()
            .map_or(Ok(()), |hook| hook(session))
    })
}

#[cfg(not(any(test, feature = "test-gh-guard")))]
fn invoke_finalized_session_post_save_hook(_session: &gwt_agent::Session) -> std::io::Result<()> {
    Ok(())
}

fn interrupt_exact_running_session_after_persistence_error(
    sessions_dir: &Path,
    session: &mut gwt_agent::Session,
    running_snapshot: &gwt_agent::Session,
    context: &str,
    error: impl std::fmt::Display,
) -> String {
    session.update_status(gwt_agent::AgentStatus::Interrupted);
    match session.save_if_unchanged(sessions_dir, running_snapshot) {
        Ok(true) => format!("{context}; Session was marked Interrupted: {error}"),
        Ok(false) => {
            format!("{context} and no exact Running Session remained: {error}")
        }
        Err(interruption_error) => {
            format!("{context} and Session interruption failed: {error}; {interruption_error}")
        }
    }
}

/// Persist the finalized Session, retaining its exact execution authority.
/// `None` defers runtime observation until physical child identity publication.
pub fn persist_finalized_launch_session(
    sessions_dir: &Path,
    runtime_path: Option<&Path>,
    session: &mut gwt_agent::Session,
    docker_runtime_worktree: Option<&str>,
) -> Result<(), String> {
    if let Some(runtime_worktree) = docker_runtime_worktree {
        let project_state_root = session
            .project_state_root
            .as_deref()
            .filter(|root| !root.as_os_str().is_empty())
            .ok_or_else(|| {
                "Docker launch is missing the host Project State root before Session persistence"
                    .to_string()
            })?
            .to_path_buf();
        session.bind_docker_runtime(runtime_worktree, &project_state_root)?;
    }

    let saved_identity = gwt_agent::SessionExecutionIdentity::from_session(session)?;
    let running_snapshot = session.clone();
    let save_result = if let Some(expected) = saved_identity.as_ref() {
        session.save_if_execution_identity_matches(sessions_dir, expected)
    } else {
        session.save_if_absent_or_unchanged(sessions_dir, session)
    };
    match save_result {
        Ok(true) => {
            if let Err(error) = invoke_finalized_session_post_save_hook(session) {
                return Err(interrupt_exact_running_session_after_persistence_error(
                    sessions_dir,
                    session,
                    &running_snapshot,
                    "failed to durably confirm launch Session",
                    error,
                ));
            }
        }
        Ok(false) if saved_identity.is_some() => {
            return Err(
                "bound Session identity changed before final launch persistence".to_string(),
            )
        }
        Ok(false) => {
            return Err("unbound Session changed before final launch persistence".to_string())
        }
        Err(error) => {
            // Atomic replace followed by parent-directory fsync has an
            // unknown outcome: the intended Running TOML may already be
            // visible even though persistence returned Err. Transition only
            // that exact intended snapshot to Interrupted; a concurrent
            // same-id replacement remains byte-identical.
            return Err(interrupt_exact_running_session_after_persistence_error(
                sessions_dir,
                session,
                &running_snapshot,
                "failed to durably persist launch Session",
                error,
            ));
        }
    }
    let Some(runtime_path) = runtime_path else {
        return Ok(());
    };
    if let Err(error) =
        gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running).save(runtime_path)
    {
        let running_snapshot = session.clone();
        session.update_status(gwt_agent::AgentStatus::Interrupted);
        let rollback = if let Some(expected) = saved_identity.as_ref() {
            session.save_if_execution_identity_matches(sessions_dir, expected)
        } else {
            session.save_if_unchanged(sessions_dir, &running_snapshot)
        };
        return Err(match rollback {
            Ok(true) => format!(
                "failed to persist launch runtime state; Session was marked Interrupted: {error}"
            ),
            Ok(false) => format!(
                "failed to persist launch runtime state and Session changed before interruption: {error}"
            ),
            Err(rollback_error) => format!(
                "failed to persist launch runtime state and Session interruption failed: {error}; {rollback_error}"
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_3730_recovery_metadata_preserves_errors_and_not_found() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!durable_launch_recovery_exists(dir.path(), "session").unwrap());
        let path = durable_launch_recovery_path(dir.path(), "session").unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "{}").unwrap();
        assert!(durable_launch_recovery_exists(dir.path(), "session").unwrap());
        let error =
            durable_launch_recovery_exists(&dir.path().join("unreadable\0sessions"), "session")
                .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            durable_launch_recovery_exists(dir.path(), "../session")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn named_issue_history_records_start_handover_pr_and_completion_once() {
        use gwt_github::{FakeIssueClient, IssueNumber, IssueSnapshot, IssueState, UpdatedAt};
        let client = FakeIssueClient::new();
        client.seed(IssueSnapshot {
            author_login: None,
            number: IssueNumber(42),
            title: "work".into(),
            body: String::new(),
            labels: vec![],
            state: IssueState::Open,
            updated_at: UpdatedAt::new("2026-10-15T00:00:00Z"),
            comments: vec![],
        });
        let mut session =
            gwt_agent::Session::new("/tmp/work", "work/issue-42", gwt_agent::AgentId::Codex);
        session.linked_issue_number = Some(42);
        session.agent_name = Some("ハヤテ".into());
        assert!(record_named_issue_assignment(&client, &session, None).unwrap());
        assert!(!record_named_issue_assignment(&client, &session, None).unwrap());
        let mut successor = session.clone();
        successor.id = "successor".into();
        successor.agent_name = Some("ミオ".into());
        assert!(record_named_issue_assignment(&client, &successor, Some("ハヤテ")).unwrap());
        assert!(!record_named_issue_assignment(&client, &successor, Some("ミオ")).unwrap());
        assert!(record_named_issue_activity(
            &client,
            &successor,
            NamedIssueActivity::PrCreated { number: 43 }
        )
        .unwrap());
        assert!(
            record_named_issue_activity(&client, &successor, NamedIssueActivity::Completed)
                .unwrap()
        );
        assert!(
            !record_named_issue_activity(&client, &successor, NamedIssueActivity::Completed)
                .unwrap()
        );
        let comments = client.comments(IssueNumber(42));
        assert_eq!(comments.len(), 4);
        assert!(comments[0].body.contains("ハヤテ（Codex）が担当を開始"));
        assert!(comments[1].body.contains("ハヤテ から ミオ に引き継ぎ"));
        assert!(comments[2].body.contains("ミオ が対応し、PR #43 を作成"));
        assert!(comments[3].body.contains("ミオ が対応を完了"));
    }

    #[test]
    fn named_pr_body_keeps_the_assigned_name_and_provider_snapshot() {
        let mut session =
            gwt_agent::Session::new("/tmp/work", "work/issue-42", gwt_agent::AgentId::Codex);
        session.agent_name = Some("Maya".into());
        let body = with_named_pr_author("Existing body", Some(&session));
        assert!(body.starts_with("Existing body"));
        assert!(body.contains("— Maya（Codex）"));
        assert!(body.contains("\"phase\":\"pr_author\""));
        assert_eq!(with_named_pr_author(&body, Some(&session)), body);
        assert_eq!(with_named_pr_author("legacy", None), "legacy");
    }

    fn agent_writer(name: &str, provider: &str) -> GithubWriter {
        GithubWriter {
            name: name.to_string(),
            detail: provider.to_string(),
            role: GithubWriterRole::Agent,
            session_id: "session-1".to_string(),
            phase: "write".to_string(),
        }
    }

    #[test]
    fn github_body_signature_names_the_agent_once_and_skips_an_unnamed_writer() {
        let writer = agent_writer("サクラ", "Claude Code");
        let stamped = stamp_github_body("本文", Some(&writer));
        assert!(stamped.starts_with("本文\n\n<!-- gwt-agent-work "));
        assert!(stamped.contains("\"role\":\"agent\""));
        assert_eq!(
            stamped
                .lines()
                .filter(|line| line.trim() == "— サクラ（Claude Code）")
                .count(),
            1
        );
        assert_eq!(stamp_github_body(&stamped, Some(&writer)), stamped);
        let edited = stamped.replace("本文", "更新した本文");
        assert_eq!(stamp_github_body(&edited, Some(&writer)), edited);
        assert_eq!(stamp_github_body("人間の本文", None), "人間の本文");
        assert_eq!(
            stamp_github_body("空の担当者", Some(&agent_writer("", "Claude Code"))),
            "空の担当者"
        );
    }

    #[test]
    fn github_body_signature_uses_the_pm_host_and_issues_writer() {
        let pm = GithubWriter {
            name: "ソノ・キムラ".to_string(),
            detail: "ccs-win-125".to_string(),
            role: GithubWriterRole::Pm,
            session_id: "pm-1".to_string(),
            phase: "write".to_string(),
        };
        let stamped = stamp_github_body("裁定", Some(&pm));
        assert!(stamped.contains("\"role\":\"pm\""));
        assert_eq!(
            stamped
                .lines()
                .filter(|line| line.trim() == "— PM ソノ・キムラ（ccs-win-125）")
                .count(),
            1
        );
        let issues = issues_github_writer();
        let settlement = stamp_github_body("close します", Some(&issues));
        assert_eq!(
            settlement
                .lines()
                .filter(|line| line.trim() == "— gwt Issues")
                .count(),
            1
        );
        assert!(settlement.contains("\"role\":\"issues\""));
        assert_eq!(stamp_github_body(&settlement, Some(&issues)), settlement);
        let both = stamp_github_body(&stamped, Some(&issues));
        assert!(both.contains("— PM ソノ・キムラ（ccs-win-125）"));
        assert!(both.contains("— gwt Issues"));
    }

    #[test]
    fn current_github_writer_reads_the_named_session_and_pm_registration() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let _unset = gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        assert!(current_github_writer(&repo).is_none());
        drop(_unset);

        let mut session =
            gwt_agent::Session::new(&repo, "work/issue-7707", gwt_agent::AgentId::GrokBuild);
        session.agent_name = Some("テツヤ".into());
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save session");
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, &session.id);
        let writer = current_github_writer(&repo).expect("named session");
        assert_eq!(writer.role, GithubWriterRole::Agent);
        assert_eq!(writer.name, "テツヤ");
        assert_eq!(writer.detail, "Grok Build");

        session.agent_name = None;
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save unnamed session");
        assert!(current_github_writer(&repo).is_none());

        session.agent_name = Some("ソノ・キムラ".into());
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save pm session");
        let prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&repo);
        std::fs::create_dir_all(prefs_path.parent().unwrap()).unwrap();
        crate::pm_registry::save_pm_prefs(
            &prefs_path,
            &crate::pm_registry::PmPrefs {
                registration: Some(crate::pm_registry::PmRegistration {
                    session_id: session.id.clone(),
                    agent_id: "grok".into(),
                    worktree_path: repo.display().to_string(),
                    created_at: None,
                    consecutive_crashes: 0,
                    next_not_before: None,
                }),
                ..Default::default()
            },
        )
        .expect("save pm prefs");
        let pm = current_github_writer(&repo).expect("pm session");
        assert_eq!(pm.role, GithubWriterRole::Pm);
        assert_eq!(pm.name, "ソノ・キムラ");
        assert_eq!(pm.detail, crate::process::current_hostname());
        assert!(!pm.detail.is_empty());
    }

    #[test]
    fn launch_session_initializes_and_persists_without_gui() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = gwt_core::test_support::ScopedGwtHome::set(tmp.path());
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode).build();
        config.tool_version = Some("2.1.156".into());
        config.session_mode = gwt_agent::SessionMode::Resume;
        config.resume_session_id = Some("conversation".to_string());
        config.branch = Some("work/example".to_string());
        config
            .env_vars
            .insert("GWT_REVIEW_DISPATCH".to_string(), "1".to_string());
        config
            .env_vars
            .insert("GWT_AGENT_NAME".into(), "ミオ".into());
        let project = tmp.path().join("project");
        let worktree = tmp.path().join("worktree");
        let mut session = initialize_launch_session(&worktree, &project, &config);
        assert_eq!(session.project_state_root.as_ref(), Some(&project));
        assert_eq!(session.agent_name.as_deref(), Some("ミオ"));
        assert_eq!(session.exact_resume_session_id(), Some("conversation"));
        assert_eq!(session.launch_command, "claude");
        let sessions = tmp.path().join("sessions");
        let runtime = gwt_agent::runtime_state_path(&sessions, &session.id);
        persist_finalized_launch_session(&sessions, Some(&runtime), &mut session, None)
            .expect("persist launch");
        let saved = gwt_agent::Session::load(&sessions.join(format!("{}.toml", session.id)))
            .expect("load session");
        assert_eq!(saved.status, gwt_agent::AgentStatus::Running);
        assert_eq!(saved.agent_name.as_deref(), Some("ミオ"));
        assert_eq!(saved.exact_resume_session_id(), Some("conversation"));
        assert_eq!(saved.tool_version.as_deref(), Some("2.1.156"));
        assert_eq!(
            serde_json::to_value(&saved).unwrap()["review_dispatch"],
            true
        );
        assert!(saved.tool_version_selector.is_none());
        assert!(saved.tool_runtime_provenance.is_none());
        assert!(runtime.exists());
    }

    #[test]
    fn deferred_monitor_finalization_persists_session_without_blank_runtime() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = gwt_core::test_support::ScopedGwtHome::set(tmp.path());
        let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex).build();
        let mut candidate = initialize_launch_session(tmp.path(), tmp.path(), &config);
        let sessions = tmp.path().join("sessions");
        let runtime = gwt_agent::runtime_state_path(&sessions, &candidate.id);

        persist_finalized_launch_session(&sessions, None, &mut candidate, None)
            .expect("defer physical runtime publication");

        let saved = gwt_agent::Session::load(&sessions.join(format!("{}.toml", candidate.id)))
            .expect("finalized Session remains durable before physical spawn");
        assert_eq!(saved.id, candidate.id);
        assert_eq!(saved.status, gwt_agent::AgentStatus::Running);
        assert!(
            !runtime.exists(),
            "Monitor finalization must not publish a no-child runtime"
        );
    }
    #[test]
    fn finalized_bound_session_persistence_retains_same_id_replacement() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let sessions_dir = temp.path().join("sessions");
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, "prepared-candidate");
        let mut candidate = gwt_agent::Session::new(
            temp.path().join("worktree"),
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        candidate.id = "prepared-candidate".to_string();
        candidate.project_state_root = Some(temp.path().join("project"));
        candidate.repo_hash = Some("repo-hash".to_string());
        candidate.linked_issue_number = Some(2359);
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: candidate.id.clone(),
            repo_hash: "repo-hash".to_string(),
            owner_kind: "spec".to_string(),
            owner_number: 2359,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation".to_string(),
                binding_id: "binding".to_string(),
                ledger_head_hash: "ledger-head".to_string(),
            },
            capability_generation: 1,
        };
        candidate
            .set_execution_binding(Some(binding))
            .expect("bind candidate");
        candidate.save(&sessions_dir).expect("save exact candidate");

        let session_path = sessions_dir.join("prepared-candidate.toml");
        let mut replacement = candidate.clone();
        replacement.agent_id = gwt_agent::AgentId::Custom("replacement".to_string());
        replacement
            .save(&sessions_dir)
            .expect("save same-id replacement");
        let replacement_before =
            std::fs::read(&session_path).expect("read replacement Session bytes");

        let error = persist_finalized_launch_session(
            &sessions_dir,
            Some(&runtime_path),
            &mut candidate,
            None,
        )
        .expect_err("final persistence must reject a same-id replacement");

        assert!(
            error.contains("changed"),
            "replacement conflict should be actionable: {error}"
        );
        assert_eq!(
            std::fs::read(&session_path).expect("read retained replacement"),
            replacement_before,
            "final persistence must retain the replacement byte-identically"
        );
        assert!(
            !runtime_path.exists(),
            "runtime state must not publish after Session CAS rejection"
        );
    }

    #[test]
    fn finalized_unbound_session_persistence_retains_same_id_replacement() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let sessions_dir = temp.path().join("sessions");
        let mut candidate = gwt_agent::Session::new(
            temp.path().join("worktree"),
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        candidate.id = "unbound-candidate".to_string();
        candidate.project_state_root = Some(temp.path().join("project"));
        let mut replacement = candidate.clone();
        replacement.agent_id = gwt_agent::AgentId::Custom("replacement".to_string());
        replacement
            .save(&sessions_dir)
            .expect("save same-id replacement");
        let session_path = sessions_dir.join("unbound-candidate.toml");
        let replacement_before =
            std::fs::read(&session_path).expect("read replacement Session bytes");
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &candidate.id);

        let error = persist_finalized_launch_session(
            &sessions_dir,
            Some(&runtime_path),
            &mut candidate,
            None,
        )
        .expect_err("final persistence must reject an unbound same-id replacement");

        assert!(error.contains("changed"), "{error}");
        assert_eq!(
            std::fs::read(&session_path).expect("read retained replacement"),
            replacement_before
        );
        assert!(!runtime_path.exists());
    }

    #[test]
    fn finalized_session_sidecar_failure_marks_durable_session_interrupted() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let sessions_dir = temp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("create Sessions directory");
        let runtime_root = sessions_dir.join("runtime");
        std::fs::write(&runtime_root, b"not-a-directory").expect("block runtime directory");
        let mut candidate = gwt_agent::Session::new(
            temp.path().join("worktree"),
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        candidate.id = "sidecar-failure-candidate".to_string();
        candidate.project_state_root = Some(temp.path().join("project"));
        candidate.update_status(gwt_agent::AgentStatus::Running);
        let runtime_path = runtime_root.join(format!("{}.json", candidate.id));

        let error = persist_finalized_launch_session(
            &sessions_dir,
            Some(&runtime_path),
            &mut candidate,
            None,
        )
        .expect_err("blocked runtime root must fail final persistence");

        assert!(error.contains("marked Interrupted"), "{error}");
        assert_eq!(
            gwt_agent::Session::load(&sessions_dir.join("sidecar-failure-candidate.toml"))
                .expect("load interrupted Session")
                .status,
            gwt_agent::AgentStatus::Interrupted
        );
    }

    #[test]
    fn finalized_session_unknown_save_outcome_marks_visible_running_session_interrupted() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let sessions_dir = temp.path().join("sessions");
        let mut candidate = gwt_agent::Session::new(
            temp.path().join("worktree"),
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        candidate.id = "unknown-save-outcome-candidate".to_string();
        candidate.project_state_root = Some(temp.path().join("project"));
        candidate.update_status(gwt_agent::AgentStatus::Running);
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &candidate.id);
        set_finalized_session_post_save_hook_for_test(Box::new(|_| {
            Err(std::io::Error::other(
                "simulated parent-directory fsync failure after rename",
            ))
        }));

        let error = persist_finalized_launch_session(
            &sessions_dir,
            Some(&runtime_path),
            &mut candidate,
            None,
        )
        .expect_err("unknown Session save outcome must fail launch persistence");

        assert!(error.contains("marked Interrupted"), "{error}");
        assert_eq!(
            gwt_agent::Session::load(&sessions_dir.join("unknown-save-outcome-candidate.toml"))
                .expect("load recovered Session")
                .status,
            gwt_agent::AgentStatus::Interrupted,
            "a launch that was never returned to the spawner must not remain Running"
        );
        assert!(
            !runtime_path.exists(),
            "runtime state must not publish after an unknown Session save outcome"
        );
    }
}
