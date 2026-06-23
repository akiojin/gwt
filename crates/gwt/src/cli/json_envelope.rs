use gwt_agent::session::GWT_SESSION_ID_ENV;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::protocol::{IndexSearchMatchMode, IndexSearchScope};

use super::{
    memory::MemoryAddCommand, ActionsCommand, CliCommand, CliEnv, CliParseError, DaemonCommand,
    DiagnosticsCommand, HookCommand, IndexCommand, IndexScope, IssueCommand, MemoryCommand,
    PaneCommand, PrCommand, SearchCommand, SkillStateAction, WorkspaceCommand,
};
use super::{BoardCommand, BoardPostCommand};

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default = "default_schema_version")]
    schema_version: u64,
    operation: String,
    #[serde(default)]
    params: Value,
}

fn default_schema_version() -> u64 {
    1
}

struct ParsedEnvelope {
    operation: String,
    command: CliCommand,
}

pub(crate) fn dispatch<E: CliEnv>(env: &mut E, prog: &str) -> i32 {
    let input = match env.read_stdin() {
        Ok(input) => input,
        Err(err) => {
            let _ = writeln!(env.stderr(), "{prog}: failed to read JSON envelope: {err}");
            return 1;
        }
    };
    let parsed = match parse(&input) {
        Ok(parsed) => parsed,
        Err(err) => {
            let _ = writeln!(env.stderr(), "{prog}: {err}");
            return 2;
        }
    };
    let operation = parsed.operation.clone();
    match super::run_collect(env, parsed.command) {
        Ok((code, output)) => {
            let payload = serde_json::json!({
                "ok": code == 0,
                "operation": operation,
                "exit_code": code,
                "output": output,
            });
            let _ = writeln!(env.stdout(), "{}", payload);
            code
        }
        Err(err) => {
            let _ = writeln!(env.stderr(), "{prog} {operation}: {err}");
            1
        }
    }
}

fn parse(input: &str) -> Result<ParsedEnvelope, CliParseError> {
    if input.trim().is_empty() {
        return Err(CliParseError::InvalidJson(
            "stdin must contain a JSON envelope".to_string(),
        ));
    }
    let envelope: Envelope =
        serde_json::from_str(input).map_err(|err| CliParseError::InvalidJson(err.to_string()))?;
    if envelope.schema_version != 1 {
        return Err(CliParseError::InvalidJson(
            "schema_version must be 1".to_string(),
        ));
    }
    let params = params_object(&envelope.params)?;
    let command = match envelope.operation.as_str() {
        "workspace.update" => workspace_update(params)?,
        "workspace.candidates" => workspace_candidates(params)?,
        "workspace.join" => workspace_join(params)?,
        "workspace.create" => workspace_create(params)?,
        "workspace.ensure" => workspace_ensure(params)?,
        "workspace.projection_list" | "workspace.projection-list" => {
            CliCommand::Workspace(WorkspaceCommand::ProjectionList {
                stale: optional_bool(params, "stale")?.unwrap_or(false),
                all: optional_bool(params, "all")?.unwrap_or(false),
            })
        }
        "workspace.projection_prune" | "workspace.projection-prune" => {
            CliCommand::Workspace(WorkspaceCommand::ProjectionPrune {
                dry_run: optional_bool(params, "dry_run")?.unwrap_or(false),
                ids: optional_string_vec(params, "ids")?,
            })
        }
        "board.show" => board_show(params)?,
        "board.post" => board_post(params)?,
        "board.config.show" | "board.config-show" => {
            CliCommand::Board(crate::cli::board::BoardCommand::ConfigShow)
        }
        "issue.view" => CliCommand::Issue(IssueCommand::View {
            number: required_u64(params, "number")?,
            refresh: optional_bool(params, "refresh")?.unwrap_or(false),
        }),
        "issue.comments" => CliCommand::Issue(IssueCommand::Comments {
            number: required_u64(params, "number")?,
            refresh: optional_bool(params, "refresh")?.unwrap_or(false),
        }),
        "issue.linked_prs" | "issue.linked-prs" => CliCommand::Issue(IssueCommand::LinkedPrs {
            number: required_u64(params, "number")?,
            refresh: optional_bool(params, "refresh")?.unwrap_or(false),
        }),
        "issue.spec.read" => CliCommand::Issue(IssueCommand::SpecReadAll {
            number: required_u64(params, "number")?,
        }),
        "issue.spec.section" => CliCommand::Issue(IssueCommand::SpecReadSection {
            number: required_u64(params, "number")?,
            section: required_string(params, "section")?,
        }),
        "issue.spec.list" => CliCommand::Issue(IssueCommand::SpecList {
            phase: optional_string(params, "phase")?,
            state: optional_string(params, "state")?,
        }),
        "issue.spec.pull" => CliCommand::Issue(IssueCommand::SpecPull {
            all: optional_bool(params, "all")?.unwrap_or(false),
            numbers: optional_u64_vec(params, "numbers")?,
        }),
        "issue.spec.repair" => CliCommand::Issue(IssueCommand::SpecRepair {
            number: required_u64(params, "number")?,
        }),
        "issue.spec.rename" => CliCommand::Issue(IssueCommand::SpecRename {
            number: required_u64(params, "number")?,
            title: required_string(params, "title")?,
        }),
        "issue.spec.edit" => issue_spec_edit(params)?,
        "issue.spec.create" => issue_spec_create(params)?,
        "issue.create" => CliCommand::Issue(IssueCommand::CreateBody {
            title: required_string(params, "title")?,
            body: required_string(params, "body")?,
            labels: optional_string_vec(params, "labels")?,
        }),
        "issue.comment" => CliCommand::Issue(IssueCommand::CommentBody {
            number: required_u64(params, "number")?,
            body: required_string(params, "body")?,
        }),
        "pr.current" => CliCommand::Pr(PrCommand::Current),
        "pr.create" => CliCommand::Pr(PrCommand::CreateBody {
            base: required_string(params, "base")?,
            head: optional_string(params, "head")?,
            title: required_string(params, "title")?,
            body: required_string(params, "body")?,
            labels: optional_string_vec(params, "labels")?,
            draft: optional_bool(params, "draft")?.unwrap_or(false),
        }),
        "pr.edit" => CliCommand::Pr(PrCommand::EditBody {
            number: required_u64(params, "number")?,
            title: optional_string(params, "title")?,
            body: optional_string(params, "body")?,
            add_labels: optional_string_vec(params, "add_labels")?,
        }),
        "pr.view" => CliCommand::Pr(PrCommand::View {
            number: required_u64(params, "number")?,
        }),
        "pr.comment" => CliCommand::Pr(PrCommand::CommentBody {
            number: required_u64(params, "number")?,
            body: required_string(params, "body")?,
        }),
        "pr.checks" => CliCommand::Pr(PrCommand::Checks {
            number: required_u64(params, "number")?,
        }),
        "pr.reviews" => CliCommand::Pr(PrCommand::Reviews {
            number: required_u64(params, "number")?,
        }),
        "pr.review_threads" | "pr.review-threads" => CliCommand::Pr(PrCommand::ReviewThreads {
            number: required_u64(params, "number")?,
        }),
        "pr.review_threads.reply_and_resolve" | "pr.review-threads.reply-and-resolve" => {
            CliCommand::Pr(PrCommand::ReviewThreadsReplyAndResolveBody {
                number: required_u64(params, "number")?,
                body: required_string(params, "body")?,
            })
        }
        "actions.logs" => CliCommand::Actions(ActionsCommand::Logs {
            run_id: required_u64(params, "run_id")?,
        }),
        "actions.job_logs" | "actions.job-logs" => CliCommand::Actions(ActionsCommand::JobLogs {
            job_id: required_u64(params, "job_id")?,
        }),
        "index.status" => CliCommand::Index(IndexCommand::Status),
        "index.rebuild" => CliCommand::Index(IndexCommand::Rebuild {
            scope: optional_string(params, "scope")?
                .map(|scope| index_scope(&scope))
                .transpose()?
                .unwrap_or(IndexScope::All),
        }),
        "diagnostics.cpu" => CliCommand::Diagnostics(DiagnosticsCommand::Cpu { json: true }),
        "daemon.start" => CliCommand::Daemon(DaemonCommand::Start),
        "daemon.status" => CliCommand::Daemon(DaemonCommand::Status),
        "daemon.subscribe" => daemon_subscribe(params)?,
        "hook.register_codex_managed_hook_trust" | "hook.register-codex-managed-hook-trust" => {
            hook_register_codex_trust(params)?
        }
        "hook.health" => hook_health(params)?,
        "hook.doctor" => hook_doctor(params)?,
        "memory.add" => memory_add(params)?,
        "discussion.update" => discussion_update(params)?,
        "discuss.resolve" => discuss_proposal(params, DiscussEnvelopeAction::Resolve)?,
        "discuss.park" => discuss_proposal(params, DiscussEnvelopeAction::Park)?,
        "discuss.reject" => discuss_proposal(params, DiscussEnvelopeAction::Reject)?,
        "discuss.clear_next_question" | "discuss.clear-next-question" => {
            discuss_proposal(params, DiscussEnvelopeAction::ClearNextQuestion)?
        }
        "discuss.goal_pending" | "discuss.goal-pending" => {
            discuss_proposal(params, DiscussEnvelopeAction::GoalPending)?
        }
        "discuss.goal_started" | "discuss.goal-started" => {
            discuss_proposal(params, DiscussEnvelopeAction::GoalStarted)?
        }
        "discuss.goal_failed" | "discuss.goal-failed" => {
            discuss_proposal(params, DiscussEnvelopeAction::GoalFailed)?
        }
        "discuss.goal_skipped" | "discuss.goal-skipped" => {
            discuss_proposal(params, DiscussEnvelopeAction::GoalSkipped)?
        }
        "build.start" => skill_state(params, SkillActionKind::Start).map(CliCommand::Build)?,
        "build.phase" => skill_state(params, SkillActionKind::Phase).map(CliCommand::Build)?,
        "build.complete" => {
            skill_state(params, SkillActionKind::Complete).map(CliCommand::Build)?
        }
        "build.abort" => skill_state(params, SkillActionKind::Abort).map(CliCommand::Build)?,
        "plan.start" => skill_state(params, SkillActionKind::Start).map(CliCommand::Plan)?,
        "plan.phase" => skill_state(params, SkillActionKind::Phase).map(CliCommand::Plan)?,
        "plan.complete" => skill_state(params, SkillActionKind::Complete).map(CliCommand::Plan)?,
        "plan.abort" => skill_state(params, SkillActionKind::Abort).map(CliCommand::Plan)?,
        "register.start" => {
            skill_state(params, SkillActionKind::Start).map(CliCommand::Register)?
        }
        "register.phase" => {
            skill_state(params, SkillActionKind::Phase).map(CliCommand::Register)?
        }
        "register.complete" => {
            skill_state(params, SkillActionKind::Complete).map(CliCommand::Register)?
        }
        "register.abort" => {
            skill_state(params, SkillActionKind::Abort).map(CliCommand::Register)?
        }
        "pane.list" => CliCommand::Pane(PaneCommand::List),
        "pane.read" => CliCommand::Pane(PaneCommand::Read {
            id: required_string(params, "id")?,
            lines: optional_usize(params, "lines")?.unwrap_or(200),
        }),
        "pane.close" | "pane.stop" => CliCommand::Pane(PaneCommand::Close {
            id: required_string(params, "id")?,
        }),
        "pane.send" => CliCommand::Pane(PaneCommand::Send {
            id: optional_string(params, "id")?,
            text: required_string(params, "text")?,
        }),
        "search" => search(params)?,
        other => {
            return Err(CliParseError::UnknownSubcommand(other.to_string()));
        }
    };
    Ok(ParsedEnvelope {
        operation: envelope.operation,
        command,
    })
}

fn workspace_update(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    reject_key(params, "title_summary", "workspace.update uses purpose")?;
    let purpose = optional_string(params, "purpose")?;
    if let Some(value) = purpose.as_deref() {
        super::validate_title_summary_work_name("params.purpose", value)?;
    }
    let current_focus = optional_string(params, "current_focus")?;
    let agent_session = agent_session_or_env(params)?;
    if agent_session.is_none() && (purpose.is_some() || current_focus.is_some()) {
        return Err(CliParseError::MissingFlag("agent_session"));
    }
    Ok(CliCommand::Workspace(WorkspaceCommand::Update {
        title: optional_string(params, "title")?,
        status: optional_string(params, "status")?,
        status_text: optional_string(params, "status_text")?,
        summary: optional_string(params, "summary")?,
        progress_summary: optional_string(params, "progress_summary")?,
        next_action: optional_string(params, "next_action")?,
        owner: optional_string(params, "owner")?,
        agent_session,
        current_focus,
        title_summary: purpose,
    }))
}

fn workspace_candidates(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    Ok(CliCommand::Workspace(WorkspaceCommand::Candidates {
        agent_session: agent_session_or_env(params)?
            .ok_or(CliParseError::MissingFlag("agent_session"))?,
    }))
}

fn workspace_join(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    reject_key(params, "title_summary", "workspace.join uses purpose")?;
    let purpose = optional_string(params, "purpose")?;
    if let Some(value) = purpose.as_deref() {
        super::validate_title_summary_work_name("params.purpose", value)?;
    }
    Ok(CliCommand::Workspace(WorkspaceCommand::Join {
        agent_session: agent_session_or_env(params)?
            .ok_or(CliParseError::MissingFlag("agent_session"))?,
        workspace_id: optional_string(params, "workspace_id")?
            .or_else(|| optional_string(params, "workspace").ok().flatten())
            .ok_or(CliParseError::MissingFlag("workspace_id"))?,
        current_focus: optional_string(params, "current_focus")?,
        title_summary: purpose,
    }))
}

fn workspace_create(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    reject_key(params, "title_summary", "workspace.create uses purpose")?;
    let purpose = required_string(params, "purpose")?;
    super::validate_title_summary_work_name("params.purpose", &purpose)?;
    Ok(CliCommand::Workspace(WorkspaceCommand::Create {
        agent_session: agent_session_or_env(params)?
            .ok_or(CliParseError::MissingFlag("agent_session"))?,
        title_summary: purpose,
        current_focus: optional_string(params, "current_focus")?,
        spec: optional_u64(params, "spec")?,
        issue: optional_u64(params, "issue")?,
        split_from: optional_string(params, "split_from")?,
        boundary: optional_string(params, "boundary")?,
    }))
}

fn workspace_ensure(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    reject_key(params, "title_summary", "workspace.ensure uses purpose")?;
    let purpose = required_string(params, "purpose")?;
    super::validate_title_summary_work_name("params.purpose", &purpose)?;
    Ok(CliCommand::Workspace(WorkspaceCommand::Ensure {
        agent_session: agent_session_or_env(params)?
            .ok_or(CliParseError::MissingFlag("agent_session"))?,
        title_summary: purpose,
        current_focus: optional_string(params, "current_focus")?,
        spec: optional_u64(params, "spec")?,
        issue: optional_u64(params, "issue")?,
        topic: optional_string(params, "topic")?,
        boundary: optional_string(params, "boundary")?,
    }))
}

fn board_show(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    Ok(CliCommand::Board(BoardCommand::Show {
        json: true,
        workspace: optional_string(params, "workspace")?,
        all: optional_bool(params, "all")?.unwrap_or(false),
    }))
}

fn board_post(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    reject_key(params, "purpose", "board.post must not update Work purpose")?;
    reject_key(
        params,
        "title_summary",
        "board.post must not update agent title_summary",
    )?;
    Ok(CliCommand::Board(BoardCommand::Post(Box::new(
        BoardPostCommand {
            kind: required_string(params, "kind")?,
            body: Some(required_string(params, "body")?),
            file: None,
            title: optional_string(params, "title")?,
            title_summary: None,
            parent: optional_string(params, "parent")?,
            topics: optional_string_vec(params, "topics")?,
            owners: optional_string_vec(params, "owners")?,
            targets: optional_string_vec(params, "targets")?,
            mentions: optional_string_vec(params, "mentions")?,
            broadcast: optional_bool(params, "broadcast")?.unwrap_or(false),
        },
    ))))
}

fn issue_spec_edit(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    let structured = optional_bool(params, "structured")?.unwrap_or(false);
    let replace = optional_bool(params, "replace")?.unwrap_or(false);
    let number = required_u64(params, "number")?;
    let section = required_string(params, "section")?;
    if structured {
        return Ok(CliCommand::Issue(IssueCommand::SpecEditSectionJsonBody {
            number,
            section,
            body: required_json_or_string(params, "body")?,
            replace,
        }));
    }
    if replace {
        return Err(CliParseError::InvalidJson(
            "replace is only valid when structured is true".to_string(),
        ));
    }
    Ok(CliCommand::Issue(IssueCommand::SpecEditSectionBody {
        number,
        section,
        body: required_string(params, "body")?,
    }))
}

fn issue_spec_create(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    let structured = optional_bool(params, "structured")?.unwrap_or(false);
    if structured {
        return Ok(CliCommand::Issue(IssueCommand::SpecCreateJsonBody {
            title: required_string(params, "title")?,
            body: required_json_or_string(params, "body")?,
            labels: optional_string_vec(params, "labels")?,
        }));
    }
    Ok(CliCommand::Issue(IssueCommand::SpecCreateBody {
        title: required_string(params, "title")?,
        body: required_string(params, "body")?,
        labels: optional_string_vec(params, "labels")?,
    }))
}

fn memory_add(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    Ok(CliCommand::Memory(MemoryCommand::Add(MemoryAddCommand {
        date: optional_string(params, "date")?,
        memory_type: optional_string(params, "type")?.unwrap_or_else(|| "lesson".to_string()),
        title: required_string(params, "title")?,
        context: required_string(params, "context")?,
        learning: required_string(params, "learning")?,
        future_action: required_string(params, "future_action")?,
    })))
}

fn daemon_subscribe(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    let channels = optional_string_vec(params, "channels")?;
    if channels.is_empty() {
        return Err(CliParseError::MissingFlag("channels"));
    }
    Ok(CliCommand::Daemon(DaemonCommand::Subscribe { channels }))
}

fn hook_register_codex_trust(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    let mut rest = Vec::new();
    if let Some(project_root) = optional_string(params, "project_root")? {
        rest.push("--project-root".to_string());
        rest.push(project_root);
    }
    if let Some(codex_config) = optional_string(params, "codex_config")? {
        rest.push("--codex-config".to_string());
        rest.push(codex_config);
    }
    if let Some(discovery) = optional_string(params, "codex_hook_discovery")? {
        rest.push("--codex-hook-discovery".to_string());
        rest.push(discovery);
    }
    Ok(CliCommand::Hook(HookCommand::Run {
        name: "register-codex-managed-hook-trust".to_string(),
        rest,
    }))
}

fn hook_health(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    Ok(CliCommand::Hook(HookCommand::Health {
        runtime_state_path: optional_path(params, "runtime_state_path")?,
        profile_path: optional_path(params, "profile_path")?,
        expected_hook_bin: optional_string(params, "expected_hook_bin")?,
    }))
}

fn hook_doctor(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    Ok(CliCommand::Hook(HookCommand::Doctor {
        runtime_state_path: optional_path(params, "runtime_state_path")?,
        profile_path: optional_path(params, "profile_path")?,
        expected_hook_bin: optional_string(params, "expected_hook_bin")?,
        repair: optional_bool(params, "repair")?.unwrap_or(false),
    }))
}

fn discussion_update(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    Ok(CliCommand::Discussion(super::DiscussionCommand::Update(
        super::discussion::DiscussionUpdateCommand {
            date: optional_string(params, "date")?,
            title: required_string(params, "title")?,
            status: optional_string(params, "status")?.unwrap_or_else(|| "active".to_string()),
            topics: optional_string_vec(params, "topics")?,
            related_specs: optional_u64_vec(params, "related_specs")?,
            related_works: optional_string_vec(params, "related_works")?,
            promoted_to: optional_string_vec(params, "promoted_to")?,
            summary: required_string(params, "summary")?,
            decisions: optional_string_vec(params, "decisions")?,
            open_questions: optional_string_vec(params, "open_questions")?,
            next: required_string(params, "next")?,
        },
    )))
}

enum DiscussEnvelopeAction {
    Resolve,
    Park,
    Reject,
    ClearNextQuestion,
    GoalPending,
    GoalStarted,
    GoalFailed,
    GoalSkipped,
}

fn discuss_proposal(
    params: &Map<String, Value>,
    action: DiscussEnvelopeAction,
) -> Result<CliCommand, CliParseError> {
    let proposal = required_string(params, "proposal")?;
    let action = match action {
        DiscussEnvelopeAction::Resolve => super::DiscussAction::Resolve { proposal },
        DiscussEnvelopeAction::Park => super::DiscussAction::Park { proposal },
        DiscussEnvelopeAction::Reject => super::DiscussAction::Reject { proposal },
        DiscussEnvelopeAction::ClearNextQuestion => {
            super::DiscussAction::ClearNextQuestion { proposal }
        }
        DiscussEnvelopeAction::GoalPending => super::DiscussAction::GoalPendingBody {
            proposal,
            condition: required_string(params, "condition")?,
        },
        DiscussEnvelopeAction::GoalStarted => super::DiscussAction::GoalStarted { proposal },
        DiscussEnvelopeAction::GoalFailed => super::DiscussAction::GoalFailed {
            proposal,
            reason: required_string(params, "reason")?,
        },
        DiscussEnvelopeAction::GoalSkipped => super::DiscussAction::GoalSkipped {
            proposal,
            reason: required_string(params, "reason")?,
        },
    };
    Ok(CliCommand::Discuss(action))
}

enum SkillActionKind {
    Start,
    Phase,
    Complete,
    Abort,
}

fn skill_state(
    params: &Map<String, Value>,
    kind: SkillActionKind,
) -> Result<SkillStateAction, CliParseError> {
    let spec = required_u64(params, "spec")?;
    match kind {
        SkillActionKind::Start => Ok(SkillStateAction::Start { spec }),
        SkillActionKind::Phase => Ok(SkillStateAction::Phase {
            spec,
            label: required_string(params, "label")?,
        }),
        SkillActionKind::Complete => Ok(SkillStateAction::Complete { spec }),
        SkillActionKind::Abort => Ok(SkillStateAction::Abort {
            spec,
            reason: optional_string(params, "reason")?,
        }),
    }
}

fn search(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    let scopes = optional_string_vec(params, "scopes")?
        .into_iter()
        .map(|scope| match scope.as_str() {
            "specs" => Ok(IndexSearchScope::Specs),
            "issues" => Ok(IndexSearchScope::Issues),
            "files" => Ok(IndexSearchScope::Files),
            "files_docs" | "files-docs" => Ok(IndexSearchScope::FilesDocs),
            "memory" => Ok(IndexSearchScope::Memory),
            "board" => Ok(IndexSearchScope::Board),
            "discussions" => Ok(IndexSearchScope::Discussions),
            "works" => Ok(IndexSearchScope::Works),
            _ => Err(CliParseError::InvalidJson(format!(
                "unknown search scope: {scope}"
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let match_mode = match optional_string(params, "match_mode")?
        .unwrap_or_else(|| "semantic".to_string())
        .as_str()
    {
        "semantic" => IndexSearchMatchMode::Semantic,
        "all_terms" => IndexSearchMatchMode::AllTerms,
        other => {
            return Err(CliParseError::InvalidJson(format!(
                "unknown match_mode: {other}"
            )))
        }
    };
    Ok(CliCommand::Search(SearchCommand {
        query: required_string(params, "query")?,
        scopes,
        match_mode,
        n_results: optional_usize(params, "n_results")?,
        json: true,
    }))
}

fn index_scope(value: &str) -> Result<IndexScope, CliParseError> {
    match value {
        "all" => Ok(IndexScope::All),
        "issues" => Ok(IndexScope::Issues),
        "specs" => Ok(IndexScope::Specs),
        "memory" => Ok(IndexScope::Memory),
        "discussions" => Ok(IndexScope::Discussions),
        "board" => Ok(IndexScope::Board),
        "files" => Ok(IndexScope::Files),
        "files_docs" | "files-docs" => Ok(IndexScope::FilesDocs),
        other => Err(CliParseError::InvalidJson(format!(
            "unknown index scope: {other}"
        ))),
    }
}

fn params_object(value: &Value) -> Result<&Map<String, Value>, CliParseError> {
    value
        .as_object()
        .ok_or_else(|| CliParseError::InvalidJson("params must be an object".to_string()))
}

fn agent_session_or_env(params: &Map<String, Value>) -> Result<Option<String>, CliParseError> {
    Ok(optional_string(params, "agent_session")?
        .or_else(|| std::env::var(GWT_SESSION_ID_ENV).ok())
        .filter(|value| !value.trim().is_empty()))
}

fn reject_key(
    params: &Map<String, Value>,
    key: &'static str,
    reason: &'static str,
) -> Result<(), CliParseError> {
    if params.contains_key(key) {
        return Err(CliParseError::InvalidValue { flag: key, reason });
    }
    Ok(())
}

fn required_string(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<String, CliParseError> {
    optional_string(params, key)?.ok_or(CliParseError::MissingFlag(key))
}

fn required_json_or_string(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<String, CliParseError> {
    let Some(value) = params.get(key) else {
        return Err(CliParseError::MissingFlag(key));
    };
    match value {
        Value::String(text) if !text.trim().is_empty() => Ok(text.clone()),
        Value::String(_) | Value::Null => Err(CliParseError::MissingFlag(key)),
        Value::Object(_) | Value::Array(_) => {
            serde_json::to_string(value).map_err(|err| CliParseError::InvalidJson(err.to_string()))
        }
        _ => Err(CliParseError::InvalidJson(format!(
            "{key} must be a string, object, or array"
        ))),
    }
}

fn optional_string(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<String>, CliParseError> {
    let Some(value) = params.get(key) else {
        return Ok(None);
    };
    match value {
        Value::String(text) if !text.trim().is_empty() => Ok(Some(text.clone())),
        Value::String(_) | Value::Null => Ok(None),
        _ => Err(CliParseError::InvalidJson(format!(
            "{key} must be a string"
        ))),
    }
}

fn optional_path(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<std::path::PathBuf>, CliParseError> {
    Ok(optional_string(params, key)?.map(std::path::PathBuf::from))
}

fn required_u64(params: &Map<String, Value>, key: &'static str) -> Result<u64, CliParseError> {
    optional_u64(params, key)?.ok_or(CliParseError::MissingFlag(key))
}

fn optional_u64(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<u64>, CliParseError> {
    let Some(value) = params.get(key) else {
        return Ok(None);
    };
    match value {
        Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| CliParseError::InvalidJson(format!("{key} must be a u64")))
            .map(Some),
        Value::String(text) if !text.trim().is_empty() => text
            .parse::<u64>()
            .map(Some)
            .map_err(|_| CliParseError::InvalidNumber(text.clone())),
        Value::Null => Ok(None),
        _ => Err(CliParseError::InvalidJson(format!("{key} must be a u64"))),
    }
}

fn optional_usize(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<usize>, CliParseError> {
    optional_u64(params, key)?
        .map(|value| Ok(value as usize))
        .transpose()
}

fn optional_u64_vec(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<Vec<u64>, CliParseError> {
    let Some(value) = params.get(key) else {
        return Ok(Vec::new());
    };
    match value {
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Number(number) => number
                    .as_u64()
                    .ok_or_else(|| CliParseError::InvalidJson(format!("{key} must contain u64"))),
                Value::String(text) if !text.trim().is_empty() => text
                    .parse::<u64>()
                    .map_err(|_| CliParseError::InvalidNumber(text.clone())),
                _ => Err(CliParseError::InvalidJson(format!(
                    "{key} must be an array of u64 values"
                ))),
            })
            .collect(),
        Value::Null => Ok(Vec::new()),
        _ => Err(CliParseError::InvalidJson(format!(
            "{key} must be an array of u64 values"
        ))),
    }
}

fn optional_bool(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<bool>, CliParseError> {
    let Some(value) = params.get(key) else {
        return Ok(None);
    };
    match value {
        Value::Bool(value) => Ok(Some(*value)),
        Value::Null => Ok(None),
        _ => Err(CliParseError::InvalidJson(format!("{key} must be a bool"))),
    }
}

fn optional_string_vec(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<Vec<String>, CliParseError> {
    let Some(value) = params.get(key) else {
        return Ok(Vec::new());
    };
    match value {
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::String(text) if !text.trim().is_empty() => Ok(text.clone()),
                Value::String(_) | Value::Null => Err(CliParseError::InvalidJson(format!(
                    "{key} must not contain empty strings"
                ))),
                _ => Err(CliParseError::InvalidJson(format!(
                    "{key} must be an array of strings"
                ))),
            })
            .collect(),
        Value::Null => Ok(Vec::new()),
        _ => Err(CliParseError::InvalidJson(format!(
            "{key} must be an array of strings"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse, ActionsCommand, CliCommand, CliParseError, DaemonCommand, HookCommand, IndexCommand,
        IndexScope, IssueCommand, PaneCommand, PrCommand, SkillStateAction, WorkspaceCommand,
    };
    use crate::protocol::{IndexSearchMatchMode, IndexSearchScope};
    use serde_json::{json, Value};

    fn envelope(operation: &str, params: Value) -> String {
        json!({
            "schema_version": 1,
            "operation": operation,
            "params": params,
        })
        .to_string()
    }

    fn ok(operation: &str, params: Value) -> CliCommand {
        match parse(&envelope(operation, params)) {
            Ok(parsed) => {
                assert_eq!(parsed.operation, operation);
                parsed.command
            }
            Err(err) => panic!("expected Ok for {operation}, got error: {err}"),
        }
    }

    fn err(operation: &str, params: Value) -> CliParseError {
        match parse(&envelope(operation, params)) {
            Ok(_) => panic!("expected Err for {operation}"),
            Err(err) => err,
        }
    }

    #[test]
    fn empty_or_blank_stdin_is_invalid_json() {
        assert!(matches!(parse("   \n"), Err(CliParseError::InvalidJson(_))));
        assert!(matches!(parse(""), Err(CliParseError::InvalidJson(_))));
    }

    #[test]
    fn malformed_json_is_invalid_json() {
        assert!(matches!(
            parse("{not json"),
            Err(CliParseError::InvalidJson(_))
        ));
    }

    #[test]
    fn schema_version_must_be_one() {
        let input = json!({
            "schema_version": 2,
            "operation": "pr.current",
            "params": {},
        })
        .to_string();
        match parse(&input) {
            Err(CliParseError::InvalidJson(message)) => {
                assert!(message.contains("schema_version"), "{message}");
            }
            Err(other) => panic!("expected schema_version error, got {other}"),
            Ok(_) => panic!("expected schema_version error, got Ok"),
        }
    }

    #[test]
    fn schema_version_defaults_to_one_when_omitted() {
        let input = json!({
            "operation": "pr.current",
            "params": {},
        })
        .to_string();
        assert!(parse(&input).is_ok());
    }

    #[test]
    fn omitted_params_defaults_to_null_and_is_rejected() {
        // `params` defaults to JSON null when omitted, and a null is not an object.
        let input = json!({
            "schema_version": 1,
            "operation": "pane.list",
        })
        .to_string();
        match parse(&input) {
            Err(CliParseError::InvalidJson(message)) => {
                assert!(message.contains("params must be an object"), "{message}");
            }
            Err(other) => panic!("unexpected error: {other}"),
            Ok(_) => panic!("expected params-object error"),
        }
    }

    #[test]
    fn params_must_be_an_object() {
        match err("pr.current", json!(5)) {
            CliParseError::InvalidJson(message) => {
                assert!(message.contains("params must be an object"), "{message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn unknown_operation_is_rejected() {
        match err("does.not.exist", json!({})) {
            CliParseError::UnknownSubcommand(name) => assert_eq!(name, "does.not.exist"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn workspace_update_maps_purpose_to_title_summary() {
        let command = ok(
            "workspace.update",
            json!({
                "agent_session": "session-1",
                "purpose": "Build envelope parser",
                "current_focus": "writing tests",
                "status": "active",
                "summary": "covering parse",
            }),
        );
        match command {
            CliCommand::Workspace(WorkspaceCommand::Update {
                agent_session,
                title_summary,
                current_focus,
                ..
            }) => {
                assert_eq!(agent_session.as_deref(), Some("session-1"));
                assert_eq!(title_summary.as_deref(), Some("Build envelope parser"));
                assert_eq!(current_focus.as_deref(), Some("writing tests"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn workspace_update_rejects_title_summary_key() {
        match err(
            "workspace.update",
            json!({"agent_session": "s", "title_summary": "x"}),
        ) {
            CliParseError::InvalidValue { flag, .. } => assert_eq!(flag, "title_summary"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn workspace_update_requires_agent_session_when_setting_purpose() {
        // No agent_session in params; rely on env being unset for the work fields.
        // When purpose/current_focus omitted, agent_session is optional, so test the
        // explicit error path by providing current_focus without agent_session only
        // when the env var is absent.
        if std::env::var(gwt_agent::session::GWT_SESSION_ID_ENV).is_ok() {
            return;
        }
        match err("workspace.update", json!({"current_focus": "x"})) {
            CliParseError::MissingFlag(flag) => assert_eq!(flag, "agent_session"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn workspace_lifecycle_operations_parse() {
        assert!(matches!(
            ok("workspace.candidates", json!({"agent_session": "s"})),
            CliCommand::Workspace(WorkspaceCommand::Candidates { .. })
        ));
        assert!(matches!(
            ok(
                "workspace.join",
                json!({"agent_session": "s", "workspace_id": "w1", "current_focus": "f"})
            ),
            CliCommand::Workspace(WorkspaceCommand::Join { .. })
        ));
        // workspace alias for workspace_id
        assert!(matches!(
            ok(
                "workspace.join",
                json!({"agent_session": "s", "workspace": "w1"})
            ),
            CliCommand::Workspace(WorkspaceCommand::Join { .. })
        ));
        assert!(matches!(
            ok(
                "workspace.create",
                json!({"agent_session": "s", "purpose": "Add parser", "spec": 7, "issue": 3})
            ),
            CliCommand::Workspace(WorkspaceCommand::Create { .. })
        ));
        assert!(matches!(
            ok(
                "workspace.ensure",
                json!({"agent_session": "s", "purpose": "Add parser", "topic": "t"})
            ),
            CliCommand::Workspace(WorkspaceCommand::Ensure { .. })
        ));
        assert!(matches!(
            ok(
                "workspace.projection_list",
                json!({"stale": true, "all": true})
            ),
            CliCommand::Workspace(WorkspaceCommand::ProjectionList {
                stale: true,
                all: true
            })
        ));
        assert!(matches!(
            ok("workspace.projection-list", json!({})),
            CliCommand::Workspace(WorkspaceCommand::ProjectionList { .. })
        ));
        assert!(matches!(
            ok(
                "workspace.projection_prune",
                json!({"dry_run": true, "ids": ["a", "b"]})
            ),
            CliCommand::Workspace(WorkspaceCommand::ProjectionPrune { dry_run: true, .. })
        ));
        assert!(matches!(
            ok("workspace.projection-prune", json!({})),
            CliCommand::Workspace(WorkspaceCommand::ProjectionPrune { .. })
        ));
    }

    #[test]
    fn workspace_join_requires_workspace_id() {
        match err("workspace.join", json!({"agent_session": "s"})) {
            CliParseError::MissingFlag(flag) => assert_eq!(flag, "workspace_id"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn board_operations_parse() {
        assert!(matches!(
            ok("board.show", json!({"workspace": "w", "all": true})),
            CliCommand::Board(_)
        ));
        assert!(matches!(
            ok(
                "board.post",
                json!({
                    "kind": "status",
                    "body": "hello",
                    "topics": ["a"],
                    "mentions": ["user:x"],
                    "broadcast": true,
                })
            ),
            CliCommand::Board(_)
        ));
    }

    #[test]
    fn board_post_rejects_work_keys() {
        assert!(matches!(
            err(
                "board.post",
                json!({"kind": "status", "body": "b", "purpose": "p"})
            ),
            CliParseError::InvalidValue {
                flag: "purpose",
                ..
            }
        ));
        assert!(matches!(
            err(
                "board.post",
                json!({"kind": "status", "body": "b", "title_summary": "t"})
            ),
            CliParseError::InvalidValue {
                flag: "title_summary",
                ..
            }
        ));
    }

    #[test]
    fn issue_operations_parse() {
        for op in [
            "issue.view",
            "issue.comments",
            "issue.linked_prs",
            "issue.linked-prs",
            "issue.spec.read",
            "issue.spec.repair",
        ] {
            assert!(matches!(
                ok(op, json!({"number": 12})),
                CliCommand::Issue(_)
            ));
        }
        assert!(matches!(
            ok(
                "issue.spec.section",
                json!({"number": 1, "section": "spec"})
            ),
            CliCommand::Issue(IssueCommand::SpecReadSection { .. })
        ));
        assert!(matches!(
            ok("issue.spec.list", json!({"phase": "plan", "state": "open"})),
            CliCommand::Issue(IssueCommand::SpecList { .. })
        ));
        assert!(matches!(
            ok("issue.spec.pull", json!({"all": true, "numbers": [1, 2]})),
            CliCommand::Issue(IssueCommand::SpecPull { .. })
        ));
        assert!(matches!(
            ok("issue.spec.rename", json!({"number": 1, "title": "t"})),
            CliCommand::Issue(IssueCommand::SpecRename { .. })
        ));
        assert!(matches!(
            ok(
                "issue.create",
                json!({"title": "t", "body": "b", "labels": ["l"]})
            ),
            CliCommand::Issue(IssueCommand::CreateBody { .. })
        ));
        assert!(matches!(
            ok("issue.comment", json!({"number": 1, "body": "b"})),
            CliCommand::Issue(IssueCommand::CommentBody { .. })
        ));
    }

    #[test]
    fn issue_spec_edit_variants() {
        assert!(matches!(
            ok(
                "issue.spec.edit",
                json!({"number": 1, "section": "spec", "body": "text"})
            ),
            CliCommand::Issue(IssueCommand::SpecEditSectionBody { .. })
        ));
        assert!(matches!(
            ok(
                "issue.spec.edit",
                json!({"number": 1, "section": "spec", "body": {"k": "v"}, "structured": true, "replace": true})
            ),
            CliCommand::Issue(IssueCommand::SpecEditSectionJsonBody { replace: true, .. })
        ));
        match err(
            "issue.spec.edit",
            json!({"number": 1, "section": "spec", "body": "t", "replace": true}),
        ) {
            CliParseError::InvalidJson(message) => {
                assert!(message.contains("replace is only valid"), "{message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn issue_spec_create_variants() {
        assert!(matches!(
            ok("issue.spec.create", json!({"title": "t", "body": "b"})),
            CliCommand::Issue(IssueCommand::SpecCreateBody { .. })
        ));
        assert!(matches!(
            ok(
                "issue.spec.create",
                json!({"title": "t", "body": ["a", "b"], "structured": true})
            ),
            CliCommand::Issue(IssueCommand::SpecCreateJsonBody { .. })
        ));
    }

    #[test]
    fn pr_operations_parse() {
        assert!(matches!(
            ok("pr.current", json!({})),
            CliCommand::Pr(PrCommand::Current)
        ));
        assert!(matches!(
            ok(
                "pr.create",
                json!({"base": "main", "head": "develop", "title": "t", "body": "b", "labels": ["release"], "draft": true})
            ),
            CliCommand::Pr(PrCommand::CreateBody { draft: true, .. })
        ));
        assert!(matches!(
            ok(
                "pr.edit",
                json!({"number": 1, "title": "t", "add_labels": ["x"]})
            ),
            CliCommand::Pr(PrCommand::EditBody { .. })
        ));
        for op in [
            "pr.view",
            "pr.checks",
            "pr.reviews",
            "pr.review_threads",
            "pr.review-threads",
        ] {
            assert!(matches!(ok(op, json!({"number": 9})), CliCommand::Pr(_)));
        }
        assert!(matches!(
            ok("pr.comment", json!({"number": 1, "body": "b"})),
            CliCommand::Pr(PrCommand::CommentBody { .. })
        ));
        assert!(matches!(
            ok(
                "pr.review_threads.reply_and_resolve",
                json!({"number": 1, "body": "b"})
            ),
            CliCommand::Pr(PrCommand::ReviewThreadsReplyAndResolveBody { .. })
        ));
        assert!(matches!(
            ok(
                "pr.review-threads.reply-and-resolve",
                json!({"number": 1, "body": "b"})
            ),
            CliCommand::Pr(PrCommand::ReviewThreadsReplyAndResolveBody { .. })
        ));
    }

    #[test]
    fn actions_index_diagnostics_daemon_operations_parse() {
        assert!(matches!(
            ok("actions.logs", json!({"run_id": 5})),
            CliCommand::Actions(ActionsCommand::Logs { .. })
        ));
        assert!(matches!(
            ok("actions.job_logs", json!({"job_id": 5})),
            CliCommand::Actions(ActionsCommand::JobLogs { .. })
        ));
        assert!(matches!(
            ok("actions.job-logs", json!({"job_id": 5})),
            CliCommand::Actions(ActionsCommand::JobLogs { .. })
        ));
        assert!(matches!(
            ok("index.status", json!({})),
            CliCommand::Index(IndexCommand::Status)
        ));
        assert!(matches!(
            ok("index.rebuild", json!({})),
            CliCommand::Index(IndexCommand::Rebuild { .. })
        ));
        assert!(matches!(
            ok("index.rebuild", json!({"scope": "files_docs"})),
            CliCommand::Index(IndexCommand::Rebuild { .. })
        ));
        match err("index.rebuild", json!({"scope": "nope"})) {
            CliParseError::InvalidJson(message) => {
                assert!(message.contains("unknown index scope"), "{message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(matches!(
            ok("diagnostics.cpu", json!({})),
            CliCommand::Diagnostics(_)
        ));
        match ok(
            "hook.health",
            json!({
                "runtime_state_path": "/tmp/runtime.json",
                "profile_path": "/tmp/profile.jsonl",
                "expected_hook_bin": "/tmp/gwtd"
            }),
        ) {
            CliCommand::Hook(HookCommand::Health {
                runtime_state_path,
                profile_path,
                expected_hook_bin,
            }) => {
                assert_eq!(
                    runtime_state_path.as_deref(),
                    Some(std::path::Path::new("/tmp/runtime.json"))
                );
                assert_eq!(
                    profile_path.as_deref(),
                    Some(std::path::Path::new("/tmp/profile.jsonl"))
                );
                assert_eq!(expected_hook_bin.as_deref(), Some("/tmp/gwtd"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
        assert!(matches!(
            ok("hook.doctor", json!({"repair": true})),
            CliCommand::Hook(HookCommand::Doctor { repair: true, .. })
        ));
        assert!(matches!(
            ok("daemon.start", json!({})),
            CliCommand::Daemon(DaemonCommand::Start)
        ));
        assert!(matches!(
            ok("daemon.status", json!({})),
            CliCommand::Daemon(DaemonCommand::Status)
        ));
        assert!(matches!(
            ok("daemon.subscribe", json!({"channels": ["a", "b"]})),
            CliCommand::Daemon(DaemonCommand::Subscribe { .. })
        ));
        match err("daemon.subscribe", json!({"channels": []})) {
            CliParseError::MissingFlag(flag) => assert_eq!(flag, "channels"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn index_scope_accepts_all_known_values() {
        for scope in [
            "all",
            "issues",
            "specs",
            "memory",
            "discussions",
            "board",
            "files",
            "files-docs",
        ] {
            assert!(matches!(
                ok("index.rebuild", json!({"scope": scope})),
                CliCommand::Index(IndexCommand::Rebuild { .. })
            ));
        }
    }

    #[test]
    fn hook_register_codex_trust_collects_optional_flags() {
        assert!(matches!(
            ok("hook.register_codex_managed_hook_trust", json!({})),
            CliCommand::Hook(_)
        ));
        assert!(matches!(
            ok(
                "hook.register-codex-managed-hook-trust",
                json!({
                    "project_root": "/repo",
                    "codex_config": "/cfg",
                    "codex_hook_discovery": "auto",
                })
            ),
            CliCommand::Hook(_)
        ));
    }

    #[test]
    fn memory_add_parses() {
        assert!(matches!(
            ok(
                "memory.add",
                json!({
                    "title": "t",
                    "context": "c",
                    "learning": "l",
                    "future_action": "f",
                    "type": "decision",
                    "date": "2026-06-16",
                })
            ),
            CliCommand::Memory(_)
        ));
    }

    #[test]
    fn discussion_update_parses() {
        assert!(matches!(
            ok(
                "discussion.update",
                json!({
                    "title": "t",
                    "summary": "s",
                    "next": "n",
                    "topics": ["x"],
                    "related_specs": [1],
                    "decisions": ["d"],
                    "open_questions": ["q"],
                })
            ),
            CliCommand::Discussion(_)
        ));
    }

    #[test]
    fn discuss_proposal_actions_parse() {
        for op in [
            "discuss.resolve",
            "discuss.park",
            "discuss.reject",
            "discuss.clear_next_question",
            "discuss.clear-next-question",
            "discuss.goal_started",
            "discuss.goal-started",
        ] {
            assert!(matches!(
                ok(op, json!({"proposal": "p"})),
                CliCommand::Discuss(_)
            ));
        }
        assert!(matches!(
            ok(
                "discuss.goal_pending",
                json!({"proposal": "p", "condition": "c"})
            ),
            CliCommand::Discuss(_)
        ));
        assert!(matches!(
            ok(
                "discuss.goal_failed",
                json!({"proposal": "p", "reason": "r"})
            ),
            CliCommand::Discuss(_)
        ));
        assert!(matches!(
            ok(
                "discuss.goal_skipped",
                json!({"proposal": "p", "reason": "r"})
            ),
            CliCommand::Discuss(_)
        ));
    }

    #[test]
    fn skill_state_operations_parse() {
        for prefix in ["build", "plan", "register"] {
            assert!(parse(&envelope(&format!("{prefix}.start"), json!({"spec": 1}))).is_ok());
            assert!(parse(&envelope(
                &format!("{prefix}.phase"),
                json!({"spec": 1, "label": "red"})
            ))
            .is_ok());
            assert!(parse(&envelope(&format!("{prefix}.complete"), json!({"spec": 1}))).is_ok());
            assert!(parse(&envelope(
                &format!("{prefix}.abort"),
                json!({"spec": 1, "reason": "x"})
            ))
            .is_ok());
        }
    }

    #[test]
    fn skill_state_phase_requires_label() {
        match err("build.phase", json!({"spec": 1})) {
            CliParseError::MissingFlag(flag) => assert_eq!(flag, "label"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn build_start_targets_build_command() {
        assert!(matches!(
            ok("build.start", json!({"spec": 1})),
            CliCommand::Build(SkillStateAction::Start { spec: 1 })
        ));
    }

    #[test]
    fn pane_operations_parse() {
        assert!(matches!(
            ok("pane.list", json!({})),
            CliCommand::Pane(PaneCommand::List)
        ));
        match ok("pane.read", json!({"id": "p1"})) {
            CliCommand::Pane(PaneCommand::Read { id, lines }) => {
                assert_eq!(id, "p1");
                assert_eq!(lines, 200);
            }
            other => panic!("unexpected command: {other:?}"),
        }
        assert!(matches!(
            ok("pane.read", json!({"id": "p1", "lines": 42})),
            CliCommand::Pane(PaneCommand::Read { .. })
        ));
        assert!(matches!(
            ok("pane.close", json!({"id": "p1"})),
            CliCommand::Pane(PaneCommand::Close { .. })
        ));
        assert!(matches!(
            ok("pane.stop", json!({"id": "p1"})),
            CliCommand::Pane(PaneCommand::Close { .. })
        ));
        assert!(matches!(
            ok("pane.send", json!({"text": "hi"})),
            CliCommand::Pane(PaneCommand::Send { .. })
        ));
    }

    #[test]
    fn search_parses_scopes_and_match_modes() {
        match ok(
            "search",
            json!({
                "query": "needle",
                "scopes": ["specs", "issues", "files", "files_docs", "memory", "board", "discussions"],
                "match_mode": "all_terms",
                "n_results": 5,
            }),
        ) {
            CliCommand::Search(command) => {
                assert_eq!(command.query, "needle");
                assert_eq!(command.scopes.len(), 7);
                assert!(matches!(command.match_mode, IndexSearchMatchMode::AllTerms));
                assert_eq!(command.n_results, Some(5));
                assert!(command.json);
                assert!(matches!(command.scopes[0], IndexSearchScope::Specs));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn search_defaults_to_semantic_match_mode() {
        match ok("search", json!({"query": "q"})) {
            CliCommand::Search(command) => {
                assert!(matches!(command.match_mode, IndexSearchMatchMode::Semantic));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn search_rejects_unknown_scope_and_match_mode() {
        assert!(matches!(
            err("search", json!({"query": "q", "scopes": ["bogus"]})),
            CliParseError::InvalidJson(_)
        ));
        assert!(matches!(
            err("search", json!({"query": "q", "match_mode": "fuzzy"})),
            CliParseError::InvalidJson(_)
        ));
    }

    #[test]
    fn required_string_missing_is_missing_flag() {
        match err("issue.create", json!({"body": "b"})) {
            CliParseError::MissingFlag(flag) => assert_eq!(flag, "title"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn optional_u64_accepts_numeric_string_and_rejects_garbage() {
        assert!(matches!(
            ok("issue.view", json!({"number": "37"})),
            CliCommand::Issue(IssueCommand::View { .. })
        ));
        assert!(matches!(
            err("issue.view", json!({"number": "not-a-number"})),
            CliParseError::InvalidNumber(_)
        ));
        assert!(matches!(
            err("issue.view", json!({"number": true})),
            CliParseError::InvalidJson(_)
        ));
        match err("issue.view", json!({})) {
            CliParseError::MissingFlag(flag) => assert_eq!(flag, "number"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn typed_helpers_reject_wrong_types() {
        // optional_string wants a string
        assert!(matches!(
            err("board.show", json!({"workspace": 5})),
            CliParseError::InvalidJson(_)
        ));
        // optional_bool wants a bool
        assert!(matches!(
            err("board.show", json!({"all": "yes"})),
            CliParseError::InvalidJson(_)
        ));
        // optional_string_vec wants an array of non-empty strings
        assert!(matches!(
            err(
                "issue.create",
                json!({"title": "t", "body": "b", "labels": "x"})
            ),
            CliParseError::InvalidJson(_)
        ));
        assert!(matches!(
            err(
                "issue.create",
                json!({"title": "t", "body": "b", "labels": [""]})
            ),
            CliParseError::InvalidJson(_)
        ));
        assert!(matches!(
            err(
                "issue.create",
                json!({"title": "t", "body": "b", "labels": [5]})
            ),
            CliParseError::InvalidJson(_)
        ));
        // optional_u64_vec wants an array of u64
        assert!(matches!(
            err("issue.spec.pull", json!({"numbers": ["x"]})),
            CliParseError::InvalidNumber(_)
        ));
        assert!(matches!(
            err("issue.spec.pull", json!({"numbers": [true]})),
            CliParseError::InvalidJson(_)
        ));
        assert!(matches!(
            err("issue.spec.pull", json!({"numbers": 5})),
            CliParseError::InvalidJson(_)
        ));
    }

    #[test]
    fn optional_u64_vec_accepts_numeric_strings() {
        assert!(matches!(
            ok("issue.spec.pull", json!({"numbers": ["1", 2]})),
            CliCommand::Issue(IssueCommand::SpecPull { .. })
        ));
    }

    #[test]
    fn required_json_or_string_paths() {
        // missing key
        match err(
            "issue.spec.create",
            json!({"title": "t", "structured": true}),
        ) {
            CliParseError::MissingFlag(flag) => assert_eq!(flag, "body"),
            other => panic!("unexpected error: {other:?}"),
        }
        // blank string is treated as missing
        assert!(matches!(
            err(
                "issue.spec.create",
                json!({"title": "t", "body": "   ", "structured": true})
            ),
            CliParseError::MissingFlag("body")
        ));
        // wrong type
        assert!(matches!(
            err(
                "issue.spec.create",
                json!({"title": "t", "body": 5, "structured": true})
            ),
            CliParseError::InvalidJson(_)
        ));
        // string body is accepted verbatim
        assert!(matches!(
            ok(
                "issue.spec.create",
                json!({"title": "t", "body": "raw", "structured": true})
            ),
            CliCommand::Issue(IssueCommand::SpecCreateJsonBody { .. })
        ));
    }

    #[test]
    fn null_values_fall_back_to_defaults() {
        // Null optionals should behave as absent.
        let command = ok(
            "workspace.update",
            json!({"agent_session": "s", "summary": Value::Null, "status": Value::Null}),
        );
        assert!(matches!(
            command,
            CliCommand::Workspace(WorkspaceCommand::Update { .. })
        ));
        assert!(matches!(
            ok("index.rebuild", json!({"scope": Value::Null})),
            CliCommand::Index(IndexCommand::Rebuild {
                scope: IndexScope::All
            })
        ));
    }
}
