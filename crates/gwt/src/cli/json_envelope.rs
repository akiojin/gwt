use gwt_agent::session::GWT_SESSION_ID_ENV;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::str::FromStr;

use crate::protocol::{IndexSearchMatchMode, IndexSearchScope};

use super::{
    memory::MemoryAddCommand, workflow::WorkflowBypassMode, ActionsCommand, CliCommand, CliEnv,
    CliParseError, DaemonCommand, DiagnosticsCommand, HookCommand, ImprovementCommand,
    IndexCommand, IndexScope, IssueCommand, MemoryCommand, PaneCommand, PrCommand, SearchCommand,
    SkillStateAction, WorkflowCommand, WorkspaceCommand,
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
    if envelope.operation.starts_with("improvement.") {
        reject_improvement_force(params)?;
    }
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
        "workspace.work_prune" | "workspace.work-prune" => {
            CliCommand::Workspace(WorkspaceCommand::WorkPrune {
                dry_run: optional_bool(params, "dry_run")?.unwrap_or(false),
                ids: optional_string_vec(params, "ids")?,
                project_root: optional_string(params, "project_root")?,
            })
        }
        "board.show" => board_show(params)?,
        "board.post" => board_post(params)?,
        "board.config.show" | "board.config-show" => {
            CliCommand::Board(crate::cli::board::BoardCommand::ConfigShow)
        }
        "improvement.capture" => improvement_capture(params)?,
        "improvement.list" => improvement_list(params)?,
        "intake.outcome.record" | "intake.outcome-record" => {
            CliCommand::Intake(crate::cli::intake_outcome::IntakeCommand::OutcomeRecord {
                kind: required_string(params, "kind")?,
                number: optional_u64(params, "number")?,
                reason: optional_string(params, "reason")?,
            })
        }
        "improvement.dismiss" => improvement_dismiss(params)?,
        "improvement.resolve" => improvement_resolve(params)?,
        "improvement.link_issue" | "improvement.link-issue" => improvement_link_issue(params)?,
        "improvement.promote_issue" | "improvement.promote-issue" => {
            improvement_promote_issue(params)?
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
        "issue.monitor.review_verdict" | "issue.monitor.review-verdict" => {
            CliCommand::Issue(IssueCommand::MonitorReviewVerdict {
                issue_number: required_u64(params, "issue_number")?,
                reviewed_sha: required_string(params, "reviewed_sha")?,
                verdict_raw: required_string(params, "verdict_raw")?,
            })
        }
        "issue.monitor.status" => CliCommand::Issue(IssueCommand::MonitorStatus {
            project_root: optional_path(params, "project_root")?,
        }),
        "issue.monitor.questions" => CliCommand::Issue(IssueCommand::MonitorQuestions {
            project_root: optional_path(params, "project_root")?,
        }),
        "issue.monitor.question.answer" | "issue.monitor.question-answer" => {
            CliCommand::Issue(IssueCommand::MonitorQuestionAnswer {
                project_root: optional_path(params, "project_root")?,
                handoff_id: required_string(params, "handoff_id")?,
                answer: required_string(params, "answer")?,
            })
        }
        "issue.monitor.priority.move" | "issue.monitor.priority-move" => {
            CliCommand::Issue(IssueCommand::MonitorPriorityMove {
                project_root: optional_path(params, "project_root")?,
                number: required_u64(params, "number")?,
                position: issue_monitor_priority_position(params)?,
            })
        }
        "issue.monitor.launch_now" | "issue.monitor.launch-now" => {
            CliCommand::Issue(IssueCommand::MonitorLaunchNow {
                project_root: optional_path(params, "project_root")?,
                number: required_u64(params, "number")?,
            })
        }
        "issue.monitor.stop" => {
            reject_unknown_params(
                params,
                ISSUE_MONITOR_CONTROL_TARGET_PARAMS,
                "issue.monitor.stop",
            )?;
            CliCommand::Issue(IssueCommand::MonitorStop {
                project_root: optional_path(params, "project_root")?,
                number: required_u64(params, "number")?,
                operation_id: optional_control_operation_id(params)?,
                reason: required_issue_monitor_control_reason(params)?,
                launch_generation: optional_u64(params, "launch_generation")?,
                claim_id: optional_string(params, "claim_id")?,
                claim_owner: optional_string(params, "claim_owner")?,
                delivery_id: optional_string(params, "delivery_id")?,
                materializer_window_id: optional_string(params, "materializer_window_id")?,
                window_id: optional_string(params, "window_id")?,
                launch_profile_fingerprint: optional_string(params, "launch_profile_fingerprint")?,
            })
        }
        "issue.monitor.failover" => {
            reject_unknown_params(
                params,
                ISSUE_MONITOR_CONTROL_TARGET_PARAMS,
                "issue.monitor.failover",
            )?;
            CliCommand::Issue(IssueCommand::MonitorFailover {
                project_root: optional_path(params, "project_root")?,
                number: required_u64(params, "number")?,
                operation_id: optional_control_operation_id(params)?,
                reason: required_issue_monitor_control_reason(params)?,
                launch_generation: optional_u64(params, "launch_generation")?,
                claim_id: optional_string(params, "claim_id")?,
                claim_owner: optional_string(params, "claim_owner")?,
                delivery_id: optional_string(params, "delivery_id")?,
                materializer_window_id: optional_string(params, "materializer_window_id")?,
                window_id: optional_string(params, "window_id")?,
                launch_profile_fingerprint: optional_string(params, "launch_profile_fingerprint")?,
            })
        }
        "issue.monitor.recover" => {
            reject_unknown_params(
                params,
                ISSUE_MONITOR_CONTROL_TARGET_PARAMS,
                "issue.monitor.recover",
            )?;
            CliCommand::Issue(IssueCommand::MonitorRecover {
                project_root: optional_path(params, "project_root")?,
                number: required_u64(params, "number")?,
                operation_id: required_control_operation_id(params)?,
                reason: required_issue_monitor_control_reason(params)?,
                launch_generation: required_u64(params, "launch_generation")?,
                claim_id: optional_string(params, "claim_id")?,
                claim_owner: optional_string(params, "claim_owner")?,
                delivery_id: optional_string(params, "delivery_id")?,
                materializer_window_id: optional_string(params, "materializer_window_id")?,
                window_id: optional_string(params, "window_id")?,
                launch_profile_fingerprint: optional_string(params, "launch_profile_fingerprint")?,
            })
        }
        "issue.monitor.control.reconcile" => {
            const ALLOWED: &[&str] = &["project_root", "operation_id", "revoked_generation"];
            reject_unknown_params(params, ALLOWED, "issue.monitor.control.reconcile")?;
            CliCommand::Issue(IssueCommand::MonitorControlReconcile {
                project_root: optional_path(params, "project_root")?,
                operation_id: required_control_operation_id(params)?,
                revoked_generation: required_u64(params, "revoked_generation")?,
            })
        }
        "issue.monitor.priority.set" | "issue.monitor.priority-set" => {
            CliCommand::Issue(IssueCommand::MonitorPrioritySet {
                project_root: optional_path(params, "project_root")?,
                issue_numbers: required_u64_vec(params, "issue_numbers")?,
            })
        }
        "issue.monitor.config.set" | "issue.monitor.config-set" => {
            let enabled = optional_bool(params, "enabled")?;
            let autonomous_mode = optional_bool(params, "autonomous_mode")?;
            let max_active = optional_usize(params, "max_active")?;
            if enabled.is_none() && autonomous_mode.is_none() && max_active.is_none() {
                return Err(CliParseError::MissingFlag(
                    "enabled|autonomous_mode|max_active",
                ));
            }
            // SPEC-3431 FR-008/FR-009: Issue #3357's asymmetric boundary keeps
            // applying to every agent session — raising a switch stays a GUI
            // action — except for the project's registered PM, which the SPEC
            // grants full authority. Merges are unaffected either way: SPEC
            // #3200's fail-closed merge gate still decides every merge.
            let pm_privileged = params_caller_is_registered_pm(params);
            if !pm_privileged {
                if enabled == Some(true) {
                    return Err(CliParseError::InvalidJson(
                        "enabled=true requires an explicit GUI action".to_string(),
                    ));
                }
                if autonomous_mode == Some(true) {
                    return Err(CliParseError::InvalidJson(
                        "autonomous_mode=true requires an explicit GUI action".to_string(),
                    ));
                }
            }
            if max_active == Some(0) {
                return Err(CliParseError::InvalidJson(
                    "max_active must be greater than zero".to_string(),
                ));
            }
            CliCommand::Issue(IssueCommand::MonitorConfigSet {
                project_root: optional_path(params, "project_root")?,
                enabled,
                autonomous_mode,
                max_active,
            })
        }
        "pr.current" => CliCommand::Pr(PrCommand::Current),
        "pr.create" => CliCommand::Pr(PrCommand::CreateBody {
            base: required_string(params, "base")?,
            head: optional_string(params, "head")?,
            title: required_string(params, "title")?,
            body: required_string(params, "body")?,
            labels: optional_string_vec(params, "labels")?,
            draft: optional_bool(params, "draft")?.unwrap_or(false),
        }),
        "pr.edit" => {
            let number = required_u64(params, "number")?;
            let title = optional_string(params, "title")?;
            let body = optional_string(params, "body")?;
            let add_labels = optional_string_vec(params, "add_labels")?;
            // Reject nothing-to-update like the argv path's Usage guard; a
            // silent no-op success would mask caller bugs (e.g. sending
            // pr.create's "labels" key instead of "add_labels").
            if title.is_none() && body.is_none() && add_labels.is_empty() {
                return Err(CliParseError::MissingFlag("title|body|add_labels"));
            }
            CliCommand::Pr(PrCommand::EditBody {
                number,
                title,
                body,
                add_labels,
            })
        }
        "pr.view" => CliCommand::Pr(PrCommand::View {
            number: required_u64(params, "number")?,
        }),
        "pr.ready" => CliCommand::Pr(PrCommand::Ready {
            number: required_u64(params, "number")?,
        }),
        "pr.draft" => CliCommand::Pr(PrCommand::Draft {
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
        "verify.run" => {
            let commands = optional_string_vec(params, "commands")?;
            CliCommand::Verify(crate::cli::verification_record::VerifyCommand::Run { commands })
        }
        "verify.plan" => {
            let commands = optional_string_vec(params, "commands")?;
            let derive = optional_bool(params, "derive")?.unwrap_or(false);
            let generated_outputs = optional_string_vec(params, "generated_outputs")?;
            if commands.is_empty() && !derive {
                return Err(CliParseError::MissingFlag("commands"));
            }
            if generated_outputs.is_empty() {
                CliCommand::Verify(crate::cli::verification_record::VerifyCommand::Plan {
                    commands,
                    derive,
                })
            } else {
                CliCommand::Verify(
                    crate::cli::verification_record::VerifyCommand::PlanWithOutputs {
                        commands,
                        derive,
                        generated_outputs,
                    },
                )
            }
        }
        "execution.status" => {
            if !params.is_empty() {
                return Err(CliParseError::InvalidJson(
                    "execution.status accepts no params".to_string(),
                ));
            }
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::Status)
        }
        "execution.complete" => {
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::Complete)
        }
        "execution.blocked" => {
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::Blocked {
                reason: required_string(params, "reason")?,
                missing_verification: optional_string(params, "missing_verification")?,
            })
        }
        "execution.adopt" => {
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::Adopt {
                reason: required_string(params, "reason")?,
            })
        }
        "execution.repair" => {
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::Repair {
                reason: required_string(params, "reason")?,
            })
        }
        "execution.continue" => {
            let operation_id = required_string(params, "operation_id")?;
            if params.len() != 1 || !params.contains_key("operation_id") {
                return Err(CliParseError::InvalidJson(
                    "execution.continue only accepts params.operation_id".to_string(),
                ));
            }
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::Continue {
                operation_id,
            })
        }
        "execution.reopen" => {
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::Reopen {
                reason: required_string(params, "reason")?,
            })
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
        "pm.message.send" | "pm.pane.send" => CliCommand::Pane(PaneCommand::PmSend {
            project_root: optional_string(params, "project_root")?,
            id: required_string(params, "id")?,
            text: required_string(params, "text")?,
        }),
        "pm.status" => CliCommand::Pm(crate::cli::pm::PmCommand::Status {
            project_root: optional_string(params, "project_root")?,
        }),
        "workflow.bypass" => CliCommand::Workflow(WorkflowCommand::Bypass {
            mode: WorkflowBypassMode::parse(&required_string(params, "mode")?).ok_or(
                CliParseError::InvalidValue {
                    flag: "mode",
                    reason: "expected one of: release, chore, off",
                },
            )?,
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
    let agent_session = Some(workspace_update_session(params)?);
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

fn improvement_capture(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    for key in [
        "producer_id",
        "source_event_id",
        "session_id",
        "routing_basis_revision",
        "budget_profile",
        "fingerprint",
        "occurrence_key",
    ] {
        reject_key(
            params,
            key,
            "untrusted identity field is not accepted by public improvement.capture",
        )?;
    }
    let typed_evidence = improvement_typed_evidence(params)?;
    Ok(CliCommand::Improvement(ImprovementCommand::Capture(
        Box::new(super::improvement::ImprovementCaptureCommand {
            source: required_string(params, "source")?,
            target_artifact: required_string(params, "target_artifact")?,
            classification: required_string(params, "classification")?,
            confidence: required_string(params, "confidence")?,
            summary: optional_string(params, "summary")?.unwrap_or_default(),
            details: optional_string(params, "details")?,
            evidence_digest: optional_string(params, "evidence_digest")?,
            dedupe_key: optional_string(params, "dedupe_key")?,
            local_evidence: optional_json_array(params, "local_evidence")?,
            typed_evidence,
        }),
    )))
}

fn improvement_typed_evidence(
    params: &Map<String, Value>,
) -> Result<Option<super::improvement::ImprovementTypedEvidenceCommand>, CliParseError> {
    let typed_keys = [
        "subsystem",
        "contract_id",
        "contract_schema_revision",
        "failure_code",
        "evidence",
    ];
    if !typed_keys.iter().any(|key| params.contains_key(*key)) {
        return Ok(None);
    }

    let evidence = params
        .get("evidence")
        .and_then(Value::as_object)
        .ok_or_else(|| CliParseError::InvalidJson("evidence must be an object".to_string()))?;
    for key in evidence.keys() {
        if !matches!(key.as_str(), "expected_outcome" | "observed_outcome") {
            return Err(CliParseError::InvalidJson(format!(
                "evidence contains unknown field: {key}"
            )));
        }
    }

    Ok(Some(super::improvement::ImprovementTypedEvidenceCommand {
        subsystem: required_string(params, "subsystem")?,
        contract_id: required_string(params, "contract_id")?,
        contract_schema_revision: required_u64(params, "contract_schema_revision")?,
        failure_code: required_string(params, "failure_code")?,
        expected_outcome: required_string(evidence, "expected_outcome")?,
        observed_outcome: required_string(evidence, "observed_outcome")?,
    }))
}

fn improvement_list(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    let state = optional_string(params, "state")?
        .map(|value| {
            super::improvement::CandidateState::from_str(&value)
                .map_err(|message| CliParseError::InvalidJson(message.to_string()))
        })
        .transpose()?;
    let blocked_reason = optional_string(params, "blocked_reason")?
        .map(|value| {
            super::improvement::BlockedReason::from_str(&value)
                .map_err(|message| CliParseError::InvalidJson(message.to_string()))
        })
        .transpose()?;
    let failure_subcode = optional_string(params, "failure_subcode")?
        .map(|value| {
            super::improvement::FailureSubcode::from_str(&value)
                .map_err(|message| CliParseError::InvalidJson(message.to_string()))
        })
        .transpose()?;
    let classification = optional_string(params, "classification")?;
    if classification.as_deref().is_some_and(|value| {
        !["gwt-caused", "ambiguous", "target-project", "external"].contains(&value)
    }) {
        return Err(CliParseError::InvalidJson(
            "invalid classification".to_string(),
        ));
    }
    let confidence = optional_string(params, "confidence")?;
    if confidence
        .as_deref()
        .is_some_and(|value| !["low", "medium", "high"].contains(&value))
    {
        return Err(CliParseError::InvalidJson("invalid confidence".to_string()));
    }
    let owner_number = optional_u64(params, "owner_number")?;
    if owner_number == Some(0) {
        return Err(CliParseError::InvalidJson(
            "owner_number must be greater than zero".to_string(),
        ));
    }
    let limit = optional_usize(params, "limit")?;
    if limit == Some(0) {
        return Err(CliParseError::InvalidJson(
            "limit must be greater than zero".to_string(),
        ));
    }
    Ok(CliCommand::Improvement(ImprovementCommand::List(
        super::improvement::ImprovementListCommand {
            state,
            blocked_reason,
            failure_subcode,
            classification,
            confidence,
            owner_number,
            limit,
        },
    )))
}

fn improvement_dismiss(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    Ok(CliCommand::Improvement(ImprovementCommand::Dismiss(
        super::improvement::ImprovementDismissCommand {
            id: required_string(params, "id")?,
            reason: required_string(params, "reason")?,
        },
    )))
}

fn improvement_link_issue(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    if ["number", "url", "repository"]
        .iter()
        .any(|key| params.contains_key(*key))
    {
        return Err(CliParseError::InvalidJson(
            "MIGRATION_REQUIRED: improvement.link_issue v2 accepts id, owner_number, and resolver_revision only"
                .to_string(),
        ));
    }
    Ok(CliCommand::Improvement(ImprovementCommand::LinkIssue(
        super::improvement::ImprovementLinkIssueCommand {
            id: required_string(params, "id")?,
            owner_number: required_u64(params, "owner_number")?,
            resolver_revision: required_string(params, "resolver_revision")?,
        },
    )))
}

fn improvement_resolve(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    Ok(CliCommand::Improvement(ImprovementCommand::Resolve(
        super::improvement::ImprovementResolveCommand {
            id: required_string(params, "id")?,
            expected_resolver_revision: optional_string(params, "expected_resolver_revision")?,
        },
    )))
}

fn improvement_promote_issue(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    Ok(CliCommand::Improvement(ImprovementCommand::PromoteIssue(
        super::improvement::ImprovementPromoteIssueCommand {
            id: required_string(params, "id")?,
            force: optional_bool(params, "force")?.unwrap_or(false),
            labels: optional_string_vec(params, "labels")?,
        },
    )))
}

fn reject_improvement_force(params: &Map<String, Value>) -> Result<(), CliParseError> {
    if optional_bool(params, "force")? == Some(true) {
        return Err(CliParseError::InvalidJson(
            "UNSAFE_FORCE_REMOVED".to_string(),
        ));
    }
    Ok(())
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
    let timeout_seconds = optional_u64(params, "timeout_seconds")?;
    if timeout_seconds == Some(0) {
        return Err(CliParseError::InvalidValue {
            flag: "timeout_seconds",
            reason: "must be at least 1 second",
        });
    }
    Ok(CliCommand::Daemon(DaemonCommand::Subscribe {
        channels,
        timeout_seconds,
    }))
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

fn workspace_update_session(params: &Map<String, Value>) -> Result<String, CliParseError> {
    let ambient = std::env::var(GWT_SESSION_ID_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(CliParseError::MissingFlag("GWT_SESSION_ID"))?;
    gwt_agent::validate_session_id_path_component(&ambient).map_err(|_| {
        CliParseError::InvalidValue {
            flag: "GWT_SESSION_ID",
            reason: "must be a safe Session id path component",
        }
    })?;
    if let Some(explicit) = optional_string(params, "agent_session")? {
        if explicit != ambient {
            return Err(CliParseError::InvalidValue {
                flag: "agent_session",
                reason: "must exactly match ambient GWT_SESSION_ID",
            });
        }
    }
    Ok(ambient)
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

const ISSUE_MONITOR_CONTROL_TARGET_PARAMS: &[&str] = &[
    "project_root",
    "number",
    "operation_id",
    "reason",
    "launch_generation",
    "claim_id",
    "claim_owner",
    "delivery_id",
    "materializer_window_id",
    "window_id",
    "launch_profile_fingerprint",
];

fn optional_control_operation_id(
    params: &Map<String, Value>,
) -> Result<Option<String>, CliParseError> {
    control_operation_id(params, false)
}

fn required_control_operation_id(params: &Map<String, Value>) -> Result<String, CliParseError> {
    control_operation_id(params, true)?.ok_or(CliParseError::MissingFlag("operation_id"))
}

fn control_operation_id(
    params: &Map<String, Value>,
    required: bool,
) -> Result<Option<String>, CliParseError> {
    let Some(value) = params.get("operation_id") else {
        return if required {
            Err(CliParseError::MissingFlag("operation_id"))
        } else {
            Ok(None)
        };
    };
    let Value::String(value) = value else {
        return Err(CliParseError::InvalidJson(
            "operation_id must be an ASCII printable token of 1..=128 bytes".to_string(),
        ));
    };
    let value = value.trim();
    if value.is_empty()
        || value.len() > 128
        || !value.is_ascii()
        || !value.as_bytes().iter().all(u8::is_ascii_graphic)
    {
        return Err(CliParseError::InvalidJson(
            "operation_id must be an ASCII printable token of 1..=128 bytes".to_string(),
        ));
    }
    Ok(Some(value.to_string()))
}

fn required_issue_monitor_control_reason(
    params: &Map<String, Value>,
) -> Result<String, CliParseError> {
    let Some(value) = params.get("reason") else {
        return Err(CliParseError::MissingFlag("reason"));
    };
    let Value::String(value) = value else {
        return Err(CliParseError::InvalidJson(
            "reason must be a UTF-8 string of at most 1024 bytes without control characters"
                .to_string(),
        ));
    };
    if value.chars().any(char::is_control) {
        return Err(CliParseError::InvalidJson(
            "reason must be a UTF-8 string of at most 1024 bytes without control characters"
                .to_string(),
        ));
    }
    let value = value.trim();
    if value.is_empty() {
        return Err(CliParseError::MissingFlag("reason"));
    }
    if value.len() > 1024 {
        return Err(CliParseError::InvalidJson(
            "reason must be a UTF-8 string of at most 1024 bytes without control characters"
                .to_string(),
        ));
    }
    Ok(value.to_string())
}

fn reject_unknown_params(
    params: &Map<String, Value>,
    allowed: &[&str],
    operation: &str,
) -> Result<(), CliParseError> {
    for key in params.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(CliParseError::InvalidJson(format!(
                "{operation} does not accept the parameter {key}; accepted: {}",
                allowed.join(", ")
            )));
        }
    }
    Ok(())
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

fn required_u64_vec(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<Vec<u64>, CliParseError> {
    if !params.contains_key(key) {
        return Err(CliParseError::MissingFlag(key));
    }
    if !params.get(key).is_some_and(Value::is_array) {
        return Err(CliParseError::InvalidJson(format!(
            "{key} must be an array of u64 values"
        )));
    }
    optional_u64_vec(params, key)
}

/// SPEC-3431 FR-009: is this caller the project's registered PM?
///
/// The identity comes from the ambient `GWT_SESSION_ID` only — params may
/// name the project, never the subject — so no caller can claim PM authority
/// it does not hold. The project is the explicit `project_root` when given,
/// otherwise the current directory, matching how the handler resolves it.
/// Anything unresolvable is not privileged (fail-closed).
fn params_caller_is_registered_pm(params: &Map<String, Value>) -> bool {
    let Ok(session_id) = std::env::var(gwt_agent::GWT_SESSION_ID_ENV) else {
        return false;
    };
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return false;
    }
    let project_root = match params.get("project_root").and_then(Value::as_str) {
        Some(path) => std::path::PathBuf::from(path),
        None => match std::env::current_dir() {
            Ok(cwd) => cwd,
            Err(_) => return false,
        },
    };
    crate::pm_registry::session_is_registered_pm(
        &crate::pm_registry::pm_prefs_path_for_repo_path(&project_root),
        &session_id,
    )
}

fn issue_monitor_priority_position(
    params: &Map<String, Value>,
) -> Result<super::IssueMonitorPriorityPosition, CliParseError> {
    let Some(value) = params.get("position") else {
        return Ok(super::IssueMonitorPriorityPosition::Head);
    };
    match value {
        Value::String(value) if value == "head" => Ok(super::IssueMonitorPriorityPosition::Head),
        Value::Number(value) => value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .map(super::IssueMonitorPriorityPosition::Index)
            .ok_or_else(|| {
                CliParseError::InvalidJson(
                    "position must be \"head\" or a non-negative numeric index".to_string(),
                )
            }),
        _ => Err(CliParseError::InvalidJson(
            "position must be \"head\" or a non-negative numeric index".to_string(),
        )),
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

fn optional_json_array(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<Vec<Value>, CliParseError> {
    let Some(value) = params.get(key) else {
        return Ok(Vec::new());
    };
    match value {
        Value::Array(items) => Ok(items.clone()),
        Value::Null => Ok(Vec::new()),
        _ => Err(CliParseError::InvalidJson(format!(
            "{key} must be an array"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse, ActionsCommand, CliCommand, CliParseError, DaemonCommand, HookCommand, IndexCommand,
        IndexScope, IssueCommand, PaneCommand, PrCommand, SkillStateAction, WorkflowBypassMode,
        WorkflowCommand, WorkspaceCommand,
    };
    use crate::cli::IssueMonitorPriorityPosition;
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
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ambient = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::session::GWT_SESSION_ID_ENV,
            "session-1",
        );
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
    fn workspace_update_requires_ambient_session_even_with_explicit_session() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ambient =
            gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::session::GWT_SESSION_ID_ENV);

        match err(
            "workspace.update",
            json!({
                "agent_session": "explicit-session",
                "summary": "session-bound mutation",
            }),
        ) {
            CliParseError::MissingFlag(flag) => assert_eq!(flag, "GWT_SESSION_ID"),
            other => panic!("expected missing ambient Session error, got {other:?}"),
        }
    }

    #[test]
    fn workspace_update_rejects_explicit_session_mismatch_with_ambient_session() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ambient = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::session::GWT_SESSION_ID_ENV,
            "ambient-session",
        );

        match err(
            "workspace.update",
            json!({
                "agent_session": "foreign-explicit-session",
                "summary": "session-bound mutation",
            }),
        ) {
            CliParseError::InvalidValue { flag, reason } => {
                assert_eq!(flag, "agent_session");
                assert!(reason.contains("GWT_SESSION_ID"), "{reason}");
                assert!(reason.contains("match"), "{reason}");
            }
            other => panic!("expected explicit/ambient Session mismatch, got {other:?}"),
        }
    }

    #[test]
    fn workspace_update_rejects_unsafe_ambient_session_before_dispatch() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ambient = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::session::GWT_SESSION_ID_ENV,
            "../escaped-session",
        );

        match err(
            "workspace.update",
            json!({
                "summary": "unsafe Session must not reach proxy or ledger lookup",
            }),
        ) {
            CliParseError::InvalidValue { flag, reason } => {
                assert_eq!(flag, "GWT_SESSION_ID");
                assert!(
                    reason.contains("unsafe") || reason.contains("path component"),
                    "{reason}"
                );
            }
            other => panic!("expected unsafe ambient Session error, got {other:?}"),
        }
    }

    #[test]
    fn workspace_update_accepts_matching_explicit_and_ambient_session() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ambient = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::session::GWT_SESSION_ID_ENV,
            "ambient-session",
        );

        let command = ok(
            "workspace.update",
            json!({
                "agent_session": "ambient-session",
                "summary": "matching explicit Session",
            }),
        );
        match command {
            CliCommand::Workspace(WorkspaceCommand::Update { agent_session, .. }) => {
                assert_eq!(agent_session.as_deref(), Some("ambient-session"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn workspace_update_rejects_provider_actor_without_ambient_session() {
        const RAW_PROVIDER_ACTOR_ID: &str = "provider-thread-private-sentinel-86";

        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ambient =
            gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::session::GWT_SESSION_ID_ENV);
        let _provider_actor =
            gwt_core::test_support::ScopedEnvVar::set("CODEX_THREAD_ID", RAW_PROVIDER_ACTOR_ID);

        match err(
            "workspace.update",
            json!({
                "summary": "provider actor must not authorize mutation",
            }),
        ) {
            CliParseError::MissingFlag(flag) => assert_eq!(flag, "GWT_SESSION_ID"),
            other => panic!("expected missing ambient Session error, got {other:?}"),
        }
    }

    #[test]
    fn workspace_update_uses_ambient_session_independent_of_provider_actor_id() {
        const RAW_PROVIDER_ACTOR_ID: &str = "provider-thread-private-sentinel-86";

        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ambient = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::session::GWT_SESSION_ID_ENV,
            "ambient-session",
        );

        for provider_actor_id in [None, Some(RAW_PROVIDER_ACTOR_ID)] {
            let _provider_actor = match provider_actor_id {
                Some(value) => gwt_core::test_support::ScopedEnvVar::set("CODEX_THREAD_ID", value),
                None => gwt_core::test_support::ScopedEnvVar::unset("CODEX_THREAD_ID"),
            };
            let command = ok(
                "workspace.update",
                json!({
                    "summary": "provider-neutral mutation principal",
                }),
            );

            match command {
                CliCommand::Workspace(WorkspaceCommand::Update { agent_session, .. }) => {
                    assert_eq!(agent_session.as_deref(), Some("ambient-session"));
                }
                other => panic!("unexpected command: {other:?}"),
            }
        }
    }

    /// Issue #3184: `workspace.update params.purpose` is the write path behind
    /// the Agent titlebar; transient helper-workflow activity labels must be
    /// rejected so browser-check-style phases cannot replace a work purpose.
    #[test]
    fn workspace_update_rejects_transient_activity_purpose() {
        for label in [
            "browser check",
            "browser-check",
            "Headless browser check",
            "browser check for issue 3184",
            "ｂｒｏｗｓｅｒ ｃｈｅｃｋ",
            "verification",
            "merging",
            "server startup",
            "ブラウザ確認",
            "ヘッドレスブラウザ確認",
            "検証",
        ] {
            match err(
                "workspace.update",
                json!({"agent_session": "s", "purpose": label}),
            ) {
                CliParseError::InvalidValue { flag, reason } => {
                    assert_eq!(flag, "params.purpose", "{label}");
                    assert!(reason.contains("transient activity"), "{label}: {reason}");
                    assert!(reason.contains("current_focus"), "{label}: {reason}");
                }
                other => panic!("unexpected error for {label}: {other:?}"),
            }
        }
    }

    /// Issue #3184: real work names that mention an activity domain without
    /// being a bare activity label stay valid purposes.
    #[test]
    fn workspace_update_accepts_work_purpose_mentioning_activity_domain() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ambient =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::session::GWT_SESSION_ID_ENV, "s");
        for label in [
            "browser-check purpose overwrite guard",
            "Fix browser check",
            "Issue #3184 title guard",
            "release verification pipeline",
        ] {
            let command = ok(
                "workspace.update",
                json!({"agent_session": "s", "purpose": label}),
            );
            match command {
                CliCommand::Workspace(WorkspaceCommand::Update { title_summary, .. }) => {
                    assert_eq!(title_summary.as_deref(), Some(label));
                }
                other => panic!("unexpected command for {label}: {other:?}"),
            }
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
    fn workspace_update_requires_ambient_session_when_setting_purpose() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ambient =
            gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::session::GWT_SESSION_ID_ENV);
        match err("workspace.update", json!({"current_focus": "x"})) {
            CliParseError::MissingFlag(flag) => assert_eq!(flag, "GWT_SESSION_ID"),
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
    fn issue_monitor_review_verdict_parses() {
        // SPEC #3200 Option A: the review agent's verdict-report op.
        let cmd = ok(
            "issue.monitor.review_verdict",
            json!({
                "issue_number": 42,
                "reviewed_sha": "abc123",
                "verdict_raw": "{\"schema\":\"gwt-autonomous-review/v1\"}",
            }),
        );
        match cmd {
            CliCommand::Issue(IssueCommand::MonitorReviewVerdict {
                issue_number,
                reviewed_sha,
                verdict_raw,
            }) => {
                assert_eq!(issue_number, 42);
                assert_eq!(reviewed_sha, "abc123");
                assert!(verdict_raw.contains("gwt-autonomous-review"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
        // Missing the reviewed SHA is rejected.
        assert!(matches!(
            err("issue.monitor.review_verdict", json!({"issue_number": 42})),
            CliParseError::MissingFlag(_)
        ));
    }

    // SPEC-3431 T-030 (FR-008/FR-009): the #3357 asymmetric boundary keeps
    // applying to every agent session; only the project's registered PM may
    // raise the switches. The privileged subject comes from the ambient
    // session id, never from params, so a caller cannot claim it.
    #[test]
    fn issue_monitor_config_set_on_direction_is_pm_only() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        std::fs::create_dir_all(&project_root).expect("repo dir");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", temp.path());

        let prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&project_root);
        crate::pm_registry::try_register_pm(
            &prefs_path,
            crate::pm_registry::PmRegistration {
                session_id: "pm-session".to_string(),
                agent_id: "claude".to_string(),
                worktree_path: project_root.to_string_lossy().into_owned(),
                created_at: None,
                consecutive_crashes: 0,
                next_not_before: None,
            },
            |_| false,
        )
        .expect("register PM");

        let params = json!({
            "project_root": project_root.to_string_lossy(),
            "autonomous_mode": true,
        });

        // positive: a non-PM session is still refused, with the GUI guidance.
        let _other = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_ID_ENV,
            "other-session",
        );
        assert!(matches!(
            err("issue.monitor.config.set", params.clone()),
            CliParseError::InvalidJson(message) if message.contains("requires an explicit GUI action")
        ));
        drop(_other);

        // ...and so is a session with no ambient identity at all.
        let _unset = gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        assert!(matches!(
            err("issue.monitor.config.set", params.clone()),
            CliParseError::InvalidJson(_)
        ));
        drop(_unset);

        // false-positive negative: the registered PM is allowed through.
        let _pm =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        assert_eq!(
            ok("issue.monitor.config.set", params),
            CliCommand::Issue(IssueCommand::MonitorConfigSet {
                project_root: Some(project_root.clone()),
                enabled: None,
                autonomous_mode: Some(true),
                max_active: None,
            })
        );

        // OFF direction stays open to everyone, PM or not.
        assert!(matches!(
            ok(
                "issue.monitor.config.set",
                json!({"project_root": project_root.to_string_lossy(), "autonomous_mode": false})
            ),
            CliCommand::Issue(IssueCommand::MonitorConfigSet { .. })
        ));
    }

    // SPEC-3431 T-020 (FR-006): launch_now is the PM's launch instruction —
    // priority head-move plus an immediate scan. It never spawns anything
    // itself; the Monitor's claim/slot path stays the only launcher.
    #[test]
    fn issue_monitor_launch_now_parses() {
        assert_eq!(
            ok("issue.monitor.launch_now", json!({"number": 42})),
            CliCommand::Issue(IssueCommand::MonitorLaunchNow {
                project_root: None,
                number: 42,
            })
        );
        assert_eq!(
            ok(
                "issue.monitor.launch-now",
                json!({"project_root": "/tmp/project", "number": 7})
            ),
            CliCommand::Issue(IssueCommand::MonitorLaunchNow {
                project_root: Some(std::path::PathBuf::from("/tmp/project")),
                number: 7,
            })
        );
        assert!(matches!(
            err("issue.monitor.launch_now", json!({})),
            CliParseError::MissingFlag("number")
        ));
    }

    /// SPEC-3431 FR-033 / T-087b: the PM's stop instruction.
    ///
    /// The identity components are optional in the wire format because a
    /// materializing launch has no window and a launched one has no delivery.
    /// Which of them must be present is decided against the live state, not by
    /// the parser — the parser cannot know, and guessing here would either
    /// reject valid stops or let an under-specified one through.
    #[test]
    fn issue_monitor_stop_parses() {
        assert_eq!(
            ok(
                "issue.monitor.stop",
                json!({
                    "number": 42,
                    "reason": "provider rate limit",
                    "window_id": "tab-1::agent-1",
                })
            ),
            CliCommand::Issue(IssueCommand::MonitorStop {
                project_root: None,
                number: 42,
                operation_id: None,
                reason: "provider rate limit".to_string(),
                launch_generation: None,
                claim_id: None,
                claim_owner: None,
                delivery_id: None,
                materializer_window_id: None,
                window_id: Some("tab-1::agent-1".to_string()),
                launch_profile_fingerprint: None,
            })
        );
        assert_eq!(
            ok(
                "issue.monitor.stop",
                json!({
                    "project_root": "/tmp/project",
                    "number": 7,
                    "reason": "switch provider",
                    "claim_id": "claim-1",
                    "delivery_id": "launch:effect-1",
                })
            ),
            CliCommand::Issue(IssueCommand::MonitorStop {
                project_root: Some(std::path::PathBuf::from("/tmp/project")),
                number: 7,
                operation_id: None,
                reason: "switch provider".to_string(),
                launch_generation: None,
                claim_id: Some("claim-1".to_string()),
                claim_owner: None,
                delivery_id: Some("launch:effect-1".to_string()),
                materializer_window_id: None,
                window_id: None,
                launch_profile_fingerprint: None,
            })
        );
        assert!(matches!(
            err("issue.monitor.stop", json!({"reason": "x"})),
            CliParseError::MissingFlag("number")
        ));
        // FR-031: an unexplained stop is not auditable, so the reason is not
        // optional even though every identity component is.
        assert!(matches!(
            err("issue.monitor.stop", json!({"number": 42})),
            CliParseError::MissingFlag("reason")
        ));
        assert!(matches!(
            err("issue.monitor.stop", json!({"number": 42, "reason": "  "})),
            CliParseError::MissingFlag("reason")
        ));
    }

    /// SPEC-3431 FR-029〜031 / T-081: the failover takes the same request shape
    /// as the stop, because it enforces the same identity — only the outcome
    /// differs.
    #[test]
    fn issue_monitor_failover_parses() {
        assert_eq!(
            ok(
                "issue.monitor.failover",
                json!({
                    "number": 3476,
                    "reason": "codex rate limit",
                    "claim_id": "claim-1",
                    "window_id": "tab-1::agent-1",
                })
            ),
            CliCommand::Issue(IssueCommand::MonitorFailover {
                project_root: None,
                number: 3476,
                operation_id: None,
                reason: "codex rate limit".to_string(),
                launch_generation: None,
                claim_id: Some("claim-1".to_string()),
                claim_owner: None,
                delivery_id: None,
                materializer_window_id: None,
                window_id: Some("tab-1::agent-1".to_string()),
                launch_profile_fingerprint: None,
            })
        );
        assert!(matches!(
            err("issue.monitor.failover", json!({"number": 42})),
            CliParseError::MissingFlag("reason")
        ));
        assert!(matches!(
            err("issue.monitor.failover", json!({"reason": "x"})),
            CliParseError::MissingFlag("number")
        ));
    }

    /// SPEC-3431 Phase 24 T250 (FR-128/FR-130): all three PM control intents
    /// retain the complete exact target from the wire.  Structural matching is
    /// intentional: every field must exist in the command model and bind to
    /// the exact parsed value before handler work can start.
    #[test]
    fn t250_pm_control_operations_retain_the_complete_generation_target() {
        let target = json!({
            "project_root": "/tmp/project",
            "number": 42,
            "operation_id": "control-op-42",
            "reason": "stalled provider",
            "launch_generation": 17,
            "claim_id": "claim-42",
            "claim_owner": "owner-42",
            "delivery_id": "delivery-42",
            "materializer_window_id": "tab-1::materializer-42",
            "window_id": "tab-1::agent-42",
            "launch_profile_fingerprint": "profile-sha256-42",
        });

        let assert_target =
            |project_root: Option<std::path::PathBuf>,
             number: u64,
             reason: String,
             launch_generation: Option<u64>,
             claim_id: Option<String>,
             claim_owner: Option<String>,
             delivery_id: Option<String>,
             materializer_window_id: Option<String>,
             window_id: Option<String>,
             launch_profile_fingerprint: Option<String>| {
                assert_eq!(
                    project_root.as_deref(),
                    Some(std::path::Path::new("/tmp/project"))
                );
                assert_eq!(number, 42);
                assert_eq!(reason, "stalled provider");
                assert_eq!(launch_generation, Some(17));
                assert_eq!(claim_id.as_deref(), Some("claim-42"));
                assert_eq!(claim_owner.as_deref(), Some("owner-42"));
                assert_eq!(delivery_id.as_deref(), Some("delivery-42"));
                assert_eq!(
                    materializer_window_id.as_deref(),
                    Some("tab-1::materializer-42")
                );
                assert_eq!(window_id.as_deref(), Some("tab-1::agent-42"));
                assert_eq!(
                    launch_profile_fingerprint.as_deref(),
                    Some("profile-sha256-42")
                );
            };

        match ok("issue.monitor.stop", target.clone()) {
            CliCommand::Issue(IssueCommand::MonitorStop {
                project_root,
                number,
                operation_id,
                reason,
                launch_generation,
                claim_id,
                claim_owner,
                delivery_id,
                materializer_window_id,
                window_id,
                launch_profile_fingerprint,
            }) => {
                assert_eq!(operation_id.as_deref(), Some("control-op-42"));
                assert_target(
                    project_root,
                    number,
                    reason,
                    launch_generation,
                    claim_id,
                    claim_owner,
                    delivery_id,
                    materializer_window_id,
                    window_id,
                    launch_profile_fingerprint,
                );
            }
            other => panic!("unexpected stop command: {other:?}"),
        }
        match ok("issue.monitor.failover", target.clone()) {
            CliCommand::Issue(IssueCommand::MonitorFailover {
                project_root,
                number,
                operation_id,
                reason,
                launch_generation,
                claim_id,
                claim_owner,
                delivery_id,
                materializer_window_id,
                window_id,
                launch_profile_fingerprint,
            }) => {
                assert_eq!(operation_id.as_deref(), Some("control-op-42"));
                assert_target(
                    project_root,
                    number,
                    reason,
                    launch_generation,
                    claim_id,
                    claim_owner,
                    delivery_id,
                    materializer_window_id,
                    window_id,
                    launch_profile_fingerprint,
                );
            }
            other => panic!("unexpected failover command: {other:?}"),
        }
        match ok("issue.monitor.recover", target) {
            CliCommand::Issue(IssueCommand::MonitorRecover {
                project_root,
                number,
                operation_id,
                reason,
                launch_generation,
                claim_id,
                claim_owner,
                delivery_id,
                materializer_window_id,
                window_id,
                launch_profile_fingerprint,
            }) => {
                assert_eq!(operation_id, "control-op-42");
                assert_target(
                    project_root,
                    number,
                    reason,
                    Some(launch_generation),
                    claim_id,
                    claim_owner,
                    delivery_id,
                    materializer_window_id,
                    window_id,
                    launch_profile_fingerprint,
                );
            }
            other => panic!("unexpected recover command: {other:?}"),
        }
    }

    fn t250_operation_id_params(operation: &str, operation_id: &str) -> Value {
        match operation {
            "issue.monitor.control.reconcile" => {
                json!({"operation_id": operation_id, "revoked_generation": 17})
            }
            "issue.monitor.recover" => json!({
                "number": 42,
                "operation_id": operation_id,
                "reason": "operator request",
                "launch_generation": 17,
            }),
            _ => json!({
                "number": 42,
                "operation_id": operation_id,
                "reason": "operator request",
            }),
        }
    }

    fn t250_parsed_operation_id(operation: &str, operation_id: &str) -> String {
        match ok(operation, t250_operation_id_params(operation, operation_id)) {
            CliCommand::Issue(IssueCommand::MonitorStop {
                operation_id: Some(operation_id),
                ..
            })
            | CliCommand::Issue(IssueCommand::MonitorFailover {
                operation_id: Some(operation_id),
                ..
            })
            | CliCommand::Issue(IssueCommand::MonitorRecover { operation_id, .. })
            | CliCommand::Issue(IssueCommand::MonitorControlReconcile { operation_id, .. }) => {
                operation_id
            }
            other => panic!("{operation} did not retain its operation ID: {other:?}"),
        }
    }

    /// SPEC-3431 Phase 24 T250 (FR-129): supplied operation IDs are trimmed
    /// ASCII tokens. Legacy stop/failover remain parse-compatible without an
    /// ID, while recover/reconcile never receive generated replay identities.
    #[test]
    fn t250_pm_control_operation_id_boundaries_apply_to_every_control_action() {
        for operation in ["issue.monitor.stop", "issue.monitor.failover"] {
            assert!(
                parse(&envelope(
                    operation,
                    json!({"number": 42, "reason": "legacy caller"})
                ))
                .is_ok(),
                "legacy {operation} must remain parse-compatible"
            );
        }

        for operation in [
            "issue.monitor.stop",
            "issue.monitor.failover",
            "issue.monitor.recover",
            "issue.monitor.control.reconcile",
        ] {
            assert_eq!(t250_parsed_operation_id(operation, "x"), "x");
            assert_eq!(
                t250_parsed_operation_id(operation, &"x".repeat(128)),
                "x".repeat(128),
                "{operation} must accept exactly 128 ASCII bytes"
            );
            assert_eq!(
                t250_parsed_operation_id(operation, "  op-token_42.v1  "),
                "op-token_42.v1",
                "{operation} must trim the supplied token"
            );
        }

        let invalid = [
            ("blank", "   ".to_string()),
            ("internal-space", "op token".to_string()),
            ("display-control", "op\u{7f}".to_string()),
            ("non-ascii", "op-é".to_string()),
            ("129-bytes", "x".repeat(129)),
        ];
        let mut accepted = Vec::new();
        for operation in [
            "issue.monitor.stop",
            "issue.monitor.failover",
            "issue.monitor.recover",
            "issue.monitor.control.reconcile",
        ] {
            for (label, operation_id) in &invalid {
                if parse(&envelope(
                    operation,
                    t250_operation_id_params(operation, operation_id),
                ))
                .is_ok()
                {
                    accepted.push(format!("{operation}:{label}"));
                }
            }
        }
        assert!(
            accepted.is_empty(),
            "invalid operation IDs were accepted: {accepted:?}"
        );

        for operation in ["issue.monitor.recover", "issue.monitor.control.reconcile"] {
            let params = if operation.ends_with("reconcile") {
                json!({"revoked_generation": 17})
            } else {
                json!({"number": 42, "reason": "stalled"})
            };
            assert!(matches!(
                err(operation, params),
                CliParseError::MissingFlag("operation_id")
            ));
        }
    }

    /// Reasons are bounded by UTF-8 bytes, not scalar count, and display
    /// controls can never enter a durable receipt or diagnostic projection.
    #[test]
    fn t250_pm_control_reason_is_trimmed_bounded_utf8_without_controls() {
        let params = |operation: &str, reason: &str| {
            let mut params = json!({
                "number": 42,
                "operation_id": "control-op-42",
                "reason": reason,
            });
            if operation == "issue.monitor.recover" {
                params["launch_generation"] = json!(17);
            }
            params
        };
        let exactly_1024_bytes = "é".repeat(512);
        let invalid = [
            ("blank", "   ".to_string()),
            ("1025-utf8-bytes", format!("{}x", "é".repeat(512))),
            ("nul", "operator\0request".to_string()),
            ("bell", "operator\u{7}request".to_string()),
            ("delete", "operator\u{7f}request".to_string()),
        ];
        let mut accepted = Vec::new();
        for operation in [
            "issue.monitor.stop",
            "issue.monitor.failover",
            "issue.monitor.recover",
        ] {
            for (input, expected) in [
                ("  operator request  ", "operator request"),
                ("x", "x"),
                (&exactly_1024_bytes, exactly_1024_bytes.as_str()),
            ] {
                let reason = match ok(operation, params(operation, input)) {
                    CliCommand::Issue(IssueCommand::MonitorStop { reason, .. })
                    | CliCommand::Issue(IssueCommand::MonitorFailover { reason, .. })
                    | CliCommand::Issue(IssueCommand::MonitorRecover { reason, .. }) => reason,
                    other => panic!("unexpected {operation} command: {other:?}"),
                };
                assert_eq!(reason, expected, "{operation} reason normalization");
            }
            assert!(matches!(
                err(
                    operation,
                    if operation == "issue.monitor.recover" {
                        json!({
                            "number": 42,
                            "operation_id": "control-op-42",
                            "launch_generation": 17,
                        })
                    } else {
                        json!({"number": 42, "operation_id": "control-op-42"})
                    }
                ),
                CliParseError::MissingFlag("reason")
            ));
            for (label, reason) in &invalid {
                if parse(&envelope(operation, params(operation, reason))).is_ok() {
                    accepted.push(format!("{operation}:{label}"));
                }
            }
        }
        assert!(
            accepted.is_empty(),
            "invalid control reasons were accepted: {accepted:?}"
        );
    }

    /// SPEC-3431 FR-131: manual reconciliation identifies an already pending
    /// operation and the exact revoked generation.  Absence itself is never a
    /// caller-supplied field; it comes from the owning AppRuntime proof path.
    #[test]
    fn t250_issue_monitor_control_reconcile_parses_only_pending_identity() {
        match ok(
            "issue.monitor.control.reconcile",
            json!({
                "project_root": "/tmp/project",
                "operation_id": "recover-op-42",
                "revoked_generation": 17,
            }),
        ) {
            CliCommand::Issue(IssueCommand::MonitorControlReconcile {
                project_root,
                operation_id,
                revoked_generation,
            }) => {
                assert_eq!(
                    project_root.as_deref(),
                    Some(std::path::Path::new("/tmp/project"))
                );
                assert_eq!(operation_id, "recover-op-42");
                assert_eq!(revoked_generation, 17);
            }
            other => panic!("unexpected reconcile command: {other:?}"),
        }

        assert!(matches!(
            err(
                "issue.monitor.control.reconcile",
                json!({"revoked_generation": 17})
            ),
            CliParseError::MissingFlag("operation_id")
        ));
        assert!(matches!(
            err(
                "issue.monitor.control.reconcile",
                json!({"operation_id": "recover-op-42"})
            ),
            CliParseError::MissingFlag("revoked_generation")
        ));
        for caller_proof in [
            json!({"operation_id": "recover-op-42", "revoked_generation": 17, "absent": true}),
            json!({"operation_id": "recover-op-42", "revoked_generation": 17, "observed_absent": true}),
            json!({"operation_id": "recover-op-42", "revoked_generation": 17, "runtime_inventory_proof": {"present": false}}),
        ] {
            assert!(
                parse(&envelope("issue.monitor.control.reconcile", caller_proof)).is_err(),
                "caller-supplied absence/proof must be rejected"
            );
        }
    }

    /// No JSON field can nominate the PM control subject. Implementations may
    /// reject spoof fields or ignore them, but an accepted envelope must still
    /// contain only the ambient-authorized command identity.
    #[test]
    fn t250_pm_control_caller_spoof_fields_are_rejected_or_ignored() {
        let exact_target = json!({
            "number": 42,
            "operation_id": "control-op-42",
            "reason": "control requested",
            "launch_generation": 17,
            "claim_id": "claim-42",
            "claim_owner": "owner-42",
            "delivery_id": "delivery-42",
            "materializer_window_id": "tab-1::materializer-42",
            "window_id": "tab-1::agent-42",
            "launch_profile_fingerprint": "profile-sha256-42",
        });
        for (operation, baseline_params) in [
            ("issue.monitor.stop", exact_target.clone()),
            ("issue.monitor.failover", exact_target.clone()),
            ("issue.monitor.recover", exact_target),
            (
                "issue.monitor.control.reconcile",
                json!({"operation_id": "control-op-42", "revoked_generation": 17}),
            ),
        ] {
            let baseline = parse(&envelope(operation, baseline_params.clone()))
                .unwrap_or_else(|error| panic!("baseline {operation} must parse: {error}"));
            for field in ["caller", "actor", "session_id"] {
                let mut spoofed_params = baseline_params.clone();
                spoofed_params
                    .as_object_mut()
                    .expect("object params")
                    .insert(field.to_string(), json!("forged-pm-session"));
                if let Ok(spoofed) = parse(&envelope(operation, spoofed_params)) {
                    assert_eq!(
                        spoofed.command, baseline.command,
                        "{operation} accepted {field} as command identity"
                    );
                }
            }
        }
    }

    /// SPEC-3431 FR-111 (T-206): the PM's privileged pane delivery is its own
    /// operation with a mandatory exact target — never a loosened pane.send.
    #[test]
    fn pm_message_send_parses_with_mandatory_target_and_text() {
        assert_eq!(
            ok(
                "pm.message.send",
                json!({"id": "tab-1::agent-1", "text": "please report status"})
            ),
            CliCommand::Pane(PaneCommand::PmSend {
                project_root: None,
                id: "tab-1::agent-1".to_string(),
                text: "please report status".to_string(),
            })
        );
        assert!(matches!(
            err("pm.message.send", json!({"text": "hello"})),
            CliParseError::MissingFlag("id")
        ));
        assert!(matches!(
            err("pm.message.send", json!({"id": "tab-1::agent-1"})),
            CliParseError::MissingFlag("text")
        ));

        let default_scope = ok(
            "pm.message.send",
            json!({"id": "tab-1::agent-1", "text": "status"}),
        );
        let explicit_scope = ok(
            "pm.message.send",
            json!({
                "project_root": "/projects/canonical",
                "id": "tab-1::agent-1",
                "text": "status"
            }),
        );
        assert_ne!(
            explicit_scope, default_scope,
            "pm.message.send must preserve the same explicit project scope accepted by pm.status"
        );
        assert!(matches!(
            explicit_scope,
            CliCommand::Pane(PaneCommand::PmSend {
                project_root: Some(project_root),
                ..
            }) if project_root == "/projects/canonical"
        ));
    }

    /// Issue #3478 (AC-5/AC-9): the canonical operations a human uses to see
    /// and answer what an autonomous agent is waiting on.
    #[test]
    fn issue_monitor_question_operations_parse() {
        assert_eq!(
            ok("issue.monitor.questions", json!({})),
            CliCommand::Issue(IssueCommand::MonitorQuestions { project_root: None })
        );
        assert_eq!(
            ok(
                "issue.monitor.question.answer",
                json!({"handoff_id": "handoff-1", "answer": "Yes"})
            ),
            CliCommand::Issue(IssueCommand::MonitorQuestionAnswer {
                project_root: None,
                handoff_id: "handoff-1".to_string(),
                answer: "Yes".to_string(),
            })
        );
        // An answer can never be attached to an unidentified parked question,
        // and an identified one can never be answered with nothing.
        assert!(matches!(
            err("issue.monitor.question.answer", json!({"answer": "Yes"})),
            CliParseError::MissingFlag(_)
        ));
        assert!(matches!(
            err(
                "issue.monitor.question.answer",
                json!({"handoff_id": "handoff-1"})
            ),
            CliParseError::MissingFlag(_)
        ));
    }

    #[test]
    fn issue_monitor_queue_operations_parse() {
        assert_eq!(
            ok("issue.monitor.status", json!({})),
            CliCommand::Issue(IssueCommand::MonitorStatus { project_root: None })
        );
        assert_eq!(
            ok(
                "issue.monitor.status",
                json!({"project_root": "/tmp/project"})
            ),
            CliCommand::Issue(IssueCommand::MonitorStatus {
                project_root: Some(std::path::PathBuf::from("/tmp/project")),
            })
        );
        assert_eq!(
            ok("issue.monitor.priority.move", json!({"number": 42})),
            CliCommand::Issue(IssueCommand::MonitorPriorityMove {
                project_root: None,
                number: 42,
                position: IssueMonitorPriorityPosition::Head,
            })
        );
        assert_eq!(
            ok(
                "issue.monitor.priority.move",
                json!({"project_root": "/tmp/project", "number": 42, "position": 3})
            ),
            CliCommand::Issue(IssueCommand::MonitorPriorityMove {
                project_root: Some(std::path::PathBuf::from("/tmp/project")),
                number: 42,
                position: IssueMonitorPriorityPosition::Index(3),
            })
        );
        assert_eq!(
            ok(
                "issue.monitor.priority.set",
                json!({"issue_numbers": [9, 4, 7]})
            ),
            CliCommand::Issue(IssueCommand::MonitorPrioritySet {
                project_root: None,
                issue_numbers: vec![9, 4, 7],
            })
        );
        assert_eq!(
            ok(
                "issue.monitor.config.set",
                json!({"enabled": false, "autonomous_mode": false, "max_active": 3})
            ),
            CliCommand::Issue(IssueCommand::MonitorConfigSet {
                project_root: None,
                enabled: Some(false),
                autonomous_mode: Some(false),
                max_active: Some(3),
            })
        );
    }

    #[test]
    fn issue_monitor_queue_operations_reject_unsafe_or_incomplete_params() {
        for params in [
            json!({}),
            json!({"enabled": true}),
            json!({"max_active": 0}),
        ] {
            assert!(matches!(
                err("issue.monitor.config.set", params),
                CliParseError::InvalidJson(_) | CliParseError::MissingFlag(_)
            ));
        }
        assert!(matches!(
            err("issue.monitor.config.set", json!({"autonomous_mode": true})),
            CliParseError::InvalidJson(_)
        ));
        assert!(matches!(
            err(
                "issue.monitor.priority.move",
                json!({"number": 42, "position": "tail"})
            ),
            CliParseError::InvalidJson(_)
        ));
        assert!(matches!(
            err("issue.monitor.priority.set", json!({})),
            CliParseError::MissingFlag("issue_numbers")
        ));
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

    // SPEC-3431: PM agent diagnostics parse variants.
    #[test]
    fn pm_status_variants() {
        assert!(matches!(
            ok("pm.status", json!({})),
            CliCommand::Pm(crate::cli::pm::PmCommand::Status { project_root: None })
        ));
        match ok("pm.status", json!({"project_root": "/tmp/elsewhere"})) {
            CliCommand::Pm(crate::cli::pm::PmCommand::Status { project_root }) => {
                assert_eq!(project_root.as_deref(), Some("/tmp/elsewhere"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    // SPEC-3248 P8a: execution settlement parse variants.
    #[test]
    fn execution_settlement_variants() {
        assert!(matches!(
            ok("execution.status", json!({})),
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::Status)
        ));
        assert!(matches!(
            err("execution.status", json!({"unexpected": true})),
            CliParseError::InvalidJson(message)
                if message.contains("accepts no params")
        ));
        assert!(matches!(
            ok("execution.complete", json!({})),
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::Complete)
        ));
        assert!(matches!(
            ok(
                "execution.blocked",
                json!({"reason": "E2E runner unavailable", "missing_verification": "lifecycle E2E"})
            ),
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::Blocked { .. })
        ));
        assert!(matches!(
            err("execution.blocked", json!({})),
            CliParseError::MissingFlag("reason")
        ));
        assert!(matches!(
            ok(
                "execution.repair",
                json!({"reason": "trusted authority is corrupt"})
            ),
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::Repair { .. })
        ));
        assert!(matches!(
            err("execution.repair", json!({})),
            CliParseError::MissingFlag("reason")
        ));
        assert!(matches!(
            ok("execution.reopen", json!({"reason": "blocker resolved"})),
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::Reopen { .. })
        ));
        assert!(matches!(
            err("execution.reopen", json!({})),
            CliParseError::MissingFlag("reason")
        ));
        assert!(matches!(
            ok(
                "execution.continue",
                json!({"operation_id": "continue-operation-1"})
            ),
            CliCommand::Execution(
                crate::cli::execution_state::ExecutionCommand::Continue { operation_id }
            ) if operation_id == "continue-operation-1"
        ));
        assert!(matches!(
            err("execution.continue", json!({})),
            CliParseError::MissingFlag("operation_id")
        ));
        assert!(matches!(
            err(
                "execution.continue",
                json!({"operation_id": "continue-operation-1", "unexpected": true})
            ),
            CliParseError::InvalidJson(message)
                if message.contains("only accepts params.operation_id")
        ));
    }

    #[test]
    fn verification_plan_generated_output_allowlist_is_typed() {
        assert!(matches!(
            ok(
                "verify.plan",
                json!({
                    "commands": ["cargo test -p gwt --lib"],
                    "generated_outputs": ["artifacts/report.json"]
                })
            ),
            CliCommand::Verify(
                crate::cli::verification_record::VerifyCommand::PlanWithOutputs {
                    generated_outputs,
                    ..
                }
            ) if generated_outputs == vec!["artifacts/report.json"]
        ));
    }

    // SPEC-3248 P7A (T-072/FR-012): intake.outcome.record parse variants.
    #[test]
    fn intake_outcome_record_variants() {
        assert!(matches!(
            ok(
                "intake.outcome.record",
                json!({"kind": "no_action", "reason": "duplicate of #3248"})
            ),
            CliCommand::Intake(crate::cli::intake_outcome::IntakeCommand::OutcomeRecord { .. })
        ));
        assert!(matches!(
            ok(
                "intake.outcome-record",
                json!({"kind": "issue_updated", "number": 3248})
            ),
            CliCommand::Intake(crate::cli::intake_outcome::IntakeCommand::OutcomeRecord { .. })
        ));
        // kind is required at the parse layer.
        assert!(matches!(
            err("intake.outcome.record", json!({"number": 1})),
            CliParseError::MissingFlag("kind")
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
        // pr.edit with nothing to update is rejected at parse time, matching
        // the argv path's Usage guard (a silent no-op success would mask caller
        // bugs such as passing pr.create's "labels" key instead of "add_labels").
        assert!(matches!(
            err("pr.edit", json!({"number": 1})),
            CliParseError::MissingFlag("title|body|add_labels")
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
            ok("pr.ready", json!({"number": 9})),
            CliCommand::Pr(PrCommand::Ready { number: 9 })
        ));
        assert!(matches!(
            ok("pr.draft", json!({"number": 9})),
            CliCommand::Pr(PrCommand::Draft { number: 9 })
        ));
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

    /// SPEC-3431 FR-025: the PM's resident loop subscribes, then reconciles
    /// against a fresh snapshot. That only works if the subscribe returns.
    #[test]
    fn daemon_subscribe_accepts_a_bounded_timeout() {
        assert!(matches!(
            ok(
                "daemon.subscribe",
                json!({"channels": ["issue_monitor"], "timeout_seconds": 30})
            ),
            CliCommand::Daemon(DaemonCommand::Subscribe {
                timeout_seconds: Some(30),
                ..
            })
        ));
        assert!(matches!(
            ok("daemon.subscribe", json!({"channels": ["board"]})),
            CliCommand::Daemon(DaemonCommand::Subscribe {
                timeout_seconds: None,
                ..
            })
        ));
        // Zero would mean "return before reading anything", which is never
        // what a caller wants and silently degrades the loop to a busy poll.
        assert!(err(
            "daemon.subscribe",
            json!({"channels": ["board"], "timeout_seconds": 0})
        )
        .to_string()
        .contains("timeout_seconds"));
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
    fn workflow_bypass_parses_all_modes() {
        assert!(matches!(
            ok("workflow.bypass", json!({"mode": "release"})),
            CliCommand::Workflow(WorkflowCommand::Bypass {
                mode: WorkflowBypassMode::Release
            })
        ));
        assert!(matches!(
            ok("workflow.bypass", json!({"mode": "chore"})),
            CliCommand::Workflow(WorkflowCommand::Bypass {
                mode: WorkflowBypassMode::Chore
            })
        ));
        assert!(matches!(
            ok("workflow.bypass", json!({"mode": "off"})),
            CliCommand::Workflow(WorkflowCommand::Bypass {
                mode: WorkflowBypassMode::Off
            })
        ));
    }

    #[test]
    fn workflow_bypass_requires_valid_mode() {
        assert!(matches!(
            err("workflow.bypass", json!({})),
            CliParseError::MissingFlag("mode")
        ));
        assert!(matches!(
            err("workflow.bypass", json!({"mode": "Release"})),
            CliParseError::InvalidValue { flag: "mode", .. }
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
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ambient =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::session::GWT_SESSION_ID_ENV, "s");
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
