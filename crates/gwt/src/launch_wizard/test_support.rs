//! Shared test fixtures for the `launch_wizard` module tests.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Utc;

use super::{AgentOption, LaunchWizardContext, LinkedIssueKind, QuickStartEntry};
use crate::BranchListEntry;

pub(super) fn sample_agent_options() -> Vec<AgentOption> {
    vec![
        AgentOption {
            id: "claude".to_string(),
            name: "Claude Code".to_string(),
            available: true,
            installed_version: Some("1.0.0".to_string()),
            versions: vec!["0.9.0".to_string(), "1.0.0".to_string()],
            custom_agent: None,
        },
        AgentOption {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            available: true,
            installed_version: Some("0.110.0".to_string()),
            versions: vec!["0.109.0".to_string(), "0.110.0".to_string()],
            custom_agent: None,
        },
    ]
}

pub(super) fn sample_custom_agent(
    id: &str,
    display_name: &str,
    agent_type: gwt_agent::custom::CustomAgentType,
    command: impl Into<String>,
) -> gwt_agent::CustomCodingAgent {
    gwt_agent::CustomCodingAgent {
        id: id.to_string(),
        display_name: display_name.to_string(),
        agent_type,
        command: command.into(),
        default_args: vec!["--serve".to_string()],
        mode_args: Some(gwt_agent::custom::ModeArgs {
            normal: Vec::new(),
            continue_mode: vec!["--continue".to_string()],
            resume: vec!["--resume".to_string()],
        }),
        skip_permissions_args: vec!["--unsafe".to_string()],
        env: HashMap::from([("API_KEY".to_string(), "secret".to_string())]),
        supports_resume_picker: false,
    }
}

pub(super) fn branch(name: &str) -> BranchListEntry {
    BranchListEntry {
        name: name.to_string(),
        scope: crate::BranchScope::Local,
        is_head: false,
        upstream: Some(format!("origin/{name}")),
        ahead: 0,
        behind: 0,
        last_commit_date: None,
        cleanup_ready: true,
        cleanup: crate::BranchCleanupInfo::default(),
        resume: crate::BranchResumeInfo::unavailable(),
    }
}

pub(super) fn context(branch: BranchListEntry, normalized: &str) -> LaunchWizardContext {
    LaunchWizardContext {
        selected_branch: branch,
        normalized_branch_name: normalized.to_string(),
        worktree_path: None,
        quick_start_root: PathBuf::from("/tmp/repo"),
        live_sessions: Vec::new(),
        docker_context: None,
        docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
        linked_issue_number: None,
        linked_issue_kind: None,
        ultracode_supported: false,
    }
}

pub(super) fn context_with_linked_issue(
    branch: BranchListEntry,
    normalized: &str,
    kind: LinkedIssueKind,
    number: u64,
) -> LaunchWizardContext {
    let mut ctx = context(branch, normalized);
    ctx.linked_issue_kind = Some(kind);
    ctx.linked_issue_number = Some(number);
    ctx
}

pub(super) fn sample_session(
    dir: &Path,
    branch: &str,
    worktree_path: &Path,
    agent_id: gwt_agent::AgentId,
    updated_at: chrono::DateTime<Utc>,
    resume_id: &str,
) {
    sample_session_with_resume(
        dir,
        branch,
        worktree_path,
        agent_id,
        updated_at,
        Some(resume_id),
    );
}

pub(super) fn sample_session_with_resume(
    dir: &Path,
    branch: &str,
    worktree_path: &Path,
    agent_id: gwt_agent::AgentId,
    updated_at: chrono::DateTime<Utc>,
    resume_id: Option<&str>,
) {
    let mut session = gwt_agent::Session::new(worktree_path, branch, agent_id);
    session.display_name = session.agent_id.display_name().to_string();
    session.agent_session_id = resume_id.map(str::to_string);
    session.tool_version = Some("installed".to_string());
    session.model = Some("gpt-5.5".to_string());
    session.reasoning_level = Some("high".to_string());
    session.skip_permissions = true;
    session.codex_fast_mode = true;
    session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
    session.docker_service = Some("gwt".to_string());
    session.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;
    session.created_at = updated_at;
    session.updated_at = updated_at;
    session.last_activity_at = updated_at;
    session.save(dir).expect("save session");
}

pub(super) fn sample_session_record(
    branch: &str,
    worktree_path: &Path,
    agent_id: gwt_agent::AgentId,
    updated_at: chrono::DateTime<Utc>,
    resume_id: Option<&str>,
) -> gwt_agent::Session {
    let mut session = gwt_agent::Session::new(worktree_path, branch, agent_id);
    session.display_name = session.agent_id.display_name().to_string();
    session.agent_session_id = resume_id.map(str::to_string);
    session.tool_version = Some("installed".to_string());
    session.model = Some("gpt-5.5".to_string());
    session.reasoning_level = Some("high".to_string());
    session.skip_permissions = true;
    session.codex_fast_mode = true;
    session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
    session.docker_service = Some("gwt".to_string());
    session.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;
    session.created_at = updated_at;
    session.updated_at = updated_at;
    session.last_activity_at = updated_at;
    session
}

pub(super) fn init_repo_with_origin(path: &Path, origin: &str) {
    std::fs::create_dir_all(path).expect("repo dir");
    let status = gwt_core::process::hidden_command("git")
        .args(["init"])
        .current_dir(path)
        .status()
        .expect("git init");
    assert!(status.success(), "git init failed");
    let status = gwt_core::process::hidden_command("git")
        .args(["remote", "add", "origin", origin])
        .current_dir(path)
        .status()
        .expect("git remote add");
    assert!(status.success(), "git remote add failed");
}

pub(super) fn quick_start_entry(
    session_id: &str,
    agent_id: &str,
    resume_session_id: Option<&str>,
    live_window_id: Option<&str>,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    docker_service: Option<&str>,
) -> QuickStartEntry {
    let (tool_label, model, reasoning, version, codex_fast_mode) = match agent_id {
        "claude" => (
            "Claude Code",
            Some("sonnet"),
            Some("medium"),
            Some("latest"),
            false,
        ),
        "codex" => (
            "Codex",
            Some("gpt-5.5"),
            Some("high"),
            Some("0.110.0"),
            true,
        ),
        _ => ("Custom", None, None, None, false),
    };
    QuickStartEntry {
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        tool_label: tool_label.to_string(),
        model: model.map(str::to_string),
        reasoning: reasoning.map(str::to_string),
        version: version.map(str::to_string),
        resume_session_id: resume_session_id.map(str::to_string),
        live_window_id: live_window_id.map(str::to_string),
        skip_permissions: true,
        codex_fast_mode,
        runtime_target,
        docker_service: docker_service.map(str::to_string),
        docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
    }
}
