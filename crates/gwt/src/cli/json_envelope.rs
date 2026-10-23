use gwt_agent::session::GWT_SESSION_ID_ENV;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::protocol::{IndexSearchMatchMode, IndexSearchScope};

use super::{
    memory::MemoryAddCommand, perf::PerfCommand, workflow::WorkflowBypassMode, ActionsCommand,
    CliCommand, CliEnv, CliParseError, DaemonCommand, DiagnosticsCommand, HookCommand,
    IndexCommand, IndexScope, IssueCommand, IssueLabelAction, MemoryCommand, PaneCommand,
    PrCommand, SearchCommand, SkillStateAction, WorkflowCommand, WorkspaceCommand,
};
use super::{verification_lease::VerificationLeaseCommand, BoardCommand, BoardPostCommand};

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
    requested_project_root: Option<String>,
    /// Issue #3655 AC-1: the reason an agent gave when it declared itself
    /// unable to proceed, carried past parsing so the escalation can be raised
    /// from the same call rather than from a Board post the agent also has to
    /// remember to write.
    declared_block: Option<DeclaredBlock>,
}

/// An agent's own `execution.blocked` declaration, as escalation material.
pub(crate) struct DeclaredBlock {
    pub reason: String,
    pub missing_verification: Option<String>,
}

pub(crate) fn run_collect_governed<E: CliEnv>(
    env: &mut E,
    cmd: CliCommand,
) -> Result<super::governance::GovernedCommandOutput, Box<super::governance::GovernedCommandFailure>>
{
    match cmd {
        CliCommand::Execution(inner) => {
            let mut output = String::new();
            let result = super::execution_state::run_governed(env, inner, &mut output)?;
            Ok(super::governance::GovernedCommandOutput {
                exit_code: result.exit_code,
                output,
                refusal: result.refusal,
            })
        }
        other => {
            let (exit_code, output) = super::run_collect(env, other).map_err(|error| {
                Box::new(super::governance::GovernedCommandFailure {
                    error,
                    refusal: None,
                })
            })?;
            Ok(super::governance::GovernedCommandOutput {
                exit_code,
                output,
                refusal: None,
            })
        }
    }
}

pub(crate) fn dispatch<E: CliEnv>(env: &mut E, prog: &str) -> i32 {
    let input = match env.read_stdin() {
        Ok(input) => input,
        Err(err) => {
            let _ = writeln!(env.stderr(), "{prog}: failed to read JSON envelope: {err}");
            return 1;
        }
    };
    dispatch_input(env, prog, &input, None)
}

pub(super) fn parse_operation(
    operation: &str,
    params: &Value,
) -> Result<CliCommand, CliParseError> {
    parse(
        &serde_json::json!({"schema_version":1,"operation":operation,"params":params}).to_string(),
    )
    .map(|parsed| parsed.command)
}

/// The same dispatcher with an explicit input/output sink for bounded delegation.
pub(super) fn dispatch_input<E: CliEnv>(
    env: &mut E,
    prog: &str,
    input: &str,
    mut captured: Option<&mut Vec<u8>>,
) -> i32 {
    let parsed = match parse(input) {
        Ok(parsed) => parsed,
        Err(err) => {
            let _ = writeln!(env.stderr(), "{prog}: {err}");
            return 2;
        }
    };
    let operation = parsed.operation.clone();
    let declared_block = parsed.declared_block;
    let requested_project_root = parsed.requested_project_root;
    // SPEC #3700 FR-002 / Issue #4145 AC-1: every JSON-envelope operation
    // funnels through here, so one timer covers the whole `op` stream. The
    // collector is fail-open and is only installed by the `gwtd` binary, so
    // this is a no-op in tests and in the argv path.
    let read_only = super::hook::workflow_policy::is_read_only_json_envelope_operation(&operation);
    let operation_started = std::time::Instant::now();
    // Issue #4850: the warning sink is per thread; clear whatever an earlier
    // operation on this thread left behind before this one runs.
    super::operation_warnings::take();
    let mut pr_checks_data = None;
    let mut verification_response = None;
    let outcome = match parsed.command {
        CliCommand::Verify(command) => {
            let mut output = String::new();
            super::verification_record::run_with_response(
                env,
                command,
                &mut output,
                &mut verification_response,
            )
            .map(|exit_code| super::governance::GovernedCommandOutput {
                exit_code,
                output,
                refusal: None,
            })
            .map_err(|error| {
                Box::new(super::governance::GovernedCommandFailure {
                    error,
                    refusal: None,
                })
            })
        }
        CliCommand::Pr(PrCommand::Checks { number }) => env
            .fetch_pr_checks(number)
            .map(|report| {
                pr_checks_data =
                    Some(serde_json::to_value(&report).expect("PR checks must serialize"));
                let mut output = String::new();
                super::pr::render_pr_checks(&mut output, &report);
                super::governance::GovernedCommandOutput {
                    exit_code: 0,
                    output,
                    refusal: None,
                }
            })
            .map_err(|error| {
                Box::new(super::governance::GovernedCommandFailure {
                    error: super::io_as_api_error(error),
                    refusal: None,
                })
            }),
        command => run_collect_governed(env, command),
    };
    let warnings = super::operation_warnings::take();
    crate::perf::record_operation(&operation, operation_started.elapsed(), read_only);
    match outcome {
        Ok(result) => {
            let super::governance::GovernedCommandOutput {
                exit_code: code,
                output,
                refusal,
            } = result;
            let mut payload = serde_json::json!({
                "ok": code == 0,
                "operation": operation,
                "exit_code": code,
                "output": output,
            });
            if code == 0 {
                match operation.as_str() {
                    "issue.view" | "issue.monitor.status" => payload["kind"] = "issue".into(),
                    "pr.view" | "pr.list" => payload["kind"] = "pull_request".into(),
                    _ => {}
                }
            }
            if let Some(data) = pr_checks_data {
                payload["data"] = data;
            }
            if let Some(response) = verification_response {
                payload["status"] = response.status.into();
                payload["data"] = response.data;
            } else if operation == "verify.run" && code != 0 {
                attach_verification_error(&mut payload, &output);
            }
            if let Some(refusal) = refusal.as_ref() {
                payload["refusal"] = serde_json::to_value(refusal)
                    .expect("operation refusal metadata must serialize");
            }
            // Issue #4850 AC-1: a successful remote mutation whose local
            // follow-up was skipped answers `ok:true` plus what it skipped.
            if !warnings.is_empty() {
                payload["warnings"] =
                    serde_json::to_value(&warnings).expect("operation warnings must serialize");
            }
            attach_project_store(&mut payload);
            if let Err(err) = write_dispatch_response(env, &mut captured, &payload) {
                return report_undelivered_response(env, prog, &operation, &err);
            }
            match (code, declared_block) {
                (0, Some(block)) => super::board::auto_file_declared_block(env, &block),
                (0, None) => {}
                _ => {
                    report_operation_refusal(
                        &operation,
                        &output,
                        requested_project_root.as_deref(),
                    );
                    if let Some(refusal) = refusal.as_ref() {
                        super::board::auto_file_structured_operation_refusal(
                            env, &operation, &output, refusal,
                        );
                    } else if !operation.starts_with("execution.") {
                        super::board::auto_file_operation_refusal(env, &operation, &output);
                    }
                }
            }
            code
        }
        // Issue #3510: a failure still answers on the JSON-only operation
        // surface. Without this, stdout stayed empty and a machine caller
        // could not distinguish "the operation failed at stage X" from "the
        // process never answered". The stderr line stays for humans.
        Err(failure) => {
            let message = match (&failure.error, operation.as_str()) {
                (
                    gwt_github::SpecOpsError::Api(gwt_github::client::ApiError::Unexpected(cause)),
                    "verify.run",
                ) => {
                    format!("verification operation failed: {cause}")
                }
                _ => failure.error.to_string(),
            };
            let mut payload = serde_json::json!({
                "ok": false,
                "operation": operation,
                "exit_code": 1,
                "error": message,
            });
            if matches!(failure.error, gwt_github::SpecOpsError::CacheConflict(_)) {
                payload["error_code"] = "ISSUE_CACHE_CONFLICT".into();
            }
            if operation == "verify.run" {
                attach_verification_error(&mut payload, &message);
            }
            if let Some(refusal) = failure.refusal.as_ref() {
                payload["refusal"] = serde_json::to_value(refusal)
                    .expect("operation refusal metadata must serialize");
            }
            attach_project_store(&mut payload);
            if let Err(err) = write_dispatch_response(env, &mut captured, &payload) {
                return report_undelivered_response(env, prog, &operation, &err);
            }
            let _ = writeln!(env.stderr(), "{prog} {operation}: {message}");
            // Issue #3655 AC-2: a governance refusal reaches the PM without
            // depending on the agent noticing it is stuck. Answering the caller
            // comes first — the escalation must never delay or replace the
            // operation's own reply.
            report_operation_refusal(&operation, &message, requested_project_root.as_deref());
            if let Some(refusal) = failure.refusal.as_ref() {
                super::board::auto_file_structured_operation_refusal(
                    env, &operation, &message, refusal,
                );
            } else if !operation.starts_with("execution.") {
                super::board::auto_file_operation_refusal(env, &operation, &message);
            }
            1
        }
    }
}

fn write_dispatch_response<E: CliEnv>(
    env: &mut E,
    captured: &mut Option<&mut Vec<u8>>,
    payload: &Value,
) -> Result<(), String> {
    match captured {
        Some(output) => write_response(*output, payload),
        None => write_response(env.stdout(), payload),
    }
}

fn attach_verification_error(payload: &mut Value, cause: &str) {
    payload["status"] = "error".into();
    payload["data"] = serde_json::json!({
        "cause": cause,
        "recovery": "Inspect verify.status and verify.lease.status, repair the reported cause, then retry the identical full verify.run matrix; re-register verify.plan if its inputs changed.",
    });
}

/// Exit code for an operation whose response never reached the caller.
///
/// Issue #4435 AC-1: swallowing the write turns a delivery failure into a
/// zero-byte, exit-0 answer that reads exactly like silent success. Distinct
/// from the ordinary failure code so a caller can tell the two apart.
const RESPONSE_NOT_DELIVERED_EXIT: i32 = 3;

/// Write the response envelope. The envelope is the operation's only answer,
/// so a failed write is reported, never dropped.
fn write_response(stdout: &mut dyn std::io::Write, payload: &Value) -> Result<(), String> {
    writeln!(stdout, "{payload}").map_err(|err| err.to_string())
}

fn report_undelivered_response<E: CliEnv>(
    env: &mut E,
    prog: &str,
    operation: &str,
    error: &str,
) -> i32 {
    let _ = writeln!(
        env.stderr(),
        "{prog} {operation}: response envelope was not delivered: {error}"
    );
    RESPONSE_NOT_DELIVERED_EXIT
}

fn report_operation_refusal(operation: &str, error: &str, requested_project_root: Option<&str>) {
    // gwtd executes one operation per process; the recorded store belongs to that operation.
    // An explicit target wins even if resolution failed before a store could be recorded.
    let project_root = requested_project_root.map(str::to_owned).or_else(|| {
        gwt_core::paths::operation_project_store()
            .map(|store| store.project_root.display().to_string())
    });
    let session_id = std::env::var(GWT_SESSION_ID_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    crate::error_report::report_error(
        gwt_core::error_ledger::ErrorKind::OperationRefusal,
        format!("{operation}: {error}"),
        gwt_core::error_ledger::ErrorTarget {
            session_id,
            project_root,
            ..gwt_core::error_ledger::ErrorTarget::default()
        },
    );
}

/// Issue #3606: name the project store the operation acted on.
///
/// A `project_root` that resolves no repository identity still produces a
/// perfectly working store — just an isolated one, keyed by path, that no
/// running gwt opened under a different path will ever read. The PM lost half a
/// day to exactly that: `issue.monitor.priority.move` answered `ok: true` and
/// read the new order straight back, from a store nothing else was using. The
/// envelope now carries the store's hash, its directory, and whether the hash
/// came from a repository identity, so the caller can assert on the landing
/// instead of comparing mtimes under `~/.gwt/projects/`.
///
/// Absent when the operation never resolved a project store — including when it
/// failed before doing so. Absence therefore means "no store was touched", not
/// "the store is fine".
fn attach_project_store(payload: &mut Value) {
    let Some(store) = gwt_core::paths::operation_project_store() else {
        return;
    };
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    let reported = serde_json::to_value(
        crate::runtime_daemon_events::ProjectStoreIdentity::from_operation_store(&store),
    )
    .expect("project store identity serializes");
    object.insert("project_store".to_string(), reported);
}

/// Issue #4581: the `params` keys the operation's parser actually consulted.
///
/// The envelope used to drop any key no parser asked for, so an
/// `issue.close` call that carried `dry_run: true` as a safety net closed the
/// Issue for real. The fix cannot be a hand-kept allowlist per operation —
/// there are hundreds of arms and the list drifts the moment one gains a
/// parameter — so the accessors below record every key they look at and
/// [`parse`] refuses whatever the caller sent that nothing read. An alias
/// records both spellings, because the parser consults both.
mod param_audit {
    use std::cell::RefCell;
    use std::collections::BTreeSet;

    thread_local! {
        static CONSULTED: RefCell<Option<BTreeSet<String>>> = const { RefCell::new(None) };
    }

    /// Records consulted keys for as long as it is alive, restoring the
    /// enclosing operation's recording after synchronous nested parsing.
    pub(super) struct Recording {
        previous: Option<BTreeSet<String>>,
    }

    impl Recording {
        pub(super) fn start() -> Self {
            let previous = CONSULTED.with(|consulted| consulted.replace(Some(BTreeSet::new())));
            Recording { previous }
        }

        pub(super) fn consulted(&self) -> BTreeSet<String> {
            CONSULTED.with(|consulted| consulted.borrow().clone().unwrap_or_default())
        }
    }

    impl Drop for Recording {
        fn drop(&mut self) {
            CONSULTED.with(|consulted| *consulted.borrow_mut() = self.previous.take());
        }
    }

    pub(super) fn note(key: &str) {
        CONSULTED.with(|consulted| {
            if let Some(keys) = consulted.borrow_mut().as_mut() {
                keys.insert(key.to_string());
            }
        });
    }
}

/// Read `params[key]`, recording that this operation understands `key`.
///
/// Every parameter accessor goes through here, so "the operation read it" and
/// "the operation accepts it" cannot drift apart.
fn lookup<'a>(params: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    param_audit::note(key);
    params.get(key)
}

/// Refuse the `params` keys no accessor read (Issue #4581 AC-1).
///
/// Applied to every operation, read-only ones included, so a caller never has
/// to remember which operations are strict (AC-4).
fn reject_unconsulted_params(
    params: &Map<String, Value>,
    consulted: &std::collections::BTreeSet<String>,
    operation: &str,
) -> Result<(), CliParseError> {
    let unknown: Vec<&str> = params
        .keys()
        .map(String::as_str)
        .filter(|key| !consulted.contains(*key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let accepted: Vec<&str> = consulted.iter().map(String::as_str).collect();
    Err(CliParseError::InvalidJson(format!(
        "{operation} does not accept the parameter{} {}; accepted: {}",
        if unknown.len() == 1 { "" } else { "s" },
        unknown.join(", "),
        if accepted.is_empty() {
            "(this operation takes no parameters)".to_string()
        } else {
            accepted.join(", ")
        }
    )))
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
    let recording = param_audit::Recording::start();
    let command = match envelope.operation.as_str() {
        "feedback.create" => {
            params.keys().for_each(|key| param_audit::note(key));
            let input = serde_json::from_value(Value::Object(params.clone()))
                .map_err(|err| CliParseError::InvalidJson(err.to_string()))?;
            CliCommand::Feedback(super::feedback::FeedbackCommand::Create(Box::new(input)))
        }
        "feedback.status" | "feedback.retry" | "feedback.process" => {
            reject_unknown_params(params, &["id"], &envelope.operation)?;
            let id = required_string(params, "id")?;
            let command = match envelope.operation.as_str() {
                "feedback.status" => super::feedback::FeedbackCommand::Status { id },
                "feedback.retry" => super::feedback::FeedbackCommand::Retry { id },
                _ => super::feedback::FeedbackCommand::Process { id },
            };
            CliCommand::Feedback(command)
        }
        "concern.create" | "concern.update" | "concern.list" | "concern.measure"
        | "concern.resolve" => {
            // These decode the whole `params` object through serde with
            // `deny_unknown_fields`, so the parser already refuses a key it
            // does not know and the audit below has nothing to add.
            params.keys().for_each(|key| param_audit::note(key));
            CliCommand::Concern(Box::new(super::concern::parse(
                &envelope.operation,
                params,
            )?))
        }
        "workspace.update" => workspace_update(params)?,
        "workspace.close" => {
            reject_unknown_params(params, &["work_id", "close_kind"], "workspace.close")?;
            let close_kind = required_string(params, "close_kind")?;
            if !matches!(close_kind.as_str(), "done" | "discarded" | "superseded") {
                return Err(CliParseError::InvalidValue {
                    flag: "close_kind",
                    reason: "expected done, discarded, or superseded",
                });
            }
            CliCommand::Workspace(WorkspaceCommand::Close {
                work_id: required_string(params, "work_id")?,
                close_kind,
            })
        }
        "workspace.receipt" => CliCommand::Workspace(WorkspaceCommand::Receipt {
            operation_id: required_string(params, "operation_id")?,
        }),
        "workspace.candidates" => workspace_candidates(params)?,
        "workspace.join" => workspace_join(params)?,
        "workspace.create" => workspace_create(params)?,
        "workspace.ensure" => workspace_ensure(params)?,
        "workspace.events_reconcile" => {
            reject_unknown_params(
                params,
                &["reference", "dry_run"],
                "workspace.events_reconcile",
            )?;
            CliCommand::Workspace(WorkspaceCommand::EventsReconcile {
                reference: required_string(params, "reference")?,
                dry_run: optional_bool(params, "dry_run")?.unwrap_or(true),
            })
        }
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
            // Issue #4465 AC-4'': this operation closes, discards and detaches
            // durable Work state across every Work in the project, so a scope
            // parameter it does not implement is a refusal. `params.work` was
            // silently dropped and the call then applied machine-wide.
            reject_unknown_params(
                params,
                &["project_root", "dry_run", "ids"],
                "workspace.work_prune",
            )?;
            CliCommand::Workspace(WorkspaceCommand::WorkPrune {
                // Issue #4465 AC-8: an unqualified call reports candidates
                // only; applying requires an explicit opt-out.
                dry_run: optional_bool(params, "dry_run")?.unwrap_or(true),
                ids: optional_string_vec(params, "ids")?,
                project_root: optional_string(params, "project_root")?,
            })
        }
        "workspace.store_consolidate" | "workspace.store-consolidate" => {
            // Issue #3524 (folded into #3606): this operation moves durable
            // state, so a parameter it does not understand is a refusal rather
            // than something to ignore. A silently-dropped `project_root` used
            // to leave the apply targeting whatever cwd it happened to run in.
            reject_unknown_params(
                params,
                &["project_root", "dry_run", "manifest_hash"],
                "workspace.store_consolidate",
            )?;
            CliCommand::Workspace(WorkspaceCommand::StoreConsolidate {
                project_root: optional_path(params, "project_root")?,
                // Consolidation moves durable state, so it dry-runs unless the
                // caller explicitly opts out (#3466 AC-8).
                dry_run: optional_bool(params, "dry_run")?.unwrap_or(true),
                manifest_hash: optional_string(params, "manifest_hash")?,
            })
        }
        "board.show" => board_show(params)?,
        "board.post" => board_post(params)?,
        "board.sync" => {
            reject_unknown_params(params, &[], "board.sync")?;
            CliCommand::Board(BoardCommand::Sync)
        }
        "board.config.show" | "board.config-show" => {
            CliCommand::Board(crate::cli::board::BoardCommand::ConfigShow)
        }
        "branch.prune_merged" | "branch.prune-merged" => {
            reject_unknown_params(
                params,
                &["dry_run", "base", "branches"],
                "branch.prune_merged",
            )?;
            CliCommand::Branch(crate::cli::branch::BranchCommand::PruneMerged {
                // Deleting remote branches is not reversible from here, so an
                // unqualified call only reports candidates (Issue #3970 AC-3).
                dry_run: optional_bool(params, "dry_run")?.unwrap_or(true),
                base: optional_string(params, "base")?,
                branches: optional_string_vec(params, "branches")?,
            })
        }
        "worktree.gc_build_artifacts" | "worktree.gc-build-artifacts" => {
            reject_unknown_params(
                params,
                &[
                    "dry_run",
                    "base",
                    "include_unmerged",
                    "include_protected_workspaces",
                ],
                "worktree.gc_build_artifacts",
            )?;
            CliCommand::Worktree(crate::cli::worktree_gc::WorktreeCommand::GcBuildArtifacts {
                // Removing a build cache is recoverable but slow to undo, so
                // an unqualified call only reports (Issue #4009 AC-1).
                dry_run: optional_bool(params, "dry_run")?.unwrap_or(true),
                base: optional_string(params, "base")?,
                include_unmerged: optional_bool(params, "include_unmerged")?.unwrap_or(false),
                include_protected_workspaces: optional_bool(
                    params,
                    "include_protected_workspaces",
                )?
                .unwrap_or(false),
            })
        }
        "intake.outcome.record" | "intake.outcome-record" => {
            CliCommand::Intake(crate::cli::intake_outcome::IntakeCommand::OutcomeRecord {
                kind: required_string(params, "kind")?,
                number: optional_u64(params, "number")?,
                reason: optional_string(params, "reason")?,
            })
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
        "issue.spec.audit" => CliCommand::Issue(IssueCommand::SpecAudit {
            state: optional_string(params, "state")?,
        }),
        "issue.spec.pull" => CliCommand::Issue(IssueCommand::SpecPull {
            all: optional_bool(params, "all")?.unwrap_or(false),
            numbers: optional_u64_vec(params, "numbers")?,
        }),
        "issue.spec.repair" => CliCommand::Issue(IssueCommand::SpecRepair {
            number: required_u64(params, "number")?,
        }),
        "issue.cache.repair" => CliCommand::Issue(IssueCommand::CacheRepair {
            number: required_u64(params, "number")?,
        }),
        "issue.spec.lint" => CliCommand::Issue(IssueCommand::SpecLint {
            number: required_u64(params, "number")?,
            sections: optional_string_vec(params, "sections")?,
            snapshot: optional_bool(params, "snapshot")?.unwrap_or(true),
            directive_epoch: optional_string(params, "directive_epoch")?,
            phase_slice: optional_string(params, "phase_slice")?,
        }),
        "issue.spec.inspection.complete" | "issue.spec.inspection-complete" => {
            CliCommand::Issue(IssueCommand::SpecInspectionComplete {
                number: required_u64(params, "number")?,
            })
        }
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
            owner_check: issue_owner_check(params)?,
        }),
        "issue.edit" => CliCommand::Issue(IssueCommand::Edit {
            number: required_u64(params, "number")?,
            title: optional_string(params, "title")?,
            body: optional_string(params, "body")?,
            // Absent or `null` means "leave labels alone"; only an explicit
            // empty array clears them.
            labels: lookup(params, "labels")
                .filter(|value| !value.is_null())
                .map(|_| optional_string_vec(params, "labels"))
                .transpose()?,
        }),
        "issue.label" => issue_label(params)?,
        "issue.close" => CliCommand::Issue(IssueCommand::Close {
            number: required_u64(params, "number")?,
            reason: issue_close_reason(params)?,
            comment: optional_string(params, "comment")?,
        }),
        "issue.reopen" => CliCommand::Issue(IssueCommand::Reopen {
            number: required_u64(params, "number")?,
            comment: optional_string(params, "comment")?,
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
        "update.settings.status" => CliCommand::Update(super::UpdateCommand::SettingsStatus {
            project_root: optional_path(params, "project_root")?,
        }),
        "issue.monitor.status" => CliCommand::Issue(IssueCommand::MonitorStatus {
            project_root: optional_path(params, "project_root")?,
        }),
        "issue.monitor.recovery.record" => {
            let number = required_u64(params, "number")?;
            let recovery_id = required_string(params, "recovery_id")?;
            let body = required_string(params, "body")?;
            let recovered_at = required_string(params, "recovered_at")?;
            let recovered_at = chrono::DateTime::parse_from_rfc3339(&recovered_at)
                .map_err(|_| {
                    CliParseError::InvalidJson(
                        "recovered_at must be an RFC3339 timestamp of the actual intervention"
                            .into(),
                    )
                })?
                .with_timezone(&chrono::Utc)
                .to_rfc3339();
            if number == 0
                || body.trim().is_empty()
                || recovery_id.is_empty()
                || recovery_id.len() > 128
                || !recovery_id.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
                })
            {
                return Err(CliParseError::InvalidJson(
                    "recovery.record requires a positive Issue number, a nonempty body and a stable ASCII recovery_id (1–128 letters, digits, -, _, . or :)".into(),
                ));
            }
            let key = format!("{number}:{recovery_id}");
            CliCommand::Board(BoardCommand::RecoveryPost {
                intent_id: format!("manual-recovery:{key}"),
                command: Box::new(BoardPostCommand {
                    kind: "decision".into(),
                    body: Some(body),
                    topics: vec![
                        format!("{}{key}", super::reliability::MANUAL_RECOVERY_TOPIC),
                        format!(
                            "{}{recovered_at}",
                            super::reliability::MANUAL_RECOVERY_AT_TOPIC
                        ),
                    ],
                    owners: vec![number.to_string()],
                    broadcast: true,
                    ..Default::default()
                }),
            })
        }
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
                expected_priority_order: lookup(params, "expected_priority_order")
                    .map(|_| required_u64_vec(params, "expected_priority_order"))
                    .transpose()?,
                project_root: optional_path(params, "project_root")?,
                number: required_u64(params, "number")?,
                position: issue_monitor_priority_position(params)?,
            })
        }
        "issue.monitor.launch_now" | "issue.monitor.launch-now" => {
            CliCommand::Issue(IssueCommand::MonitorLaunchNow {
                project_root: optional_path(params, "project_root")?,
                number: required_u64(params, "number")?,
                expected_priority_order: lookup(params, "expected_priority_order")
                    .map(|_| required_u64_vec(params, "expected_priority_order"))
                    .transpose()?,
            })
        }
        "issue.monitor.stop" => CliCommand::Issue(IssueCommand::MonitorStop {
            project_root: optional_path(params, "project_root")?,
            number: required_u64(params, "number")?,
            // FR-031: an unexplained stop is not auditable.
            reason: required_string(params, "reason")?,
            remove_from_queue: optional_bool(params, "remove_from_queue")?.unwrap_or(true),
            // Which identity components are required is a property of the live
            // launch, not of the request shape, so the state layer decides.
            claim_id: optional_string(params, "claim_id")?,
            delivery_id: optional_string(params, "delivery_id")?,
            window_id: optional_string(params, "window_id")?,
        }),
        "issue.monitor.failover" => CliCommand::Issue(IssueCommand::MonitorFailover {
            project_root: optional_path(params, "project_root")?,
            number: required_u64(params, "number")?,
            reason: required_string(params, "reason")?,
            claim_id: optional_string(params, "claim_id")?,
            delivery_id: optional_string(params, "delivery_id")?,
            window_id: optional_string(params, "window_id")?,
        }),
        "issue.monitor.requeue" => CliCommand::Issue(IssueCommand::MonitorRequeue {
            project_root: optional_path(params, "project_root")?,
            number: required_u64(params, "number")?,
            // An unexplained release of a recorded failure is not auditable.
            reason: required_string(params, "reason")?,
            force: optional_bool(params, "force")?.unwrap_or(false),
        }),
        "issue.monitor.quota_hold.list" | "issue.monitor.quota-hold.list" => {
            CliCommand::Issue(IssueCommand::MonitorQuotaHoldList {
                project_root: optional_path(params, "project_root")?,
            })
        }
        "issue.monitor.release_idle" | "issue.monitor.release-idle" => {
            CliCommand::Issue(IssueCommand::MonitorReleaseIdle {
                project_root: optional_path(params, "project_root")?,
                // Issue #4084 AC-5: no `number` means every idle row.
                number: optional_u64(params, "number")?,
                dry_run: optional_bool(params, "dry_run")?.unwrap_or(false),
            })
        }
        "issue.monitor.reconcile" => CliCommand::Issue(IssueCommand::MonitorReconcile {
            project_root: optional_path(params, "project_root")?,
        }),
        "issue.monitor.quota_hold.clear" | "issue.monitor.quota-hold.clear" => {
            CliCommand::Issue(IssueCommand::MonitorQuotaHoldClear {
                project_root: optional_path(params, "project_root")?,
                provider: required_string(params, "provider")?,
                // An unexplained release of a provider hold is not auditable.
                reason: required_string(params, "reason")?,
            })
        }
        "issue.monitor.wait" => {
            let clear = optional_bool(params, "clear")?.unwrap_or(false);
            let (reason, resume_condition) = if clear {
                (
                    optional_string(params, "reason")?,
                    optional_string(params, "resume_condition")?,
                )
            } else {
                // An unexplained wait is exactly the state the PM cannot act on.
                (
                    Some(required_string(params, "reason")?),
                    Some(required_string(params, "resume_condition")?),
                )
            };
            CliCommand::Issue(IssueCommand::MonitorWait {
                project_root: optional_path(params, "project_root")?,
                number: optional_u64(params, "number")?,
                reason,
                resume_condition,
                clear,
            })
        }
        "issue.monitor.wait.invalidate" | "issue.monitor.wait-invalidate" => {
            CliCommand::Issue(IssueCommand::MonitorWaitInvalidate {
                project_root: optional_path(params, "project_root")?,
                number: required_u64(params, "number")?,
                // An unexplained invalidation is exactly the record AC-2 needs.
                reason: required_string(params, "reason")?,
                by: optional_string(params, "by")?,
            })
        }
        "issue.monitor.priority.set" | "issue.monitor.priority-set" => {
            CliCommand::Issue(IssueCommand::MonitorPrioritySet {
                expected_priority_order: lookup(params, "expected_priority_order")
                    .map(|_| required_u64_vec(params, "expected_priority_order"))
                    .transpose()?,
                project_root: optional_path(params, "project_root")?,
                issue_numbers: required_u64_vec(params, "issue_numbers")?,
            })
        }
        "issue.monitor.queue.list" => CliCommand::Issue(IssueCommand::MonitorQueueList {
            project_root: optional_path(params, "project_root")?,
            terminal: optional_string(params, "terminal")?,
        }),
        "issue.monitor.queue.push" => CliCommand::Issue(IssueCommand::MonitorQueuePush {
            project_root: optional_path(params, "project_root")?,
            issue_numbers: required_u64_vec(params, "numbers")?,
            position: optional_usize(params, "position")?,
            force: optional_bool(params, "force")?.unwrap_or(false),
        }),
        "issue.monitor.queue.remove" => CliCommand::Issue(IssueCommand::MonitorQueueRemove {
            project_root: optional_path(params, "project_root")?,
            expected_queue_order: lookup(params, "expected_queue_order")
                .map(|_| required_u64_vec(params, "expected_queue_order"))
                .transpose()?,
            issue_numbers: required_u64_vec(params, "numbers")?,
        }),
        "issue.monitor.queue.move" => CliCommand::Issue(IssueCommand::MonitorQueueMove {
            project_root: optional_path(params, "project_root")?,
            expected_queue_order: lookup(params, "expected_queue_order")
                .map(|_| required_u64_vec(params, "expected_queue_order"))
                .transpose()?,
            number: required_u64(params, "number")?,
            position: required_usize(params, "position")?,
        }),
        "issue.monitor.queue.auto_refill" => {
            CliCommand::Issue(IssueCommand::MonitorQueueAutoRefill {
                project_root: optional_path(params, "project_root")?,
                expected_config_token: lookup(params, "expected_config_token")
                    .map(|_| required_string(params, "expected_config_token"))
                    .transpose()?,
                enabled: optional_bool(params, "enabled")?
                    .ok_or(CliParseError::MissingFlag("enabled"))?,
                limit: required_usize(params, "limit")?,
            })
        }
        "issue.monitor.queue.urgent_limit" => {
            CliCommand::Issue(IssueCommand::MonitorQueueUrgentLimit {
                project_root: optional_path(params, "project_root")?,
                expected_config_token: lookup(params, "expected_config_token")
                    .map(|_| required_string(params, "expected_config_token"))
                    .transpose()?,
                limit: required_usize(params, "limit")?,
            })
        }
        "issue.monitor.queue.demote" => CliCommand::Issue(IssueCommand::MonitorQueueDemote {
            project_root: optional_path(params, "project_root")?,
            expected_urgent_queue_token: lookup(params, "expected_urgent_queue_token")
                .map(|_| required_string(params, "expected_urgent_queue_token"))
                .transpose()?,
            number: required_u64(params, "number")?,
        }),
        "discussion.create"
        | "board.config.set"
        | "repository.required_checks.update"
        | "repository.settings.set"
        | "release.metadata.set"
        | "pr.reopen"
        | "pr.review.submit"
        | "pr.auto_merge.enable"
        | "update.settings.set"
        | "pm.approval.request"
        | "pm.approval.status"
        | "pm.approval.revoke"
        | "pm.operation.execute" => {
            params.keys().for_each(|key| param_audit::note(key));
            CliCommand::PmAuthorization(super::pm_authorization::parse(
                &envelope.operation,
                &envelope.params,
            )?)
        }
        "issue.monitor.config.set" | "issue.monitor.config-set" => {
            let expected_config_token = lookup(params, "expected_config_token")
                .map(|_| required_string(params, "expected_config_token"))
                .transpose()?;
            if lookup(params, "auto_apply_updates").is_some() {
                return Err(CliParseError::InvalidJson(
                    "unknown parameter auto_apply_updates; use JSON operation update.settings.set with instruction_id or approval_id (also available in Settings > System)"
                        .to_string(),
                ));
            }
            let allowed_labels = match lookup(params, "allowed_labels") {
                None | Some(Value::Null) => None,
                Some(value) => Some(
                    serde_json::from_value::<Vec<String>>(value.clone()).map_err(|error| {
                        CliParseError::InvalidJson(format!(
                            "allowed_labels must be an array of strings: {error}"
                        ))
                    })?,
                ),
            };
            let enabled = optional_bool(params, "enabled")?;
            let autonomous_mode = optional_bool(params, "autonomous_mode")?;
            let max_active = optional_usize(params, "max_active")?;
            let max_active_auto = match optional_string(params, "max_active_mode")?.as_deref() {
                None => false,
                Some("manual") if max_active.is_some() => false,
                Some("auto") if max_active.is_none() => true,
                Some(_) => return Err(CliParseError::InvalidJson(
                    "max_active_mode must be auto (without max_active) or manual (with max_active)"
                        .to_string(),
                )),
            };
            let auto_close_merged_issues = optional_bool(params, "auto_close_merged_issues")?;
            // Issue #3923 AC-5: the PM's CLI route off a held provider.
            let launch_agent = optional_string(params, "launch_agent")?;
            // Issue #4037 AC-5: the non-destructive update drain.
            let update_drain = optional_update_drain_control(params)?;
            if allowed_labels.is_none()
                && enabled.is_none()
                && autonomous_mode.is_none()
                && max_active.is_none()
                && !max_active_auto
                && auto_close_merged_issues.is_none()
                && launch_agent.is_none()
                && update_drain.is_none()
            {
                return Err(CliParseError::MissingFlag(
                    "allowed_labels|enabled|autonomous_mode|max_active|auto_close_merged_issues|launch_agent|update_drain",
                ));
            }
            // The handler owns the resident-PM caller policy so dispatch can return
            // the normal structured `ok:false` operation envelope. This layer
            // only rejects malformed config values.
            if max_active == Some(0) {
                return Err(CliParseError::InvalidJson(
                    "max_active must be greater than zero".to_string(),
                ));
            }
            CliCommand::Issue(IssueCommand::MonitorConfigSet {
                expected_config_token,
                project_root: optional_path(params, "project_root")?,
                allowed_labels,
                enabled,
                autonomous_mode,
                max_active,
                max_active_auto,
                auto_close_merged_issues,
                launch_agent,
                update_drain,
            })
        }
        "issue.monitor.tiers" => CliCommand::Issue(IssueCommand::MonitorTiers {
            project_root: optional_path(params, "project_root")?,
        }),
        "issue.monitor.tiers.set" => {
            let auto = optional_bool(params, "auto")?.ok_or(CliParseError::MissingFlag("auto"))?;
            let tiers = lookup(params, "tiers")
                .map(|value| {
                    serde_json::from_value::<Vec<Vec<crate::IssueMonitorLaunchProfile>>>(
                        value.clone(),
                    )
                })
                .transpose()
                .map_err(|error| {
                    CliParseError::InvalidJson(format!(
                        "tiers must be an array of profile arrays: {error}"
                    ))
                })?;
            CliCommand::Issue(IssueCommand::MonitorTiersSet {
                project_root: optional_path(params, "project_root")?,
                expected_config_token: lookup(params, "expected_config_token")
                    .map(|_| required_string(params, "expected_config_token"))
                    .transpose()?,
                auto,
                tiers,
            })
        }
        "issue.monitor.tier.set" => {
            let tier = u8::try_from(required_u64(params, "tier")?).map_err(|_| {
                CliParseError::InvalidJson("tier must fit in an unsigned byte".to_string())
            })?;
            CliCommand::Issue(IssueCommand::MonitorTierSet {
                project_root: optional_path(params, "project_root")?,
                expected_config_token: lookup(params, "expected_config_token")
                    .map(|_| required_string(params, "expected_config_token"))
                    .transpose()?,
                number: required_u64(params, "number")?,
                tier,
            })
        }
        "issue.monitor.profiles" => CliCommand::Issue(IssueCommand::MonitorProfiles {
            project_root: optional_path(params, "project_root")?,
        }),
        "issue.monitor.profiles.set" | "issue.monitor.profiles-set" => {
            if lookup(params, "usage_threshold_percent").is_some() {
                return Err(CliParseError::InvalidJson(
                    "usage_threshold_percent is retired because usage percentages no longer control launch selection; omit this parameter"
                        .to_string(),
                ));
            }
            let Some(profiles) = lookup(params, "profiles") else {
                return Err(CliParseError::MissingFlag("profiles"));
            };
            // Issue #4079 AC-3: keep the caller's element shape. Parsing
            // straight into a full profile cannot tell an omitted field from
            // one explicitly cleared, so a reorder reset the pool's settings.
            let profiles = serde_json::from_value::<Vec<crate::IssueMonitorLaunchProfilePatch>>(
                profiles.clone(),
            )
            .map_err(|error| {
                CliParseError::InvalidJson(format!(
                    "profiles must be an array of launch profiles with agent_id: {error}"
                ))
            })?;
            CliCommand::Issue(IssueCommand::MonitorProfilesSet {
                project_root: optional_path(params, "project_root")?,
                expected_profiles_token: lookup(params, "expected_profiles_token")
                    .map(|_| required_string(params, "expected_profiles_token"))
                    .transpose()?,
                profiles,
                usage_threshold_percent: None,
            })
        }
        "pr.current" => CliCommand::Pr(PrCommand::Current),
        "pr.head_check" => {
            reject_unknown_params(params, &["base", "head"], "pr.head_check")?;
            CliCommand::Pr(PrCommand::HeadCheck {
                base: required_string(params, "base")?,
                head: optional_string(params, "head")?,
            })
        }
        "pr.list" => CliCommand::Pr(PrCommand::List {
            stale_after_hours: optional_u64(params, "stale_after_hours")?
                .map(|hours| i64::try_from(hours).unwrap_or(i64::MAX)),
            escalate_after_cycles: optional_u64(params, "escalate_after_cycles")?
                .map(|cycles| u32::try_from(cycles).unwrap_or(u32::MAX)),
            refresh: optional_bool(params, "refresh")?.unwrap_or(false),
            include: params
                .contains_key("include")
                .then(|| parse_pr_inventory_include(&optional_string_vec(params, "include")?))
                .transpose()?,
            force_reason: optional_string(params, "force_reason")?,
        }),
        // Issue #3891 AC-3: GitHub API budget observation.
        "github.budget" => CliCommand::GithubBudget(super::github_budget::GithubBudgetCommand {
            refresh: optional_bool(params, "refresh")?.unwrap_or(false),
        }),
        // Issue #3516: interrupted-release standing check. Read-only unless the
        // caller explicitly opts into opening the missing Release PR.
        "release.status" => {
            reject_unknown_params(
                params,
                &[
                    "release_branch",
                    "base_branch",
                    "scan_commits",
                    "ensure_release_pr",
                ],
                "release.status",
            )?;
            CliCommand::Release(super::release::ReleaseCommand::Status {
                release_branch: optional_string(params, "release_branch")?,
                base_branch: optional_string(params, "base_branch")?,
                scan_commits: optional_u64(params, "scan_commits")?,
                ensure_release_pr: optional_bool(params, "ensure_release_pr")?.unwrap_or(false),
            })
        }
        "release.update.defer" => {
            reject_unknown_params(params, &["version"], "release.update.defer")?;
            CliCommand::Release(super::release::ReleaseCommand::UpdateDefer {
                version: required_string(params, "version")?,
            })
        }
        "pr.create_from_ci" => {
            reject_unknown_params(
                params,
                &["base", "head", "head_sha", "title", "body", "labels"],
                "pr.create_from_ci",
            )?;
            let head_sha = required_string(params, "head_sha")?;
            if !matches!(head_sha.len(), 40 | 64)
                || !head_sha.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(CliParseError::InvalidJson(
                    "head_sha must be a full commit SHA".into(),
                ));
            }
            CliCommand::Pr(PrCommand::CreateFromCi(
                super::pr::ci_delivery::CiCreateRequest {
                    base: required_string(params, "base")?,
                    head: required_string(params, "head")?,
                    head_sha,
                    title: required_string(params, "title")?,
                    body: required_string(params, "body")?,
                    labels: optional_string_vec(params, "labels")?,
                },
            ))
        }
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
            let base = optional_string(params, "base")?;
            if lookup(params, "base")
                .and_then(Value::as_str)
                .is_some_and(|value| value.trim().is_empty())
            {
                return Err(CliParseError::InvalidJson("base must not be empty".into()));
            }
            let title = optional_string(params, "title")?;
            let body = optional_string(params, "body")?;
            let add_labels = optional_string_vec(params, "add_labels")?;
            // Reject nothing-to-update like the argv path's Usage guard; a
            // silent no-op success would mask caller bugs (e.g. sending
            // pr.create's "labels" key instead of "add_labels").
            if base.is_none() && title.is_none() && body.is_none() && add_labels.is_empty() {
                return Err(CliParseError::MissingFlag("base|title|body|add_labels"));
            }
            CliCommand::Pr(PrCommand::EditBody {
                number,
                base,
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
        "pr.close" => CliCommand::Pr(PrCommand::Close {
            number: required_u64(params, "number")?,
            comment: optional_string(params, "comment")?,
        }),
        // SPEC #3835 AC-15 / AC-17: the operation behind the `update-branch`
        // default action, which `pr.list` recommended for a year without one.
        "pr.update_branch" | "pr.update-branch" => CliCommand::Pr(PrCommand::UpdateBranch {
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
                thread_id: lookup(params, "thread_id")
                    .map(|_| required_string(params, "thread_id"))
                    .transpose()?,
            })
        }
        "actions.dispatch" => {
            reject_unknown_params(params, &["workflow", "ref", "inputs"], "actions.dispatch")?;
            let workflow = match lookup(params, "workflow") {
                Some(Value::String(name)) => name.clone(),
                Some(Value::Number(id)) if id.as_u64().is_some() => id.to_string(),
                _ => {
                    return Err(CliParseError::InvalidValue {
                        flag: "workflow",
                        reason: "must be a workflow filename or positive ID",
                    })
                }
            };
            let inputs = match lookup(params, "inputs") {
                None => Map::new(),
                Some(Value::Object(inputs)) => inputs.clone(),
                _ => {
                    return Err(CliParseError::InvalidValue {
                        flag: "inputs",
                        reason: "must be a JSON object",
                    })
                }
            };
            let call = super::ActionsDispatchCall {
                workflow,
                git_ref: required_string(params, "ref")?,
                inputs,
            };
            super::actions::validate_dispatch_call(&call)?;
            CliCommand::Actions(ActionsCommand::Dispatch { call })
        }
        "actions.logs" => CliCommand::Actions(ActionsCommand::Logs {
            run_id: required_u64(params, "run_id")?,
        }),
        "actions.job_logs" | "actions.job-logs" => CliCommand::Actions(ActionsCommand::JobLogs {
            job_id: required_u64(params, "job_id")?,
            // Issue #4849 AC-2: `failed_only` narrows the log to the lines
            // around failure markers; `context_lines` sizes the window.
            failed_only: optional_bool(params, "failed_only")?.unwrap_or(false),
            context_lines: optional_u64(params, "context_lines")?.map_or(
                super::actions::DEFAULT_FAILURE_CONTEXT_LINES,
                super::actions::clamp_failure_context_lines,
            ),
        }),
        "actions.rerun" => CliCommand::Actions(ActionsCommand::Rerun {
            target: actions_rerun_target(params)?,
        }),
        "actions.cancel" => CliCommand::Actions(ActionsCommand::Cancel {
            run_id: required_u64(params, "run_id")?,
        }),
        "actions.queued" => CliCommand::Actions(ActionsCommand::Queued),
        "index.status" => CliCommand::Index(IndexCommand::Status),
        "index.cancel" | "index.repair" => {
            if optional_string(params, "scope")?.is_some_and(|scope| scope != "issues") {
                return Err(CliParseError::InvalidJson(
                    "index recovery supports only the issues scope".to_string(),
                ));
            }
            let wait = optional_bool(params, "wait")?.unwrap_or(false);
            if envelope.operation == "index.cancel" && wait {
                return Err(CliParseError::InvalidJson(
                    "index.cancel does not take wait".to_string(),
                ));
            }
            CliCommand::Index(if envelope.operation == "index.cancel" {
                IndexCommand::Cancel
            } else {
                // Issue #4435: `index.repair` submits and answers; `wait`
                // blocks until the coordinated job settles.
                IndexCommand::Repair { wait }
            })
        }
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
        "hook.register_codex_managed_project_trust"
        | "hook.register-codex-managed-project-trust" => hook_register_codex_project_trust(params)?,
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
        "verify.cancel" => {
            reject_unknown_params(params, &["attempt_id", "reason"], "verify.cancel")?;
            CliCommand::Verify(crate::cli::verification_record::VerifyCommand::Cancel {
                attempt_id: required_string(params, "attempt_id")?,
                reason: required_string(params, "reason")?,
            })
        }
        "verify.status" => {
            reject_unknown_params(params, &["attempt_id"], "verify.status")?;
            CliCommand::Verify(crate::cli::verification_record::VerifyCommand::Status {
                attempt_id: optional_string(params, "attempt_id")?,
            })
        }
        "verify.run" => {
            let commands = optional_string_vec(params, "commands")?;
            let headed_e2e_commands = optional_string_vec(params, "headed_e2e_commands")?;
            if headed_e2e_commands
                .iter()
                .any(|command| !commands.contains(command))
            {
                return Err(CliParseError::InvalidJson(
                    "headed_e2e_commands must name exact entries in commands".to_string(),
                ));
            }
            // Issue #3913: bound on the host admission wait.
            let max_wait_secs = optional_u64(params, "max_wait_secs")?;
            CliCommand::Verify(crate::cli::verification_record::VerifyCommand::Run {
                commands,
                max_wait_secs,
                user_verification_result: optional_string(params, "user_verification_result")?,
                headed_e2e_commands,
            })
        }
        "verify.adjudicate" => {
            reject_unknown_params(
                params,
                &["record_id", "command", "board_entry_id"],
                "verify.adjudicate",
            )?;
            let command = required_string(params, "command")?;
            if command == "*" {
                return Err(CliParseError::InvalidValue {
                    flag: "command",
                    reason: "must identify one exact command, not the whole record",
                });
            }
            CliCommand::Verify(crate::cli::verification_record::VerifyCommand::Adjudicate {
                record_id: required_string(params, "record_id")?,
                command,
                board_entry_id: required_string(params, "board_entry_id")?,
            })
        }
        "verify.plan" => {
            let commands = optional_string_vec(params, "commands")?;
            let derive = optional_bool(params, "derive")?.unwrap_or(false);
            let generated_outputs = optional_string_vec(params, "generated_outputs")?;
            let quarantines = verification_quarantine_requests(params)?;
            let mode = optional_string(params, "mode")?.unwrap_or_else(|| "full".into());
            let acceptance_commands = optional_string_vec(params, "acceptance_commands")?;
            if mode == "pre-pr" {
                if !derive {
                    return Err(CliParseError::InvalidJson(
                        "verify.plan mode pre-pr requires derive:true".into(),
                    ));
                }
                CliCommand::Verify(crate::cli::verification_record::VerifyCommand::PrePrPlan {
                    commands,
                    acceptance_commands,
                    generated_outputs,
                    quarantines,
                })
            } else {
                if mode != "full" || !acceptance_commands.is_empty() {
                    return Err(CliParseError::InvalidJson("verify.plan mode must be full or pre-pr; acceptance_commands requires mode pre-pr".into()));
                }
                if commands.is_empty() && !derive {
                    return Err(CliParseError::MissingFlag("commands"));
                }
                if generated_outputs.is_empty() && quarantines.is_empty() {
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
                            quarantines,
                        },
                    )
                }
            }
        }
        // SPEC #3576: host-wide verification lease.
        "verify.lease.status" | "verify.lease-status" => {
            CliCommand::VerifyLease(VerificationLeaseCommand::Status)
        }
        "verify.lease.acquire" | "verify.lease-acquire" => {
            CliCommand::VerifyLease(VerificationLeaseCommand::Acquire {
                ttl_minutes: optional_u64(params, "ttl_minutes")?
                    .unwrap_or(crate::cli::verification_lease::DEFAULT_TTL_MINUTES),
                reason: optional_string(params, "reason")?,
            })
        }
        "verify.lease.release" | "verify.lease-release" => {
            CliCommand::VerifyLease(VerificationLeaseCommand::Release {
                lease_id: required_string(params, "lease_id")?,
                reason: optional_string(params, "reason")?,
            })
        }
        "verify.lease.extend" | "verify.lease-extend" => {
            CliCommand::VerifyLease(VerificationLeaseCommand::Extend {
                lease_id: required_string(params, "lease_id")?,
                ttl_minutes: optional_u64(params, "ttl_minutes")?
                    .unwrap_or(crate::cli::verification_lease::DEFAULT_TTL_MINUTES),
            })
        }
        "verify.lease.hold" => CliCommand::VerifyLease(VerificationLeaseCommand::Hold {
            ttl_minutes: required_u64(params, "ttl_minutes")?,
            control: std::path::PathBuf::from(required_string(params, "control")?),
            reason: optional_string(params, "reason")?,
        }),
        "execution.status" => {
            // Issue #3934: with no params this stays the caller's own
            // diagnosis; `issue` or `spec` asks about any owner in this
            // repository, which is how an operator finds out who holds a
            // generation that keeps refusing the queue.
            reject_unknown_params(params, &["issue", "spec"], "execution.status")?;
            let issue = optional_u64(params, "issue")?;
            let spec = optional_u64(params, "spec")?;
            match (issue, spec) {
                (Some(_), Some(_)) => {
                    return Err(CliParseError::InvalidJson(
                        "execution.status accepts issue or spec, not both".to_string(),
                    ))
                }
                (Some(number), None) | (None, Some(number)) if number == 0 => {
                    return Err(CliParseError::InvalidJson(
                        "execution.status owner number must be greater than zero".to_string(),
                    ))
                }
                (Some(number), None) => CliCommand::Execution(
                    crate::cli::execution_state::ExecutionCommand::OwnerStatus {
                        owner: crate::cli::execution_state::ExecutionOwnerKey {
                            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                            number,
                        },
                    },
                ),
                (None, Some(number)) => CliCommand::Execution(
                    crate::cli::execution_state::ExecutionCommand::OwnerStatus {
                        owner: crate::cli::execution_state::ExecutionOwnerKey {
                            kind: crate::cli::execution_state::ExecutionOwnerKind::Spec,
                            number,
                        },
                    },
                ),
                (None, None) => {
                    CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::Status)
                }
            }
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
        "execution.no_action" | "execution.no-action" => {
            let reason = required_string(params, "reason")?;
            reject_unknown_params(params, &["reason"], "execution.no_action")?;
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::NoAction {
                reason,
            })
        }
        "execution.settle_delivered" => {
            let reason = required_string(params, "reason")?;
            reject_unknown_params(params, &["reason"], "execution.settle_delivered")?;
            CliCommand::Execution(
                crate::cli::execution_state::ExecutionCommand::SettleDelivered { reason },
            )
        }
        "execution.release_prepared" => {
            // Issue #4161: owner-addressed like `execution.status`, because the
            // Session that left the Prepared fence behind is gone and the
            // operator clearing it is somewhere else in the same repository.
            let issue = optional_u64(params, "issue")?;
            let spec = optional_u64(params, "spec")?;
            let reason = required_string(params, "reason")?;
            let operation_id = optional_string(params, "operation_id")?;
            reject_unknown_params(
                params,
                &["issue", "spec", "reason", "operation_id"],
                "execution.release_prepared",
            )?;
            let owner = match (issue, spec) {
                (Some(_), Some(_)) => {
                    return Err(CliParseError::InvalidJson(
                        "execution.release_prepared accepts issue or spec, not both".to_string(),
                    ))
                }
                (None, None) => {
                    return Err(CliParseError::InvalidJson(
                        "execution.release_prepared requires params.issue or params.spec"
                            .to_string(),
                    ))
                }
                (Some(number), None) | (None, Some(number)) if number == 0 => {
                    return Err(CliParseError::InvalidJson(
                        "execution.release_prepared owner number must be greater than zero"
                            .to_string(),
                    ))
                }
                (Some(number), None) => crate::cli::execution_state::ExecutionOwnerKey {
                    kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                    number,
                },
                (None, Some(number)) => crate::cli::execution_state::ExecutionOwnerKey {
                    kind: crate::cli::execution_state::ExecutionOwnerKind::Spec,
                    number,
                },
            };
            CliCommand::Execution(
                crate::cli::execution_state::ExecutionCommand::ReleasePrepared {
                    owner,
                    operation_id,
                    reason,
                },
            )
        }
        "build.start" => skill_state(params, SkillActionKind::Start).map(CliCommand::Build)?,
        "build.phase" => skill_state(params, SkillActionKind::Phase).map(CliCommand::Build)?,
        "build.complete" => CliCommand::Build(SkillStateAction::Complete {
            spec: required_u64(params, "spec")?,
            pr: optional_u64(params, "pr")?,
        }),
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
        "provider.reset" => {
            reject_unknown_params(params, &["provider", "window_id"], "provider.reset")?;
            CliCommand::ProviderReset(super::provider_reset::ProviderResetCommand::Execute {
                provider: required_string(params, "provider")?,
                window_id: required_string(params, "window_id")?,
            })
        }
        "provider.reset.proposals" => {
            reject_unknown_params(params, &["min_reset_wait_secs"], "provider.reset.proposals")?;
            CliCommand::ProviderReset(super::provider_reset::ProviderResetCommand::Proposals {
                min_reset_wait_secs: optional_u64(params, "min_reset_wait_secs")?.unwrap_or(86400),
            })
        }
        "fleet.inventory" | "fleet.orphans" => {
            reject_unknown_params(params, &[], &envelope.operation)?;
            CliCommand::Fleet(if envelope.operation == "fleet.orphans" {
                super::fleet_disposition::FleetCommand::Orphans
            } else {
                super::fleet_disposition::FleetCommand::Inventory
            })
        }
        "fleet.disposition" | "fleet.orphan.terminate" => {
            reject_unknown_params(params, &["targets", "allow_unknown"], &envelope.operation)?;
            let targets =
                super::fleet_disposition::parse_targets(lookup(params, "targets").ok_or_else(
                    || CliParseError::InvalidJson("fleet targets are required".into()),
                )?)?;
            if envelope.operation == "fleet.orphan.terminate"
                && targets
                    .iter()
                    .any(|target| !target.id.starts_with("process:"))
            {
                return Err(CliParseError::InvalidJson(
                    "orphan termination accepts only explicit process targets".into(),
                ));
            }
            CliCommand::Fleet(super::fleet_disposition::FleetCommand::Disposition {
                targets,
                allow_unknown: optional_bool(params, "allow_unknown")?.unwrap_or(false),
                orphan_only: envelope.operation == "fleet.orphan.terminate",
            })
        }
        "pane.list" => CliCommand::Pane(PaneCommand::List),
        "pane.health" => {
            reject_unknown_params(params, &["id"], "pane.health")?;
            CliCommand::Pane(PaneCommand::Health {
                id: optional_string(params, "id")?,
            })
        }
        "pane.name" => {
            reject_unknown_params(params, &["candidate"], "pane.name")?;
            CliCommand::Pane(PaneCommand::Name {
                candidate: optional_string(params, "candidate")?.unwrap_or_default(),
            })
        }
        "pane.read" => CliCommand::Pane(PaneCommand::Read {
            id: required_string(params, "id")?,
            lines: optional_usize(params, "lines")?.unwrap_or(200),
        }),
        "pane.close" | "pane.stop" => CliCommand::Pane(PaneCommand::Close {
            id: required_string(params, "id")?,
        }),
        "pane.recover" => {
            reject_unknown_params(
                params,
                &["started_after", "started_before", "apply"],
                "pane.recover",
            )?;
            let started_after = required_string(params, "started_after")?;
            let started_before = required_string(params, "started_before")?;
            super::pane::parse_recovery_bounds(&started_after, &started_before)
                .map_err(CliParseError::InvalidJson)?;
            CliCommand::Pane(PaneCommand::Recover {
                started_after,
                started_before,
                apply: optional_bool(params, "apply")?.unwrap_or(false),
            })
        }
        "pane.send" => CliCommand::Pane(PaneCommand::Send {
            id: optional_string(params, "id")?,
            text: required_string(params, "text")?,
        }),
        "pm.message.send" | "pm.pane.send" => {
            // An explicitly supplied identity must never become a fresh delivery.
            let operation_id = match lookup(params, "operation_id") {
                None => None,
                Some(Value::String(value))
                    if uuid::Uuid::parse_str(value)
                        .is_ok_and(|id| id.hyphenated().to_string() == value.as_str()) =>
                {
                    Some(value.clone())
                }
                Some(_) => return Err(CliParseError::InvalidValue {
                    flag: "operation_id",
                    reason:
                        "must be a canonical hyphenated UUID exactly as reported by pm.message.send",
                }),
            };
            CliCommand::Pane(PaneCommand::PmSend {
                project_root: optional_string(params, "project_root")?,
                id: required_string(params, "id")?,
                text: required_string(params, "text")?,
                operation_id,
            })
        }
        "pm.capabilities" => CliCommand::Pm(crate::cli::pm::PmCommand::Capabilities),
        "pm.plan.status" => {
            reject_unknown_params(params, &[], "pm.plan.status")?;
            CliCommand::Pm(crate::cli::pm::PmCommand::PlanStatus)
        }
        "pm.plan.apply" => {
            reject_unknown_params(
                params,
                &["action", "expected_revision", "reason", "idempotency_key"],
                "pm.plan.apply",
            )?;
            let request: crate::pm_planning::PlanningApplyRequest =
                serde_json::from_value(Value::Object(params.clone()))
                    .map_err(|error| CliParseError::InvalidJson(error.to_string()))?;
            request
                .validate()
                .map_err(|error| CliParseError::InvalidJson(error.to_string()))?;
            CliCommand::Pm(crate::cli::pm::PmCommand::PlanApply {
                request: Box::new(request),
            })
        }
        "pm.peer.send" => {
            reject_unknown_params(
                params,
                &["project", "kind", "text", "reply_to", "timeout_seconds"],
                "pm.peer.send",
            )?;
            let project = required_string(params, "project")?;
            let kind = serde_json::from_value::<gwt_core::pm_peer::PeerKind>(Value::String(
                required_string(params, "kind")?,
            ))
            .map_err(|_| {
                CliParseError::InvalidJson("PM peer kind must be request, reply, or notice".into())
            })?;
            let text = required_string(params, "text")?;
            let reply_to = optional_string(params, "reply_to")?;
            let timeout_seconds = optional_u64(params, "timeout_seconds")?.unwrap_or(900);
            gwt_core::pm_peer::validate(kind, &text, reply_to.as_deref(), timeout_seconds)
                .map_err(|error| CliParseError::InvalidJson(error.to_string()))?;
            CliCommand::Pm(crate::cli::pm::PmCommand::PeerSend {
                project,
                kind,
                text,
                reply_to,
                timeout_seconds,
            })
        }
        "pm.report.post" => {
            let kind: gwt_core::pm_report::PmReportKind = serde_json::from_value(Value::String(
                required_string(params, "kind")?,
            ))
            .map_err(|_| CliParseError::InvalidValue {
                flag: "kind",
                reason: "expected one of: progress, decision, blocker, qa",
            })?;
            let question = if kind == gwt_core::pm_report::PmReportKind::Qa {
                Some(required_string(params, "question")?)
            } else {
                optional_string(params, "question")?
            };
            let instruction = lookup(params, "instruction")
                .map(|value| {
                    serde_json::from_value::<gwt_core::user_instruction::UserInstructionInput>(
                        value.clone(),
                    )
                    .map(Box::new)
                    .map_err(|error| {
                        CliParseError::InvalidJson(format!("invalid instruction capture: {error}"))
                    })
                })
                .transpose()?;
            if instruction.is_some() && kind != gwt_core::pm_report::PmReportKind::Qa {
                return Err(CliParseError::InvalidValue {
                    flag: "instruction",
                    reason: "requires kind: qa",
                });
            }
            CliCommand::Pm(crate::cli::pm::PmCommand::ReportPost {
                project_root: optional_string(params, "project_root")?,
                kind,
                question,
                body: required_string(params, "body")?,
                instruction,
            })
        }
        "pm.report.list" => CliCommand::Pm(crate::cli::pm::PmCommand::ReportList {
            project_root: optional_string(params, "project_root")?,
        }),
        "pm.status" => CliCommand::Pm(crate::cli::pm::PmCommand::Status {
            project_root: optional_string(params, "project_root")?,
        }),
        "pm.status.set" => {
            reject_unknown_params(
                params,
                &[
                    "project_root",
                    "items",
                    "next_trigger",
                    "user_action",
                    "stale_after_minutes",
                ],
                "pm.status.set",
            )?;
            let mut content = params.clone();
            content.remove("project_root");
            let input: gwt_core::pm_status::PmStatusInput =
                serde_json::from_value(Value::Object(content))
                    .map_err(|error| CliParseError::InvalidJson(error.to_string()))?;
            input
                .validate()
                .map_err(|error| CliParseError::InvalidJson(error.to_string()))?;
            CliCommand::Pm(crate::cli::pm::PmCommand::StatusSet {
                project_root: optional_string(params, "project_root")?,
                input,
            })
        }
        "pm.pause" | "pm.resume" => CliCommand::Pm(crate::cli::pm::PmCommand::SetPaused {
            project_root: optional_string(params, "project_root")?,
            paused: envelope.operation == "pm.pause",
        }),
        "pm.stop" | "pm.deregister" => CliCommand::Pm(crate::cli::pm::PmCommand::Stop {
            project_root: optional_string(params, "project_root")?,
            session_id: optional_string(params, "session_id")?,
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
        "errors.list" => errors_list(params)?,
        "perf.summary" => perf_read(params, "perf.summary")?,
        "perf.startup" => {
            reject_unknown_params(params, &[], "perf.startup")?;
            CliCommand::Perf(PerfCommand::Startup)
        }
        "perf.violations" => perf_read(params, "perf.violations")?,
        other => {
            return Err(CliParseError::UnknownSubcommand(other.to_string()));
        }
    };
    // Matched on the operation first so `reason` is only consulted where it
    // means something. Reading it for every operation would mark it accepted
    // everywhere and hide a misplaced `reason` from the audit below.
    let declared_block = match envelope.operation.as_str() {
        "execution.blocked" => match optional_string(params, "reason") {
            Ok(Some(reason)) => Some(DeclaredBlock {
                reason,
                missing_verification: optional_string(params, "missing_verification")
                    .ok()
                    .flatten(),
            }),
            _ => None,
        },
        _ => None,
    };
    reject_unconsulted_params(params, &recording.consulted(), &envelope.operation)?;
    let requested_project_root = params
        .get("project_root")
        .and_then(Value::as_str)
        .filter(|root| !root.trim().is_empty())
        .map(str::to_owned);
    Ok(ParsedEnvelope {
        operation: envelope.operation,
        command,
        requested_project_root,
        declared_block,
    })
}

fn issue_label(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    reject_unknown_params(
        params,
        &[
            "number",
            "action",
            "labels",
            "confirm_queue",
            "confirm_design_gate",
            "confirm_auto_merge",
        ],
        "issue.label",
    )?;
    let action = match required_string(params, "action")?.as_str() {
        "add" => IssueLabelAction::Add,
        "remove" => IssueLabelAction::Remove,
        _ => {
            return Err(CliParseError::InvalidValue {
                flag: "action",
                reason: "expected add or remove",
            })
        }
    };
    let labels = optional_string_vec(params, "labels")?;
    if labels.is_empty() {
        return Err(CliParseError::MissingFlag("labels"));
    }
    if matches!(action, IssueLabelAction::Remove) && labels.len() != 1 {
        return Err(CliParseError::InvalidValue {
            flag: "labels",
            reason: "remove accepts exactly one label",
        });
    }
    Ok(CliCommand::Issue(IssueCommand::Label {
        number: required_u64(params, "number")?,
        action,
        labels,
        confirm_queue: optional_bool(params, "confirm_queue")?.unwrap_or(false),
        confirm_design_gate: optional_bool(params, "confirm_design_gate")?.unwrap_or(false),
        confirm_auto_merge: optional_bool(params, "confirm_auto_merge")?.unwrap_or(false),
    }))
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
        // Both spellings are read unconditionally so the alias is never
        // mistaken for an unknown parameter (Issue #4581 AC-3).
        workspace_id: optional_string(params, "workspace_id")?
            .or(optional_string(params, "workspace")?)
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
    reject_unknown_params(
        params,
        &["workspace", "all", "limit", "unresolved"],
        "board.show",
    )?;
    Ok(CliCommand::Board(BoardCommand::Show {
        json: true,
        workspace: optional_string(params, "workspace")?,
        all: optional_bool(params, "all")?.unwrap_or(false),
        limit: optional_usize(params, "limit")?,
        unresolved: optional_bool(params, "unresolved")?.unwrap_or(false),
    }))
}

fn board_post(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    reject_key(params, "purpose", "board.post must not update Work purpose")?;
    reject_key(
        params,
        "title_summary",
        "board.post must not update agent title_summary",
    )?;
    for key in [
        "session_id",
        "project_id",
        "provider",
        "worktree_form",
        "recovery_id",
        "entry_id",
        "payload_digest",
        "provider_receipt",
        "operation_id",
    ] {
        reject_key(
            params,
            key,
            "untrusted recovery identity field is not accepted by public board.post",
        )?;
    }
    let intent_id = optional_string(params, "intent_id")?;
    let command = Box::new(BoardPostCommand {
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
        resolves: optional_string_or_string_vec(params, "resolves")?,
        broadcast: optional_bool(params, "broadcast")?.unwrap_or(false),
    });
    Ok(CliCommand::Board(match intent_id {
        Some(intent_id) => BoardCommand::RecoveryPost { intent_id, command },
        None => BoardCommand::Post(command),
    }))
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
    reject_unknown_params(
        params,
        &["channels", "project_root", "timeout_seconds"],
        "daemon.subscribe",
    )?;
    let channels = optional_string_vec(params, "channels")?;
    if channels.is_empty() {
        return Err(CliParseError::MissingFlag("channels"));
    }
    let project_root = optional_path(params, "project_root")?;
    let timeout_seconds = optional_u64(params, "timeout_seconds")?;
    if timeout_seconds == Some(0) {
        return Err(CliParseError::InvalidValue {
            flag: "timeout_seconds",
            reason: "must be at least 1 second",
        });
    }
    Ok(CliCommand::Daemon(DaemonCommand::Subscribe {
        channels,
        project_root,
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

fn hook_register_codex_project_trust(
    params: &Map<String, Value>,
) -> Result<CliCommand, CliParseError> {
    let mut rest = Vec::new();
    if let Some(project_root) = optional_string(params, "project_root")? {
        rest.push("--project-root".to_string());
        rest.push(project_root);
    }
    if let Some(codex_config) = optional_string(params, "codex_config")? {
        rest.push("--codex-config".to_string());
        rest.push(codex_config);
    }
    Ok(CliCommand::Hook(HookCommand::Run {
        name: "register-codex-managed-project-trust".to_string(),
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
    let target = if matches!(
        &action,
        DiscussEnvelopeAction::Resolve
            | DiscussEnvelopeAction::Park
            | DiscussEnvelopeAction::Reject
    ) {
        let title = optional_string(params, "title")?;
        let origin_session = optional_string(params, "origin_session")?;
        if origin_session.is_some() && title.is_none() {
            return Err(CliParseError::MissingFlag("title"));
        }
        for (flag, value) in [
            ("title", title.as_deref()),
            ("origin_session", origin_session.as_deref()),
        ] {
            if lookup(params, flag).is_some() && value.is_none() {
                return Err(CliParseError::InvalidValue {
                    flag,
                    reason: "must not be empty",
                });
            }
        }
        title.map(|title| crate::discussion_resume::ProposalTarget {
            title,
            origin_session,
        })
    } else {
        None
    };
    let action = match action {
        DiscussEnvelopeAction::Resolve => super::DiscussAction::Resolve { proposal, target },
        DiscussEnvelopeAction::Park => super::DiscussAction::Park { proposal, target },
        DiscussEnvelopeAction::Reject => super::DiscussAction::Reject { proposal, target },
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
        SkillActionKind::Complete => Ok(SkillStateAction::Complete { spec, pr: None }),
        SkillActionKind::Abort => Ok(SkillStateAction::Abort {
            spec,
            reason: optional_string(params, "reason")?,
        }),
    }
}

fn errors_list(params: &Map<String, Value>) -> Result<CliCommand, CliParseError> {
    reject_unknown_params(params, &["since", "scope", "project_root"], "errors.list")?;
    let since = optional_string(params, "since")?;
    if let Some(raw) = since.as_deref() {
        super::diagnostics::errors::parse_since(raw)?;
    }
    let scope = optional_string(params, "scope")?
        .as_deref()
        .map(super::diagnostics::errors::ErrorListScope::parse)
        .transpose()?
        .unwrap_or_default();
    let project_root = optional_string(params, "project_root")?;
    Ok(CliCommand::Diagnostics(DiagnosticsCommand::ErrorsList {
        since,
        scope,
        project_root,
    }))
}

/// SPEC #3700 FR-007: `perf.summary` and `perf.violations` share one filter
/// shape, so they share one parser keyed by the operation name.
fn perf_read(params: &Map<String, Value>, operation: &str) -> Result<CliCommand, CliParseError> {
    reject_unknown_params(params, &["since", "stream", "target"], operation)?;
    let since = optional_string(params, "since")?;
    if let Some(raw) = since.as_deref() {
        super::perf::parse_since(raw)?;
    }
    let stream = optional_string(params, "stream")?
        .map(|raw| super::perf::parse_stream(&raw))
        .transpose()?;
    let target = optional_string(params, "target")?;

    Ok(CliCommand::Perf(if operation == "perf.violations" {
        PerfCommand::Violations {
            since,
            stream,
            target,
        }
    } else {
        PerfCommand::Summary {
            since,
            stream,
            target,
        }
    }))
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
            "reports" => Ok(IndexSearchScope::Reports),
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
        "works" => Ok(IndexScope::Works),
        "reports" => Ok(IndexScope::Reports),
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

fn issue_owner_check(
    params: &Map<String, Value>,
) -> Result<Option<super::hook::pm_spec_survey::IssueOwnerCheck>, CliParseError> {
    let Some(value) = lookup(params, "owner_check").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let evidence: super::hook::pm_spec_survey::IssueOwnerCheck =
        serde_json::from_value(value.clone()).map_err(|error| {
            CliParseError::InvalidJson(format!(
                "invalid owner_check: {error}. {}",
                super::hook::pm_spec_survey::OWNER_CHECK_RECOVERY
            ))
        })?;
    evidence.validate().map_err(CliParseError::InvalidJson)?;
    Ok(Some(evidence))
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
    // Not recorded as consulted: a key this operation refuses outright must
    // not turn up in the "accepted" list of an unknown-parameter refusal.
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
    let Some(value) = lookup(params, key) else {
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
    let Some(value) = lookup(params, key) else {
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

/// Refuse a parameter this operation does not understand.
///
/// Only for operations where ignoring an unrecognised key would change what
/// the caller believes it authorized — a mistyped `project_root` that silently
/// falls back to the process cwd is exactly the failure #3524 recorded.
fn reject_unknown_params(
    params: &Map<String, Value>,
    allowed: &[&str],
    operation: &str,
) -> Result<(), CliParseError> {
    for key in allowed {
        param_audit::note(key);
    }
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

/// Issue #3515: `actions.rerun` takes `run_id` **or** `job_id`, never both.
/// `failed_only` narrows a run rerun to its failed jobs and is meaningless for
/// a job target, so passing it there is rejected instead of silently ignored.
fn actions_rerun_target(
    params: &Map<String, Value>,
) -> Result<crate::cli::ActionsRerunTarget, CliParseError> {
    let job_id = optional_u64(params, "job_id")?;
    let run_id = optional_u64(params, "run_id")?;
    match (run_id, job_id) {
        (Some(_), Some(_)) => Err(CliParseError::InvalidValue {
            flag: "job_id",
            reason: "run_id and job_id are mutually exclusive",
        }),
        (None, Some(job_id)) => {
            if optional_bool(params, "failed_only")?.is_some() {
                return Err(CliParseError::InvalidValue {
                    flag: "failed_only",
                    reason: "only applies to a run_id target",
                });
            }
            Ok(crate::cli::ActionsRerunTarget::Job { job_id })
        }
        (run_id, None) => Ok(crate::cli::ActionsRerunTarget::Run {
            run_id: run_id.ok_or(CliParseError::MissingFlag("run_id"))?,
            failed_only: optional_bool(params, "failed_only")?.unwrap_or(false),
        }),
    }
}

fn required_u64(params: &Map<String, Value>, key: &'static str) -> Result<u64, CliParseError> {
    optional_u64(params, key)?.ok_or(CliParseError::MissingFlag(key))
}

fn optional_u64(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<u64>, CliParseError> {
    let Some(value) = lookup(params, key) else {
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

fn required_usize(params: &Map<String, Value>, key: &'static str) -> Result<usize, CliParseError> {
    optional_usize(params, key)?.ok_or(CliParseError::MissingFlag(key))
}

fn optional_u64_vec(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<Vec<u64>, CliParseError> {
    let Some(value) = lookup(params, key) else {
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

fn issue_monitor_priority_position(
    params: &Map<String, Value>,
) -> Result<super::IssueMonitorPriorityPosition, CliParseError> {
    let Some(value) = lookup(params, "position") else {
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

/// SPEC #4249 FR-001: `reason` is the optional GitHub `state_reason` of a
/// close. An unrecognised spelling is refused rather than silently dropped,
/// because a dropped reason closes the Issue with the wrong rationale.
fn issue_close_reason(
    params: &Map<String, Value>,
) -> Result<Option<gwt_github::client::IssueCloseReason>, CliParseError> {
    let Some(raw) = optional_string(params, "reason")? else {
        return Ok(None);
    };
    gwt_github::client::IssueCloseReason::parse(&raw)
        .map(Some)
        .ok_or_else(|| {
            CliParseError::InvalidJson(format!(
                "reason must be one of {:?}",
                gwt_github::client::IssueCloseReason::ACCEPTED
            ))
        })
}

/// Issue #4037 AC-5 / #3906 AC-3: `update_drain` is the operator bool or the
/// auto-drain object `{reason, version}`.
fn optional_update_drain_control(
    params: &Map<String, Value>,
) -> Result<Option<crate::IssueMonitorUpdateDrainControl>, CliParseError> {
    match lookup(params, "update_drain") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone()).map(Some).map_err(|_| {
            CliParseError::InvalidJson(
                "update_drain must be a bool or {\"reason\":\"auto\"|\"manual\",\"version\":\"x.y.z\"}"
                    .to_string(),
            )
        }),
    }
}

fn optional_bool(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<bool>, CliParseError> {
    let Some(value) = lookup(params, key) else {
        return Ok(None);
    };
    match value {
        Value::Bool(value) => Ok(Some(*value)),
        Value::Null => Ok(None),
        _ => Err(CliParseError::InvalidJson(format!("{key} must be a bool"))),
    }
}

/// Issue #3891 AC-2: `params.include` names the heavy `pr.list` fields to
/// hydrate. An empty list is the bare light inventory.
fn parse_pr_inventory_include(
    names: &[String],
) -> Result<gwt_git::PrInventoryInclude, CliParseError> {
    let mut include = gwt_git::PrInventoryInclude {
        checks: false,
        body: false,
    };
    for name in names {
        match name.as_str() {
            "checks" => include.checks = true,
            "body" => include.body = true,
            _ => {
                return Err(CliParseError::InvalidValue {
                    flag: "include",
                    reason: "expected \"checks\" and/or \"body\"",
                })
            }
        }
    }
    Ok(include)
}

fn optional_string_vec(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<Vec<String>, CliParseError> {
    let Some(value) = lookup(params, key) else {
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

/// Like [`optional_string_vec`], but a bare string is accepted as a one-element
/// list. Used where the single-item case dominates (Issue #3655 `resolves`).
fn optional_string_or_string_vec(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<Vec<String>, CliParseError> {
    match lookup(params, key) {
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(vec![text.clone()]),
        Some(Value::String(_)) => Err(CliParseError::InvalidJson(format!(
            "{key} must not be an empty string"
        ))),
        _ => optional_string_vec(params, key),
    }
}

fn optional_json_array(
    params: &Map<String, Value>,
    key: &'static str,
) -> Result<Vec<Value>, CliParseError> {
    let Some(value) = lookup(params, key) else {
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

fn verification_quarantine_requests(
    params: &Map<String, Value>,
) -> Result<Vec<crate::cli::verification_record::VerificationQuarantineRequest>, CliParseError> {
    let requests = optional_json_array(params, "quarantines")?
        .into_iter()
        .map(|value| {
            serde_json::from_value::<
                crate::cli::verification_record::VerificationQuarantineRequest,
            >(value)
            .map_err(|error| {
                CliParseError::InvalidJson(format!(
                    "quarantines must contain typed quarantine requests: {error}"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut identities = std::collections::HashSet::new();
    for request in &requests {
        request.validate().map_err(CliParseError::InvalidJson)?;
        if !identities.insert((
            request.failed_command.as_str(),
            request.test_identity.as_str(),
        )) {
            return Err(CliParseError::InvalidJson(
                "quarantines must not contain duplicate failed_command/test_identity pairs"
                    .to_string(),
            ));
        }
    }
    Ok(requests)
}

#[cfg(test)]
mod tests {
    #[test]
    fn feedback_create_accepts_structured_evidence() {
        let input = serde_json::json!({"schema_version":1,"operation":"feedback.create","params":{
            "title":"Host bridge refusal", "kind":"bug", "operation":"workspace.update",
            "refusal":"  exact\r\nrefusal  ", "reproduction":"unconfirmed", "workaround":null
        }});
        assert!(
            super::parse(&input.to_string()).is_ok(),
            "feedback.create must accept a structured report"
        );
        for forbidden in ["approved", "repo", "project_root"] {
            let mut forged = input.clone();
            forged["params"][forbidden] = serde_json::json!(true);
            assert!(super::parse(&forged.to_string()).is_err());
        }
    }

    use super::{
        parse, report_undelivered_response, write_response, ActionsCommand, CliCommand,
        CliParseError, DaemonCommand, DiagnosticsCommand, HookCommand, IndexCommand, IndexScope,
        IssueCommand, PaneCommand, PerfCommand, PrCommand, SkillStateAction, WorkflowBypassMode,
        WorkflowCommand, WorkspaceCommand, RESPONSE_NOT_DELIVERED_EXIT,
    };
    use crate::cli::verification_lease::VerificationLeaseCommand;
    use crate::cli::{dispatch_json_input, TestEnv};
    use crate::cli::{BoardCommand, IssueMonitorPriorityPosition};
    use crate::protocol::{IndexSearchMatchMode, IndexSearchScope};
    use serde_json::{json, Value};

    #[test]
    fn provided_json_input_keeps_envelope_validation_and_response_without_rereading_stdin() {
        let temp = tempfile::tempdir().unwrap();
        let mut env = TestEnv::new(temp.path().to_path_buf());
        env.stdin = "stdin was already read".to_string();

        for input in [
            "{",
            r#"{"schema_version":1,"operation":"search","params":{"query":"admission","project_root":"/elsewhere"}}"#,
        ] {
            assert_eq!(dispatch_json_input(&mut env, "gwtd", input), 2);
            assert!(env.stdout.is_empty());
            assert!(String::from_utf8_lossy(&env.stderr).starts_with("gwtd: "));
            env.stderr.clear();
        }

        env.seed_run_log(273, "run log from provided JSON");
        assert_eq!(
            dispatch_json_input(
                &mut env,
                "gwtd",
                r#"{"schema_version":1,"operation":"actions.logs","params":{"run_id":273}}"#,
            ),
            0
        );
        let payload: serde_json::Value = serde_json::from_slice(&env.stdout).unwrap();
        assert_eq!(payload["operation"], "actions.logs");
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["exit_code"], 0);
        assert_eq!(payload["output"], "run log from provided JSON\n");
        assert_eq!(env.run_log_call_log, vec![273]);
        assert_eq!(env.stdin, "stdin was already read");
    }

    #[test]
    fn provider_reset_operations_never_accept_caller_supplied_approval() {
        let request = json!({"provider":"codex", "window_id":"tab-1::agent-2"});
        assert!(parse(&envelope("provider.reset", request.clone())).is_ok());
        for key in ["approved", "confirmed", "approval_token", "autonomous_mode"] {
            let mut supplied = request.clone();
            supplied[key] = json!(true);
            assert!(matches!(
                err("provider.reset", supplied),
                CliParseError::InvalidJson(_)
            ));
        }
        assert!(parse(&envelope(
            "provider.reset.proposals",
            json!({"min_reset_wait_secs":3600})
        ))
        .is_ok());
        assert!(parse(&envelope("provider.reset", json!({"provider":"codex"}))).is_err());
    }

    fn envelope(operation: &str, params: Value) -> String {
        json!({
            "schema_version": 1,
            "operation": operation,
            "params": params,
        })
        .to_string()
    }

    #[test]
    fn pm_delegated_operation_executes_authorized_cleanup_dry_run() {
        use gwt_core::pm_authorization::{self, OperationIntent};
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let _session = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_ID_ENV,
            "pm-delegated-test",
        );
        let repo = home.path().join("repo");
        std::fs::create_dir_all(repo.join("target")).unwrap();
        std::fs::write(repo.join("target/keep"), "build cache").unwrap();
        assert!(gwt_core::process::hidden_command("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .status
            .success());
        let prefs = serde_json::from_value(json!({"registration":{
            "session_id":"pm-delegated-test", "agent_id":"codex", "worktree_path":repo,
        }}))
        .unwrap();
        crate::pm_registry::save_pm_prefs(
            &crate::pm_registry::pm_prefs_path_for_repo_path(&repo),
            &prefs,
        )
        .unwrap();
        let intent = OperationIntent {
            operation: "worktree.gc_build_artifacts".into(),
            params: json!({"dry_run":true,"include_unmerged":true}),
        };
        let approval = pm_authorization::record_approval(
            &repo,
            &intent,
            "Inspect build caches",
            "No deletion in dry run",
            false,
        )
        .unwrap();
        let mut env = TestEnv::new(repo.clone());
        env.stdin = envelope(
            "pm.operation.execute",
            json!({
                "operation":intent.operation,"params":intent.params,"approval_id":approval.id,
            }),
        );
        assert_eq!(
            super::dispatch(&mut env, "gwtd"),
            0,
            "{}",
            String::from_utf8_lossy(&env.stderr)
        );
        let reply: Value = serde_json::from_slice(&env.stdout).unwrap();
        let result: Value = serde_json::from_str(reply["output"].as_str().unwrap()).unwrap();
        assert_eq!(result["after"]["operation"], "worktree.gc_build_artifacts");
        assert_eq!(result["after"]["ok"], true);
        let report: Value =
            serde_json::from_str(result["after"]["output"].as_str().unwrap()).unwrap();
        assert_eq!(report["dry_run"], true);
        assert!(repo.join("target/keep").exists());
        assert!(pm_authorization::approvals(&repo).unwrap()[0].consumed);
        assert!(std::fs::read_to_string(pm_authorization::audit_path(&repo))
            .unwrap()
            .contains("succeeded"));
    }

    #[test]
    fn pm_delegated_operation_propagates_inner_refusal() {
        use gwt_core::pm_authorization::{self, OperationIntent};
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let _session = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_ID_ENV,
            "pm-delegated-test",
        );
        let _pane = gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let prefs = serde_json::from_value(json!({"registration":{
            "session_id":"pm-delegated-test", "agent_id":"codex", "worktree_path":repo,
        }}))
        .unwrap();
        crate::pm_registry::save_pm_prefs(
            &crate::pm_registry::pm_prefs_path_for_repo_path(&repo),
            &prefs,
        )
        .unwrap();
        let intent = OperationIntent {
            operation: "pm.message.send".into(),
            params: json!({"id":"fixture-agent","text":"Approved exact action"}),
        };
        let approval = pm_authorization::record_approval(
            &repo,
            &intent,
            "Forward approved action",
            "No input if target is unavailable",
            false,
        )
        .unwrap();
        let mut env = TestEnv::new(repo.clone());
        env.stdin = envelope(
            "pm.operation.execute",
            json!({
                "operation":intent.operation,"params":intent.params,"approval_id":approval.id,
            }),
        );
        assert_eq!(super::dispatch(&mut env, "gwtd"), 1);
        let reply: Value = serde_json::from_slice(&env.stdout).unwrap();
        assert_eq!(reply["ok"], false);
        let result: Value = serde_json::from_str(reply["output"].as_str().unwrap()).unwrap();
        assert_eq!(result["after"]["ok"], false);
        assert_eq!(result["after"]["exit_code"], 1);
        assert!(result["after"]["error"]
            .as_str()
            .unwrap()
            .contains(gwt_agent::GWT_PANE_WS_URL_ENV));
        assert!(std::fs::read_to_string(pm_authorization::audit_path(&repo))
            .unwrap()
            .contains("failed"));
    }

    #[test]
    fn pm_delegated_operation_rejects_other_operations_and_foreign_targets() {
        for params in [
            json!({"operation":"actions.dispatch","params":{"workflow":"release.yml"}}),
            json!({"operation":"pm.operation.execute","params":{}}),
            json!({"operation":"pm.message.send","params":{"id":"agent","text":"yes","project_root":"/other"}}),
            json!({"operation":"pr.close","params":{"number":7,"repo":"other/repo"}}),
        ] {
            assert!(parse(&envelope("pm.operation.execute", params)).is_err());
        }
        assert!(parse(&envelope(
            "pm.operation.execute",
            json!({
                "operation":"pr.close","params":{"number":7},"approval_id":"scoped-id"
            })
        ))
        .is_ok());
    }

    #[test]
    fn issue_4323_view_envelopes_identify_the_resource_kind() {
        let repo = tempfile::tempdir().unwrap();
        let mut env = TestEnv::new(repo.path().to_path_buf());
        env.client.seed(gwt_github::client::IssueSnapshot {
            author_login: None,
            number: gwt_github::client::IssueNumber(42),
            title: "typed resource".into(),
            body: "".into(),
            labels: vec![],
            state: gwt_github::client::IssueState::Open,
            updated_at: gwt_github::client::UpdatedAt::new("2026-10-15T00:00:00Z"),
            comments: vec![],
        });
        env.seed_pr(7, gwt_git::pr_status::parse_pr_status_json(
            r#"{"number":7,"title":"typed resource","url":"https://github.com/octo/gwt/pull/7","state":"OPEN","isDraft":false,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN"}"#
        ).unwrap());
        for (operation, number, kind, header) in [
            ("issue.view", 42, "issue", "Issue #42"),
            ("pr.view", 7, "pull_request", "PR #7"),
        ] {
            env.stdin = envelope(operation, json!({"number":number}));
            env.stdout.clear();
            assert_eq!(super::dispatch(&mut env, "gwtd"), 0);
            let response: Value = serde_json::from_slice(&env.stdout).unwrap();
            assert_eq!(response["kind"], kind);
            assert!(response["output"].as_str().unwrap().starts_with(header));
        }
    }

    #[test]
    fn issue_5108_pr_checks_envelope_exposes_same_structured_verdict() {
        let repo = tempfile::tempdir().unwrap();
        let mut env = TestEnv::new(repo.path().to_path_buf());
        env.seed_pr_checks(12, serde_json::from_value(json!({
            "summary": "PR #12 | CI: PENDING (1 unfinished) | Merge: BLOCKED | Review: APPROVED",
            "ci_status": "PENDING", "merge_status": "BLOCKED", "review_status": "APPROVED",
            "check_counts": {"success":1,"failure":0,"skipped":0,"in_progress":1,"total":2},
            "checks": []
        })).unwrap());
        env.stdin = envelope("pr.checks", json!({"number":12}));
        assert_eq!(super::dispatch(&mut env, "gwtd"), 0);
        let response: Value = serde_json::from_slice(&env.stdout).unwrap();
        assert_eq!(response["data"]["ci_status"], "PENDING");
        assert_eq!(response["data"]["merge_status"], "BLOCKED");
        assert_eq!(response["data"]["check_counts"]["in_progress"], 1);
        assert!(response["output"]
            .as_str()
            .unwrap()
            .contains("CI: PENDING (1 unfinished)"));
        assert_eq!(env.pr_checks_call_log, vec![12]);
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

    fn concern_call(env: &mut TestEnv, operation: &str, params: Value) -> Value {
        env.stdout.clear();
        env.stderr.clear();
        env.stdin = envelope(operation, params);
        let code = super::dispatch(env, "gwtd");
        assert_eq!(
            code,
            0,
            "{} {}",
            String::from_utf8_lossy(&env.stdout),
            String::from_utf8_lossy(&env.stderr)
        );
        let response: Value = serde_json::from_slice(&env.stdout).unwrap();
        serde_json::from_str(response["output"].as_str().unwrap()).unwrap()
    }

    fn concern_create_params() -> Value {
        json!({
            "summary": "Unnecessary windows are restored",
            "symptom_measurement": {"kind": "shell_command", "command": "printf '{\"count\":3}'"},
            "baseline": {"count": 3},
            "verification_predicate": {"pointer": "/count", "op": "eq", "expected": 0},
            "owner_issues": [4059]
        })
    }

    #[test]
    fn concern_list_is_read_only_but_measurements_and_resolutions_are_mutations() {
        use crate::cli::hook::workflow_policy::is_read_only_json_envelope_operation;
        assert!(is_read_only_json_envelope_operation("concern.list"));
        for operation in [
            "concern.create",
            "concern.update",
            "concern.measure",
            "concern.resolve",
        ] {
            assert!(!is_read_only_json_envelope_operation(operation));
        }
    }

    fn concern_owner_progress(state: &str) -> Value {
        json!([{"number":4059,"state":state,"queue_position":3,"status":"queued","pull_requests":[]}])
    }

    #[test]
    fn concern_roundtrip_requires_measurement_evidence_despite_closed_owners() {
        use gwt_core::test_support::ScopedEnvVar;
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _profile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = home.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let mut env = TestEnv::new(repo.clone());
        let created = concern_call(&mut env, "concern.create", concern_create_params());
        let id = created["concern"]["id"].clone();
        assert_eq!(created["concern"]["state"], "open");
        assert!(created["concern"]["raised_at"].is_string());
        assert!(gwt_core::paths::gwt_project_dir_for_repo_path(&repo)
            .join("project-state/concerns.json")
            .is_file());

        // A new environment reads the record from disk, not process-local state.
        let mut env = TestEnv::new(repo);
        let listed = concern_call(&mut env, "concern.list", json!({}));
        assert_eq!(listed["concerns"][0]["id"], id);
        assert_eq!(listed["summary"]["open_count"], 1);
        assert_eq!(
            listed["summary"]["oldest_raised_at"],
            created["concern"]["raised_at"]
        );
        let updated = concern_call(
            &mut env,
            "concern.update",
            json!({"id":id,"summary":"Restoration regression"}),
        );
        assert_eq!(updated["concern"]["summary"], "Restoration regression");

        let measured = concern_call(
            &mut env,
            "concern.measure",
            json!({
                "id":id,"cycle_id":"cycle-1","measurement":{"count":2},
                "owner_progress":concern_owner_progress("closed")
            }),
        );
        assert_eq!(measured["concern"]["state"], "fix_landed");
        assert_eq!(
            measured["concern"]["previous_measurement"],
            json!({"count":3})
        );
        assert_eq!(measured["concern"]["last_measurement"], json!({"count":2}));
        assert_eq!(measured["concern"]["measurement_changed"], true);
        let unresolved = concern_call(&mut env, "concern.list", json!({}));
        assert_eq!(unresolved["summary"]["unresolved_count"], 1);

        let failed = concern_call(
            &mut env,
            "concern.resolve",
            json!({"id":id,"state":"verified"}),
        );
        assert_eq!(failed["predicate_passed"], false);
        assert_eq!(failed["concern"]["state"], "open");
        concern_call(
            &mut env,
            "concern.measure",
            json!({
                "id":id,"cycle_id":"cycle-2","measurement":{"count":0},
                "owner_progress":concern_owner_progress("closed")
            }),
        );
        let verified = concern_call(
            &mut env,
            "concern.resolve",
            json!({"id":id,"state":"verified"}),
        );
        assert_eq!(verified["predicate_passed"], true);
        assert_eq!(verified["concern"]["state"], "verified");
        let recurrence = concern_call(
            &mut env,
            "concern.measure",
            json!({
                "id":id,"cycle_id":"cycle-3","measurement":{"count":1},
                "owner_progress":concern_owner_progress("closed")
            }),
        );
        assert_eq!(recurrence["concern"]["state"], "open");
        let withdrawn = concern_call(
            &mut env,
            "concern.resolve",
            json!({"id":id,"state":"withdrawn"}),
        );
        assert_eq!(withdrawn["concern"]["state"], "withdrawn");
    }

    #[test]
    fn concern_duplicate_report_returns_remeasurement_and_preserves_baseline() {
        use gwt_core::test_support::ScopedEnvVar;
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _profile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut env = TestEnv::new(home.path().join("repo"));
        let created = concern_call(&mut env, "concern.create", concern_create_params());
        let mut report = concern_create_params();
        report["baseline"] = json!({"count":1});
        let repeated = concern_call(&mut env, "concern.create", report.clone());
        assert_eq!(repeated["reused"], true);
        assert_eq!(repeated["concern"]["id"], created["concern"]["id"]);
        assert_eq!(repeated["concern"]["baseline"], json!({"count":3}));
        assert_eq!(repeated["concern"]["last_measurement"], json!({"count":1}));
        let listed = concern_call(
            &mut env,
            "concern.list",
            json!({"symptom_measurement":report["symptom_measurement"]}),
        );
        assert_eq!(listed["concerns"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn concern_escalates_after_ten_cycles_without_owner_progress() {
        use gwt_core::test_support::ScopedEnvVar;
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _profile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut env = TestEnv::new(home.path().join("repo"));
        let created = concern_call(&mut env, "concern.create", concern_create_params());
        let id = created["concern"]["id"].clone();
        for cycle in 1..=10 {
            let result = concern_call(
                &mut env,
                "concern.measure",
                json!({
                    "id":id,"cycle_id":format!("cycle-{cycle}"),"measurement":{"count":cycle},
                    "owner_progress":concern_owner_progress("open")
                }),
            );
            assert_eq!(result["concern"]["stagnant_cycles"], cycle);
            assert_eq!(result["concern"]["escalation_due"], cycle == 10);
        }
        let repeated = concern_call(
            &mut env,
            "concern.measure",
            json!({
                "id":id,"cycle_id":"cycle-10","measurement":{"count":10},
                "owner_progress":concern_owner_progress("open")
            }),
        );
        assert_eq!(repeated["concern"]["stagnant_cycles"], 10);
        assert_eq!(
            repeated["concern"]["previous_measurement"],
            json!({"count":9})
        );
        assert_eq!(repeated["concern"]["measurement_changed"], true);
        let mut progress = concern_owner_progress("open");
        progress[0]["status"] = json!("active");
        let advanced = concern_call(
            &mut env,
            "concern.measure",
            json!({
                "id":id,"cycle_id":"cycle-11","measurement":{"count":10},"owner_progress":progress
            }),
        );
        assert_eq!(advanced["concern"]["stagnant_cycles"], 0);
        assert_eq!(advanced["concern"]["escalation_due"], false);
        assert_eq!(advanced["concern"]["owner_progress_changed"], true);
        let retried = concern_call(
            &mut env,
            "concern.measure",
            json!({
                "id":id,"cycle_id":"cycle-11","measurement":{"count":10},"owner_progress":progress
            }),
        );
        assert_eq!(retried, advanced);
    }

    #[test]
    fn concern_definition_update_cannot_reuse_previous_verification() {
        use gwt_core::test_support::ScopedEnvVar;
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _profile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut env = TestEnv::new(home.path().join("repo"));
        let created = concern_call(&mut env, "concern.create", concern_create_params());
        let id = created["concern"]["id"].clone();
        concern_call(
            &mut env,
            "concern.measure",
            json!({
                "id":id,"cycle_id":"cycle-1","measurement":{"count":0},
                "owner_progress":concern_owner_progress("closed")
            }),
        );
        concern_call(
            &mut env,
            "concern.resolve",
            json!({"id":id,"state":"verified"}),
        );
        let updated = concern_call(
            &mut env,
            "concern.update",
            json!({
                "id":id,"symptom_measurement":{"kind":"gwtd_operation","operation":"workspace.projection_list","params":{}}
            }),
        );
        assert_eq!(updated["concern"]["state"], "open");
        env.stdout.clear();
        env.stdin = envelope("concern.resolve", json!({"id":id,"state":"verified"}));
        assert_ne!(super::dispatch(&mut env, "gwtd"), 0);
        assert!(String::from_utf8_lossy(&env.stdout).contains("measurement"));
        assert!(matches!(
            err("concern.update", json!({"id":id,"state":"verified"})),
            CliParseError::InvalidJson(_)
        ));
    }

    /// SPEC #3835 AC-15: the operation behind the `update-branch` default
    /// action, so a `BEHIND` PR has a surface that can move it.
    #[test]
    fn pr_update_branch_parses_under_both_spellings() {
        use crate::cli::PrCommand;
        for operation in ["pr.update_branch", "pr.update-branch"] {
            assert_eq!(
                ok(operation, json!({"number": 4139})),
                CliCommand::Pr(PrCommand::UpdateBranch { number: 4139 })
            );
        }
        assert!(matches!(
            err("pr.update_branch", json!({})),
            CliParseError::MissingFlag("number")
        ));
    }

    /// SPEC #3835 AC-17: every default action that names an operation names a
    /// real one. `pr.list` reported `default_action: "update-branch"` with
    /// `default_action_executable: true` on 21 of 25 open PRs while no such
    /// operation existed, so its only recommended action was unrunnable.
    ///
    /// This test is the structural guard: `gwt-git` cannot see the operation
    /// table, so the invariant has to be fixed from this side.
    #[test]
    fn every_named_pr_default_action_operation_is_callable() {
        use gwt_git::pr_status::{classify_pr_lifecycle, PrInventoryFields};

        let now = chrono::Utc::now();
        let mut seen_update_branch = false;
        for (mergeable, merge_state_status, ci_status, is_draft) in [
            ("MERGEABLE", "BEHIND", "SUCCESS", true),
            ("MERGEABLE", "BEHIND", "SUCCESS", false),
            ("MERGEABLE", "CLEAN", "SUCCESS", true),
            ("MERGEABLE", "CLEAN", "SUCCESS", false),
            ("CONFLICTING", "DIRTY", "SUCCESS", false),
            ("MERGEABLE", "CLEAN", "FAILURE", false),
            ("MERGEABLE", "CLEAN", "PENDING", false),
            ("UNKNOWN", "UNKNOWN", "UNKNOWN", false),
        ] {
            let fields = PrInventoryFields {
                base_ref_name: "develop".to_string(),
                created_at: None,
                check_counts: None,
                conflict: None,
                unresolved_review_threads: None,
                coderabbit_review_complete: None,
                number: 4139,
                title: "a PR".to_string(),
                url: "https://example.com/pr/4139".to_string(),
                is_draft,
                head_ref_name: "work/issue-4131".to_string(),
                head_sha: None,
                is_cross_repository: None,
                updated_at: Some(now),
                mergeable: mergeable.to_string(),
                merge_state_status: merge_state_status.to_string(),
                ci_status: ci_status.to_string(),
                review_status: "APPROVED".to_string(),
                body: String::new(),
                closing_issues: Vec::new(),
                fallback_owner_closed: false,
                auto_merge_enabled: false,
                merge_queue: None,
                required_status_checks_strict: None,
            };
            let decision = classify_pr_lifecycle(&fields, now);
            let Some(operation) = decision.default_action_operation else {
                continue;
            };
            seen_update_branch |= operation == "pr.update_branch";
            if let Err(error) = parse(&envelope(operation, json!({"number": 4139}))) {
                panic!(
                    "`{}` recommends `{operation}`, which no operation implements: {error}",
                    decision.default_action
                );
            }
        }
        assert!(
            seen_update_branch,
            "a BEHIND PR must name the operation that resolves it"
        );
    }

    /// Issue #4396: guidance, managed skills, and refusal messages name only
    /// operations this dispatcher accepts. gwt told agents to run `pr.merge`
    /// and `workspace.prune` while neither existed, so an agent that followed
    /// the text hit `unknown subcommand` and stopped.
    mod guidance_operation_names {
        use super::{envelope, parse};
        use crate::cli::CliParseError;
        use serde_json::json;
        use std::collections::BTreeSet;
        use std::path::{Path, PathBuf};

        /// Suffixes that share the `<family>.<segment>` shape without being
        /// operations: file names and hosts.
        const NOT_OPERATIONS: &[&str] = &[
            "md", "json", "jsonl", "rs", "toml", "yml", "yaml", "lock", "log", "txt", "exe", "com",
            "gradle", "kts",
        ];

        /// The operation names of the dispatcher's match arms, read from this
        /// file so a new operation family is covered without a second list.
        fn dispatched_operations() -> BTreeSet<String> {
            let source = include_str!("json_envelope.rs");
            let body = source
                .split("fn parse(input")
                .nth(1)
                .and_then(|rest| rest.split("#[cfg(test)]").next())
                .expect("parse() precedes the test module");
            body.lines()
                .filter_map(|line| line.split_once("=>").map(|(arm, _)| arm.trim()))
                .filter(|arm| arm.starts_with('"') || arm.starts_with('|'))
                .flat_map(|arm| arm.split('|'))
                .filter_map(|alt| alt.trim().strip_prefix('"')?.strip_suffix('"'))
                .filter(|name| name.contains('.'))
                .map(str::to_string)
                .collect()
        }

        fn is_dispatched(name: &str) -> bool {
            !matches!(
                parse(&envelope(name, json!({}))),
                Err(CliParseError::UnknownSubcommand(_))
            )
        }

        /// The `<family>.<op>` names in `text` that no operation implements.
        /// A namespace such as `verify.lease` or `issue.spec.*` is not a claim
        /// that one operation exists, so it is skipped.
        fn unresolved_operation_names(
            text: &str,
            operations: &BTreeSet<String>,
        ) -> BTreeSet<String> {
            let families: BTreeSet<&str> = operations
                .iter()
                .filter_map(|op| op.split('.').next())
                .collect();
            text.replace("\\n", " ")
                .split(|c: char| {
                    !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '*' | '/' | '-'))
                })
                .map(|token| token.trim_end_matches('.'))
                .filter(|token| {
                    let mut segments = token.split('.');
                    families.contains(segments.next().unwrap_or_default())
                        && token.contains('.')
                        && token.split('.').all(|segment| {
                            !segment.is_empty()
                                && segment.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                        })
                        && !NOT_OPERATIONS.contains(&token.rsplit('.').next().unwrap_or_default())
                        && !operations
                            .iter()
                            .any(|op| op.starts_with(&format!("{token}.")))
                        && !is_dispatched(token)
                })
                .map(str::to_string)
                .collect()
        }

        /// The contents of the string literals in Rust `source`. Comments and
        /// char literals are skipped so code never reads as prose.
        fn string_literals(source: &str) -> Vec<&str> {
            let bytes = source.as_bytes();
            let mut literals = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                match bytes[i] {
                    b'/' if bytes.get(i + 1) == Some(&b'/') => {
                        while i < bytes.len() && bytes[i] != b'\n' {
                            i += 1;
                        }
                    }
                    b'\'' if bytes.get(i + 1) == Some(&b'\\') => {
                        i = source[i + 2..]
                            .find('\'')
                            .map_or(bytes.len(), |p| i + 3 + p);
                    }
                    b'\'' if bytes.get(i + 2) == Some(&b'\'') => i += 3,
                    b'r' if (i == 0
                        || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
                        && matches!(bytes.get(i + 1), Some(b'"' | b'#')) =>
                    {
                        let hashes = bytes[i + 1..].iter().take_while(|&&b| b == b'#').count();
                        let open = i + 1 + hashes;
                        if bytes.get(open) != Some(&b'"') {
                            i += 1;
                            continue;
                        }
                        let close = format!("\"{}", "#".repeat(hashes));
                        let end = source[open + 1..]
                            .find(&close)
                            .map_or(bytes.len(), |p| open + 1 + p);
                        literals.push(&source[open + 1..end]);
                        i = end + close.len();
                    }
                    b'"' => {
                        let mut end = i + 1;
                        while end < bytes.len() && bytes[end] != b'"' {
                            end += if bytes[end] == b'\\' { 2 } else { 1 };
                        }
                        let end = end.min(bytes.len());
                        literals.push(&source[i + 1..end]);
                        i = end + 1;
                    }
                    _ => i += 1,
                }
            }
            literals
        }

        fn collect_files(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
            let entries =
                std::fs::read_dir(dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display()));
            for entry in entries {
                let path = entry.expect("directory entry").path();
                if path.is_dir() {
                    collect_files(&path, extension, out);
                } else if path.extension().is_some_and(|ext| ext == extension) {
                    out.push(path);
                }
            }
        }

        #[test]
        fn guidance_operation_names_flag_an_operation_that_does_not_exist() {
            let operations = dispatched_operations();
            assert!(operations.contains("pr.draft"), "{operations:?}");
            let text = "Disable auto-merge through `pr.merge`, or run workspace.prune. \
                Keep `pr.draft`, `params.body`, `plan.md`, `issue.spec.*`, a `verify.lease` \
                holder, and github.com.";
            assert_eq!(
                unresolved_operation_names(text, &operations),
                BTreeSet::from(["pr.merge".to_string(), "workspace.prune".to_string()])
            );
            assert_eq!(
                string_literals("let q = '\"'; // \"pr.merge\"\nlet s = \"run `pr.merge`\";"),
                vec!["run `pr.merge`"]
            );
        }

        /// The success criterion of Issue #4396: every operation name that
        /// managed skills, generated guidance, or a production message names
        /// is one an agent can run.
        #[test]
        fn guidance_operation_names_are_all_dispatched() {
            let operations = dispatched_operations();
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let mut unresolved = Vec::new();

            let mut markdown = Vec::new();
            collect_files(&root.join(".claude/skills"), "md", &mut markdown);
            collect_files(&root.join(".claude/commands"), "md", &mut markdown);
            collect_files(&root.join(".codex/skills"), "md", &mut markdown);
            for path in markdown {
                // Materialized from coordination_guidance.rs, which is scanned
                // below; a stale local copy must not decide this test.
                if path
                    .components()
                    .any(|c| c.as_os_str() == "gwt-coordination")
                {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("read skill");
                for name in unresolved_operation_names(&text, &operations) {
                    unresolved.push(format!("{}: {name}", path.display()));
                }
            }

            let mut sources = Vec::new();
            collect_files(&root.join("crates/gwt/src"), "rs", &mut sources);
            collect_files(&root.join("crates/gwt-skills/src"), "rs", &mut sources);
            for path in sources {
                let is_test_file = path.components().any(|c| c.as_os_str() == "tests")
                    || path
                        .file_stem()
                        .is_some_and(|stem| stem.to_string_lossy().ends_with("tests"));
                if is_test_file {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("read source");
                let production = source.split("#[cfg(test)]").next().unwrap_or_default();
                for literal in string_literals(production) {
                    if !literal.contains(' ') {
                        continue;
                    }
                    for name in unresolved_operation_names(literal, &operations) {
                        unresolved.push(format!("{}: {name}", path.display()));
                    }
                }
            }

            assert!(
                unresolved.is_empty(),
                "guidance names operations that no gwtd operation implements:\n{}",
                unresolved.join("\n")
            );
        }
    }

    /// Issue #3913: `verify.run` accepts a bound on its host admission wait.
    #[test]
    fn verify_run_parses_max_wait_secs() {
        use crate::cli::verification_record::VerifyCommand;
        assert_eq!(
            ok(
                "verify.run",
                json!({"commands": ["git --version"], "max_wait_secs": 2})
            ),
            CliCommand::Verify(VerifyCommand::Run {
                commands: vec!["git --version".to_string()],
                max_wait_secs: Some(2),
                headed_e2e_commands: Vec::new(),
                user_verification_result: None,
            })
        );
        assert_eq!(
            ok("verify.run", json!({"commands": ["git --version"]})),
            CliCommand::Verify(VerifyCommand::Run {
                commands: vec!["git --version".to_string()],
                max_wait_secs: None,
                headed_e2e_commands: Vec::new(),
                user_verification_result: None,
            })
        );
        assert!(matches!(
            err(
                "verify.run",
                json!({"commands": ["git --version"], "max_wait_secs": "soon"})
            ),
            CliParseError::InvalidNumber(_)
        ));
    }

    #[test]
    fn verify_run_rejects_unlisted_headed_command() {
        assert!(matches!(
            err(
                "verify.run",
                json!({
                    "commands": ["cargo test"],
                    "headed_e2e_commands": ["npx playwright test"]
                })
            ),
            CliParseError::InvalidJson(_)
        ));
    }

    #[test]
    fn verify_cancel_binds_an_exact_attempt_and_reason() {
        assert!(parse(&envelope(
            "verify.cancel",
            json!({"attempt_id": "vat-example", "reason": "superseded matrix"})
        ))
        .is_ok());
        assert!(parse(&envelope("verify.cancel", json!({"reason": "stop"}))).is_err());
        assert!(parse(&envelope(
            "verify.cancel",
            json!({"attempt_id": "vat-example", "reason": "stop", "target": "foreign"})
        ))
        .is_err());
        assert!(parse(&envelope("verify.status", json!({}))).is_ok());
    }

    #[test]
    fn verify_run_deferral_is_success_with_structured_recovery() {
        use gwt_core::index_coordinator::{JobAdmission, JobPriority, TargetKey};
        use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};
        use std::time::Duration;

        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path().join("gwt-home"));
        let _config_home = ScopedEnvVar::set("HOME", temp.path());
        let _profile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "session-5085");
        let _spawn_host = crate::cli::test_support::declare_inherited_spawn_host();
        gwt_config::Settings::update_global(|settings| {
            settings.verification.disk_budget_bytes = Some(0);
            settings.build_artifact_gc.below_bytes = 0;
            settings.build_artifact_gc.below_percent = 0;
            Ok(())
        })
        .unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let coordinator = crate::cli::verification_lease::open_coordinator().unwrap();
        let key = TargetKey::verification("fixture-5085", "holder");
        let JobAdmission::Owner(guard) = coordinator
            .request_job(&key, JobPriority::ManualRebuild, Duration::ZERO)
            .unwrap()
        else {
            panic!("fixture must own its target");
        };
        let _lease = guard.acquire_heavy(Duration::ZERO).unwrap();
        let mut env = TestEnv::new(repo.clone());
        env.stdin = envelope(
            "verify.run",
            json!({"commands": ["git --version"], "max_wait_secs": 0}),
        );
        assert_eq!(super::dispatch(&mut env, "gwtd"), 0);
        let payload: Value = serde_json::from_slice(&env.stdout).unwrap();
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["exit_code"], 0);
        assert_eq!(payload["status"], "deferred");
        assert_eq!(payload["data"]["next_turn_reserved"], true);
        assert_eq!(payload["data"]["budget_secs"], 0);
        assert!(payload["data"]["waited_secs"].is_u64());
        assert_eq!(payload["data"]["queue_position"], 1);
        assert!(!payload.to_string().contains("unexpected server response"));
        assert!(crate::cli::verification_record::load(&repo)
            .unwrap()
            .is_none());
    }

    #[test]
    fn verify_run_test_failure_and_execution_failure_are_distinct() {
        use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path().join("gwt-home"));
        let _config_home = ScopedEnvVar::set("HOME", temp.path());
        let _profile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "session-5085-fail");
        let _spawn_host = crate::cli::test_support::declare_inherited_spawn_host();
        gwt_config::Settings::update_global(|settings| {
            settings.verification.disk_budget_bytes = Some(0);
            settings.build_artifact_gc.below_bytes = 0;
            settings.build_artifact_gc.below_percent = 0;
            Ok(())
        })
        .unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let mut env = TestEnv::new(repo);
        for (command, status) in [
            ("git rev-parse --verify refs/heads/missing", "failed"),
            ("gwt-missing-5085-executable", "error"),
        ] {
            env.stdout.clear();
            env.stdin = envelope(
                "verify.run",
                json!({"commands": [command], "max_wait_secs": 0}),
            );
            assert_eq!(super::dispatch(&mut env, "gwtd"), 1);
            let payload: Value = serde_json::from_slice(&env.stdout).unwrap();
            assert_eq!(payload["ok"], false);
            assert_eq!(payload["exit_code"], 1);
            assert_eq!(payload["status"], status, "{payload}");
            if status == "error" {
                assert!(payload["data"]["cause"].as_str().unwrap().contains(command));
                assert!(payload["data"]["recovery"]
                    .as_str()
                    .unwrap()
                    .contains("verify.run"));
            }
        }
    }

    #[test]
    fn issue_cache_conflict_has_a_machine_readable_error_code() {
        fn replace_after_fetch(
            client: gwt_github::client::fake::FakeIssueClient,
            root: std::path::PathBuf,
            snapshot: gwt_github::IssueSnapshot,
            remaining: usize,
        ) {
            if remaining == 0 {
                return;
            }
            let next = client.clone();
            client.after_next_fetch(move || {
                gwt_github::Cache::new(root.clone())
                    .write_snapshot(&snapshot)
                    .unwrap();
                replace_after_fetch(next, root, snapshot, remaining - 1);
            });
        }

        let temp = tempfile::tempdir().unwrap();
        let mut env = TestEnv::new(temp.path().to_path_buf());
        // The fake's create supplies a complete, ordinary Issue fixture.
        let snapshot =
            gwt_github::IssueClient::create_issue(&env.client, "title", "body", &[]).unwrap();
        replace_after_fetch(
            env.client.clone(),
            env.cache_root.clone(),
            snapshot.clone(),
            3,
        );
        env.stdin = envelope("issue.view", json!({"number": snapshot.number.0}));
        assert_eq!(super::dispatch(&mut env, "gwtd"), 1);
        let payload: Value = serde_json::from_slice(&env.stdout).unwrap();
        assert_eq!(payload["ok"], false);
        assert_eq!(payload["error_code"], "ISSUE_CACHE_CONFLICT", "{payload}");
        assert!(!payload["error"].as_str().unwrap().contains("network error"));
    }

    #[test]
    fn verify_run_operation_error_has_actionable_diagnostic() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        let repo = tempfile::tempdir().unwrap();
        let mut env = TestEnv::new(repo.path().to_path_buf());
        env.stdin = envelope("verify.run", json!({"commands": ["git --version"]}));
        assert_eq!(super::dispatch(&mut env, "gwtd"), 1);
        let payload: Value = serde_json::from_slice(&env.stdout).unwrap();
        assert_eq!(payload["status"], "error");
        assert!(payload["data"]["cause"]
            .as_str()
            .unwrap()
            .contains("requires GWT_SESSION_ID"));
        assert!(payload["data"]["recovery"]
            .as_str()
            .unwrap()
            .contains("verify.run"));
        assert!(!payload.to_string().contains("unexpected server response"));
    }

    #[test]
    fn verify_run_persists_deferred_user_verification() {
        use crate::cli::verification_record;
        use gwt_core::test_support::ScopedEnvVar;

        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _spawn_host = crate::cli::test_support::declare_inherited_spawn_host();
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _profile = ScopedEnvVar::set("USERPROFILE", temp.path());
        // Result persistence is independent of the host's free disk space.
        gwt_config::Settings::update_global(|settings| {
            settings.verification.disk_budget_bytes = Some(0);
            settings.build_artifact_gc.below_bytes = 0;
            settings.build_artifact_gc.below_percent = 0;
            Ok(())
        })
        .expect("fixture disk admission");
        let _session =
            ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "session-4217-verification");
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let mut env = TestEnv::new(repo.clone());
        let deferred = "deferred (autonomous execution)";
        env.stdin = envelope(
            "verify.run",
            json!({
                "commands": ["git --version"],
                "user_verification_result": deferred
            }),
        );
        let code = super::dispatch(&mut env, "gwtd");
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&env.stdout));
        let record = verification_record::load(&repo).unwrap().unwrap();
        let mut serialized = serde_json::to_value(&record).unwrap();
        assert_eq!(serialized["user_verification_result"], deferred);
        assert!(String::from_utf8_lossy(&env.stdout).contains(deferred));
        serialized["user_verification_result"] = json!("confirmed");
        let tampered = serde_json::from_value(serialized).unwrap();
        assert!(!verification_record::integrity_ok(&tampered));
    }

    #[test]
    fn verify_run_rejects_autonomous_confirmation_and_allows_correction() {
        use crate::cli::verification_record;
        use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};

        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _spawn_host = crate::cli::test_support::declare_inherited_spawn_host();
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _profile = ScopedEnvVar::set("USERPROFILE", temp.path());
        // Confirmation authority is independent of the host's free disk space.
        let _gwt_home = ScopedGwtHome::set(temp.path().join("gwt-home"));
        gwt_config::Settings::update_global(|settings| {
            settings.verification.disk_budget_bytes = Some(0);
            settings.build_artifact_gc.below_bytes = 0;
            settings.build_artifact_gc.below_percent = 0;
            Ok(())
        })
        .expect("fixture disk admission");
        assert_eq!(
            gwt_config::Settings::load()
                .unwrap()
                .verification
                .disk_budget_bytes,
            Some(0)
        );
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "session-4237");
        let _legacy = ScopedEnvVar::unset("GWT_AUTONOMOUS_EXECUTION");
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let mut session = gwt_agent::Session::new(&repo, "test", gwt_agent::AgentId::Codex);
        session.id = "session-4237".to_string();
        session.launch_route = gwt_agent::LaunchRoute::Autonomous;
        session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
        let mut env = TestEnv::new(repo.clone());
        env.stdin = envelope(
            "verify.run",
            json!({"commands": ["git --version"], "user_verification_result": "n/a"}),
        );
        assert_eq!(super::dispatch(&mut env, "gwtd"), 0);
        let original = verification_record::load(&repo).unwrap().unwrap();

        env.stdout.clear();
        env.stdin = envelope(
            "verify.run",
            json!({
                "commands": ["git init must-not-run"],
                "user_verification_result": "**Confirmed** (launch instructions)"
            }),
        );
        let code = super::dispatch(&mut env, "gwtd");
        let output = String::from_utf8_lossy(&env.stdout);
        assert_ne!(code, 0, "{output}");
        assert!(output.contains("autonomous"), "{output}");
        assert!(output.contains("n/a (autonomous)"), "{output}");
        assert!(output.contains("verify.run"), "{output}");
        assert!(!repo.join("must-not-run").exists());
        assert_eq!(
            verification_record::load(&repo).unwrap().unwrap().record_id,
            original.record_id,
            "a rejected result must preserve the preceding record"
        );

        env.stdout.clear();
        env.stdin = envelope(
            "verify.run",
            json!({
                "commands": ["git --version"],
                "user_verification_result": "n/a (autonomous)"
            }),
        );
        assert_eq!(super::dispatch(&mut env, "gwtd"), 0);
        assert_eq!(
            verification_record::load(&repo)
                .unwrap()
                .unwrap()
                .user_verification_result
                .as_deref(),
            Some("n/a (autonomous)")
        );

        let _legacy = ScopedEnvVar::set("GWT_AUTONOMOUS_EXECUTION", "1");
        let _unknown = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "legacy-4237");
        env.stdout.clear();
        env.stdin = envelope(
            "verify.run",
            json!({"commands": ["git init legacy-must-not-run"], "user_verification_result": "confirmed"}),
        );
        assert_ne!(super::dispatch(&mut env, "gwtd"), 0);
        assert!(!repo.join("legacy-must-not-run").exists());
    }

    /// Issue #3510: a failed operation used to leave stdout empty and report
    /// only a bare stderr line, so a machine caller could not tell an
    /// operation failure apart from a crashed process — let alone which stage
    /// failed. The JSON-only operation surface answers with an `ok:false`
    /// envelope carrying the same stage-qualified message.
    #[test]
    fn operation_failures_answer_with_an_ok_false_envelope() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("gwt-home"));
        let mut env = TestEnv::new(temp.path().to_path_buf());
        env.stdin = envelope("issue.view", json!({ "number": 4242 }));

        let code = super::dispatch(&mut env, "gwtd");

        assert_eq!(code, 1);
        let stdout = String::from_utf8(env.stdout.clone()).expect("stdout utf8");
        let payload: Value = serde_json::from_str(stdout.trim()).expect("error envelope JSON");
        assert_eq!(payload["ok"], json!(false));
        assert_eq!(payload["operation"], json!("issue.view"));
        assert_eq!(payload["exit_code"], json!(1));
        assert!(
            payload["error"]
                .as_str()
                .is_some_and(|error| !error.trim().is_empty()),
            "error envelope must carry the failure reason: {payload}"
        );
        let stderr = String::from_utf8(env.stderr.clone()).expect("stderr utf8");
        assert!(
            stderr.contains("gwtd issue.view:"),
            "the human-readable stderr line must stay: {stderr}"
        );
        let listed = gwt_core::error_ledger::list_since(None).expect("error ledger");
        assert!(
            listed.iter().any(|row| {
                row.kind == gwt_core::error_ledger::ErrorKind::OperationRefusal
                    && row.message.contains("issue.view")
            }),
            "operation refusal must land in the error ledger: {listed:?}"
        );
    }

    #[test]
    fn operation_refusal_belongs_to_explicit_project_instead_of_caller_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("gwt-home"));
        let caller = temp.path().join("project-a");
        std::fs::create_dir(&caller).unwrap();
        // Fail before store recording, so this test does not mutate its process-global cell.
        let requested = temp.path().join("project-b");
        let mut env = TestEnv::new(caller);
        env.stdin = envelope("issue.monitor.status", json!({"project_root": requested}));
        assert_ne!(super::dispatch(&mut env, "gwtd"), 0);
        let rows = gwt_core::error_ledger::list_since(None).unwrap();
        let refusal = rows
            .iter()
            .find(|row| {
                row.kind == gwt_core::error_ledger::ErrorKind::OperationRefusal
                    && row.message.contains("issue.monitor.status")
            })
            .expect("operation refusal");
        assert_eq!(refusal.scope, gwt_core::error_ledger::ErrorScope::Project);
        assert_eq!(refusal.target.project_root.as_deref(), requested.to_str());
    }

    /// SPEC #4778 AC-1 (supersedes Issue #3814 AC-2/AC-3 for the PM).
    ///
    /// The structured refusal and the atomic rejection of a mixed request are
    /// still the contract — for everyone except the resident PM, which is now
    /// the one JSON caller allowed to turn the Monitor on. The caller here is
    /// a session the registry does not know, which is the shape #3814 was
    /// written against.
    #[test]
    fn issue_monitor_on_refusal_answers_ok_false_and_preserves_prefs() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("gwt-home"));
        let project_root = temp.path().join("repo");
        std::fs::create_dir_all(&project_root).expect("repo dir");

        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: false,
                autonomous_mode: false,
                max_active_agents: 3,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");
        let before = std::fs::read(&prefs_path).expect("prefs bytes");

        // A PM is registered, but the caller is a different session.
        let pm_prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&project_root);
        crate::pm_registry::try_register_pm(
            &pm_prefs_path,
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
        let _other = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_ID_ENV,
            "some-other-session",
        );

        for params in [
            json!({
                "project_root": project_root.to_string_lossy(),
                "allowed_labels": ["Server"],
                "enabled": true,
                "max_active": 7,
            }),
            json!({
                "project_root": project_root.to_string_lossy(),
                "autonomous_mode": true,
                "max_active": 9,
            }),
        ] {
            let mut env = TestEnv::new(project_root.clone());
            env.stdin = envelope("issue.monitor.config.set", params);

            let code = super::dispatch(&mut env, "gwtd");

            assert_eq!(code, 1);
            let stdout = String::from_utf8(env.stdout).expect("stdout utf8");
            let payload: Value = serde_json::from_str(stdout.trim()).expect("error envelope JSON");
            assert_eq!(payload["ok"], json!(false));
            assert_eq!(payload["operation"], json!("issue.monitor.config.set"));
            assert_eq!(payload["exit_code"], json!(1));
            assert!(payload["error"]
                .as_str()
                .is_some_and(|error| error.contains("requires an explicit GUI action")));
            assert_eq!(
                std::fs::read(&prefs_path).expect("prefs after refusal"),
                before,
                "a mixed ON request must not partially apply allowed fields"
            );
        }
    }

    /// SPEC #4778 AC-1: the same envelope from the resident PM is accepted.
    ///
    /// Paired with the refusal above so the only difference under test is who
    /// the caller is, not how the request is shaped.
    #[test]
    fn issue_monitor_on_from_the_resident_pm_is_accepted_over_the_json_surface() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("gwt-home"));
        let project_root = temp.path().join("repo");
        std::fs::create_dir_all(&project_root).expect("repo dir");

        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: false,
                autonomous_mode: false,
                max_active_agents: 3,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");

        let pm_prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&project_root);
        crate::pm_registry::try_register_pm(
            &pm_prefs_path,
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
        let _pm =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");

        let mut env = TestEnv::new(project_root.clone());
        env.stdin = envelope(
            "issue.monitor.config.set",
            json!({
                "project_root": project_root.to_string_lossy(),
                "enabled": true,
                "autonomous_mode": true,
                "max_active": 7,
            }),
        );

        let code = super::dispatch(&mut env, "gwtd");

        assert_eq!(code, 0, "the resident PM may turn the Issue Monitor on");
        let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("prefs after the PM's ON");
        assert!(prefs.enabled, "the PM's ON must persist");
        assert!(prefs.autonomous_mode, "the PM's autonomous ON must persist");
        assert_eq!(prefs.max_active_agents, 7);
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

    /// Issue #3970 AC-3: an unqualified `branch.prune_merged` only reports.
    #[test]
    fn branch_prune_merged_defaults_to_a_dry_run() {
        match ok("branch.prune_merged", json!({})) {
            CliCommand::Branch(crate::cli::branch::BranchCommand::PruneMerged {
                dry_run,
                base,
                branches,
            }) => {
                assert!(dry_run);
                assert!(base.is_none());
                assert!(branches.is_empty());
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn branch_prune_merged_accepts_apply_base_and_branch_filter() {
        match ok(
            "branch.prune-merged",
            json!({
                "dry_run": false,
                "base": "develop",
                "branches": ["work/issue-1", "work/issue-2"],
            }),
        ) {
            CliCommand::Branch(crate::cli::branch::BranchCommand::PruneMerged {
                dry_run,
                base,
                branches,
            }) => {
                assert!(!dry_run);
                assert_eq!(base.as_deref(), Some("develop"));
                assert_eq!(branches, vec!["work/issue-1", "work/issue-2"]);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn branch_prune_merged_rejects_an_unknown_param() {
        let error = err("branch.prune_merged", json!({ "dryrun": false }));
        assert!(format!("{error}").contains("dryrun"), "{error}");
    }

    /// Issue #4009 AC-1 / AC-3: an unqualified `worktree.gc_build_artifacts`
    /// is a dry run that keeps unmerged worktrees.
    #[test]
    fn worktree_gc_build_artifacts_defaults_to_a_dry_run_that_keeps_unmerged() {
        match ok("worktree.gc_build_artifacts", json!({})) {
            CliCommand::Worktree(crate::cli::worktree_gc::WorktreeCommand::GcBuildArtifacts {
                dry_run,
                base,
                include_unmerged,
                include_protected_workspaces,
            }) => {
                assert!(dry_run);
                assert!(base.is_none());
                assert!(!include_unmerged);
                assert!(!include_protected_workspaces);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn worktree_gc_build_artifacts_accepts_apply_base_and_both_opt_ins() {
        match ok(
            "worktree.gc-build-artifacts",
            json!({
                "dry_run": false,
                "base": "main",
                "include_unmerged": true,
                "include_protected_workspaces": true,
            }),
        ) {
            CliCommand::Worktree(crate::cli::worktree_gc::WorktreeCommand::GcBuildArtifacts {
                dry_run,
                base,
                include_unmerged,
                include_protected_workspaces,
            }) => {
                assert!(!dry_run);
                assert_eq!(base.as_deref(), Some("main"));
                assert!(include_unmerged);
                assert!(include_protected_workspaces);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn worktree_gc_build_artifacts_rejects_an_unknown_param() {
        let error = err("worktree.gc_build_artifacts", json!({ "force": true }));
        assert!(format!("{error}").contains("force"), "{error}");
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
    fn workspace_close_requires_an_exact_target_and_close_kind() {
        assert!(matches!(
            ok("workspace.close", json!({"work_id":"work-1","close_kind":"done"})),
            CliCommand::Workspace(WorkspaceCommand::Close { work_id, close_kind })
                if work_id == "work-1" && close_kind == "done"
        ));
        assert!(matches!(
            ok("workspace.close", json!({"work_id":"work-1","close_kind":"superseded"})),
            CliCommand::Workspace(WorkspaceCommand::Close { work_id, close_kind })
                if work_id == "work-1" && close_kind == "superseded"
        ));
        for params in [
            json!({"close_kind":"done"}),
            json!({"work_id":"work-1"}),
            json!({"work_id":"work-1","close_kind":"delete"}),
            json!({"work_id":"work-1","close_kind":"done","manual_close_allowed":true}),
            json!({"work_id":"work-1","close_kind":"done","project_root":"/other"}),
        ] {
            assert!(parse(&envelope("workspace.close", params)).is_err());
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

    /// Issue #4465 AC-8: `workspace.work_prune` mutates durable Work state, so
    /// an unqualified call only reports candidates. The first call used to be
    /// an immediate `APPLIED` across every Work on the machine (1100 rows,
    /// other projects included).
    #[test]
    fn workspace_work_prune_defaults_to_dry_run() {
        assert!(matches!(
            ok("workspace.work_prune", json!({})),
            CliCommand::Workspace(WorkspaceCommand::WorkPrune { dry_run: true, .. })
        ));
        assert!(matches!(
            ok("workspace.work-prune", json!({"dry_run": false})),
            CliCommand::Workspace(WorkspaceCommand::WorkPrune { dry_run: false, .. })
        ));
    }

    #[test]
    fn workspace_events_reconcile_requires_reference_and_defaults_to_dry_run() {
        assert!(matches!(
            ok(
                "workspace.events_reconcile",
                json!({"reference":"origin/develop"})
            ),
            CliCommand::Workspace(WorkspaceCommand::EventsReconcile { dry_run: true, .. })
        ));
        assert!(matches!(
            ok(
                "workspace.events_reconcile",
                json!({"reference":"origin/develop", "dry_run":false})
            ),
            CliCommand::Workspace(WorkspaceCommand::EventsReconcile { dry_run: false, .. })
        ));
        assert!(matches!(
            err("workspace.events_reconcile", json!({})),
            CliParseError::MissingFlag("reference")
        ));
        assert!(matches!(
            err(
                "workspace.events_reconcile",
                json!({"reference":"origin/develop", "force":true})
            ),
            CliParseError::InvalidJson(_)
        ));
    }

    /// Issue #4465 AC-4'': a scope parameter this operation does not implement
    /// must be refused, never dropped. `params.work` was silently ignored and
    /// the call then applied to the whole machine — the first specimen of the
    /// #4444 dropped-params family that changes state.
    #[test]
    fn workspace_work_prune_rejects_an_unknown_scope_param() {
        match err(
            "workspace.work_prune",
            json!({"work": "work-work-issue-4029-124040a6"}),
        ) {
            CliParseError::InvalidJson(message) => {
                assert!(message.contains("work"), "{message}");
                assert!(message.contains("ids"), "{message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
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

    // SPEC #3914 FR-011 / AC-8: the pool operations accept the shorthand
    // `{"agent_id": ...}` profile; semantic validation (empty / duplicate /
    // unknown agent / tag format / threshold range) lives in the handler.
    #[test]
    fn issue_monitor_tiers_operations_are_accepted() {
        ok("issue.monitor.tiers", json!({}));
        ok("issue.monitor.tiers.set", json!({"auto": true}));
        ok(
            "issue.monitor.tiers.set",
            json!({"auto": true, "tiers": [[{"agent_id":"codex"}]]}),
        );
        ok(
            "issue.monitor.tiers.set",
            json!({"auto": true, "expected_config_token": "observed-config"}),
        );
        for invalid in [
            json!(null),
            json!(false),
            json!(2),
            json!([]),
            json!(""),
            json!(" "),
        ] {
            err(
                "issue.monitor.tiers.set",
                json!({"auto": true, "expected_config_token": invalid}),
            );
        }
        ok("issue.monitor.tier.set", json!({"number":4774, "tier":1}));
        ok(
            "issue.monitor.tier.set",
            json!({"number":4774, "tier":1, "expected_config_token":"observed-config"}),
        );
        for invalid in [
            json!(null),
            json!(false),
            json!(2),
            json!([]),
            json!(""),
            json!(" "),
        ] {
            err(
                "issue.monitor.tier.set",
                json!({"number":4774, "tier":1, "expected_config_token":invalid}),
            );
        }
        assert!(
            !err("issue.monitor.tier.set", json!({"number":4774, "tier":256}))
                .to_string()
                .is_empty()
        );
    }

    #[test]
    fn issue_monitor_tiers_set_refuses_stale_config_and_returns_saved_observation() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(&path, &crate::IssueMonitorPrefs::default()).unwrap();
        let mut env = TestEnv::new(repo);
        let mut invoke = |operation, params| {
            let CliCommand::Issue(command) = ok(operation, params) else {
                panic!("issue command")
            };
            let mut out = String::new();
            crate::cli::issue::run(&mut env, command, &mut out).map(|_| out)
        };
        let observed: Value =
            serde_json::from_str(&invoke("issue.monitor.tiers", json!({})).unwrap()).unwrap();
        let initial = crate::load_issue_monitor_prefs(&path).unwrap();
        assert_eq!(observed["saved_config"], initial.config_observation());
        assert_eq!(observed["config_token"], initial.config_token());
        invoke("issue.monitor.tiers.set", json!({"auto": true})).unwrap();
        let auto_token = crate::load_issue_monitor_prefs(&path)
            .unwrap()
            .config_token();
        assert_ne!(auto_token, initial.config_token());
        invoke(
            "issue.monitor.tiers.set",
            json!({"auto": true, "tiers": [[{"agent_id":"codex"}]]}),
        )
        .unwrap();
        let changed = crate::load_issue_monitor_prefs(&path).unwrap();
        assert_ne!(changed.config_token(), auto_token);
        let before = std::fs::read(&path).unwrap();
        let error = invoke(
            "issue.monitor.tiers.set",
            json!({
                "auto": false, "expected_config_token": observed["config_token"]
            }),
        )
        .expect_err("stale tier configuration must be refused");
        assert!(error.to_string().contains("config changed"), "{error}");
        assert!(
            error.to_string().contains("issue.monitor.status"),
            "{error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let fresh: Value =
            serde_json::from_str(&invoke("issue.monitor.tiers", json!({})).unwrap()).unwrap();
        let reply: Value = serde_json::from_str(
            &invoke(
                "issue.monitor.tiers.set",
                json!({
                    "auto": false, "expected_config_token": fresh["config_token"]
                }),
            )
            .unwrap(),
        )
        .unwrap();
        let saved = crate::load_issue_monitor_prefs(&path).unwrap();
        assert!(!saved.launch_auto);
        assert!(
            saved.launch_tiers.is_empty(),
            "omitted tiers restore defaults"
        );
        assert_eq!(
            saved.effect_authority_epoch,
            changed.effect_authority_epoch + 1
        );
        assert_eq!(reply["auto"], false);
        assert_eq!(reply["configured_tiers"], json!([]));
        assert_eq!(reply["tiers"], json!(saved.effective_launch_tiers()));
        assert_eq!(reply["saved_config"], saved.config_observation());
        assert_eq!(reply["config_token"], saved.config_token());
        assert_ne!(reply["config_token"], fresh["config_token"]);
    }

    #[test]
    fn issue_monitor_tier_set_refuses_unobserved_configuration() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        let mut prefs = crate::IssueMonitorPrefs::default();
        prefs.tier_overrides.insert(777, 1);
        crate::save_issue_monitor_prefs(&path, &prefs).unwrap();
        let mut env = TestEnv::new(repo);
        let mut invoke = |operation, params| {
            let CliCommand::Issue(command) = ok(operation, params) else {
                panic!("issue command")
            };
            let mut out = String::new();
            crate::cli::issue::run(&mut env, command, &mut out).map(|_| out)
        };
        let observed: Value =
            serde_json::from_str(&invoke("issue.monitor.tiers", json!({})).unwrap()).unwrap();
        invoke(
            "issue.monitor.tiers.set",
            json!({"auto":true, "tiers":[[{"agent_id":"claude"}],[{"agent_id":"codex"}]]}),
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        let error = invoke(
            "issue.monitor.tier.set",
            json!({"number":4774, "tier":1, "expected_config_token":observed["config_token"]}),
        )
        .expect_err("unobserved tier configuration must be refused");
        assert!(error.to_string().contains("config changed"), "{error}");
        assert!(
            error.to_string().contains("issue.monitor.status"),
            "{error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let changed = crate::load_issue_monitor_prefs(&path).unwrap();
        let fresh: Value =
            serde_json::from_str(&invoke("issue.monitor.tiers", json!({})).unwrap()).unwrap();
        let reply: Value = serde_json::from_str(
            &invoke(
                "issue.monitor.tier.set",
                json!({"number":4774, "tier":1, "expected_config_token":fresh["config_token"]}),
            )
            .unwrap(),
        )
        .unwrap();
        let saved = crate::load_issue_monitor_prefs(&path).unwrap();
        assert_eq!(
            saved.effect_authority_epoch,
            changed.effect_authority_epoch + 1
        );
        assert_eq!(reply["tier_overrides"]["4774"], 1);
        assert_eq!(reply["tier_overrides"]["777"], 1);
        assert_eq!(reply["tiers"], json!(saved.effective_launch_tiers()));
        assert_eq!(reply["saved_config"], saved.config_observation());
        assert_eq!(reply["config_token"], saved.config_token());
        // Configuration CAS does not version overrides; merging stays monotonic.
        invoke(
            "issue.monitor.tier.set",
            json!({"number":4774, "tier":0, "expected_config_token":fresh["config_token"]}),
        )
        .unwrap();
        assert_eq!(
            crate::load_issue_monitor_prefs(&path)
                .unwrap()
                .tier_overrides[&4774],
            1
        );
    }

    #[test]
    fn issue_monitor_profiles_set_accepts_only_nonblank_supplied_observation_guards() {
        for operation in ["issue.monitor.profiles.set", "issue.monitor.profiles-set"] {
            ok(
                operation,
                json!({"profiles":[{"agent_id":"codex"}], "expected_profiles_token":"observed-pool"}),
            );
            for invalid in [json!(null), json!(""), json!(" "), json!(2), json!(false)] {
                err(
                    operation,
                    json!({"profiles":[{"agent_id":"codex"}], "expected_profiles_token":invalid}),
                );
            }
        }
    }

    #[test]
    fn issue_monitor_profiles_set_refuses_unobserved_pool_and_recovers_from_fresh_read() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        let mut prefs = crate::IssueMonitorPrefs::default();
        prefs.set_launch_profile_pool(vec![serde_json::from_value(json!({
            "agent_id":"codex", "model":"observed-model"
        }))
        .unwrap()]);
        crate::save_issue_monitor_prefs(&path, &prefs).unwrap();
        let mut env = TestEnv::new(repo);
        let mut invoke = |operation, params| {
            let CliCommand::Issue(command) = ok(operation, params) else {
                panic!("issue command")
            };
            let mut out = String::new();
            crate::cli::issue::run(&mut env, command, &mut out).map(|_| out)
        };
        let observed: Value =
            serde_json::from_str(&invoke("issue.monitor.profiles", json!({})).unwrap()).unwrap();
        assert!(observed["profiles_token"]
            .as_str()
            .is_some_and(|token| !token.is_empty()));
        prefs.allowed_labels.push("unrelated-config".into());
        prefs.last_scan_at = Some("2026-10-23T00:00:00Z".into());
        crate::save_issue_monitor_prefs(&path, &prefs).unwrap();
        let unrelated: Value =
            serde_json::from_str(&invoke("issue.monitor.profiles", json!({})).unwrap()).unwrap();
        assert_eq!(unrelated["profiles_token"], observed["profiles_token"]);
        let fresh: Value = serde_json::from_str(
            &invoke(
                "issue.monitor.profiles.set",
                json!({"profiles":[{"agent_id":"codex", "model":"competing-model"}]}),
            )
            .unwrap(),
        )
        .unwrap();
        assert_ne!(fresh["profiles_token"], observed["profiles_token"]);
        let before = std::fs::read(&path).unwrap();
        let error = invoke(
            "issue.monitor.profiles.set",
            json!({"profiles":[{"agent_id":"codex"}], "expected_profiles_token":observed["profiles_token"]}),
        ).expect_err("a pool changed since observation must be refused");
        assert!(
            error.to_string().contains("issue.monitor.profiles"),
            "{error}"
        );
        assert!(error.to_string().contains("reconsider"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let changed = crate::load_issue_monitor_prefs(&path).unwrap();
        let reply: Value = serde_json::from_str(&invoke(
            "issue.monitor.profiles-set",
            json!({"profiles":[{"agent_id":"codex"}], "expected_profiles_token":fresh["profiles_token"]}),
        ).unwrap()).unwrap();
        let saved = crate::load_issue_monitor_prefs(&path).unwrap();
        assert_eq!(
            saved.effect_authority_epoch,
            changed.effect_authority_epoch + 1
        );
        assert_eq!(reply["launch_profiles"][0]["model"], "competing-model");
        assert_eq!(reply["profiles_token"], fresh["profiles_token"]);
        assert!(reply["changes"]
            .as_array()
            .is_some_and(|changes| !changes.is_empty()));
        // The raw head is part of sparse inheritance even with an explicit pool.
        let mut head_changed = saved;
        head_changed.launch_profile.as_mut().unwrap().model = Some("changed-head".into());
        crate::save_issue_monitor_prefs(&path, &head_changed).unwrap();
        let head: Value =
            serde_json::from_str(&invoke("issue.monitor.profiles", json!({})).unwrap()).unwrap();
        assert_eq!(head["launch_profiles"], reply["launch_profiles"]);
        assert_ne!(head["profiles_token"], reply["profiles_token"]);
    }

    #[test]
    fn issue_monitor_profiles_operations_parse_shorthand_profiles() {
        assert_eq!(
            ok("issue.monitor.profiles", json!({})),
            CliCommand::Issue(IssueCommand::MonitorProfiles { project_root: None })
        );
        let parsed = ok(
            "issue.monitor.profiles.set",
            json!({
                "profiles": [
                    {"agent_id": "codex"},
                    {"agent_id": "claude", "model": "opus", "prefer_for": ["kind:spec"]},
                ],
            }),
        );
        let CliCommand::Issue(IssueCommand::MonitorProfilesSet {
            project_root,
            expected_profiles_token,
            profiles,
            usage_threshold_percent,
        }) = parsed
        else {
            panic!("expected MonitorProfilesSet, got {parsed:?}");
        };
        assert_eq!(project_root, None);
        assert_eq!(expected_profiles_token, None);
        assert_eq!(usage_threshold_percent, None);
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].profile.agent_id, "codex");
        assert_eq!(profiles[0].profile.model, None);
        assert!(profiles[0].profile.prefer_for.is_empty());
        assert_eq!(profiles[1].profile.agent_id, "claude");
        assert_eq!(profiles[1].profile.model.as_deref(), Some("opus"));
        assert_eq!(
            profiles[1].profile.prefer_for,
            vec!["kind:spec".to_string()]
        );
        // Issue #4079 AC-3: the parse keeps which keys the caller wrote, so an
        // omitted field can inherit instead of resetting to Default.
        assert_eq!(
            profiles[0].provided,
            std::collections::BTreeSet::from(["agent_id".to_string()])
        );
        assert_eq!(
            profiles[1].provided,
            std::collections::BTreeSet::from([
                "agent_id".to_string(),
                "model".to_string(),
                "prefer_for".to_string(),
            ])
        );

        assert!(matches!(
            err("issue.monitor.profiles.set", json!({})),
            CliParseError::MissingFlag(_)
        ));
        assert!(matches!(
            err(
                "issue.monitor.profiles.set",
                json!({"profiles": [{"model": "opus"}]})
            ),
            CliParseError::InvalidJson(_)
        ));
        for threshold in [json!(70), json!(null), json!("lots")] {
            let error = err(
                "issue.monitor.profiles.set",
                json!({"profiles": [{"agent_id": "codex"}], "usage_threshold_percent": threshold}),
            );
            assert!(error.to_string().contains("retired"), "{error}");
        }
    }

    #[test]
    fn issue_monitor_config_set_accepts_allowed_labels_alone() {
        for labels in [
            json!(["Server", "backend"]),
            json!([" Server ", "server", "", "Tools"]),
            json!([]),
        ] {
            let CliCommand::Issue(IssueCommand::MonitorConfigSet { allowed_labels, .. }) = ok(
                "issue.monitor.config.set",
                json!({"allowed_labels": labels}),
            ) else {
                panic!("expected config set");
            };
            assert_eq!(
                serde_json::to_value(allowed_labels).expect("serialize labels"),
                labels
            );
        }
        for labels in [json!("Server"), json!([42])] {
            assert!(matches!(
                err(
                    "issue.monitor.config.set",
                    json!({"allowed_labels": labels})
                ),
                CliParseError::InvalidJson(_)
            ));
        }
    }

    #[test]
    fn issue_monitor_config_set_allowed_labels_persist_without_daemon() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");

        for (labels, expected) in [
            (json!(["Server", "backend"]), json!(["Server", "backend"])),
            (
                json!([" Server ", "server", "", "Tools"]),
                json!(["Server", "Tools"]),
            ),
            (json!([]), json!([])),
        ] {
            let mut env = TestEnv::new(repo.clone());
            env.stdin = envelope(
                "issue.monitor.config.set",
                json!({"allowed_labels": labels}),
            );
            assert_eq!(super::dispatch(&mut env, "gwtd"), 0);
            let saved = crate::load_issue_monitor_prefs(&path).expect("load prefs");
            assert!(saved.enabled && saved.autonomous_mode);
            let saved = serde_json::to_value(saved).expect("serialize prefs");
            assert_eq!(
                saved.get("allowed_labels").cloned().unwrap_or(json!([])),
                expected
            );
            let reply: Value = serde_json::from_slice(&env.stdout).expect("JSON reply");
            let output: Value = serde_json::from_str(reply["output"].as_str().expect("output"))
                .expect("config output");
            assert_eq!(output["allowed_labels"], expected);
        }
    }

    // Issue #3814 AC-2: semantic policy belongs to the handler so dispatch can
    // report a structured operation refusal instead of a parse-only stderr.
    #[test]
    fn issue_monitor_config_set_on_direction_reaches_handler_validation() {
        assert_eq!(
            ok(
                "issue.monitor.config.set",
                json!({"enabled": true, "max_active": 7})
            ),
            CliCommand::Issue(IssueCommand::MonitorConfigSet {
                expected_config_token: None,
                allowed_labels: None,
                project_root: None,
                enabled: Some(true),
                autonomous_mode: None,
                max_active: Some(7),
                max_active_auto: false,
                auto_close_merged_issues: None,
                launch_agent: None,
                update_drain: None,
            })
        );
        assert_eq!(
            ok("issue.monitor.config.set", json!({"autonomous_mode": true})),
            CliCommand::Issue(IssueCommand::MonitorConfigSet {
                expected_config_token: None,
                allowed_labels: None,
                project_root: None,
                enabled: None,
                autonomous_mode: Some(true),
                max_active: None,
                max_active_auto: false,
                auto_close_merged_issues: None,
                launch_agent: None,
                update_drain: None,
            })
        );
        assert_eq!(
            ok(
                "issue.monitor.config.set",
                json!({"auto_close_merged_issues": false})
            ),
            CliCommand::Issue(IssueCommand::MonitorConfigSet {
                expected_config_token: None,
                allowed_labels: None,
                project_root: None,
                enabled: None,
                autonomous_mode: None,
                max_active: None,
                max_active_auto: false,
                auto_close_merged_issues: Some(false),
                launch_agent: None,
                update_drain: None,
            }),
            "Issue #3917 AC-5: the auto-close override is settable on its own"
        );
    }

    #[test]
    fn issue_monitor_config_set_guard_is_presence_aware() {
        for operation in ["issue.monitor.config.set", "issue.monitor.config-set"] {
            ok(operation, json!({"max_active": 3}));
            let CliCommand::Issue(IssueCommand::MonitorConfigSet {
                expected_config_token,
                ..
            }) = ok(
                operation,
                json!({"max_active": 3, "expected_config_token": "observed-config"}),
            )
            else {
                panic!("expected config command")
            };
            assert_eq!(expected_config_token.as_deref(), Some("observed-config"));
            for invalid in [json!(null), json!(false), json!(2), json!([]), json!("")] {
                err(
                    operation,
                    json!({"max_active": 3, "expected_config_token": invalid}),
                );
            }
        }
    }

    #[test]
    fn issue_monitor_config_set_rejects_auto_apply_updates() {
        for operation in ["issue.monitor.config.set", "issue.monitor.config-set"] {
            for enabled in [true, false] {
                for params in [
                    json!({"auto_apply_updates": enabled}),
                    json!({"auto_apply_updates": enabled, "enabled": false}),
                ] {
                    let error = err(operation, params).to_string();
                    assert!(
                        error.contains("unknown parameter auto_apply_updates"),
                        "{error}"
                    );
                    assert!(error.contains("Settings > System"), "{error}");
                }
            }
        }
    }

    #[test]
    fn update_settings_status_is_read_only_and_does_not_write_config() {
        let temp = tempfile::TempDir::new().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let mut env = TestEnv::new(repo.clone());
        env.stdin = envelope("update.settings.status", json!({"project_root": repo}));

        assert_eq!(
            super::dispatch(&mut env, "gwtd"),
            0,
            "{}",
            String::from_utf8_lossy(&env.stderr)
        );
        let reply: Value = serde_json::from_slice(&env.stdout).unwrap();
        let status: Value = serde_json::from_str(reply["output"].as_str().unwrap()).unwrap();
        assert_eq!(
            status,
            json!({
                "auto_apply_updates": null,
                "auto_apply_updates_effective": false,
                "update_drain": null,
            })
        );
        assert!(!gwt_core::paths::gwt_config_path().exists());
        assert!(
            crate::cli::hook::workflow_policy::is_read_only_json_envelope_operation(
                "update.settings.status"
            )
        );
        assert!(matches!(
            ok("update.settings.set", json!({"auto_apply_updates": false})),
            CliCommand::PmAuthorization(_)
        ));
    }

    #[test]
    fn agent_capacity_config_envelope_accepts_explicit_auto() {
        let _ = ok(
            "issue.monitor.config.set",
            json!({"max_active_mode": "auto"}),
        );
        let _ = ok(
            "issue.monitor.config.set",
            json!({"max_active_mode": "manual", "max_active": 4, "enabled": false}),
        );
        for params in [
            json!({"max_active_mode": "invalid", "max_active": 4}),
            json!({"max_active_mode": "auto", "max_active": 4}),
            json!({"max_active_mode": "manual", "enabled": false}),
        ] {
            assert!(matches!(
                err("issue.monitor.config.set", params),
                CliParseError::InvalidJson(_)
            ));
        }
    }

    /// Issue #3923 AC-5: `launch_agent` alone is a complete config.set.
    /// Issue #4037 AC-5: the drain is settable on its own, as a plain bool.
    #[test]
    fn issue_monitor_config_set_accepts_update_drain_alone() {
        assert_eq!(
            ok("issue.monitor.config.set", json!({"update_drain": true})),
            CliCommand::Issue(IssueCommand::MonitorConfigSet {
                expected_config_token: None,
                allowed_labels: None,
                project_root: None,
                enabled: None,
                autonomous_mode: None,
                max_active: None,
                max_active_auto: false,
                auto_close_merged_issues: None,
                launch_agent: None,
                update_drain: Some(crate::IssueMonitorUpdateDrainControl::Toggle(true)),
            })
        );
        assert_eq!(
            ok("issue.monitor.config.set", json!({"update_drain": false})),
            CliCommand::Issue(IssueCommand::MonitorConfigSet {
                expected_config_token: None,
                allowed_labels: None,
                project_root: None,
                enabled: None,
                autonomous_mode: None,
                max_active: None,
                max_active_auto: false,
                auto_close_merged_issues: None,
                launch_agent: None,
                update_drain: Some(crate::IssueMonitorUpdateDrainControl::Toggle(false)),
            })
        );
        assert!(matches!(
            err("issue.monitor.config.set", json!({"update_drain": "yes"})),
            CliParseError::InvalidJson(_)
        ));
    }

    #[test]
    fn issue_monitor_config_set_accepts_launch_agent_alone() {
        assert_eq!(
            ok(
                "issue.monitor.config.set",
                json!({"launch_agent": "claude"})
            ),
            CliCommand::Issue(IssueCommand::MonitorConfigSet {
                expected_config_token: None,
                allowed_labels: None,
                project_root: None,
                enabled: None,
                autonomous_mode: None,
                max_active: None,
                max_active_auto: false,
                auto_close_merged_issues: None,
                launch_agent: Some("claude".to_string()),
                update_drain: None,
            })
        );
        assert!(matches!(
            err("issue.monitor.config.set", json!({})),
            CliParseError::MissingFlag(
                "allowed_labels|enabled|autonomous_mode|max_active|auto_close_merged_issues|launch_agent|update_drain"
            )
        ));
    }

    #[test]
    fn issue_monitor_queue_demote_guard_parses() {
        let CliCommand::Issue(IssueCommand::MonitorQueueDemote { .. }) = ok(
            "issue.monitor.queue.demote",
            json!({"number": 2, "expected_urgent_queue_token": "observed-urgent"}),
        ) else {
            panic!("demote command")
        };
        for invalid in [
            json!(null),
            json!(false),
            json!(2),
            json!([]),
            json!(""),
            json!(" "),
        ] {
            err(
                "issue.monitor.queue.demote",
                json!({"number": 2, "expected_urgent_queue_token": invalid}),
            );
        }
    }

    #[test]
    fn issue_monitor_queue_urgent_limit_config_guard_parses() {
        let CliCommand::Issue(IssueCommand::MonitorQueueUrgentLimit { .. }) = ok(
            "issue.monitor.queue.urgent_limit",
            json!({"limit": 0, "expected_config_token": "observed-config"}),
        ) else {
            panic!("urgent limit command")
        };
        for invalid in [
            json!(null),
            json!(false),
            json!(2),
            json!([]),
            json!(""),
            json!(" "),
        ] {
            err(
                "issue.monitor.queue.urgent_limit",
                json!({"limit": 0, "expected_config_token": invalid}),
            );
        }
    }

    #[test]
    fn issue_monitor_queue_auto_refill_persists_without_changing_other_preferences() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = gwt_core::test_support::ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                priority_order: vec![7, 42],
                ..Default::default()
            },
        )
        .expect("seed prefs");
        let mut env = TestEnv::new(repo);
        let command = ok(
            "issue.monitor.queue.auto_refill",
            json!({"enabled": true, "limit": 3}),
        );
        let CliCommand::Issue(command) = command else {
            panic!("issue command")
        };
        let mut out = String::new();
        crate::cli::issue::run(&mut env, command, &mut out).expect("auto refill config");
        let prefs = crate::load_issue_monitor_prefs(&path).expect("prefs");
        assert!(prefs.terminal_queue_auto_refill);
        assert_eq!(prefs.terminal_queue_auto_refill_limit, 3);
        assert!(prefs.enabled);
        assert_eq!(prefs.priority_order, vec![7, 42]);

        let mut invoke = |params| {
            let CliCommand::Issue(command) = ok("issue.monitor.queue.auto_refill", params) else {
                panic!("issue command")
            };
            let mut out = String::new();
            crate::cli::issue::run(&mut env, command, &mut out).map(|_| out)
        };
        let token = prefs.config_token();
        // An intervening writer changes only the observed refill limit.
        crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
            prefs.terminal_queue_auto_refill_limit = 4;
            Ok(())
        })
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        let error = invoke(json!({
            "enabled": false, "limit": 0, "expected_config_token": token
        }))
        .expect_err("stale refill configuration must be refused");
        assert!(error.to_string().contains("config changed"), "{error}");
        assert!(
            error.to_string().contains("issue.monitor.status"),
            "{error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let token = crate::load_issue_monitor_prefs(&path)
            .unwrap()
            .config_token();
        let reply: Value = serde_json::from_str(
            &invoke(json!({
                "enabled": false, "limit": 0, "expected_config_token": token
            }))
            .expect("fresh observation must succeed"),
        )
        .unwrap();
        let saved = crate::load_issue_monitor_prefs(&path).unwrap();
        assert!(!saved.terminal_queue_auto_refill);
        assert_eq!(saved.terminal_queue_auto_refill_limit, 0);
        assert!(saved.enabled);
        assert_eq!(saved.priority_order, vec![7, 42]);
        assert_eq!(reply["enabled"], false);
        assert_eq!(reply["limit"], 0);
        assert_eq!(reply["saved_config"], saved.config_observation());
        assert_eq!(reply["config_token"], saved.config_token());
        assert_ne!(reply["config_token"], token);
        for invalid in [
            json!(null),
            json!(false),
            json!(2),
            json!([]),
            json!(""),
            json!(" "),
        ] {
            err(
                "issue.monitor.queue.auto_refill",
                json!({"enabled": true, "limit": 3, "expected_config_token": invalid}),
            );
        }
        let _ = err("issue.monitor.queue.auto_refill", json!({"enabled": true}));
        let _ = err("issue.monitor.queue.auto_refill", json!({"limit": 3}));
    }

    // SPEC-3431 T-020 (FR-006): launch_now is the PM's launch instruction —
    // priority head-move plus an immediate scan. It never spawns anything
    // itself; the Monitor's claim/slot path stays the only launcher.
    #[test]
    fn issue_monitor_launch_now_parses() {
        assert_eq!(
            ok("issue.monitor.launch_now", json!({"number": 42})),
            CliCommand::Issue(IssueCommand::MonitorLaunchNow {
                expected_priority_order: None,
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
                expected_priority_order: None,
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
                reason: "provider rate limit".to_string(),
                remove_from_queue: true,
                claim_id: None,
                delivery_id: None,
                window_id: Some("tab-1::agent-1".to_string()),
            })
        );
        assert_eq!(
            ok(
                "issue.monitor.stop",
                json!({
                    "project_root": "/tmp/project",
                    "number": 7,
                    "reason": "switch provider",
                    "remove_from_queue": false,
                    "claim_id": "claim-1",
                    "delivery_id": "launch:effect-1",
                })
            ),
            CliCommand::Issue(IssueCommand::MonitorStop {
                project_root: Some(std::path::PathBuf::from("/tmp/project")),
                number: 7,
                reason: "switch provider".to_string(),
                remove_from_queue: false,
                claim_id: Some("claim-1".to_string()),
                delivery_id: Some("launch:effect-1".to_string()),
                window_id: None,
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
                reason: "codex rate limit".to_string(),
                claim_id: Some("claim-1".to_string()),
                delivery_id: None,
                window_id: Some("tab-1::agent-1".to_string()),
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

    /// Issue #3645 / #3628: the recovery takes no launch identity, because the
    /// row it recovers has no launch left to name — that is precisely why the
    /// stop and failover operations cannot reach it. A reason stays mandatory:
    /// releasing a hold that something recorded for a cause is auditable work.
    /// Issue #3844: an agent declares (or clears) a wait for its own launch.
    /// `number` is optional because the launch context already names the
    /// owner Issue; `reason` and `resume_condition` are mandatory unless the
    /// declaration is being cleared, because an unexplained wait is exactly
    /// the state the PM cannot act on.
    #[test]
    fn issue_monitor_wait_parses() {
        assert_eq!(
            ok(
                "issue.monitor.wait",
                json!({
                    "reason": "host 排他の順番待ち",
                    "resume_condition": "#3791 の verify が完了する",
                })
            ),
            CliCommand::Issue(IssueCommand::MonitorWait {
                project_root: None,
                number: None,
                reason: Some("host 排他の順番待ち".to_string()),
                resume_condition: Some("#3791 の verify が完了する".to_string()),
                clear: false,
            })
        );
        assert_eq!(
            ok(
                "issue.monitor.wait",
                json!({ "project_root": "/tmp/project", "number": 42, "clear": true })
            ),
            CliCommand::Issue(IssueCommand::MonitorWait {
                project_root: Some(std::path::PathBuf::from("/tmp/project")),
                number: Some(42),
                reason: None,
                resume_condition: None,
                clear: true,
            })
        );
        assert!(matches!(
            err("issue.monitor.wait", json!({"resume_condition": "x"})),
            CliParseError::MissingFlag("reason")
        ));
        assert!(matches!(
            err("issue.monitor.wait", json!({"reason": "x"})),
            CliParseError::MissingFlag("resume_condition")
        ));
    }

    /// Issue #4286 AC-1/AC-2: the PM invalidates one wait declaration. The
    /// target and the reason are mandatory because an unexplained
    /// invalidation is exactly the record AC-2 says must exist.
    #[test]
    fn issue_monitor_wait_invalidate_parses() {
        assert_eq!(
            ok(
                "issue.monitor.wait.invalidate",
                json!({ "number": 42, "reason": "bootstrap build needs no lease" })
            ),
            CliCommand::Issue(IssueCommand::MonitorWaitInvalidate {
                project_root: None,
                number: 42,
                reason: "bootstrap build needs no lease".to_string(),
                by: None,
            })
        );
        assert_eq!(
            ok(
                "issue.monitor.wait.invalidate",
                json!({
                    "project_root": "/tmp/project",
                    "number": 42,
                    "reason": "ruled out",
                    "by": "session:pm",
                })
            ),
            CliCommand::Issue(IssueCommand::MonitorWaitInvalidate {
                project_root: Some(std::path::PathBuf::from("/tmp/project")),
                number: 42,
                reason: "ruled out".to_string(),
                by: Some("session:pm".to_string()),
            })
        );
        assert!(matches!(
            err("issue.monitor.wait.invalidate", json!({"reason": "x"})),
            CliParseError::MissingFlag("number")
        ));
        assert!(matches!(
            err("issue.monitor.wait.invalidate", json!({"number": 42})),
            CliParseError::MissingFlag("reason")
        ));
    }

    /// Issue #3883 AC-6: the PM's recovery for launches that are still running
    /// but no longer tracked. It takes no target and no reason because it
    /// revokes nothing — it only re-adopts what the canvas already shows.
    #[test]
    fn issue_monitor_reconcile_parses_with_an_optional_project_scope() {
        assert_eq!(
            ok("issue.monitor.reconcile", json!({})),
            CliCommand::Issue(IssueCommand::MonitorReconcile { project_root: None })
        );
        assert_eq!(
            ok(
                "issue.monitor.reconcile",
                json!({"project_root": "/tmp/gwt-project"})
            ),
            CliCommand::Issue(IssueCommand::MonitorReconcile {
                project_root: Some(std::path::PathBuf::from("/tmp/gwt-project")),
            })
        );
    }

    /// Issue #3923 AC-1: provider quota holds are listed and cleared through
    /// JSON operations, and a clear must name its provider and reason.
    #[test]
    fn issue_monitor_quota_hold_operations_parse() {
        assert_eq!(
            ok("issue.monitor.quota_hold.list", json!({})),
            CliCommand::Issue(IssueCommand::MonitorQuotaHoldList { project_root: None })
        );
        assert_eq!(
            ok(
                "issue.monitor.quota_hold.clear",
                json!({"provider": "codex", "reason": "Codex is not rate limited"})
            ),
            CliCommand::Issue(IssueCommand::MonitorQuotaHoldClear {
                project_root: None,
                provider: "codex".to_string(),
                reason: "Codex is not rate limited".to_string(),
            })
        );
        assert!(matches!(
            err(
                "issue.monitor.quota_hold.clear",
                json!({"provider": "codex"})
            ),
            CliParseError::MissingFlag("reason")
        ));
        assert!(matches!(
            err("issue.monitor.quota_hold.clear", json!({"reason": "x"})),
            CliParseError::MissingFlag("provider")
        ));
    }

    #[test]
    fn issue_monitor_requeue_parses() {
        assert_eq!(
            ok(
                "issue.monitor.requeue",
                json!({
                    "number": 3624,
                    "reason": "manual recovery after the launch-fence outage",
                })
            ),
            CliCommand::Issue(IssueCommand::MonitorRequeue {
                project_root: None,
                number: 3624,
                reason: "manual recovery after the launch-fence outage".to_string(),
                force: false,
            })
        );
        assert!(matches!(
            err("issue.monitor.requeue", json!({"number": 42})),
            CliParseError::MissingFlag("reason")
        ));
        assert!(matches!(
            err("issue.monitor.requeue", json!({"reason": "x"})),
            CliParseError::MissingFlag("number")
        ));
        assert!(matches!(
            ok(
                "issue.monitor.requeue",
                json!({"number": 42, "reason": "explicit recovery", "force": true})
            ),
            CliCommand::Issue(IssueCommand::MonitorRequeue { force: true, .. })
        ));
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
                operation_id: None,
            })
        );
        assert_ne!(
            ok(
                "pm.message.send",
                json!({
                    "id": "tab-1::agent-1",
                    "text": "please report status",
                    "operation_id": "72fc3cd4-ad49-43e3-bf3d-d791357643c2"
                })
            ),
            ok(
                "pm.message.send",
                json!({"id": "tab-1::agent-1", "text": "please report status"})
            ),
            "same-operation recovery must preserve the caller's operation identity"
        );
        ok(
            "pm.operation.execute",
            json!({
                "operation": "pm.message.send",
                "params": {
                    "id": "tab-1::agent-1",
                    "text": "please report status",
                    "operation_id": "72fc3cd4-ad49-43e3-bf3d-d791357643c2"
                },
                "approval_id": "scoped-authority"
            }),
        );
        for operation_id in [
            json!(""),
            json!("not-a-uuid"),
            json!("72FC3CD4-AD49-43E3-BF3D-D791357643C2"),
            json!("72fc3cd4ad4943e3bf3dd791357643c2"),
            Value::Null,
        ] {
            assert!(matches!(
                err(
                    "pm.message.send",
                    json!({"id": "tab-1::agent-1", "text": "status", "operation_id": operation_id})
                ),
                CliParseError::InvalidValue {
                    flag: "operation_id",
                    ..
                }
            ));
            assert!(parse(&envelope(
                "pm.operation.execute",
                json!({
                    "operation": "pm.message.send",
                    "params": {"id": "tab-1::agent-1", "text": "status", "operation_id": operation_id},
                    "approval_id": "scoped-authority"
                })
            ))
            .is_err());
        }
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
    fn manual_recovery_record_uses_the_exact_board_delivery_path() {
        let command = ok(
            "issue.monitor.recovery.record",
            json!({
                "number":5876, "recovery_id":"incident-a", "body":"停止窓を再投入した",
                "recovered_at":"2026-10-20T10:00:00+09:00"
            }),
        );
        assert!(matches!(command,
            CliCommand::Board(BoardCommand::RecoveryPost { intent_id, command })
            if intent_id == "manual-recovery:5876:incident-a"
                && command.kind == "decision"
                && command.owners == ["5876"]
                && command.topics == ["gwt-manual-recovery:5876:incident-a", "gwt-manual-recovery-at:2026-10-20T01:00:00+00:00"]
                && command.body.as_deref() == Some("停止窓を再投入した")
        ));
        for mut params in [
            json!({"number":5876,"body":"recovered"}),
            json!({"number":5876,"recovery_id":" ","body":"recovered"}),
            json!({"number":0,"recovery_id":"incident","body":"recovered"}),
            json!({"number":5876,"recovery_id":"incident","body":" "}),
            json!({"number":5876,"recovery_id":"incident","body":"recovered","session_id":"fake"}),
        ] {
            params["recovered_at"] = json!("2026-10-20T01:00:00Z");
            let _ = err("issue.monitor.recovery.record", params);
        }
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
                expected_priority_order: None,
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
                expected_priority_order: None,
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
                expected_priority_order: None,
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
                expected_config_token: None,
                allowed_labels: None,
                project_root: None,
                enabled: Some(false),
                autonomous_mode: Some(false),
                max_active: Some(3),
                max_active_auto: false,
                auto_close_merged_issues: None,
                launch_agent: None,
                update_drain: None,
            })
        );
    }

    #[test]
    fn issue_monitor_priority_guard_parsing_preserves_presence() {
        for (operation, mut params) in [
            ("issue.monitor.priority-move", json!({"number":42})),
            ("issue.monitor.priority-set", json!({"issue_numbers":[42]})),
            ("issue.monitor.launch_now", json!({"number":42})),
            ("issue.monitor.launch-now", json!({"number":42})),
        ] {
            let unconditional = ok(operation, params.clone());
            params["expected_priority_order"] = json!([]);
            assert_ne!(ok(operation, params.clone()), unconditional);
            for invalid in [Value::Null, json!("1,2")] {
                params["expected_priority_order"] = invalid;
                assert!(matches!(
                    err(operation, params.clone()),
                    CliParseError::InvalidJson(_)
                ));
            }
        }
    }

    #[test]
    fn issue_monitor_queue_guard_parsing_preserves_presence() {
        for (operation, mut params) in [
            (
                "issue.monitor.queue.move",
                json!({"number":42,"position":0}),
            ),
            ("issue.monitor.queue.remove", json!({"numbers":[42]})),
        ] {
            let unconditional = ok(operation, params.clone());
            params["expected_queue_order"] = json!([]);
            assert_ne!(ok(operation, params.clone()), unconditional);
            for invalid in [Value::Null, json!("1,2")] {
                params["expected_queue_order"] = invalid;
                assert!(matches!(
                    err(operation, params.clone()),
                    CliParseError::InvalidJson(_)
                ));
            }
        }
    }

    #[test]
    fn issue_monitor_queue_operations_reject_unsafe_or_incomplete_params() {
        for params in [json!({}), json!({"max_active": 0})] {
            assert!(matches!(
                err("issue.monitor.config.set", params),
                CliParseError::InvalidJson(_) | CliParseError::MissingFlag(_)
            ));
        }
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
    fn board_show_rejects_unknown_params() {
        let error = err("board.show", json!({"limti": 15})).to_string();
        assert!(error.contains("limti"), "{error}");
        assert!(
            error.contains("workspace, all, limit, unresolved"),
            "{error}"
        );
    }

    fn board_show_page(params: Value) -> Value {
        use gwt_core::{
            coordination::{post_entry, AuthorKind, BoardEntry, BoardEntryKind},
            test_support::{ScopedEnvVar, ScopedGwtHome},
        };
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::unset(gwt_agent::session::GWT_SESSION_ID_ENV);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        for index in 0..25 {
            post_entry(
                temp.path(),
                BoardEntry::new(
                    AuthorKind::Agent,
                    "Codex",
                    BoardEntryKind::Status,
                    format!("entry-{index}"),
                    None,
                    None,
                    vec![],
                    vec![],
                ),
            )
            .unwrap();
        }
        let mut env = TestEnv::new(temp.path().to_path_buf());
        let (code, output) = crate::cli::run_collect(&mut env, ok("board.show", params)).unwrap();
        assert_eq!(code, 0);
        serde_json::from_str(&output).unwrap()
    }

    #[test]
    fn board_show_explicit_limit_returns_latest_fifteen_even_with_all() {
        let page = board_show_page(json!({"limit": 15, "all": true}));
        let entries = page["board"]["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 15);
        assert_eq!(entries[0]["body"], "entry-10");
        assert_eq!(entries[14]["body"], "entry-24");
        assert_eq!(page["board"]["oldest_entry_id"], entries[0]["id"]);
        assert_eq!(page["board"]["newest_entry_id"], entries[14]["id"]);
        assert_eq!(page["board"]["has_more_before"], true);
        assert_eq!(page["page"]["total_entries"], 25);
        assert_eq!(page["page"]["returned_entries"], 15);
        assert_eq!(page["page"]["truncated"], true);
    }

    #[test]
    fn board_show_default_cap_and_all_override() {
        let page = board_show_page(json!({}));
        assert_eq!(page["board"]["entries"].as_array().unwrap().len(), 20);
        assert_eq!(page["board"]["entries"][0]["body"], "entry-5");
        assert_eq!(page["page"]["returned_entries"], 20);
        let all = board_show_page(json!({"all": true}));
        assert_eq!(all["board"]["entries"].as_array().unwrap().len(), 25);
        assert_eq!(all["page"]["truncated"], false);
    }

    /// Issue #4609: `board.show` exposes whether each escalation is still open,
    /// and `unresolved:true` narrows the read to the open ones.
    #[test]
    fn board_show_reports_escalation_resolution_before_and_after_resolve() {
        use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::unset(gwt_agent::session::GWT_SESSION_ID_ENV);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let mut env = TestEnv::new(temp.path().to_path_buf());
        let blocked_body = "事象: build.start が拒否された\n原因: Host が古い\n依頼: Host 更新\n再開条件: build.start が成功すること";
        let mut run = |operation: &str, params: Value| {
            let (code, output) = crate::cli::run_collect(&mut env, ok(operation, params)).unwrap();
            assert_eq!(code, 0, "{output}");
            output
        };
        for owner in ["4609", "4610"] {
            run(
                "board.post",
                json!({"kind": "blocked", "body": blocked_body, "owners": [owner]}),
            );
        }
        run(
            "board.post",
            json!({"kind": "status", "body": "not an escalation", "owners": ["4609"]}),
        );
        let show = |run: &mut dyn FnMut(&str, Value) -> String, params: Value| -> Vec<Value> {
            let page: Value = serde_json::from_str(&run("board.show", params)).unwrap();
            page["board"]["entries"].as_array().unwrap().clone()
        };
        let entries = show(&mut run, json!({"all": true}));
        let blocked: Vec<&Value> = entries.iter().filter(|e| e["kind"] == "blocked").collect();
        assert_eq!(blocked.len(), 2);
        let target_id = blocked[0]["id"].as_str().unwrap().to_string();
        assert_eq!(blocked[0]["escalation"]["resolved"], false);
        assert!(blocked[0]["escalation"]["resolved_at"].is_null());
        let status = entries.iter().find(|e| e["kind"] == "status").unwrap();
        assert!(status.get("escalation").is_none(), "{status}");

        let open = show(&mut run, json!({"all": true, "unresolved": true}));
        assert_eq!(open.len(), 2, "only the two open escalations: {open:?}");

        let resolved_out = run(
            "board.post",
            json!({"kind": "decision", "body": "Host を更新しました", "owners": ["4609"], "resolves": [target_id]}),
        );
        assert!(
            resolved_out.contains("board escalations resolved:"),
            "{resolved_out}"
        );
        let entries = show(&mut run, json!({"all": true}));
        let resolver_id = entries.iter().find(|e| e["kind"] == "decision").unwrap()["id"].clone();
        let target = entries
            .iter()
            .find(|e| e["id"] == target_id.as_str())
            .unwrap();
        assert_eq!(target["escalation"]["resolved"], true, "{target}");
        assert_eq!(target["escalation"]["resolved_by_entry_id"], resolver_id);
        assert!(target["escalation"]["resolved_at"].is_string(), "{target}");

        let open = show(&mut run, json!({"all": true, "unresolved": true}));
        assert_eq!(open.len(), 1, "{open:?}");
        assert_ne!(open[0]["id"], target_id.as_str());
        assert_eq!(open[0]["escalation"]["resolved"], false);

        // AC-5: resolving an already-closed escalation stays idempotent.
        let again = run(
            "board.post",
            json!({"kind": "decision", "body": "再度閉じる", "owners": ["4609"], "resolves": [target_id]}),
        );
        assert!(
            again.contains("board escalations already closed:"),
            "{again}"
        );
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
    fn board_sync_is_available_without_enabling_external_io() {
        let command = ok("board.sync", json!({}));
        let dir = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(dir.path().to_path_buf());
        let (code, output) = crate::cli::run_collect(&mut env, command).unwrap();
        assert_eq!(code, 0);
        assert!(output.contains("disabled"), "{output}");
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
    fn issue_label_accepts_directional_params_and_rejects_invalid_shapes() {
        for action in ["add", "remove"] {
            assert!(matches!(
                ok(
                    "issue.label",
                    json!({"number":7,"action":action,"labels":["bug"]})
                ),
                CliCommand::Issue(_)
            ));
        }
        for params in [
            json!({"number":7,"action":"remove","labels":["bug","hold"]}),
            json!({"number":7,"action":"add","labels":[]}),
            json!({"number":7,"action":"replace","labels":["bug"]}),
            json!({"number":7,"action":"add","labels":["bug"],"confirm_auto_merge":"true"}),
        ] {
            let _ = err("issue.label", params);
        }
    }

    /// SPEC #4249 FR-001: `issue.close` / `issue.reopen` need only `number`;
    /// `reason` accepts the GitHub spellings and is refused — never silently
    /// dropped — when it is not one of them, because a dropped reason closes the
    /// Issue with the wrong rationale.
    #[test]
    fn issue_close_and_reopen_parse_their_optional_params() {
        assert!(matches!(
            ok("issue.close", json!({"number": 7})),
            CliCommand::Issue(IssueCommand::Close {
                number: 7,
                reason: None,
                comment: None,
            })
        ));
        assert!(matches!(
            ok(
                "issue.close",
                json!({"number": 7, "reason": "not-planned", "comment": "why"})
            ),
            CliCommand::Issue(IssueCommand::Close {
                number: 7,
                reason: Some(gwt_github::client::IssueCloseReason::NotPlanned),
                ..
            })
        ));
        for reason in ["completed", "not_planned"] {
            assert!(
                matches!(
                    ok("issue.close", json!({"number": 7, "reason": reason})),
                    CliCommand::Issue(IssueCommand::Close {
                        reason: Some(_),
                        ..
                    })
                ),
                "{reason}"
            );
        }
        // Duplicate closure needs a canonical issue ID; tracked in #4489.
        assert!(matches!(
            err("issue.close", json!({"number": 7, "reason": "duplicate"})),
            CliParseError::InvalidJson(_)
        ));
        match err("issue.close", json!({"number": 7, "reason": "wontfix"})) {
            CliParseError::InvalidJson(message) => {
                assert!(message.contains("not_planned"), "{message}")
            }
            other => panic!("unexpected error: {other:?}"),
        }
        match err("issue.close", json!({})) {
            CliParseError::MissingFlag(flag) => assert_eq!(flag, "number"),
            other => panic!("unexpected error: {other:?}"),
        }

        assert!(matches!(
            ok("issue.reopen", json!({"number": 7})),
            CliCommand::Issue(IssueCommand::Reopen {
                number: 7,
                comment: None,
            })
        ));
        match err("issue.reopen", json!({})) {
            CliParseError::MissingFlag(flag) => assert_eq!(flag, "number"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// Issue #3865 / review: `labels` absent or `null` leaves labels alone,
    /// while an explicit empty array clears them.
    #[test]
    fn issue_edit_labels_null_means_omitted_and_empty_array_clears() {
        let absent = ok("issue.edit", json!({"number": 7, "body": "b"}));
        let null = ok(
            "issue.edit",
            json!({"number": 7, "body": "b", "labels": null}),
        );
        let cleared = ok("issue.edit", json!({"number": 7, "labels": []}));
        for command in [absent, null] {
            assert!(matches!(
                command,
                CliCommand::Issue(IssueCommand::Edit { labels: None, .. })
            ));
        }
        assert!(matches!(
            cleared,
            CliCommand::Issue(IssueCommand::Edit { labels: Some(ref labels), .. }) if labels.is_empty()
        ));
    }

    #[test]
    fn board_post_intent_id_selects_recovery_and_normal_shape_stays_normal() {
        let normal = ok("board.post", json!({"kind": "status", "body": "normal"}));
        assert!(matches!(normal, CliCommand::Board(BoardCommand::Post(_))));

        let recovery = ok(
            "board.post",
            json!({
                "kind": "status",
                "body": "recover",
                "intent_id": "stable-intent-1"
            }),
        );
        assert!(matches!(
            recovery,
            CliCommand::Board(BoardCommand::RecoveryPost {
                ref intent_id,
                ..
            }) if intent_id == "stable-intent-1"
        ));
    }

    #[test]
    fn board_post_rejects_untrusted_recovery_identity_fields() {
        for intent_id in [None, Some("stable-intent-1")] {
            for key in [
                "session_id",
                "project_id",
                "provider",
                "worktree_form",
                "recovery_id",
                "entry_id",
                "payload_digest",
                "provider_receipt",
                "operation_id",
            ] {
                let mut params = serde_json::Map::from_iter([
                    ("kind".to_string(), json!("status")),
                    ("body".to_string(), json!("recover")),
                ]);
                if let Some(intent_id) = intent_id {
                    params.insert("intent_id".to_string(), json!(intent_id));
                }
                params.insert(key.to_string(), json!("untrusted"));
                assert!(matches!(
                    err("board.post", Value::Object(params)),
                    CliParseError::InvalidValue { .. }
                ));
            }
        }
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
            "issue.cache.repair",
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
    fn fleet_operations_accept_only_explicit_generation_targets() {
        let inventory = ok("fleet.inventory", json!({}));
        assert!(!format!("{inventory:?}").is_empty());
        let target = json!({"id":"pane:agent-1","session_id":"session-1",
            "child_pid":123,"child_started_at":456});
        let disposition = ok("fleet.disposition", json!({"targets":[target]}));
        assert!(!format!("{disposition:?}").is_empty());
        assert!(err("fleet.disposition", json!({"state":"delivered"}))
            .to_string()
            .contains("state"));
        assert!(err("fleet.disposition", json!({"targets":[]}))
            .to_string()
            .contains("target"));
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
    fn index_issue_recovery_operations_are_reachable() {
        for operation in ["index.cancel", "index.repair"] {
            assert!(matches!(
                ok(operation, json!({"scope": "issues"})),
                CliCommand::Index(_)
            ));
            assert!(matches!(
                err(operation, json!({"scope": "files"})),
                CliParseError::InvalidJson(_)
            ));
        }
    }

    /// Issue #4435: `index.repair` submits by default; `wait` is the blocking
    /// form the detached worker runs. `index.cancel` never blocks.
    #[test]
    fn index_repair_wait_selects_the_blocking_form() {
        assert!(matches!(
            ok("index.repair", json!({})),
            CliCommand::Index(IndexCommand::Repair { wait: false })
        ));
        assert!(matches!(
            ok("index.repair", json!({"scope": "issues", "wait": true})),
            CliCommand::Index(IndexCommand::Repair { wait: true })
        ));
        match err("index.cancel", json!({"wait": true})) {
            CliParseError::InvalidJson(message) => {
                assert!(
                    message.contains("index.cancel does not take wait"),
                    "{message}"
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
        match err("index.repair", json!({"wait": "yes"})) {
            CliParseError::InvalidJson(message) => {
                assert!(message.contains("wait must be a bool"), "{message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// Issue #4435 AC-1: the envelope is the operation's only answer, so a
    /// failed write must surface instead of leaving the caller with zero
    /// bytes on both streams and an exit code that reads as success.
    #[test]
    fn an_undelivered_response_is_reported_rather_than_dropped() {
        struct ClosedPipe;
        impl std::io::Write for ClosedPipe {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "the pipe has been ended",
                ))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let payload = json!({"ok": true, "operation": "index.repair"});

        let mut delivered = Vec::new();
        write_response(&mut delivered, &payload).expect("a writable stdout accepts the envelope");
        assert!(!delivered.is_empty());
        assert!(delivered.ends_with(b"\n"));

        let error = write_response(&mut ClosedPipe, &payload)
            .expect_err("a broken stdout must not be reported as delivered");
        assert!(error.contains("pipe"), "{error}");

        let mut env = crate::cli::env::TestEnv::new(std::path::PathBuf::from("cache"));
        let code = report_undelivered_response(&mut env, "gwtd", "index.repair", &error);
        assert_eq!(code, RESPONSE_NOT_DELIVERED_EXIT);
        assert_ne!(code, 0, "an undelivered answer must not exit as success");
        let reported = String::from_utf8(env.stderr.clone()).expect("stderr is utf-8");
        assert!(
            reported.contains("index.repair: response envelope was not delivered"),
            "{reported}"
        );
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
    fn pm_report_operations_parse_and_expose_read_only_listing() {
        let command = ok(
            "pm.report.post",
            json!({"kind": "decision", "body": "# Decision", "project_root": "/tmp/project"}),
        );
        assert!(matches!(command, CliCommand::Pm(_)));
        assert!(matches!(ok("pm.report.list", json!({})), CliCommand::Pm(_)));
        assert!(matches!(
            ok("pm.report.list", json!({"project_root": "/tmp/project"})),
            CliCommand::Pm(_)
        ));
        assert!(matches!(
            err("pm.report.post", json!({"kind":"other", "body":"text"})),
            CliParseError::InvalidValue { flag: "kind", .. }
        ));
        assert!(matches!(
            err("pm.report.post", json!({"kind":"progress"})),
            CliParseError::MissingFlag("body")
        ));
        assert!(
            crate::cli::hook::workflow_policy::is_read_only_json_envelope_operation(
                "pm.report.list"
            )
        );
        assert!(
            !crate::cli::hook::workflow_policy::is_read_only_json_envelope_operation(
                "pm.report.post"
            )
        );
        for operation in ["pm.report.post", "pm.report.list"] {
            assert!(crate::cli::pm::capabilities_report()
                .capabilities
                .iter()
                .any(|row| row.operation == operation && row.executable));
        }
    }

    #[test]
    fn qa_reports_require_a_nonempty_question() {
        assert!(matches!(
            ok(
                "pm.report.post",
                json!({"kind":"qa", "question":"Can we ship?", "body":"Checks passed."})
            ),
            CliCommand::Pm(_)
        ));
        for params in [
            json!({"kind":"qa", "body":"Checks passed."}),
            json!({"kind":"qa", "question":" \n ", "body":"Checks passed."}),
        ] {
            assert!(matches!(
                err("pm.report.post", params),
                CliParseError::MissingFlag("question")
            ));
        }
    }

    #[test]
    fn pm_report_post_requires_pm_authority_except_for_direct_cli() {
        use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "-q"],
            vec![
                "remote",
                "add",
                "origin",
                "https://github.com/example/reports-post.git",
            ],
        ] {
            assert!(gwt_core::process::hidden_command("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        let mut env = TestEnv::new(repo.clone());
        let post = || match ok(
            "pm.report.post",
            json!({"kind":"qa", "question":"Can we ship?", "body":"Checks passed."}),
        ) {
            CliCommand::Pm(command) => command,
            other => panic!("unexpected report command: {other:?}"),
        };
        let _direct = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        let mut output = String::new();
        assert_eq!(
            crate::cli::pm::run(&mut env, post(), &mut output).unwrap(),
            0
        );
        let saved: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(saved["ok"], true);
        assert_eq!(saved["report"]["kind"], "qa");
        assert_eq!(saved["report"]["question"], "Can we ship?");
        assert_eq!(saved["report"]["body"], "Checks passed.");
        let broker = gwt_core::index::broker::RefreshBroker::open_default().unwrap();
        let scope = serde_json::from_str("\"reports\"").expect("Reports refresh scope");
        let hash = crate::index_worker::detect_repo_hash(&repo).unwrap();
        let target = gwt_core::index::broker::RefreshTarget::base(hash.as_str(), [scope]);
        let posted_epoch = broker
            .inspect()
            .unwrap()
            .target(&target)
            .expect("post queues Reports")
            .desired_epoch();
        let _agent = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "registered-pm");
        output.clear();
        let error = crate::cli::pm::run(&mut env, post(), &mut output).unwrap_err();
        assert!(error.to_string().contains("registered PM"), "{error}");
        crate::pm_registry::save_pm_prefs(
            &crate::pm_registry::pm_prefs_path_for_repo_path(&repo),
            &crate::pm_registry::PmPrefs {
                registration: Some(crate::pm_registry::PmRegistration {
                    session_id: "registered-pm".into(),
                    agent_id: "codex".into(),
                    worktree_path: repo.display().to_string(),
                    created_at: None,
                    consecutive_crashes: 0,
                    next_not_before: None,
                }),
                ..crate::pm_registry::PmPrefs::default()
            },
        )
        .unwrap();
        let error = crate::cli::pm::run(&mut env, post(), &mut output).unwrap_err();
        assert!(error.to_string().contains("instruction"), "{error}");
        let capture = || match ok(
            "pm.report.post",
            json!({"kind":"qa", "question":"Can we ship?", "body":"Checks passed.",
                "instruction":{"original":"Can we ship? Please explain all results.",
                    "kind":"question", "actions":["Verified checks: passed"], "issue_numbers":[5342]}}),
        ) {
            CliCommand::Pm(command) => command,
            other => panic!("unexpected report command: {other:?}"),
        };
        let peer_path = gwt_core::paths::gwt_home().join("pm-peer-messages.json");
        std::fs::write(&peer_path, b"{").unwrap();
        assert_eq!(
            crate::cli::pm::run(&mut env, capture(), &mut output).unwrap(),
            0
        );
        let saved: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(
            saved["instruction"]["original"],
            "Can we ship? Please explain all results."
        );
        assert_eq!(saved["instruction"]["id"], saved["report"]["id"]);
        assert_eq!(gwt_core::user_instruction::load(&repo).unwrap().len(), 1);
        assert!(
            broker
                .inspect()
                .unwrap()
                .target(&target)
                .unwrap()
                .desired_epoch()
                > posted_epoch
        );
        assert_eq!(std::fs::read(&peer_path).unwrap(), b"{");
        let CliCommand::Pm(list) = ok("pm.report.list", json!({})) else {
            panic!("expected PM list")
        };
        output.clear();
        crate::cli::pm::run(&mut env, list, &mut output).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&output).unwrap()["reports"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            serde_json::from_str::<Value>(&output).unwrap()["reports"][1]["question"],
            "Can we ship?"
        );
        let source = gwt_core::user_instruction::instructions_path(&repo);
        let before = std::fs::read(&source).unwrap();
        let mut oversized = capture();
        let crate::cli::pm::PmCommand::ReportPost { question, .. } = &mut oversized else {
            panic!("expected Q&A capture")
        };
        *question = Some("q".repeat(64 * 1024 + 1));
        output.clear();
        let error = crate::cli::pm::run(&mut env, oversized, &mut output).unwrap_err();
        assert!(error.to_string().contains("question"), "{error}");
        assert_eq!(std::fs::read(&source).unwrap(), before);
        std::fs::write(gwt_core::pm_report::reports_path(&repo), b"broken\n").unwrap();
        output.clear();
        assert!(crate::cli::pm::run(&mut env, capture(), &mut output).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), before);
    }

    #[test]
    fn pm_status_set_parses_typed_content_and_rejects_invalid_cards() {
        let valid = json!({
            "items": [{"kind":"working", "text":"Reviewing CI", "started_at":"2026-10-13T00:00:00Z"}],
            "next_trigger":"CI completes", "project_root":"/tmp/project"
        });
        assert!(matches!(
            ok("pm.status.set", valid.clone()),
            CliCommand::Pm(_)
        ));
        for (key, value) in [
            ("items", json!([])),
            (
                "items",
                json!([{"kind":"done", "text":"Done", "started_at":"2026-10-13T00:00:00Z"}]),
            ),
            (
                "items",
                json!([{"kind":"working", "text":" ", "started_at":"2026-10-13T00:00:00Z"}]),
            ),
            (
                "items",
                json!([{"kind":"working", "text":"Review", "started_at":"yesterday"}]),
            ),
            ("next_trigger", json!(" ")),
            ("stale_after_minutes", json!(0)),
            ("stale_after_minutes", json!(-1)),
            ("stale_after_minutes", json!(4_294_967_296_u64)),
            ("user_action", json!(" ")),
        ] {
            let mut invalid = valid.clone();
            invalid[key] = value;
            assert!(matches!(
                err("pm.status.set", invalid),
                CliParseError::InvalidJson(_)
            ));
        }
        let mut missing = valid;
        missing.as_object_mut().unwrap().remove("next_trigger");
        assert!(matches!(
            err("pm.status.set", missing),
            CliParseError::InvalidJson(_)
        ));
        assert!(
            !crate::cli::hook::workflow_policy::is_read_only_json_envelope_operation(
                "pm.status.set"
            )
        );
        assert!(crate::cli::pm::capabilities_report()
            .capabilities
            .iter()
            .any(|row| row.operation == "pm.status.set" && row.executable));
    }

    #[test]
    fn pm_status_set_requires_pm_authority_and_preserves_registration_diagnostics() {
        use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let mut env = TestEnv::new(repo.clone());
        let set = || match ok(
            "pm.status.set",
            json!({
                "items":[{"kind":"waiting", "text":"Waiting for CI", "started_at":"2026-10-13T00:00:00Z"}],
                "next_trigger":"CI completes"
            }),
        ) {
            CliCommand::Pm(command) => command,
            other => panic!("unexpected status command: {other:?}"),
        };
        let _direct = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        let mut output = String::new();
        assert_eq!(
            crate::cli::pm::run(&mut env, set(), &mut output).unwrap(),
            0
        );
        let saved: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(saved["status"]["stale_after_minutes"], 15);
        assert_eq!(saved["status"]["items"][0]["kind"], "waiting");
        assert_eq!(saved["status"]["next_trigger"], "CI completes");
        let _agent = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "registered-pm");
        output.clear();
        let error = crate::cli::pm::run(&mut env, set(), &mut output).unwrap_err();
        assert!(error.to_string().contains("registered PM"), "{error}");
        crate::pm_registry::save_pm_prefs(
            &crate::pm_registry::pm_prefs_path_for_repo_path(&repo),
            &crate::pm_registry::PmPrefs {
                registration: Some(crate::pm_registry::PmRegistration {
                    session_id: "registered-pm".into(),
                    agent_id: "codex".into(),
                    worktree_path: repo.display().to_string(),
                    created_at: None,
                    consecutive_crashes: 0,
                    next_not_before: None,
                }),
                ..crate::pm_registry::PmPrefs::default()
            },
        )
        .unwrap();
        assert_eq!(
            crate::cli::pm::run(&mut env, set(), &mut output).unwrap(),
            0
        );
        output.clear();
        let CliCommand::Pm(status) = ok("pm.status", json!({})) else {
            panic!("expected PM status")
        };
        crate::cli::pm::run(&mut env, status, &mut output).unwrap();
        let diagnostics: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(diagnostics["registered"], true);
        assert_eq!(diagnostics["caller_is_registered_pm"], true);
        assert_eq!(diagnostics["registration"]["session_id"], "registered-pm");
        assert_eq!(diagnostics["current_status"], saved["status"]);
        assert_eq!(diagnostics["status_history"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn status_store_is_shared_by_originless_repo_pm_checkout_and_nested_cwd() {
        use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _direct = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        let repo = home.path().join("repo");
        let git_dir = repo.join(".git");
        let admin = git_dir.join("worktrees/pm");
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git_dir.join("config"), "[core]\n\tbare = false\n").unwrap();
        let project = gwt_core::paths::gwt_project_dir_for_repo_path(&repo);
        let pm = project.join("pm/worktree");
        let nested = pm.join("src/nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(admin.join("commondir"), "../..\n").unwrap();
        std::fs::write(pm.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        assert_ne!(
            gwt_core::pm_status::status_path(&repo),
            gwt_core::pm_status::status_path(&pm)
        );
        assert_eq!(
            crate::pm_registry::pm_repository_key(&repo),
            crate::pm_registry::pm_repository_key(&pm)
        );
        crate::pm_registry::save_pm_prefs(
            &project.join("project-state/pm.json"),
            &crate::pm_registry::PmPrefs {
                registration: Some(crate::pm_registry::PmRegistration {
                    session_id: "pm-session".into(),
                    agent_id: "codex".into(),
                    worktree_path: pm.display().to_string(),
                    created_at: None,
                    consecutive_crashes: 0,
                    next_not_before: None,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        {
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
            crate::cli::hook::pm_reporting::handle_user_prompt_submit(
                &pm,
                r#"{"prompt":"What is happening now?"}"#,
            )
            .unwrap();
        }
        let mut latest = Value::Null;
        for (cwd, text) in [
            (&repo, "Original repo"),
            (&pm, "PM checkout"),
            (&nested, "Nested PM cwd"),
        ] {
            let CliCommand::Pm(command) = ok(
                "pm.status.set",
                json!({
                    "items":[{"kind":"working", "text":text, "started_at":"2026-10-13T00:00:00Z"}],
                    "next_trigger":"CI completes"
                }),
            ) else {
                panic!("expected status set")
            };
            let mut output = String::new();
            crate::cli::pm::run(&mut TestEnv::new(cwd.clone()), command, &mut output).unwrap();
            latest = serde_json::from_str::<Value>(&output).unwrap()["status"].clone();
        }
        for cwd in [&repo, &pm, &nested] {
            let current = crate::pm_current::latest_status(cwd).unwrap();
            assert_eq!(
                serde_json::to_value(current).unwrap(),
                latest,
                "current status from {}",
                cwd.display()
            );
            let CliCommand::Pm(command) = ok("pm.status", json!({})) else {
                panic!("expected status")
            };
            let mut output = String::new();
            crate::cli::pm::run(&mut TestEnv::new(cwd.clone()), command, &mut output).unwrap();
            let diagnostics: Value = serde_json::from_str(&output).unwrap();
            assert_eq!(diagnostics["current_status"], latest);
            assert_eq!(diagnostics["status_history"].as_array().unwrap().len(), 3);
            assert_eq!(diagnostics["reporting"]["requires_qa"], true);
        }
        assert_eq!(
            gwt_core::pm_status::load_status_history(&repo)
                .unwrap()
                .len(),
            3
        );
        assert!(!gwt_core::pm_status::status_path(&pm).exists());
    }

    #[test]
    fn pm_peer_send_strict_boundary() {
        let valid = json!({"project":"/tmp/peer","kind":"request","text":"Please checkpoint"});
        assert!(matches!(
            ok("pm.peer.send", valid.clone()),
            CliCommand::Pm(_)
        ));
        for field in ["project", "kind", "text"] {
            let mut invalid = valid.clone();
            invalid.as_object_mut().unwrap().remove(field);
            assert!(parse(&envelope("pm.peer.send", invalid)).is_err());
        }
        for (field, value) in [
            ("id", json!("foreign-pane")),
            ("session_id", json!("forged")),
            ("project_root", json!("/tmp/foreign")),
            ("kind", json!("stop")),
            ("text", json!(" ")),
            ("timeout_seconds", json!(0)),
            ("timeout_seconds", json!(86401)),
            ("reply_to", json!("unrelated")),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            assert!(parse(&envelope("pm.peer.send", invalid)).is_err());
        }
        assert!(crate::cli::pm::capabilities_report()
            .capabilities
            .iter()
            .any(|row| row.operation == "pm.peer.send" && row.executable));
    }

    #[test]
    fn pm_peer_two_project_runtime_roundtrip_and_unavailable() {
        use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let a = home.path().join("a");
        let b = home.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let register = |repo: &std::path::Path, id: &str| {
            let worktree = crate::pm_registry::pm_worktree_path_for_repo_path(repo);
            std::fs::create_dir_all(&worktree).unwrap();
            std::fs::create_dir_all(worktree.parent().unwrap().join("runtime")).unwrap();
            let mut session = gwt_agent::Session::new(&worktree, "work", gwt_agent::AgentId::Codex);
            session.id = id.into();
            session.project_state_root = Some(repo.to_path_buf());
            session.status = gwt_agent::AgentStatus::Running;
            session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
            let mut runtime = gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running);
            runtime.child_pid = Some(std::process::id());
            runtime.child_started_at = crate::process::host_process_start_time(std::process::id());
            assert!(runtime.child_started_at.is_some());
            runtime
                .save(&gwt_agent::runtime_state_path(
                    &gwt_core::paths::gwt_sessions_dir(),
                    id,
                ))
                .unwrap();
            crate::pm_registry::save_pm_prefs(
                &crate::pm_registry::pm_prefs_path_for_repo_path(repo),
                &crate::pm_registry::PmPrefs {
                    registration: Some(crate::pm_registry::PmRegistration {
                        session_id: id.into(),
                        agent_id: "codex".into(),
                        worktree_path: worktree.display().to_string(),
                        created_at: None,
                        consecutive_crashes: 0,
                        next_not_before: None,
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
        };
        register(&a, "pm-a");
        register(&b, "pm-b");
        let send = |repo: &std::path::Path, params: Value| {
            let mut env = TestEnv::new(
                crate::pm_registry::pm_worktree_path_for_repo_path(repo)
                    .parent()
                    .unwrap()
                    .join("runtime"),
            );
            let CliCommand::Pm(command) = ok("pm.peer.send", params) else {
                panic!("expected PM command")
            };
            let mut output = String::new();
            crate::cli::pm::run(&mut env, command, &mut output)
                .map(|_| serde_json::from_str::<Value>(&output).unwrap())
        };
        let request = json!({"project":b,"kind":"request","text":"Please checkpoint"});
        let _direct = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        assert!(send(&a, request.clone()).is_err());
        let _agent = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "implementation-agent");
        assert!(send(&a, request.clone()).is_err());
        let _pm_a = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-a");
        let sent = send(&a, request.clone()).unwrap();
        assert_eq!(sent["status"], "queued");
        assert!(crate::cli::pm::peer_context(&a).unwrap().is_empty());
        {
            let _pm_b = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-b");
            let context = crate::cli::pm::peer_context(&b).unwrap();
            assert!(
                context.contains("Please checkpoint")
                    && context.contains(sent["message"]["id"].as_str().unwrap())
            );
            assert_eq!(crate::cli::pm::peer_context(&b).unwrap(), context);
            let reply = json!({"project":a,"kind":"reply","text":"Checkpoint saved","reply_to":sent["message"]["id"]});
            assert_eq!(send(&b, reply).unwrap()["status"], "queued");
            assert!(!crate::cli::pm::peer_context(&b)
                .unwrap()
                .contains("Please checkpoint"));
        }
        assert!(crate::cli::pm::peer_context(&a)
            .unwrap()
            .contains("Checkpoint saved"));
        assert_eq!(gwt_core::pm_report::load_reports(&a).unwrap().len(), 2);
        assert_eq!(gwt_core::pm_report::load_reports(&b).unwrap().len(), 2);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let mut stopped = gwt_agent::Session::load(&sessions.join("pm-b.toml")).unwrap();
        stopped.status = gwt_agent::AgentStatus::Stopped;
        stopped.save(&sessions).unwrap();
        assert_eq!(send(&a, request.clone()).unwrap()["status"], "unavailable");
        stopped.status = gwt_agent::AgentStatus::Running;
        stopped.save(&sessions).unwrap();
        std::fs::remove_file(gwt_agent::runtime_state_path(
            &gwt_core::paths::gwt_sessions_dir(),
            "pm-b",
        ))
        .unwrap();
        assert_eq!(
            send(&a, request.clone()).unwrap()["status"],
            "unavailable",
            "stale Running registration must not count as live"
        );
        let missing =
            json!({"project":home.path().join("absent"),"kind":"notice","text":"Update pending"});
        assert_eq!(send(&a, missing).unwrap()["status"], "unavailable");
        let reports = gwt_core::pm_report::load_reports(&a).unwrap();
        assert!(reports
            .iter()
            .any(|r| r.body.contains("Waiting for the peer project")));
        crate::pm_registry::save_pm_prefs(
            &crate::pm_registry::pm_prefs_path_for_repo_path(&a),
            &Default::default(),
        )
        .unwrap();
        assert!(
            send(&a, request).is_err(),
            "replaced PM loses send authority"
        );
    }

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

    #[test]
    fn pm_plan_status_accepts_only_empty_params() {
        assert!(matches!(
            ok("pm.plan.status", json!({})),
            CliCommand::Pm(crate::cli::pm::PmCommand::PlanStatus)
        ));
        for params in [
            json!({"project_root": "/tmp/foreign"}),
            json!({"path": "/tmp/planning.json"}),
            json!({"actor": "another-pm"}),
            json!({"unexpected": true}),
        ] {
            assert!(matches!(
                err("pm.plan.status", params),
                CliParseError::InvalidJson(_)
            ));
        }
        let report = crate::cli::pm::capabilities_report();
        for operation in ["pm.plan.status", "pm.plan.apply"] {
            assert!(report
                .capabilities
                .iter()
                .any(|row| row.operation == operation && row.executable));
        }
    }

    #[test]
    fn pm_plan_apply_parses_each_typed_action() {
        for kind in [
            "goal",
            "milestone",
            "risk",
            "decision",
            "policy",
            "recurring",
            "link",
        ] {
            let params = json!({
                "action": {"kind": kind, "value": {"id": kind}},
                "expected_revision": 0,
                "reason": "Record the approved plan",
                "idempotency_key": format!("planning-{kind}")
            });
            let CliCommand::Pm(crate::cli::pm::PmCommand::PlanApply { request }) =
                ok("pm.plan.apply", params)
            else {
                panic!("expected a typed planning apply command for {kind}");
            };
            assert_eq!(request.expected_revision, 0);
            assert_eq!(request.idempotency_key, format!("planning-{kind}"));
            let action = serde_json::to_value(request.action).unwrap();
            assert_eq!(action["kind"], kind);
            assert_eq!(action["value"]["id"], kind);
        }
    }

    #[test]
    fn pm_plan_apply_rejects_invalid_and_scope_overrides() {
        let valid = json!({
            "action": {"kind": "goal", "value": {"id": "goal-1"}},
            "expected_revision": 0,
            "reason": "Record the approved plan",
            "idempotency_key": "planning-1"
        });
        for field in ["action", "expected_revision", "reason", "idempotency_key"] {
            let mut params = valid.clone();
            params.as_object_mut().unwrap().remove(field);
            assert!(matches!(
                err("pm.plan.apply", params),
                CliParseError::InvalidJson(_)
            ));
        }
        for (field, value) in [
            ("project_root", json!("/tmp/foreign")),
            ("path", json!("/tmp/planning.json")),
            ("actor", json!("another-pm")),
            ("session_id", json!("another-pm")),
            ("unexpected", json!(true)),
            ("expected_revision", json!(-1)),
            ("expected_revision", json!(1.5)),
            ("expected_revision", json!("0")),
            ("reason", json!(" ")),
            ("idempotency_key", json!("")),
            (
                "action",
                json!({"kind": "delete", "value": {"id": "goal-1"}}),
            ),
            (
                "action",
                json!({"kind": "goal", "value": {"id": "goal-1", "unknown": true}}),
            ),
            (
                "action",
                json!({"kind": "goal", "value": {"id": "goal-1"}, "unknown": true}),
            ),
            ("action", json!({"kind": "goal", "value": {"id": " "}})),
        ] {
            let mut params = valid.clone();
            params[field] = value;
            assert!(matches!(
                err("pm.plan.apply", params),
                CliParseError::InvalidJson(_)
            ));
        }
    }

    #[test]
    fn pm_pause_and_resume_are_mutating_json_operations() {
        for operation in ["pm.pause", "pm.resume"] {
            assert!(matches!(ok(operation, json!({})), CliCommand::Pm(_)));
            assert!(matches!(
                ok(operation, json!({"project_root": "/tmp/project"})),
                CliCommand::Pm(_)
            ));
            assert!(
                !crate::cli::hook::workflow_policy::is_read_only_json_envelope_operation(operation)
            );
        }
    }

    #[test]
    fn pm_pause_resume_preserve_registration_and_rearm_only_on_transition() {
        use crate::pm_registry;
        use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path().join("state"));
        let _session = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let mut env = TestEnv::new(repo.clone());
        let path = pm_registry::pm_prefs_path_for_repo_path(&repo);
        let original: pm_registry::PmPrefs = serde_json::from_value(json!({
            "settings": {"auto_start": true},
            "registration": {"session_id":"resident", "agent_id":"codex", "worktree_path":repo}
        }))
        .unwrap();
        pm_registry::save_pm_prefs(&path, &original).unwrap();
        let loop_path = pm_registry::pm_loop_state_path_for_repo_path(&repo);
        let clock = pm_registry::PmLoopState {
            last_wake_at: Some("2026-10-10T00:00:00Z".into()),
            last_user_prompt_at: Some("2026-10-10T00:01:00Z".into()),
            ..Default::default()
        };
        pm_registry::save_pm_loop_state(&loop_path, &clock).unwrap();
        for operation in ["pm.pause", "pm.pause", "pm.resume", "pm.resume", "pm.pause"] {
            let (_, out) = crate::cli::run_collect(&mut env, ok(operation, json!({}))).unwrap();
            let report: Value = serde_json::from_str(&out).unwrap();
            assert_eq!(report["paused"], operation == "pm.pause");
            assert_eq!(report["registered"], true);
            assert_eq!(report["auto_start"], true);
            assert_eq!(
                pm_registry::load_pm_prefs(&path).unwrap().registration,
                original.registration
            );
            if operation == "pm.resume" {
                assert!(pm_registry::load_pm_loop_state(&loop_path)
                    .unwrap()
                    .last_wake_at
                    .is_none());
                assert_eq!(
                    pm_registry::load_pm_loop_state(&loop_path)
                        .unwrap()
                        .last_user_prompt_at,
                    clock.last_user_prompt_at,
                    "Resume must preserve the active user conversation gate"
                );
            }
        }
        crate::cli::run_collect(&mut env, ok("pm.resume", json!({}))).unwrap();
        pm_registry::save_pm_loop_state(&loop_path, &clock).unwrap();
        crate::cli::run_collect(&mut env, ok("pm.resume", json!({}))).unwrap();
        assert_eq!(
            pm_registry::load_pm_loop_state(&loop_path).unwrap(),
            clock,
            "repeated Resume must not re-arm an already active loop"
        );
        let _agent = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "unrelated-agent");
        assert!(
            crate::cli::run_collect(&mut env, ok("pm.pause", json!({}))).is_err(),
            "an unrelated Agent must not control the PM"
        );
        let _pm = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "resident");
        let (_, out) = crate::cli::run_collect(&mut env, ok("pm.pause", json!({}))).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&out).unwrap()["paused"],
            true,
            "the registered PM remains authorized"
        );
    }

    // Issue #4249 FR-003: the PM self-description is a read-only diagnostic.
    #[test]
    fn pm_capabilities_parses_and_stays_read_only() {
        assert!(matches!(
            ok("pm.capabilities", json!({})),
            CliCommand::Pm(crate::cli::pm::PmCommand::Capabilities)
        ));
        assert!(
            crate::cli::hook::workflow_policy::is_read_only_json_envelope_operation(
                "pm.capabilities"
            )
        );
    }

    // Issue #3607: PM stop/deregister parse variants.
    #[test]
    fn pm_stop_variants() {
        assert!(matches!(
            ok("pm.stop", json!({})),
            CliCommand::Pm(crate::cli::pm::PmCommand::Stop {
                project_root: None,
                session_id: None,
            })
        ));
        match ok(
            "pm.deregister",
            json!({"project_root": "/tmp/elsewhere", "session_id": "b0801016-orphan"}),
        ) {
            CliCommand::Pm(crate::cli::pm::PmCommand::Stop {
                project_root,
                session_id,
            }) => {
                assert_eq!(project_root.as_deref(), Some("/tmp/elsewhere"));
                assert_eq!(session_id.as_deref(), Some("b0801016-orphan"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    /// The stop side must never be mistaken for a read-only diagnostic: it
    /// clears a durable registration.
    #[test]
    fn pm_stop_is_not_a_read_only_operation() {
        assert!(
            crate::cli::hook::workflow_policy::is_read_only_json_envelope_operation("pm.status"),
            "pm.status stays read-only"
        );
        assert!(
            !crate::cli::hook::workflow_policy::is_read_only_json_envelope_operation("pm.stop"),
            "pm.stop writes durable PM state"
        );
    }

    // SPEC-3248 P8a: execution settlement parse variants.
    #[test]
    fn execution_settlement_variants() {
        assert!(matches!(
            ok("execution.status", json!({})),
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::Status)
        ));
        assert!(matches!(
            ok("execution.status", json!({"issue": null, "spec": null})),
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::Status)
        ));
        assert!(matches!(
            err("execution.status", json!({"unexpected": null})),
            CliParseError::InvalidJson(message)
                if message.contains("unexpected")
        ));
        // Issue #3934: an operator must be able to ask who holds any owner's
        // generation, not only the one their own session is bound to.
        assert!(matches!(
            ok("execution.status", json!({"issue": 3934, "spec": null})),
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::OwnerStatus {
                owner,
            }) if owner
                == crate::cli::execution_state::ExecutionOwnerKey {
                    kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                    number: 3934,
                }
        ));
        assert!(matches!(
            ok("execution.status", json!({"issue": null, "spec": 3885})),
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::OwnerStatus {
                owner,
            }) if owner
                == crate::cli::execution_state::ExecutionOwnerKey {
                    kind: crate::cli::execution_state::ExecutionOwnerKind::Spec,
                    number: 3885,
                }
        ));
        assert!(matches!(
            err("execution.status", json!({"issue": 3934, "spec": 3885})),
            CliParseError::InvalidJson(message) if message.contains("not both")
        ));
        assert!(matches!(
            err("execution.status", json!({"issue": 0})),
            CliParseError::InvalidJson(message) if message.contains("greater than zero")
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
        // Issue #4214 AC-6: terminal delivered owners can settle leftover
        // obligations without pretending to deliver or re-verify source.
        assert!(matches!(
            ok(
                "execution.settle_delivered",
                json!({"reason": "already delivered"})
            ),
            CliCommand::Execution(_)
        ));
        assert!(matches!(
            err("execution.settle_delivered", json!({})),
            CliParseError::MissingFlag("reason")
        ));
        assert!(matches!(
            err(
                "execution.settle_delivered",
                json!({"reason": "delivered", "issue": 4214})
            ),
            CliParseError::InvalidJson(_)
        ));
        // Issue #4545 AC-2: `execution.no_action` takes a non-empty reason and
        // nothing else — an unknown parameter is refused rather than ignored,
        // so a caller cannot smuggle a scope the operation does not honour.
        assert!(matches!(
            ok(
                "execution.no_action",
                json!({"reason": "already delivered"})
            ),
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::NoAction { .. })
        ));
        assert!(matches!(
            err("execution.no_action", json!({})),
            CliParseError::MissingFlag("reason")
        ));
        assert!(matches!(
            err(
                "execution.no_action",
                json!({"reason": "already delivered", "issue": 3290})
            ),
            CliParseError::InvalidJson(_)
        ));
        assert!(matches!(
            parse(&envelope(
                "execution.no-action",
                json!({"reason": "already delivered"})
            ))
            .expect("the dash spelling resolves")
            .command,
            CliCommand::Execution(crate::cli::execution_state::ExecutionCommand::NoAction { .. })
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
        // Issue #4161: the release is owner-addressed, so the owner is
        // required rather than inferred from the caller's own record.
        assert!(matches!(
            ok(
                "execution.release_prepared",
                json!({"issue": 4161, "reason": "the launch that prepared it is gone"})
            ),
            CliCommand::Execution(
                crate::cli::execution_state::ExecutionCommand::ReleasePrepared {
                    owner,
                    operation_id: None,
                    ..
                }
            ) if owner
                == crate::cli::execution_state::ExecutionOwnerKey {
                    kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                    number: 4161,
                }
        ));
        assert!(matches!(
            ok(
                "execution.release_prepared",
                json!({"spec": 4161, "reason": "stale fence", "operation_id": "fresh-launch-7"})
            ),
            CliCommand::Execution(
                crate::cli::execution_state::ExecutionCommand::ReleasePrepared {
                    operation_id: Some(operation_id),
                    ..
                }
            ) if operation_id == "fresh-launch-7"
        ));
        assert!(matches!(
            err("execution.release_prepared", json!({"reason": "stale fence"})),
            CliParseError::InvalidJson(message)
                if message.contains("requires params.issue or params.spec")
        ));
        assert!(matches!(
            err("execution.release_prepared", json!({"issue": 4161})),
            CliParseError::MissingFlag("reason")
        ));
        assert!(matches!(
            err(
                "execution.release_prepared",
                json!({"issue": 4161, "spec": 4161, "reason": "stale fence"})
            ),
            CliParseError::InvalidJson(message) if message.contains("not both")
        ));
        assert!(matches!(
            err(
                "execution.release_prepared",
                json!({"issue": 4161, "reason": "stale fence", "unexpected": true})
            ),
            CliParseError::InvalidJson(message)
                if message.contains("does not accept the parameter unexpected")
        ));
    }

    #[test]
    fn verification_pre_pr_plan_preserves_acceptance_and_non_ci_commands() {
        let command = ok(
            "verify.plan",
            json!({
                "mode": "pre-pr", "derive": true,
                "acceptance_commands": ["cargo test -p gwt --test ci_pre_pr_contract_test"],
                "commands": ["bash scripts/verify-local-only.sh"]
            }),
        );
        let typed = format!("{command:?}");
        assert!(
            typed.contains("PrePrPlan"),
            "pre-pr must not silently use full: {typed}"
        );
        assert!(
            typed.contains("ci_pre_pr_contract_test"),
            "AC tests were discarded: {typed}"
        );
        assert!(
            typed.contains("verify-local-only.sh"),
            "non-CI check was discarded: {typed}"
        );
    }

    #[test]
    fn verification_plan_rejects_unknown_mode() {
        let error = err(
            "verify.plan",
            json!({"mode": "pretend-fast", "derive": true}),
        );
        assert!(error.to_string().contains("mode"));
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

    // Issue #3841 / SPEC #3248 AS-48: quarantine is plan-bound typed input,
    // not prose that verify.run may invent after observing a failure.
    #[test]
    fn verification_plan_preserves_typed_quarantine_request() {
        let command = ok(
            "verify.plan",
            json!({
                "derive": true,
                "quarantines": [{
                    "failed_command": "cargo test -p gwt --all-features --lib",
                    "test_identity": "app_runtime::tests::bounded_sync",
                    "baseline_command": "cargo test -p gwt --all-features --lib app_runtime::tests::bounded_sync -- --exact",
                    "owner_issue": 3755,
                    "pr_number": 3854
                }]
            }),
        );

        let typed = format!("{command:?}");
        for expected in [
            "app_runtime::tests::bounded_sync",
            "owner_issue: 3755",
            "pr_number: 3854",
        ] {
            assert!(
                typed.contains(expected),
                "typed quarantine field was discarded: {typed}"
            );
        }
    }

    // SPEC #3576 T-007: verification lease operation parsing.
    #[test]
    fn verification_lease_operations_are_typed() {
        assert!(matches!(
            ok("verify.lease.status", json!({})),
            CliCommand::VerifyLease(VerificationLeaseCommand::Status)
        ));
        assert!(matches!(
            ok("verify.lease.acquire", json!({})),
            CliCommand::VerifyLease(VerificationLeaseCommand::Acquire { ttl_minutes, reason })
                if ttl_minutes == crate::cli::verification_lease::DEFAULT_TTL_MINUTES
                    && reason.is_none()
        ));
        assert!(matches!(
            ok("verify.lease.acquire", json!({"ttl_minutes": 20, "reason": "coverage run"})),
            CliCommand::VerifyLease(VerificationLeaseCommand::Acquire { ttl_minutes, reason })
                if ttl_minutes == 20 && reason.as_deref() == Some("coverage run")
        ));
        assert!(matches!(
            ok("verify.lease.extend", json!({"lease_id": "lease-1"})),
            CliCommand::VerifyLease(VerificationLeaseCommand::Extend { lease_id, ttl_minutes })
                if lease_id == "lease-1"
                    && ttl_minutes == crate::cli::verification_lease::DEFAULT_TTL_MINUTES
        ));
        assert!(matches!(
            ok("verify.lease.release", json!({"lease_id": "lease-1", "reason": "done"})),
            CliCommand::VerifyLease(VerificationLeaseCommand::Release { lease_id, reason })
                if lease_id == "lease-1" && reason.as_deref() == Some("done")
        ));
        for operation in ["verify.lease.release", "verify.lease.extend"] {
            assert!(matches!(
                err(operation, json!({})),
                CliParseError::MissingFlag("lease_id")
            ));
        }
        assert!(matches!(
            err("verify.lease.hold", json!({"control": "/tmp/control"})),
            CliParseError::MissingFlag("ttl_minutes")
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

    /// Issue #3516: the standing check is read-only until the caller opts in.
    #[test]
    fn release_status_defaults_to_a_read_only_check() {
        assert!(matches!(
            ok("release.status", json!({})),
            CliCommand::Release(crate::cli::release::ReleaseCommand::Status {
                release_branch: None,
                base_branch: None,
                scan_commits: None,
                ensure_release_pr: false,
            })
        ));
    }

    #[test]
    fn release_status_accepts_the_reconcile_and_topology_params() {
        let parsed = ok(
            "release.status",
            json!({
                "release_branch": "develop",
                "base_branch": "main",
                "scan_commits": 50,
                "ensure_release_pr": true,
            }),
        );
        assert!(matches!(
            parsed,
            CliCommand::Release(crate::cli::release::ReleaseCommand::Status {
                scan_commits: Some(50),
                ensure_release_pr: true,
                ..
            })
        ));
        assert!(matches!(
            err("release.status", json!({"ensure": true})),
            CliParseError::InvalidJson(_)
        ));
    }

    /// Issue #5062 AC-3: deferring a staged release is a host-wide, durable,
    /// version-scoped mutation that preserves its manual-apply payload.
    #[test]
    fn release_update_defer_envelope_preserves_payload_and_rejects_other_versions() {
        use gwt_core::update::{self, PendingUpdateManifest, PreparedPayload};

        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let payload_path = home.path().join("prepared-binary");
        std::fs::write(&payload_path, "prepared payload").unwrap();
        let manifest = PendingUpdateManifest {
            version: "9.200.0".into(),
            asset_url: "https://example.invalid/update".into(),
            payload: PreparedPayload::PortableBinary {
                path: payload_path.clone(),
            },
            downloaded_at: "2026-10-01T00:00:00Z".into(),
            origin: gwt_core::update::PendingUpdateOrigin::ManualLater,
            auto_apply_deferred: false,
        };
        // Seed the format used before the explicit defer control existed.
        let mut legacy = serde_json::to_value(&manifest).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("auto_apply_deferred");
        std::fs::create_dir_all(update::pending_update_dir()).unwrap();
        std::fs::write(
            update::pending_update_manifest_path(),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        let mut env = TestEnv::new(home.path().to_path_buf());
        let call = |env: &mut TestEnv, version: &str| {
            let parsed = parse(&envelope(
                "release.update.defer",
                json!({"version":version}),
            ))
            .expect("the explicit defer operation must be available");
            super::run_collect_governed(env, parsed.command)
        };
        let output = call(&mut env, "9.200.0").unwrap().output;
        let response: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(response["pending_update_version"], "9.200.0");
        assert_eq!(response["auto_apply_deferred"], true);
        let saved: Value =
            serde_json::from_slice(&std::fs::read(update::pending_update_manifest_path()).unwrap())
                .unwrap();
        assert_eq!(saved["auto_apply_deferred"], true);
        assert_eq!(saved["version"], "9.200.0");
        assert_eq!(saved["asset_url"], manifest.asset_url);
        assert_eq!(saved["downloaded_at"], manifest.downloaded_at);
        assert_eq!(
            saved["payload"],
            serde_json::to_value(&manifest.payload).unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(&payload_path).unwrap(),
            "prepared payload"
        );

        call(&mut env, "9.200.0").expect("same-version retry is idempotent");
        let before = std::fs::read(update::pending_update_manifest_path()).unwrap();
        update::persist_update_resume_marker(&update::UpdateResumeMarker {
            from_version: "9.199.0".into(),
            to_version: "9.200.0".into(),
            started_at: "2026-10-01T00:10:00Z".into(),
            restart_args: Vec::new(),
            projects: Vec::new(),
            attempt: 1,
        })
        .unwrap();
        assert!(
            call(&mut env, "9.200.0").is_err(),
            "a committed apply cannot be deferred"
        );
        assert_eq!(
            std::fs::read(update::pending_update_manifest_path()).unwrap(),
            before
        );
        update::clear_update_resume_marker().unwrap();
        assert!(call(&mut env, "9.201.0").is_err());
        assert_eq!(
            std::fs::read(update::pending_update_manifest_path()).unwrap(),
            before
        );
        std::fs::remove_file(payload_path).unwrap();
        assert!(call(&mut env, "9.200.0").is_err());
        assert_eq!(
            std::fs::read(update::pending_update_manifest_path()).unwrap(),
            before
        );
        update::clear_pending_update_manifest().unwrap();
        assert!(call(&mut env, "9.200.0").is_err());

        assert!(matches!(
            err("release.update.defer", json!({})),
            CliParseError::MissingFlag("version")
        ));
        assert!(matches!(
            err(
                "release.update.defer",
                json!({"version":"9.200.0", "force":true})
            ),
            CliParseError::InvalidJson(_)
        ));
    }

    #[test]
    fn pr_head_check_parses_as_a_read_only_diagnostic() {
        assert_eq!(
            ok("pr.head_check", json!({"base": "develop"})),
            CliCommand::Pr(PrCommand::HeadCheck {
                base: "develop".to_string(),
                head: None,
            })
        );
        assert_eq!(
            ok(
                "pr.head_check",
                json!({"base": "develop", "head": "work/issue-5059"})
            ),
            CliCommand::Pr(PrCommand::HeadCheck {
                base: "develop".to_string(),
                head: Some("work/issue-5059".to_string()),
            })
        );
        assert!(matches!(
            err("pr.head_check", json!({})),
            CliParseError::MissingFlag("base")
        ));
        assert!(matches!(
            err(
                "pr.head_check",
                json!({"base": "develop", "verified_head": "caller-sha"})
            ),
            CliParseError::InvalidJson(_)
        ));
        assert!(
            crate::cli::hook::workflow_policy::is_read_only_json_envelope_operation(
                "pr.head_check"
            )
        );
    }

    #[test]
    fn issue_3789_review_reply_preserves_thread_id() {
        let command = ok(
            "pr.review_threads.reply_and_resolve",
            json!({"number": 1, "thread_id": "PRRT_target", "body": "Fixed"}),
        );
        assert!(format!("{command:?}").contains("PRRT_target"));
    }

    #[test]
    fn issue_3789_review_reply_rejects_invalid_thread_selector() {
        for thread_id in [json!(""), json!("  "), Value::Null, json!(42)] {
            assert!(
                super::parse_operation(
                    "pr.review_threads.reply_and_resolve",
                    &json!({"number": 1, "thread_id": thread_id, "body": "Fixed"}),
                )
                .is_err(),
                "invalid selector must never fall back to bulk mode"
            );
        }
    }

    #[test]
    fn pr_operations_parse() {
        assert!(matches!(
            ok("pr.current", json!({})),
            CliCommand::Pr(PrCommand::Current)
        ));
        assert!(matches!(
            ok("pr.list", json!({})),
            CliCommand::Pr(PrCommand::List {
                stale_after_hours: None,
                escalate_after_cycles: None,
                refresh: false,
                include: None,
                force_reason: None,
            })
        ));
        assert!(matches!(
            ok(
                "pr.list",
                json!({"stale_after_hours": 24, "escalate_after_cycles": 2})
            ),
            CliCommand::Pr(PrCommand::List {
                stale_after_hours: Some(24),
                escalate_after_cycles: Some(2),
                refresh: false,
                include: None,
                force_reason: None,
            })
        ));
        // Issue #3891: refresh bypasses the TTL cache / throttle; include
        // names the heavy fields to hydrate.
        assert!(matches!(
            ok(
                "pr.list",
                json!({"refresh": true, "include": ["checks", "body"]})
            ),
            CliCommand::Pr(PrCommand::List {
                refresh: true,
                include: Some(gwt_git::PrInventoryInclude {
                    checks: true,
                    body: true
                }),
                ..
            })
        ));
        assert!(matches!(
            ok("pr.list", json!({"include": []})),
            CliCommand::Pr(PrCommand::List {
                include: Some(gwt_git::PrInventoryInclude {
                    checks: false,
                    body: false
                }),
                ..
            })
        ));
        assert!(matches!(
            err("pr.list", json!({"include": ["reviews"]})),
            CliParseError::InvalidValue {
                flag: "include",
                ..
            }
        ));
        // Issue #3891 AC-3: budget observation.
        assert!(matches!(
            ok("github.budget", json!({})),
            CliCommand::GithubBudget(crate::cli::github_budget::GithubBudgetCommand {
                refresh: false
            })
        ));
        assert!(matches!(
            ok("github.budget", json!({"refresh": true})),
            CliCommand::GithubBudget(crate::cli::github_budget::GithubBudgetCommand {
                refresh: true
            })
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
            CliParseError::MissingFlag("base|title|body|add_labels")
        ));
        assert!(matches!(
            ok("pr.edit", json!({"number": 1, "base": "develop"})),
            CliCommand::Pr(PrCommand::EditBody { base: Some(base), .. }) if base == "develop"
        ));
        for params in [
            json!({"number": 1, "base": " "}),
            json!({"number": 1, "base": "", "title": "t"}),
        ] {
            assert!(matches!(
                err("pr.edit", params),
                CliParseError::InvalidJson(_)
            ));
        }
        for params in [
            json!({"number": 9}),
            json!({"number": 9, "comment": "Wrong base"}),
        ] {
            assert!(matches!(
                ok("pr.close", params),
                CliCommand::Pr(PrCommand::Close { number: 9, .. })
            ));
        }
        assert!(matches!(
            err("pr.close", json!({})),
            CliParseError::MissingFlag("number")
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
    fn actions_dispatch_accepts_selectors_and_rejects_invalid_contract() {
        for workflow in [
            json!("prepare-release.yml"),
            json!("日本語 Release.yml"),
            json!(42),
        ] {
            assert!(matches!(
                ok(
                    "actions.dispatch",
                    json!({"workflow":workflow,"ref":"develop","inputs":{"bump":"auto"}})
                ),
                CliCommand::Actions(_)
            ));
        }
        for params in [
            json!({"workflow":"../release.yml","ref":"develop"}),
            json!({"workflow":0,"ref":"develop"}),
            json!({"workflow":"release.yml","ref":""}),
            json!({"workflow":"release.yml","ref":"develop","inputs":[]}),
            json!({"workflow":"release.yml","ref":"develop","force":true}),
        ] {
            let _ = err("actions.dispatch", params);
        }
    }

    #[test]
    fn issue_4188_actions_cancel_requires_a_run_id() {
        assert_eq!(
            ok("actions.cancel", json!({"run_id": 5})),
            CliCommand::Actions(ActionsCommand::Cancel { run_id: 5 })
        );
        assert!(matches!(
            err("actions.cancel", json!({})),
            CliParseError::MissingFlag("run_id")
        ));
        assert!(parse(&envelope("actions.cancel", json!({"run_id": "five"}))).is_err());
        assert!(parse(&envelope(
            "actions.cancel",
            json!({"run_id": 5, "job_id": 7})
        ))
        .is_err());
    }

    #[test]
    fn issue_4188_actions_queued_takes_no_params() {
        assert_eq!(
            ok("actions.queued", json!({})),
            CliCommand::Actions(ActionsCommand::Queued)
        );
        assert!(parse(&envelope("actions.queued", json!({"run_id": 5}))).is_err());
    }

    #[test]
    fn actions_index_diagnostics_daemon_operations_parse() {
        assert!(matches!(
            ok("actions.logs", json!({"run_id": 5})),
            CliCommand::Actions(ActionsCommand::Logs { .. })
        ));
        assert_eq!(
            ok("actions.job_logs", json!({"job_id": 5})),
            CliCommand::Actions(ActionsCommand::JobLogs {
                job_id: 5,
                failed_only: false,
                context_lines: 5,
            })
        );
        // Issue #4849 AC-2: the failures view and its (clamped) context.
        assert_eq!(
            ok(
                "actions.job_logs",
                json!({"job_id": 5, "failed_only": true, "context_lines": 400})
            ),
            CliCommand::Actions(ActionsCommand::JobLogs {
                job_id: 5,
                failed_only: true,
                context_lines: 50,
            })
        );
        assert!(matches!(
            ok("actions.job-logs", json!({"job_id": 5})),
            CliCommand::Actions(ActionsCommand::JobLogs { .. })
        ));
        assert_eq!(
            ok("actions.rerun", json!({"run_id": 5})),
            CliCommand::Actions(ActionsCommand::Rerun {
                target: crate::cli::ActionsRerunTarget::Run {
                    run_id: 5,
                    failed_only: false
                }
            })
        );
        assert_eq!(
            ok("actions.rerun", json!({"run_id": 5, "failed_only": true})),
            CliCommand::Actions(ActionsCommand::Rerun {
                target: crate::cli::ActionsRerunTarget::Run {
                    run_id: 5,
                    failed_only: true
                }
            })
        );
        assert_eq!(
            ok("actions.rerun", json!({"job_id": 7})),
            CliCommand::Actions(ActionsCommand::Rerun {
                target: crate::cli::ActionsRerunTarget::Job { job_id: 7 }
            })
        );
        assert!(matches!(
            err("actions.rerun", json!({})),
            CliParseError::MissingFlag("run_id")
        ));
        assert!(matches!(
            err("actions.rerun", json!({"run_id": 5, "job_id": 7})),
            CliParseError::InvalidValue { flag: "job_id", .. }
        ));
        assert!(matches!(
            err("actions.rerun", json!({"job_id": 7, "failed_only": true})),
            CliParseError::InvalidValue {
                flag: "failed_only",
                ..
            }
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
        match ok("daemon.subscribe", json!({"channels": ["errors"]})) {
            CliCommand::Daemon(DaemonCommand::Subscribe { channels, .. }) => {
                assert_eq!(channels, vec!["errors".to_string()]);
            }
            other => panic!("unexpected command: {other:?}"),
        }
        // Zero would mean "return before reading anything", which is never
        // what a caller wants and silently degrades the loop to a busy poll.
        assert!(err(
            "daemon.subscribe",
            json!({"channels": ["board"], "timeout_seconds": 0})
        )
        .to_string()
        .contains("timeout_seconds"));
    }

    /// Issue #3596: an explicit target must survive JSON parsing so runtime
    /// daemon discovery does not silently fall back to the gwtd process cwd.
    #[test]
    fn daemon_subscribe_preserves_explicit_project_root_in_command() {
        let command = ok(
            "daemon.subscribe",
            json!({
                "channels": ["issue_monitor"],
                "project_root": "/tmp/gwt-issue-3596-explicit-root"
            }),
        );

        match command {
            CliCommand::Daemon(DaemonCommand::Subscribe { project_root, .. }) => assert_eq!(
                project_root.as_deref(),
                Some(std::path::Path::new("/tmp/gwt-issue-3596-explicit-root"))
            ),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    /// Issue #3596: omitting the optional target preserves the legacy cwd
    /// resolution path by transporting `None` explicitly.
    #[test]
    fn daemon_subscribe_defaults_project_root_to_none() {
        let command = ok("daemon.subscribe", json!({"channels": ["issue_monitor"]}));

        match command {
            CliCommand::Daemon(DaemonCommand::Subscribe { project_root, .. }) => {
                assert_eq!(project_root, None);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    /// Issue #3596: misspelling a scope parameter must fail closed instead of
    /// subscribing to whichever project happens to own the process cwd.
    #[test]
    fn daemon_subscribe_rejects_unknown_params() {
        match err(
            "daemon.subscribe",
            json!({
                "channels": ["issue_monitor"],
                "project_rooot": "/tmp/gwt-issue-3596-typo"
            }),
        ) {
            CliParseError::InvalidJson(message) => assert!(
                message.contains("daemon.subscribe does not accept the parameter project_rooot"),
                "unexpected error message: {message}"
            ),
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
            "works",
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
        assert_eq!(
            ok(
                "hook.register_codex_managed_project_trust",
                json!({
                    "project_root": "/repo",
                    "codex_config": "/cfg",
                })
            ),
            CliCommand::Hook(HookCommand::Run {
                name: "register-codex-managed-project-trust".to_string(),
                rest: vec![
                    "--project-root".to_string(),
                    "/repo".to_string(),
                    "--codex-config".to_string(),
                    "/cfg".to_string(),
                ],
            })
        );
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
    fn ci_delivery_creation_requires_exact_head() {
        let request = json!({"base":"develop","head":"work/issue-4214",
            "head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "title":"fix: delivery","body":"CI delivery"});
        assert!(matches!(
            ok("pr.create_from_ci", request.clone()),
            CliCommand::Pr(_)
        ));
        let mut missing_sha = request.clone();
        missing_sha.as_object_mut().unwrap().remove("head_sha");
        let _ = err("pr.create_from_ci", missing_sha);
        let mut draft = request;
        draft["draft"] = json!(true);
        let _ = err("pr.create_from_ci", draft);
    }

    #[test]
    fn verification_adjudication_requires_exact_board_decision_reference() {
        assert!(parse(&envelope(
            "verify.adjudicate",
            json!({
                "record_id": "vrr-failing",
                "command": "cargo test -p gwt --all-features",
                "board_entry_id": "board-decision-1",
            }),
        ))
        .is_ok());

        for params in [
            json!({
                "command": "cargo test -p gwt --all-features",
                "board_entry_id": "board-decision-1",
            }),
            json!({
                "record_id": "vrr-failing",
                "board_entry_id": "board-decision-1",
            }),
            json!({
                "record_id": "vrr-failing",
                "command": "cargo test -p gwt --all-features",
            }),
            json!({
                "record_id": "vrr-failing",
                "command": "*",
                "board_entry_id": "board-decision-1",
            }),
            json!({
                "record_id": "vrr-failing",
                "command": "cargo test -p gwt --all-features",
                "board_entry_id": "board-decision-1",
                "waive_record": true,
            }),
        ] {
            assert!(
                parse(&envelope("verify.adjudicate", params)).is_err(),
                "invalid adjudication input must fail closed"
            );
        }
    }

    #[test]
    fn discuss_status_rejects_empty_explicit_target_fields() {
        for params in [
            json!({"proposal": "Proposal A", "title": " "}),
            json!({"proposal": "Proposal A", "title": "History", "origin_session": " "}),
            json!({"proposal": "Proposal A", "origin_session": "session-other"}),
        ] {
            assert!(parse(&envelope("discuss.park", params)).is_err());
        }
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
    fn build_complete_pr_selector_is_optional_and_typed() {
        assert!(matches!(
            ok("build.complete", json!({"spec":5842,"pr":9001})),
            CliCommand::Build(SkillStateAction::Complete {
                spec: 5842,
                pr: Some(9001)
            })
        ));
        assert!(matches!(
            ok("build.complete", json!({"spec":5842})),
            CliCommand::Build(SkillStateAction::Complete {
                spec: 5842,
                pr: None
            })
        ));
        assert!(parse(&envelope(
            "build.complete",
            json!({"spec":5842,"pr":"invalid"})
        ))
        .is_err());
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
    fn pane_recover_accepts_a_bounded_restore_burst() {
        assert!(matches!(
            ok(
                "pane.recover",
                json!({
                    "started_after": "2026-09-14T05:00:00Z",
                    "started_before": "2026-09-14T05:05:00Z"
                })
            ),
            CliCommand::Pane(_)
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
    fn pane_name_gets_assigned_identity_with_optional_legacy_candidate() {
        let command = ok("pane.name", json!({"candidate": "Aster"}));
        assert!(matches!(command,
            CliCommand::Pane(PaneCommand::Name { candidate }) if candidate == "Aster"
        ));
        assert!(matches!(ok("pane.name", json!({})),
            CliCommand::Pane(PaneCommand::Name { candidate }) if candidate.is_empty()
        ));
        for forbidden in ["id", "window_id", "session_id", "project_root", "name"] {
            let mut params = json!({"candidate": "Aster"});
            params[forbidden] = json!("foreign");
            assert!(
                matches!(err("pane.name", params), CliParseError::InvalidJson(_)),
                "pane.name must not accept caller-supplied {forbidden}"
            );
        }
    }

    #[test]
    fn pane_name_waits_for_its_exact_assigned_result_without_a_target_claim() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let command = ok("pane.name", json!({"candidate": "Aster"}));
        let CliCommand::Pane(command) = command else {
            panic!("self-naming must be a pane operation");
        };
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let runtime_path =
            gwt_core::paths::gwt_sessions_dir().join("runtime/123/name-session.json");
        std::fs::create_dir_all(runtime_path.parent().unwrap()).unwrap();
        std::fs::write(&runtime_path, "{}").unwrap();
        let _runtime_path = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            &runtime_path,
        );
        let _token = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "name-capability",
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                let (stream, _) =
                    tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, listener.accept())
                        .await
                        .expect("pane.name mock connection")
                        .unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let message = socket.next().await.unwrap().unwrap();
                let Message::Text(text) = message else {
                    panic!("name request must be JSON")
                };
                let request: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(request["kind"], "pane_name");
                assert_eq!(request["candidate"], "Aster");
                assert_eq!(request.as_object().unwrap().len(), 3);
                let request_id = request["request_id"].as_str().unwrap();
                assert!(uuid::Uuid::parse_str(request_id).is_ok());
                for (correlation, name) in [("foreign-request", "Wrong"), (request_id, "Aster2")] {
                    socket
                        .send(Message::Text(
                            json!({
                                "kind": "pane_name_result", "request_id": correlation,
                                "ok": true, "agent_name": name, "error": null,
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                }
            });
        });
        let _ws_url = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_PANE_WS_URL_ENV,
            format!("ws://{address}/internal/pane-ws"),
        );
        let project = tempfile::tempdir().unwrap();
        let mut env = TestEnv::new(project.path().to_path_buf());
        let mut output = String::new();
        let result = crate::cli::pane::run(&mut env, command, &mut output);
        assert!(
            result.is_ok(),
            "pane.name CLI failed before its result: {result:?}"
        );
        server.join().unwrap();
        assert_eq!(result.unwrap(), 0);
        assert_eq!(output.trim(), "Aster2");
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
        // Issue #3552 AC-2: `pane.stop` is not a second implementation to keep
        // in step — it resolves to the very same command, so whatever AC-1
        // fixes for `pane.close` it inherits by construction.
        match (
            ok("pane.close", json!({"id": "p1"})),
            ok("pane.stop", json!({"id": "p1"})),
        ) {
            (
                CliCommand::Pane(PaneCommand::Close { id: closed }),
                CliCommand::Pane(PaneCommand::Close { id: stopped }),
            ) => assert_eq!(closed, stopped),
            other => panic!("unexpected commands: {other:?}"),
        }
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
    fn reports_search_and_rebuild_routes_parse() {
        match ok(
            "search",
            json!({"query": "過去のPMの回答", "scopes": ["reports"]}),
        ) {
            CliCommand::Search(command) => assert_eq!(command.scopes[0].as_str(), "reports"),
            other => panic!("unexpected command: {other:?}"),
        }
        assert!(matches!(
            ok("index.rebuild", json!({"scope": "reports"})),
            CliCommand::Index(IndexCommand::Rebuild { .. })
        ));
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
    fn errors_list_filters_scope_and_root_without_cross_project_rows() {
        use gwt_core::error_ledger::{ErrorKind, ErrorRecord, ErrorTarget};
        use gwt_core::test_support::ScopedGwtHome;
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path());
        for root in [Some("/project-a"), Some("/project-b"), None] {
            gwt_core::error_ledger::record(ErrorRecord::new(
                ErrorKind::DaemonFault,
                "fault",
                ErrorTarget {
                    project_root: root.map(str::to_owned),
                    ..Default::default()
                },
            ))
            .unwrap();
        }
        let host: ErrorRecord = serde_json::from_value(json!({
            "schema_version":1,"id":"host","recorded_at":"2026-08-30T00:00:00Z",
            "kind":"daemon_fault","message":"host","scope":"host","target":{}
        }))
        .unwrap();
        gwt_core::error_ledger::record(host).unwrap();
        for (caller_root, params, expected) in [
            ("/project-a", json!({}), 1),
            ("/project-b", json!({}), 1),
            ("/project-a", json!({"scope":"project"}), 1),
            ("/project-a", json!({"scope":"host"}), 1),
            ("/project-a", json!({"scope":"unknown"}), 1),
            ("/project-a", json!({"scope":"all"}), 4),
            (
                "/project-b",
                json!({"project_root":"/project-a","scope":"all"}),
                1,
            ),
            (
                "/project-b",
                json!({"project_root":"/project-a","scope":"host"}),
                1,
            ),
            ("/project-a", json!({"project_root":"/missing"}), 0),
        ] {
            let mut env = TestEnv::new(caller_root.into());
            let (_, output) =
                crate::cli::run_collect(&mut env, ok("errors.list", params.clone())).unwrap();
            let payload: Value = serde_json::from_str(&output).unwrap();
            assert_eq!(payload["count"], expected, "{params}");
            let expected_scope = if params.get("project_root").is_some() {
                "project"
            } else {
                params
                    .get("scope")
                    .and_then(Value::as_str)
                    .unwrap_or("project")
            };
            if expected_scope != "all" {
                for row in payload["errors"].as_array().unwrap() {
                    assert_eq!(row["scope"], expected_scope, "{params}");
                }
            }
            if expected_scope == "project" && expected > 0 {
                assert_eq!(
                    payload["errors"][0]["target"]["project_root"],
                    params
                        .get("project_root")
                        .and_then(Value::as_str)
                        .unwrap_or(caller_root)
                );
            }
        }
        assert!(matches!(
            err("errors.list", json!({"scope":"invalid"})),
            CliParseError::InvalidValue { flag: "scope", .. }
        ));
    }

    #[test]
    fn errors_list_parses_optional_since() {
        assert!(matches!(
            ok("errors.list", json!({})),
            CliCommand::Diagnostics(DiagnosticsCommand::ErrorsList { since: None, .. })
        ));
        match ok("errors.list", json!({"since": "2026-08-30T00:00:00Z"})) {
            CliCommand::Diagnostics(DiagnosticsCommand::ErrorsList { since, .. }) => {
                assert_eq!(since.as_deref(), Some("2026-08-30T00:00:00Z"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
        match err("errors.list", json!({"since": "yesterday"})) {
            CliParseError::InvalidValue { flag, reason } => {
                assert_eq!(flag, "since");
                assert!(reason.contains("RFC3339"), "{reason}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// SPEC #3700 FR-007: `perf.summary` / `perf.violations` share one filter.
    #[test]
    fn perf_operations_parse_their_shared_filter() {
        assert!(matches!(
            ok("perf.summary", json!({})),
            CliCommand::Perf(PerfCommand::Summary {
                since: None,
                stream: None,
                target: None
            })
        ));
        match ok(
            "perf.violations",
            json!({"since": "2026-09-08T00:00:00Z", "stream": "op", "target": "issue."}),
        ) {
            CliCommand::Perf(PerfCommand::Violations {
                since,
                stream,
                target,
            }) => {
                assert_eq!(since.as_deref(), Some("2026-09-08T00:00:00Z"));
                assert_eq!(stream.as_deref(), Some("op"));
                assert_eq!(target.as_deref(), Some("issue."));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn perf_operations_reject_malformed_filters() {
        match err("perf.summary", json!({"since": "yesterday"})) {
            CliParseError::InvalidValue { flag, reason } => {
                assert_eq!(flag, "since");
                assert!(reason.contains("RFC3339"), "{reason}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
        match err("perf.summary", json!({"stream": "frontend"})) {
            CliParseError::InvalidValue { flag, .. } => assert_eq!(flag, "stream"),
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(matches!(
            err("perf.violations", json!({"limit": 5})),
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
    fn issue_create_accepts_explicit_no_owner_or_justified_child_evidence() {
        for owner_check in [
            json!({"owner_spec": null, "search_queries": ["PM current state"]}),
            json!({"owner_spec": 3431, "reason": "Independent current-state implementation"}),
        ] {
            assert!(matches!(
                ok(
                    "issue.create",
                    json!({"title": "fix: current state", "body": "body", "owner_check": owner_check})
                ),
                CliCommand::Issue(IssueCommand::CreateBody { .. })
            ));
        }
    }

    #[test]
    fn issue_create_rejects_incomplete_or_malformed_owner_check_with_recovery() {
        for owner_check in [
            json!({}),
            json!({"owner_spec": 0, "reason": "child"}),
            json!({"owner_spec": null}),
            json!({"owner_spec": null, "search_queries": [" "]}),
            json!({"owner_spec": 3431}),
            json!({"owner_spec": 3431, "reason": " "}),
            json!({"owner_spec": 3431, "reason": "child", "unexpected": true}),
        ] {
            let error = err(
                "issue.create",
                json!({"title": "fix: current state", "body": "body", "owner_check": owner_check}),
            )
            .to_string();
            assert!(error.contains("owner_check"), "{error}");
            assert!(
                error.contains("search_queries"),
                "recovery must name the evidence payload: {error}"
            );
            assert!(
                error.contains("reason"),
                "recovery must explain justified children: {error}"
            );
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

    /// Issue #4581: a `params` key no operation reads used to be dropped in
    /// silence, so a caller that added `dry_run` as a safety net had its
    /// Issue closed for real. Refusal is uniform across every operation, so
    /// no caller has to remember which ones are strict.
    mod unknown_params {
        use super::super::parse;
        use super::{envelope, err, ok};
        use crate::cli::{CliCommand, CliParseError, IssueCommand, WorkspaceCommand};
        use serde_json::json;

        fn refusal(operation: &str, params: serde_json::Value) -> String {
            match err(operation, params) {
                CliParseError::InvalidJson(message) => message,
                other => panic!("expected an InvalidJson refusal for {operation}, got {other}"),
            }
        }

        #[test]
        fn issue_close_refuses_dry_run_instead_of_closing_the_issue() {
            // The exact call from the report: a confirmation-shaped flag that
            // `issue.close` does not implement.
            let envelope = envelope("issue.close", json!({"number": 3699, "dry_run": true}));
            let Err(CliParseError::InvalidJson(message)) = parse(&envelope) else {
                panic!("issue.close must refuse an unknown dry_run instead of closing #3699");
            };
            assert!(
                message.contains("dry_run"),
                "the refusal must name the unknown key: {message}"
            );
            for accepted in ["number", "reason", "comment"] {
                assert!(
                    message.contains(accepted),
                    "the refusal must list the accepted key {accepted}: {message}"
                );
            }
        }

        #[test]
        fn the_refusal_names_the_unknown_key_and_the_accepted_ones() {
            let message = refusal("issue.view", json!({"number": 1, "dryrun": true}));
            assert!(message.contains("dryrun"), "{message}");
            assert!(message.contains("number"), "{message}");
        }

        #[test]
        fn read_only_operations_refuse_unknown_keys_too() {
            // AC-4: strictness does not depend on whether the operation
            // mutates anything.
            let message = refusal("issue.spec.read", json!({"number": 1, "dry_run": true}));
            assert!(message.contains("dry_run"), "{message}");
        }

        #[test]
        fn correct_keys_still_parse() {
            assert!(matches!(
                ok(
                    "issue.close",
                    json!({"number": 3699, "reason": "completed"})
                ),
                CliCommand::Issue(IssueCommand::Close { number: 3699, .. })
            ));
        }

        /// AC-3/AC-4: refusing unknown keys only helps if the calls the fleet
        /// is told to make are not themselves carrying dropped keys. Every
        /// JSON envelope printed by a managed skill or by generated guidance
        /// is parsed here, and an unknown-parameter refusal fails the test —
        /// other refusals do not, because guidance legitimately shows
        /// placeholder values.
        #[test]
        fn shipped_guidance_envelopes_carry_no_unknown_params() {
            use std::path::{Path, PathBuf};

            fn collect(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
                let Ok(entries) = std::fs::read_dir(dir) else {
                    return;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        collect(&path, extension, out);
                    } else if path.extension().is_some_and(|ext| ext == extension) {
                        out.push(path);
                    }
                }
            }

            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let mut sources = Vec::new();
            collect(&root.join(".claude/skills"), "md", &mut sources);
            collect(&root.join(".claude/commands"), "md", &mut sources);
            collect(&root.join(".codex/skills"), "md", &mut sources);
            collect(&root.join(".codex/commands"), "md", &mut sources);
            collect(&root.join("crates/gwt-skills/src"), "rs", &mut sources);

            let mut offenders = Vec::new();
            let mut examined = 0usize;
            for path in sources {
                let text = std::fs::read_to_string(&path).expect("read guidance");
                for line in text.lines() {
                    let Some(start) = line.find(r#"{"schema_version""#) else {
                        continue;
                    };
                    let Some(end) = line.rfind('}') else {
                        continue;
                    };
                    let candidate = &line[start..=end];
                    examined += 1;
                    let Err(CliParseError::InvalidJson(message)) = parse(candidate) else {
                        continue;
                    };
                    if message.contains("does not accept the parameter") {
                        offenders.push(format!("{}: {message}", path.display()));
                    }
                }
            }
            // A scan that found nothing proves nothing.
            assert!(
                examined >= 20,
                "expected the guidance sources to carry JSON envelopes, found {examined}"
            );
            assert!(
                offenders.is_empty(),
                "shipped guidance tells agents to pass parameters no operation reads:\n{}",
                offenders.join("\n")
            );
        }

        #[test]
        fn accepted_aliases_are_not_unknown() {
            // `workspace.join` takes `workspace` as an alias of `workspace_id`.
            let _guard = crate::env_test_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _ambient = gwt_core::test_support::ScopedEnvVar::set(
                gwt_agent::session::GWT_SESSION_ID_ENV,
                "s",
            );
            assert!(matches!(
                ok("workspace.join", json!({"workspace": "w-1"})),
                CliCommand::Workspace(WorkspaceCommand::Join { .. })
            ));
            assert!(matches!(
                ok("workspace.join", json!({"workspace_id": "w-1"})),
                CliCommand::Workspace(WorkspaceCommand::Join { .. })
            ));
        }
    }
}
