use gwt_agent::session::GWT_SESSION_ID_ENV;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::protocol::{IndexSearchMatchMode, IndexSearchScope};

use super::{
    memory::MemoryAddCommand, ActionsCommand, CliCommand, CliEnv, CliParseError, DaemonCommand,
    DiagnosticsCommand, IndexCommand, IndexScope, IssueCommand, MemoryCommand, PaneCommand,
    PrCommand, SearchCommand, SkillStateAction, WorkspaceCommand,
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
        super::validate_title_summary_work_name("--purpose", value)?;
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
        super::validate_title_summary_work_name("--purpose", value)?;
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
    super::validate_title_summary_work_name("--purpose", &purpose)?;
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
    super::validate_title_summary_work_name("--purpose", &purpose)?;
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
