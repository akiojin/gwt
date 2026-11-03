use gwt_core::feedback::{
    auto_report::{
        AutoFeedbackRecord, AutoReason, KnownOperation, SafeAutoReport, SafeAutoSnapshot,
    },
    FeedbackContext, FeedbackInput, FeedbackKind, FeedbackRecord, FeedbackState, FeedbackStore,
    PublicFeedback, StoredFeedback,
};
use gwt_github::client::{
    CreateRepositoryIssue, RepositoryIssue, RepositoryIssueKind, ResolutionDeadline,
};
use gwt_github::client::{OwnerMutationError, OwnerRepositoryClient, RepositoryIdentity};
use std::{
    collections::BTreeSet,
    fmt::Write as _,
    io::{self, Write as _},
    path::Path,
    process::Stdio,
    time::Duration,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedbackCommand {
    Create(Box<FeedbackInput>),
    Status { id: String },
    Retry { id: String },
    Process { id: String },
}

#[path = "feedback_consent.rs"]
mod consent;

pub fn run<E: super::CliEnv>(
    env: &mut E,
    command: FeedbackCommand,
    out: &mut String,
) -> Result<i32, gwt_github::SpecOpsError> {
    let root = env.repo_path().to_path_buf();
    let store =
        FeedbackStore::new(gwt_core::paths::gwt_project_dir_for_repo_path(&root).join("feedback"));
    // Project caller text into the closed vocabulary before ledger/context,
    // fingerprinting, persistence or any network/consent side effect.
    if let FeedbackCommand::Create(input) = &command {
        if matches!(auto_policy(&root), Ok(true)) {
            let report = SafeAutoReport::current(KnownOperation::classify(&input.operation));
            return run_auto_create(env, &store, report, out);
        }
    } else {
        let id = match &command {
            FeedbackCommand::Status { id }
            | FeedbackCommand::Retry { id }
            | FeedbackCommand::Process { id } => id,
            FeedbackCommand::Create(_) => unreachable!(),
        };
        match store.read_entry(id).map_err(|_| safe_storage_error())? {
            StoredFeedback::Auto(_) => return run_auto_existing(env, &store, command, out),
            StoredFeedback::Manual(_) => {}
        }
    }
    let result = (|| -> io::Result<FeedbackRecord> {
        match command {
            FeedbackCommand::Status { id } => store.read(&id),
            FeedbackCommand::Process { id } => {
                process_with(&store, &id, &root, consent::confirm, |record| {
                    env.deliver_feedback(record)
                })
            }
            FeedbackCommand::Create(mut input) => {
                attach_error_record(&mut input, &root);
                let context = context(&root);
                let (record, fresh) = store.submit(*input, context, chrono::Utc::now())?;
                if fresh {
                    submit(env, &store, record, &root)
                } else {
                    Ok(record)
                }
            }
            FeedbackCommand::Retry { id } => {
                let record = {
                    let _sender = store.try_sender_lock(&id)?;
                    let mut record = store.read(&id)?;
                    if record.context.project_root != root {
                        return Err(io::Error::other(
                            "feedback source checkout does not match this receipt",
                        ));
                    }
                    if !matches!(
                        record.state,
                        FeedbackState::Failed | FeedbackState::Declined
                    ) {
                        return Err(io::Error::other("feedback.retry only accepts failed or declined reports; an uncertain send must be reconciled with akiojin/gwt before another submission"));
                    }
                    record.state = if gwt_core::feedback::requires_approval(&root) {
                        FeedbackState::PendingApproval
                    } else {
                        FeedbackState::Queued
                    };
                    record.error = None;
                    store.save(&record)?;
                    record
                };
                submit(env, &store, record, &root)
            }
        }
    })();
    let result = result.map_err(super::io_as_api_error)?;
    let receipt = store.path(&result.id).map_err(super::io_as_api_error)?;
    let value = serde_json::json!({
        "request_id": result.id, "state": result.state,
        "issue_number": result.issue_number, "issue_url": result.issue_url,
        "receipt": receipt, "error": result.error,
    });
    let _ = writeln!(out, "{value}");
    Ok(0)
}

pub(super) fn auto_policy(root: &Path) -> Result<bool, AutoReason> {
    crate::issue_monitor::load_issue_monitor_prefs(
        &crate::issue_monitor::issue_monitor_prefs_path_for_repo_path(root),
    )
    .map(|prefs| prefs.autonomous_mode)
    .map_err(|_| AutoReason::PolicyUnavailable)
}

pub(super) fn safe_intake_required(root: &Path) -> bool {
    !matches!(auto_policy(root), Ok(false))
}

fn safe_storage_error() -> gwt_github::SpecOpsError {
    super::io_as_api_error(io::Error::other("feedback storage unavailable or invalid"))
}

fn render_auto_result(
    store: &FeedbackStore,
    record: &AutoFeedbackRecord,
    out: &mut String,
) -> Result<i32, gwt_github::SpecOpsError> {
    let id = record.snapshot.id();
    let receipt = store.path(&id).map_err(|_| safe_storage_error())?;
    let mut value = serde_json::json!({
        "request_id": id, "mode": "auto", "state": record.state,
        "issue_number": record.issue_number,
        "issue_url": record.issue_number.map(|number| format!("https://github.com/akiojin/gwt/issues/{number}")),
        "receipt": receipt, "error": record.error,
    });
    if let Some(owned) = &record.owned {
        value["owned"] = serde_json::to_value(owned).map_err(|_| safe_storage_error())?;
    }
    let _ = writeln!(out, "{value}");
    Ok(0)
}

fn detach_auto<E: super::CliEnv>(
    env: &mut E,
    store: &FeedbackStore,
    record: AutoFeedbackRecord,
) -> io::Result<AutoFeedbackRecord> {
    if record.state != FeedbackState::Queued {
        return Ok(record);
    }
    if env.spawn_feedback_worker(&record.snapshot.id()).is_err() {
        if record.owned.is_some() {
            // Native failure can follow child creation or a lost acceptance
            // acknowledgement. Preserve uncertainty rather than enabling retry.
            return store.reconcile_owned_auto(&record.snapshot.id());
        }
        // Do not overwrite a child that acquired the sender lock before a
        // parent's stdin/spawn failure was observed.
        if let Ok(_lock) = store.try_sender_lock(&record.snapshot.id()) {
            let mut latest = store.read_auto(&record.snapshot.id())?;
            if latest.state == FeedbackState::Queued {
                latest.state = FeedbackState::Failed;
                latest.error = Some(AutoReason::WorkerUnavailable);
                store.save_auto(&latest)?;
            }
            return Ok(latest);
        }
    }
    Ok(record)
}

fn run_auto_create<E: super::CliEnv>(
    env: &mut E,
    store: &FeedbackStore,
    report: SafeAutoReport,
    out: &mut String,
) -> Result<i32, gwt_github::SpecOpsError> {
    let now = chrono::Utc::now();
    #[cfg(windows)]
    let submitted = store.submit_owned_auto(report, now);
    #[cfg(not(windows))]
    let submitted = store.submit_auto(report, now);
    let (record, fresh) = submitted.map_err(|_| safe_storage_error())?;
    let record = if fresh {
        detach_auto(env, store, record).map_err(|_| safe_storage_error())?
    } else {
        record
    };
    render_auto_result(store, &record, out)
}

fn run_auto_existing<E: super::CliEnv>(
    env: &mut E,
    store: &FeedbackStore,
    command: FeedbackCommand,
    out: &mut String,
) -> Result<i32, gwt_github::SpecOpsError> {
    let root = env.repo_path().to_path_buf();
    let record = match command {
        FeedbackCommand::Status { id } => {
            reconcile_auto_status(store, &id).map_err(|_| safe_storage_error())?
        }
        FeedbackCommand::Process { id } => process_auto_with(
            store,
            &id,
            || auto_policy(&root),
            |snapshot| env.deliver_auto_feedback(snapshot),
        )
        .map_err(|_| safe_storage_error())?,
        FeedbackCommand::Retry { id } => {
            if store
                .read_auto(&id)
                .map_err(|_| safe_storage_error())?
                .owned
                .is_some()
            {
                #[cfg(not(windows))]
                return Err(safe_storage_error());
                #[cfg(windows)]
                {
                    let record = store
                        .retry_owned_auto(&id, || {
                            require_auto(&|| auto_policy(&root)).map_err(|_| owned_io_error())
                        })
                        .map_err(|_| safe_storage_error())?;
                    let record =
                        detach_auto(env, store, record).map_err(|_| safe_storage_error())?;
                    return render_auto_result(store, &record, out);
                }
            }
            let record = {
                let _lock = store
                    .try_sender_lock(&id)
                    .map_err(|_| safe_storage_error())?;
                let mut record = store.read_auto(&id).map_err(|_| safe_storage_error())?;
                if !matches!(
                    record.state,
                    FeedbackState::Failed | FeedbackState::Withheld
                ) {
                    return Err(super::io_as_api_error(io::Error::other("automatic feedback retry requires a failed or withheld receipt; uncertain delivery cannot be retried")));
                }
                record.state = FeedbackState::Queued;
                record.error = None;
                if !record.snapshot.report().sendable() {
                    record.state = FeedbackState::Withheld;
                    record.error = Some(AutoReason::InsufficientInformation);
                } else if let Err(reason) = require_auto(&|| auto_policy(&root)) {
                    record.state = FeedbackState::Withheld;
                    record.error = Some(reason);
                }
                store.save_auto(&record).map_err(|_| safe_storage_error())?;
                record
            };
            detach_auto(env, store, record).map_err(|_| safe_storage_error())?
        }
        FeedbackCommand::Create(_) => unreachable!(),
    };
    render_auto_result(store, &record, out)
}

fn require_auto(policy: &impl Fn() -> Result<bool, AutoReason>) -> Result<(), AutoReason> {
    match policy()? {
        true => Ok(()),
        false => Err(AutoReason::PolicyRevoked),
    }
}

#[cfg(any(windows, test))]
fn owned_io_error() -> io::Error {
    io::Error::other("automatic feedback ownership unavailable")
}

#[cfg(windows)]
pub(super) fn owned_store(root: &Path) -> FeedbackStore {
    FeedbackStore::new(gwt_core::paths::gwt_project_dir_for_repo_path(root).join("feedback"))
}

#[cfg(windows)]
fn owned_sender_gate(
    store: &FeedbackStore,
    root: &Path,
    received: &super::feedback_owner_bootstrap::ReceivedBootstrap,
    mutation: bool,
) -> Result<(), AutoReason> {
    let authorize = || {
        received
            .descriptor
            .clock
            .remaining_work()
            .map_err(|_| AutoReason::PreSubmitFailed)?;
        require_auto(&|| auto_policy(root))?;
        if !received
            .owner_alive()
            .map_err(|_| AutoReason::SenderInterrupted)?
        {
            return Err(AutoReason::SenderInterrupted);
        }
        received
            .descriptor
            .clock
            .remaining_work()
            .map_err(|_| AutoReason::PreSubmitFailed)?;
        Ok(())
    };
    owned_operation_gate(
        store,
        &received.descriptor.receipt_id.to_string(),
        received.descriptor.attempt_id,
        mutation,
        &authorize,
    )
}

#[cfg(any(windows, test))]
fn owned_operation_gate(
    store: &FeedbackStore,
    id: &str,
    attempt: uuid::Uuid,
    mutation: bool,
    authorize: &impl Fn() -> Result<(), AutoReason>,
) -> Result<(), AutoReason> {
    use std::cell::Cell;
    let reason = Cell::new(AutoReason::SenderInterrupted);
    let checked = || {
        authorize().map_err(|error| {
            reason.set(error);
            owned_io_error()
        })
    };
    let result = if mutation {
        store.commit_owned_attempt(id, attempt, checked)
    } else {
        store.check_owned_attempt(id, attempt, checked)
    };
    result.map(|_| ()).map_err(|_| reason.get())
}

#[cfg(windows)]
pub(super) fn run_owned_sender(
    root: &Path,
    received: &super::feedback_owner_bootstrap::ReceivedBootstrap,
) -> io::Result<()> {
    use gwt_core::feedback::auto_report::OwnedAutoReason;
    use gwt_github::client::http::HttpIssueClient;
    let store = owned_store(root);
    let id = received.descriptor.receipt_id.to_string();
    let attempt = received.descriptor.attempt_id;
    let _sender = store.try_sender_lock(&id)?;
    // One projection of the original D0, shared by auth/admission/GET/both POSTs.
    let deadline = received.descriptor.clock.resolution_deadline()?;
    let read = || owned_sender_gate(&store, root, received, false);
    let commit = || owned_sender_gate(&store, root, received, true);
    let outcome = (|| {
        read()?;
        let _ = received.acknowledge();
        let client = HttpIssueClient::from_noninteractive_env("akiojin", "gwt")
            .map_err(|_| AutoReason::AuthUnavailable)?;
        let record = store
            .read_auto(&id)
            .map_err(|_| AutoReason::SenderInterrupted)?;
        send_auto_to_upstream_checked(&client, &record.snapshot, &deadline, &read, &commit)
    })();
    if outcome.is_err() && deadline.remaining("automatic feedback work").is_err() {
        // Use the actual shared deadline, rather than guessing from a process
        // disappearing just before the driver's next QPC sample.
        store.cancel_owned_attempt(&id, attempt, OwnedAutoReason::WorkDeadline)?;
    }
    store.finish_owned_attempt(&id, attempt, outcome)?;
    Ok(())
}

fn file_lock_is_busy(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

fn reconcile_auto_status(store: &FeedbackStore, id: &str) -> io::Result<AutoFeedbackRecord> {
    if store.read_auto(id)?.owned.is_some() {
        return store.reconcile_owned_auto(id);
    }
    let _lock = match store.try_sender_lock(id) {
        Ok(lock) => lock,
        Err(error) if file_lock_is_busy(&error) => return store.read_auto(id),
        Err(error) => return Err(error),
    };
    let mut record = store.read_auto(id)?;
    if record.owned.is_some() {
        drop(_lock);
        return store.reconcile_owned_auto(id);
    }
    if record.state == FeedbackState::Sending {
        record.state = FeedbackState::Uncertain;
        record.error = Some(AutoReason::SenderInterrupted);
        store.save_auto(&record)?;
    }
    Ok(record)
}

fn process_auto_with(
    store: &FeedbackStore,
    id: &str,
    policy: impl Fn() -> Result<bool, AutoReason>,
    send: impl FnOnce(&SafeAutoSnapshot) -> Result<u64, AutoReason>,
) -> io::Result<AutoFeedbackRecord> {
    let _lock = match store.try_sender_lock(id) {
        Ok(lock) => lock,
        Err(error) if file_lock_is_busy(&error) => return store.read_auto(id),
        Err(error) => return Err(error),
    };
    let mut record = store.read_auto(id)?;
    if record.owned.is_some() {
        return Err(io::Error::other(
            "owned feedback requires the private inherited sender",
        ));
    }
    if record.state == FeedbackState::Sending {
        record.state = FeedbackState::Uncertain;
        record.error = Some(AutoReason::SenderInterrupted);
        store.save_auto(&record)?;
    }
    if record.state != FeedbackState::Queued {
        return Ok(record);
    }
    if !record.snapshot.report().sendable() {
        record.state = FeedbackState::Withheld;
        record.error = Some(AutoReason::InsufficientInformation);
    } else if let Err(reason) = require_auto(&policy) {
        record.state = FeedbackState::Withheld;
        record.error = Some(reason);
    } else {
        record.state = FeedbackState::Sending;
        store.save_auto(&record)?;
        match send(&record.snapshot) {
            Ok(number) => {
                record.state = FeedbackState::Sent;
                record.issue_number = Some(number);
                record.error = None;
            }
            Err(reason) => {
                record.state = match reason {
                    AutoReason::RemoteOutcomeUnknown | AutoReason::SenderInterrupted => {
                        FeedbackState::Uncertain
                    }
                    AutoReason::PolicyRevoked | AutoReason::PolicyUnavailable => {
                        FeedbackState::Withheld
                    }
                    _ => FeedbackState::Failed,
                };
                record.error = Some(reason);
            }
        }
    }
    store.save_auto(&record)?;
    Ok(record)
}

fn attach_error_record(input: &mut FeedbackInput, root: &Path) {
    if input.error_record_id.is_some() {
        return;
    }
    let expected = gwt_core::error_ledger::sanitize_error_message(&format!(
        "{}: {}",
        input.operation, input.refusal
    ));
    let since = chrono::Utc::now() - chrono::TimeDelta::hours(1);
    input.error_record_id = gwt_core::error_ledger::list_since(Some(since))
        .unwrap_or_default()
        .into_iter()
        .rev()
        .find(|record| {
            record.message == expected
                && record
                    .target
                    .project_root
                    .as_deref()
                    .is_some_and(|path| Path::new(path) == root)
        })
        .map(|record| record.id);
}

fn context(root: &Path) -> FeedbackContext {
    let active_agents =
        gwt_agent::session_ledger::load_sessions(&gwt_core::paths::gwt_sessions_dir())
            .unwrap_or_default()
            .into_iter()
            .filter(|session| {
                matches!(
                    session.status,
                    gwt_agent::AgentStatus::Running
                        | gwt_agent::AgentStatus::Idle
                        | gwt_agent::AgentStatus::WaitingInput
                )
            })
            .count();
    FeedbackContext {
        project_root: root.to_path_buf(),
        project_hash: gwt_core::paths::project_scope_hash(root).to_string(),
        gwt_version: env!("CARGO_PKG_VERSION").into(),
        gwtd_version: env!("CARGO_PKG_VERSION").into(),
        os: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        active_agents,
    }
}

fn submit<E: super::CliEnv>(
    env: &mut E,
    store: &FeedbackStore,
    mut record: FeedbackRecord,
    root: &Path,
) -> io::Result<FeedbackRecord> {
    if gwt_core::feedback::requires_approval(root) {
        if let Err(error) = env.spawn_feedback_worker(&record.id) {
            record.state = FeedbackState::Failed;
            record.error = Some(format!(
                "feedback to akiojin/gwt: could not start approval worker: {error}"
            ));
            store.save(&record)?;
        }
        Ok(record)
    } else {
        process_with(
            store,
            &record.id,
            root,
            |_| Err("approval cannot be bypassed".into()),
            |r| env.deliver_feedback(r),
        )
    }
}

pub(super) fn spawn_worker(root: &Path, id: &str) -> io::Result<()> {
    #[cfg(windows)]
    if matches!(owned_store(root).read_entry(id)?, StoredFeedback::Auto(record) if record.owned.is_some())
    {
        return super::feedback_supervisor::spawn_owner(root, id);
    }
    let envelope = serde_json::to_vec(
        &serde_json::json!({"schema_version":1,"operation":"feedback.process","params":{"id":id}}),
    )?;
    let mut command = gwt_core::process::hidden_command(std::env::current_exe()?);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("feedback worker has no stdin"))?
        .write_all(&envelope)
}

fn process_with(
    store: &FeedbackStore,
    id: &str,
    root: &Path,
    consent: impl FnOnce(&str) -> Result<bool, String>,
    mut send: impl FnMut(&PublicFeedback) -> Result<u64, OwnerMutationError>,
) -> io::Result<FeedbackRecord> {
    let _lock = match store.try_sender_lock(id) {
        Ok(lock) => lock,
        Err(error) if file_lock_is_busy(&error) => return store.read(id),
        Err(error) => return Err(error),
    };
    let mut record = store.read(id)?;
    if record.context.project_root != root {
        return Err(io::Error::other(
            "feedback source checkout does not match this receipt",
        ));
    }
    if record.state == FeedbackState::Sending {
        record.state = FeedbackState::Uncertain;
        record.error = Some("feedback to akiojin/gwt: sender interrupted; inspect the request marker upstream before resubmitting".into());
        store.save(&record)?;
    }
    if !matches!(
        record.state,
        FeedbackState::Queued | FeedbackState::PendingApproval
    ) {
        return Ok(record);
    }
    let public = record.public_snapshot();
    if gwt_core::feedback::requires_approval(root) {
        let prompt = format!("Send this report to the public GitHub repository akiojin/gwt?\nPrivacy masks are applied below; the complete original stays local.\nDeclining leaves it saved locally and your project work can continue.\n\n{}\n\nTitle: {}\n\n{}", public.privacy_summary(), public.input.title, public.render_issue());
        match consent(&prompt) {
            Ok(true) => {}
            Ok(false) => {
                record.state = FeedbackState::Declined;
                store.save(&record)?;
                return Ok(record);
            }
            Err(error) => {
                record.state = FeedbackState::Failed;
                record.error = Some(format!(
                    "feedback to akiojin/gwt: approval unavailable: {error}"
                ));
                store.save(&record)?;
                return Ok(record);
            }
        }
    }
    // Persist before the remote mutation. A crash here is uncertain, never an
    // invitation to repeat a POST. Send exactly the immutable approved snapshot.
    record.state = FeedbackState::Sending;
    store.save(&record)?;
    match send(&public) {
        Ok(number) => {
            record.state = FeedbackState::Sent;
            record.issue_number = Some(number);
            record.issue_url = Some(format!("https://github.com/akiojin/gwt/issues/{number}"));
            record.error = None;
        }
        Err(error) => {
            record.state = match error {
                OwnerMutationError::PreSubmit(_) => FeedbackState::Failed,
                OwnerMutationError::RemoteOutcomeUnknown(_) => FeedbackState::Uncertain,
            };
            record.error = Some(format!("feedback to akiojin/gwt failed: {error}. Check GitHub authentication and access to akiojin/gwt; the complete report remains local."));
        }
    }
    store.save(&record)?;
    Ok(record)
}

pub(super) fn send_to_upstream(
    client: &impl OwnerRepositoryClient,
    record: &PublicFeedback,
) -> Result<u64, OwnerMutationError> {
    send_to_upstream_with_deadline(
        client,
        record,
        &ResolutionDeadline::new(Duration::from_secs(5), Duration::from_secs(120)),
    )
}

fn auto_send_error(error: OwnerMutationError) -> AutoReason {
    match error {
        OwnerMutationError::PreSubmit(gwt_github::client::ApiError::Unauthorized) => {
            AutoReason::AuthUnavailable
        }
        OwnerMutationError::PreSubmit(_) => AutoReason::PreSubmitFailed,
        OwnerMutationError::RemoteOutcomeUnknown(_) => AutoReason::RemoteOutcomeUnknown,
    }
}

pub(super) fn send_auto_to_upstream_with_deadline(
    client: &impl OwnerRepositoryClient,
    snapshot: &SafeAutoSnapshot,
    root: &Path,
    deadline: &ResolutionDeadline,
) -> Result<u64, AutoReason> {
    let policy = || require_auto(&|| auto_policy(root));
    send_auto_to_upstream_checked(client, snapshot, deadline, &policy, &policy)
}

fn send_auto_to_upstream_checked(
    client: &impl OwnerRepositoryClient,
    snapshot: &SafeAutoSnapshot,
    deadline: &ResolutionDeadline,
    before_read: &impl Fn() -> Result<(), AutoReason>,
    before_mutation: &impl Fn() -> Result<(), AutoReason>,
) -> Result<u64, AutoReason> {
    before_read()?;
    let _lock = acquire_upstream_lock(deadline).map_err(auto_send_error)?;
    before_read()?;
    if !snapshot.report().sendable() {
        return Err(AutoReason::InsufficientInformation);
    }
    let repository = RepositoryIdentity::gwt_upstream();
    let issues = client
        .list_issues(&repository, deadline)
        .map_err(|_| AutoReason::PreSubmitFailed)?;
    let marker = snapshot.marker();
    if let Some(issue) = issues
        .items()
        .iter()
        .find(|issue| issue.body.contains(&marker))
    {
        return Ok(issue.number.0);
    }
    let title = snapshot.report().title();
    let body = snapshot.render_issue();
    if let Some(issue) = issues.items().iter().find(|issue| {
        issue.kind == RepositoryIssueKind::Plain
            && issue.title == title
            && issue.body.contains("\"safe_auto_v1\"")
    }) {
        before_read()?;
        let comments = client
            .list_comments(&repository, issue.number, deadline)
            .map_err(|_| AutoReason::PreSubmitFailed)?;
        if !comments
            .items()
            .iter()
            .any(|comment| comment.body.contains(&marker))
        {
            // Re-evaluate after auth, lock admission and remote reads, directly
            // before the mutation. A revoked Auto job never enters consent.
            before_mutation()?;
            client
                .create_owner_comment(&repository, issue.number, &body, deadline)
                .map_err(auto_send_error)?;
        }
        Ok(issue.number.0)
    } else {
        before_mutation()?;
        client
            .create_owner_issue(
                &repository,
                &CreateRepositoryIssue {
                    title,
                    body,
                    labels: vec![],
                },
                deadline,
            )
            .map(|issue| issue.number.0)
            .map_err(auto_send_error)
    }
}

fn acquire_upstream_lock(
    deadline: &ResolutionDeadline,
) -> Result<std::fs::File, OwnerMutationError> {
    use gwt_github::client::ApiError;

    let failure = |error: io::Error| {
        OwnerMutationError::PreSubmit(ApiError::Unexpected(format!(
            "feedback upstream lock failed: {error}"
        )))
    };
    let dir = gwt_core::paths::gwt_home();
    std::fs::create_dir_all(&dir).map_err(failure)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("feedback-upstream.lock"))
        .map_err(failure)?;
    loop {
        let remaining = deadline
            .remaining("acquire feedback upstream lock")
            .map_err(OwnerMutationError::PreSubmit)?;
        match fs2::FileExt::try_lock_exclusive(&lock) {
            Ok(()) => return Ok(lock),
            Err(error) if file_lock_is_busy(&error) => {
                std::thread::sleep(remaining.min(Duration::from_millis(100)));
            }
            Err(error) => return Err(failure(error)),
        }
    }
}

pub(super) fn send_to_upstream_with_deadline(
    client: &impl OwnerRepositoryClient,
    record: &PublicFeedback,
    deadline: &ResolutionDeadline,
) -> Result<u64, OwnerMutationError> {
    // Consent is already complete. Serialize lookup and mutation across every
    // source project so concurrent reports cannot both create the same Issue.
    let _upstream_lock = acquire_upstream_lock(deadline)?;
    let repository = RepositoryIdentity::gwt_upstream();
    let issues = client
        .list_issues(&repository, deadline)
        .map_err(OwnerMutationError::PreSubmit)?;
    let marker = record.marker();
    if let Some(issue) = issues
        .items()
        .iter()
        .find(|issue| issue.body.contains(&marker))
    {
        return Ok(issue.number.0);
    }
    let target = issues
        .items()
        .iter()
        .filter(|issue| {
            issue.kind == RepositoryIssueKind::Plain && same_kind(issue, record.input.kind)
        })
        .filter_map(|issue| {
            let score = similarity(&issue.title, &record.input.title);
            (score >= 0.8).then_some((issue, score))
        })
        .max_by(|(left, a), (right, b)| {
            a.total_cmp(b)
                .then_with(|| right.number.0.cmp(&left.number.0))
        });
    let body = record.render_issue();
    if let Some((issue, _)) = target {
        let comments = client
            .list_comments(&repository, issue.number, deadline)
            .map_err(OwnerMutationError::PreSubmit)?;
        if !comments
            .items()
            .iter()
            .any(|comment| comment.body.contains(&marker))
        {
            client.create_owner_comment(&repository, issue.number, &body, deadline)?;
        }
        return Ok(issue.number.0);
    }
    client
        .create_owner_issue(
            &repository,
            &CreateRepositoryIssue {
                title: record.input.title.clone(),
                body,
                // Public reporters can create Issues without push access, but
                // GitHub discards their labels. Keep classification in the body
                // so the strict mutation readback also succeeds for them.
                labels: vec![],
            },
            deadline,
        )
        .map(|issue| issue.number.0)
}

fn same_kind(issue: &RepositoryIssue, kind: FeedbackKind) -> bool {
    // Older fix/feat reports predate both the feedback marker and labels.
    let title = issue.title.to_lowercase();
    let tagged = |value: &str| {
        issue
            .body
            .contains(&format!("<!-- gwt-feedback-kind:{value} -->"))
    };
    if tagged("bug") || tagged("request") || tagged("friction") {
        return tagged(match kind {
            FeedbackKind::Bug => "bug",
            FeedbackKind::Request => "request",
            FeedbackKind::Friction => "friction",
        });
    }
    match kind {
        FeedbackKind::Bug => {
            issue
                .labels
                .iter()
                .any(|label| label.eq_ignore_ascii_case("bug"))
                || title.starts_with("fix:")
                || title.starts_with("fix(")
        }
        FeedbackKind::Request => {
            issue
                .labels
                .iter()
                .any(|label| label.eq_ignore_ascii_case("enhancement"))
                || title.starts_with("feat:")
                || title.starts_with("feat(")
        }
        FeedbackKind::Friction => title.starts_with("friction:") || title.starts_with("friction("),
    }
}

fn similarity(left: &str, right: &str) -> f64 {
    fn words(title: &str) -> BTreeSet<String> {
        let title = title.trim();
        let title = if ["fix:", "fix(", "feat:", "feat(", "friction:", "friction("]
            .iter()
            .any(|prefix| title.starts_with(prefix))
        {
            title.split_once(':').map_or(title, |(_, body)| body)
        } else {
            title
        };
        title
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .flat_map(|word| {
                let word = word.to_lowercase();
                if word.is_ascii() {
                    return vec![word];
                }
                // Japanese has no word separators. Character tokens preserve
                // shared wording, while pairs retain local character order.
                let chars: Vec<_> = word.chars().collect();
                let mut tokens: Vec<String> = chars.iter().map(char::to_string).collect();
                tokens.extend(chars.windows(2).map(|pair| pair.iter().collect::<String>()));
                tokens
            })
            .collect()
    }
    let (a, b) = (words(left), words(right));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    2.0 * a.intersection(&b).count() as f64 / (a.len() + b.len()) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_github::client::ApiError;
    use gwt_github::{
        client::{
            fake::{OwnerRepositoryFaultTiming, OwnerRepositoryOperation},
            RepositoryIssue, RepositoryIssueKind,
        },
        FakeIssueClient, IssueNumber, IssueState, UpdatedAt,
    };

    // PM0393 component proof: this entry exists only in the test executable.
    // Production run_owned_sender still uses the fixed GitHub constructor.
    mod loopback {
        use super::*;
        use gwt_core::feedback::auto_report::{
            OwnedAutoPhase, OwnedAutoReason, OwnedCancelResult, OwnedCleanupObservation,
            OwnedExitObservation,
        };
        use std::{
            io::Read,
            net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
            process::{Child, ExitStatus},
            time::Instant,
        };

        const CHILD_TEST: &str = "cli::feedback::tests::loopback::child";
        const TOKEN: &str = "gwt-loopback-dummy-token";
        const STAGE_FILE: &str = "loopback-stage";

        fn mark_stage(home: &Path, stage: u8) {
            std::fs::write(home.join(STAGE_FILE), [stage])
                .expect("write the fixed loopback diagnostic stage");
        }

        fn read_stage(path: &Path) -> Option<u8> {
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .ok()?
                .take(2)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() == 1 && bytes[0] <= 7 {
                Some(bytes[0])
            } else {
                None
            }
        }

        #[derive(serde::Serialize, serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct ChildInput {
            home: std::path::PathBuf,
            address: SocketAddrV4,
            id: String,
            attempt: String,
        }

        #[test]
        fn child() {
            let args: Vec<_> = std::env::args().collect();
            if !args.iter().any(|arg| arg == "--exact") || !args.iter().any(|arg| arg == CHILD_TEST)
            {
                return;
            }
            // The parent closes this pipe immediately after writing <=4KiB.
            let mut input = Vec::new();
            std::io::stdin().take(4097).read_to_end(&mut input).unwrap();
            if input.is_empty() {
                return;
            }
            assert!(input.len() <= 4096);
            let input: ChildInput = serde_json::from_slice(&input).unwrap();
            assert_eq!(input.address.ip(), &Ipv4Addr::LOCALHOST);
            assert_ne!(input.address.port(), 0);
            assert!(input.home.is_absolute());
            let attempt = uuid::Uuid::parse_str(&input.attempt).unwrap();
            assert!(!attempt.is_nil());
            assert!(!uuid::Uuid::parse_str(&input.id).unwrap().is_nil());
            mark_stage(&input.home, 1);
            let _home = gwt_core::test_support::ScopedGwtHome::set(&input.home);
            mark_stage(&input.home, 2);
            let store = FeedbackStore::new(input.home.join("feedback"));
            let _sender = store.try_sender_lock(&input.id).unwrap();
            let snapshot = store.read_auto(&input.id).unwrap().snapshot;
            mark_stage(&input.home, 3);
            let endpoint = format!("http://{}", input.address);
            // Explicit dummy credentials and numeric loopback only. This does
            // not attach the global budget or invoke environment authentication.
            use gwt_github::client::http::{HttpIssueClient, ReqwestTransport};
            let transport = ReqwestTransport::new().unwrap();
            mark_stage(&input.home, 4);
            let client = HttpIssueClient::with_transport(transport, TOKEN.into(), "akiojin", "gwt")
                .with_test_endpoints(&endpoint, format!("{endpoint}/graphql"), true)
                .unwrap();
            mark_stage(&input.home, 5);
            let outcome = send_auto_to_upstream_checked(
                &client,
                &snapshot,
                &ResolutionDeadline::new(Duration::from_secs(5), Duration::from_secs(30)),
                &|| owned_operation_gate(&store, &input.id, attempt, false, &|| Ok(())),
                &|| owned_operation_gate(&store, &input.id, attempt, true, &|| Ok(())),
            );
            mark_stage(&input.home, 6);
            store
                .finish_owned_attempt(&input.id, attempt, outcome)
                .unwrap();
            mark_stage(&input.home, 7);
        }

        struct ExactChild(Child);

        fn wait_child(child: &mut Child) -> io::Result<ExitStatus> {
            // test-hygiene: allow-wall-clock-deadline Bound a real test child; no application latency assertion.
            let end = Instant::now() + Duration::from_secs(30);
            loop {
                if child.try_wait()?.is_some() {
                    return child.wait();
                }
                // test-hygiene: allow-wall-clock-deadline Fail a hung OS child without renewing its original guard.
                if Instant::now() >= end {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        impl Drop for ExactChild {
            fn drop(&mut self) {
                if self.0.try_wait().ok().flatten().is_none() {
                    let _ = self.0.kill();
                    let _ = wait_child(&mut self.0);
                }
            }
        }

        fn remaining(end: Instant) -> Duration {
            end.checked_duration_since(Instant::now())
                .filter(|value| !value.is_zero())
                .expect("original loopback I/O deadline expired")
        }

        struct FailureContext<'a> {
            store: &'a FeedbackStore,
            id: &'a str,
            comment: bool,
            outcome: &'a str,
        }

        impl FailureContext<'_> {
            fn receipt_labels(&self) -> (&'static str, &'static str, &'static str) {
                // Read only this fixture's receipt once. Failure must not
                // replace the original missing-HTTP assertion or expose I/O.
                let Ok(record) = self.store.read_auto(self.id) else {
                    return ("unknown", "unknown", "unknown");
                };
                let state = match record.state {
                    FeedbackState::PendingApproval => "pending_approval",
                    FeedbackState::Queued => "queued",
                    FeedbackState::Sending => "sending",
                    FeedbackState::Sent => "sent",
                    FeedbackState::Declined => "declined",
                    FeedbackState::Failed => "failed",
                    FeedbackState::Uncertain => "uncertain",
                    FeedbackState::Withheld => "withheld",
                };
                let reason = match record.error {
                    None => "none",
                    Some(AutoReason::InsufficientInformation) => "insufficient_information",
                    Some(AutoReason::PolicyUnavailable) => "policy_unavailable",
                    Some(AutoReason::PolicyRevoked) => "policy_revoked",
                    Some(AutoReason::WorkerUnavailable) => "worker_unavailable",
                    Some(AutoReason::AuthUnavailable) => "auth_unavailable",
                    Some(AutoReason::PreSubmitFailed) => "pre_submit_failed",
                    Some(AutoReason::RemoteOutcomeUnknown) => "remote_outcome_unknown",
                    Some(AutoReason::SenderInterrupted) => "sender_interrupted",
                };
                let phase = match record.owned.and_then(|owned| owned.attempt) {
                    None => "none",
                    Some(attempt) => match attempt.phase {
                        OwnedAutoPhase::PreDispatch => "pre_dispatch",
                        OwnedAutoPhase::CancelAccepted => "cancel_accepted",
                        OwnedAutoPhase::DispatchCommitted => "dispatch_committed",
                    },
                };
                (state, reason, phase)
            }
        }

        fn receive(
            listener: &TcpListener,
            child: &mut Child,
            stage_path: &Path,
            case: &FailureContext<'_>,
            expected: &str,
        ) -> (TcpStream, String, serde_json::Value, Instant) {
            // test-hygiene: allow-wall-clock-deadline Bound real loopback accept/header/body I/O with one original guard.
            let end = Instant::now() + Duration::from_secs(30);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, peer)) => {
                        assert!(peer.ip().is_loopback());
                        break stream;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        if let Some(status) = child.try_wait().unwrap() {
                            let (state, reason, phase) = case.receipt_labels();
                            let target = if case.comment { "Comment" } else { "Issue" };
                            let outcome = match case.outcome {
                                "crash" => "crash",
                                "cancel" => "cancel",
                                "success" => "success",
                                _ => "unknown",
                            };
                            let expected = match expected {
                                "discovery" => "discovery",
                                "comments" => "comments",
                                "mutation" => "mutation",
                                "readback" => "readback",
                                _ => "unknown",
                            };
                            panic!("test child exited before expected HTTP; case={target}/{outcome} expected={expected} receipt_state={state} auto_reason={reason} owned_phase={phase} success={} code={:?} native_u32={:?} stage={:?}", status.success(), status.code(), status.code().map(|value| value as u32), read_stage(stage_path));
                        }
                        // test-hygiene: allow-wall-clock-deadline Detect missing real HTTP without extending the socket guard.
                        assert!(Instant::now() < end, "expected loopback request was absent");
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(_) => panic!("loopback accept failed"),
                }
            };
            // Winsock can retain the nonblocking listener mode after accept.
            stream.set_nonblocking(false).unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                stream.set_read_timeout(Some(remaining(end))).unwrap();
                let mut part = [0; 1024];
                let size = stream.read(&mut part).expect("bounded loopback header");
                assert_ne!(size, 0, "incomplete loopback header");
                bytes.extend_from_slice(&part[..size]);
                if let Some(at) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    assert!(at <= 16 * 1024);
                    break at + 4;
                }
                assert!(bytes.len() <= 16 * 1024, "loopback header exceeds bound");
            };
            let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
            let mut lines = header.lines();
            let request_line = lines.next().unwrap().to_string();
            let headers: Vec<_> = lines
                .filter_map(|line| line.split_once(':'))
                .map(|(key, value)| (key.to_ascii_lowercase(), value.trim()))
                .collect();
            for singleton in ["authorization", "host", "content-length", "content-type"] {
                assert_eq!(
                    headers.iter().filter(|(key, _)| key == singleton).count(),
                    1,
                    "expected one fixed request header"
                );
            }
            assert!(headers
                .iter()
                .any(|(key, value)| key == "authorization" && *value == format!("Bearer {TOKEN}")));
            assert!(headers.iter().any(|(key, value)| key == "host"
                && *value == listener.local_addr().unwrap().to_string()));
            assert!(headers
                .iter()
                .any(|(key, value)| key == "content-type" && *value == "application/json"));
            assert!(!headers.iter().any(|(key, _)| key == "transfer-encoding"));
            let length: usize = headers
                .iter()
                .find(|(key, _)| key == "content-length")
                .unwrap()
                .1
                .parse()
                .unwrap();
            assert!(length <= 16 * 1024);
            while bytes.len() < header_end + length {
                stream.set_read_timeout(Some(remaining(end))).unwrap();
                let mut part = [0; 1024];
                let size = stream.read(&mut part).expect("bounded loopback body");
                assert_ne!(size, 0, "incomplete loopback body");
                bytes.extend_from_slice(&part[..size]);
            }
            assert_eq!(bytes.len(), header_end + length);
            remaining(end);
            let body = serde_json::from_slice(&bytes[header_end..]).unwrap();
            (stream, request_line, body, end)
        }

        fn respond(mut stream: TcpStream, status: u16, body: serde_json::Value, end: Instant) {
            let body = body.to_string();
            let response = format!("HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let mut rest = response.as_bytes();
            while !rest.is_empty() {
                stream.set_write_timeout(Some(remaining(end))).unwrap();
                let written = stream.write(rest).expect("bounded loopback response");
                assert_ne!(written, 0);
                rest = &rest[written..];
            }
        }

        fn connection(nodes: serde_json::Value) -> serde_json::Value {
            serde_json::json!({"nodes":nodes,"pageInfo":{"hasNextPage":false,"endCursor":null}})
        }

        fn issue(snapshot: &SafeAutoSnapshot, submitted: bool) -> serde_json::Value {
            serde_json::json!({"number":42,"title":snapshot.report().title(),"body":if submitted { snapshot.render_issue() } else { "\"safe_auto_v1\"".into() },"state":"OPEN","updatedAt":"2026-10-31T00:00:00Z","updated_at":"2026-10-31T00:00:00Z","labels":{"totalCount":0,"nodes":[]}})
        }

        #[test]
        fn real_http_issue_and_comment_crash_cancel_and_success() {
            #[cfg(windows)]
            let system_root = {
                use std::os::windows::ffi::OsStringExt;
                let mut buffer = [0_u16; 32768];
                // SAFETY: the API receives a writable, initialized slice and
                // its fixed capacity; only the checked returned range is used.
                let length = unsafe {
                    windows::Win32::System::SystemInformation::GetSystemWindowsDirectoryW(Some(
                        &mut buffer,
                    ))
                } as usize;
                assert!(
                    length > 0 && length < buffer.len(),
                    "test OS runtime metadata unavailable"
                );
                std::ffi::OsString::from_wide(&buffer[..length])
            };
            for comment in [false, true] {
                for outcome in ["crash", "cancel", "success"] {
                    let home = tempfile::tempdir().unwrap();
                    let stage_path = home.path().join(STAGE_FILE);
                    mark_stage(home.path(), 0);
                    let store = FeedbackStore::new(home.path().join("feedback"));
                    let (record, _) = store
                        .submit_owned_auto(
                            SafeAutoReport::current(KnownOperation::VerifyRun),
                            chrono::Utc::now(),
                        )
                        .unwrap();
                    let id = record.snapshot.id();
                    let attempt = uuid::Uuid::new_v4();
                    let owner = store.try_owner_lock(&id).unwrap();
                    store.begin_owned_attempt(&id, attempt, || Ok(())).unwrap();
                    let failure_context = FailureContext {
                        store: &store,
                        id: &id,
                        comment,
                        outcome,
                    };
                    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
                    listener.set_nonblocking(true).unwrap();
                    let std::net::SocketAddr::V4(address) = listener.local_addr().unwrap() else {
                        unreachable!()
                    };
                    let mut command = gwt_core::process::resolved_command(
                        gwt_core::process::ProcessPlanRequest::new(
                            std::env::current_exe().unwrap(),
                        )
                        .inherit_env(false),
                    )
                    .unwrap();
                    command
                        .args(["--exact", CHILD_TEST, "--test-threads=1", "--nocapture"])
                        .env_clear()
                        .env("HOME", home.path())
                        .env("USERPROFILE", home.path())
                        .env("TMPDIR", home.path())
                        .env("TMP", home.path())
                        .env("TEMP", home.path())
                        .env("NO_PROXY", "127.0.0.1")
                        .env("no_proxy", "127.0.0.1")
                        .current_dir(home.path())
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null());
                    // Winsock provider paths may use this OS runtime value.
                    // Never copy parent environment or expose the directory.
                    #[cfg(windows)]
                    command.env("SystemRoot", &system_root);
                    let mut child = ExactChild(command.spawn().unwrap());
                    let input = serde_json::to_vec(&ChildInput {
                        home: home.path().to_path_buf(),
                        address,
                        id: id.clone(),
                        attempt: attempt.to_string(),
                    })
                    .unwrap();
                    assert!(input.len() <= 4096);
                    child.0.stdin.take().unwrap().write_all(&input).unwrap();
                    let (mut read_socket, line, query, mut read_end) = receive(
                        &listener,
                        &mut child.0,
                        &stage_path,
                        &failure_context,
                        "discovery",
                    );
                    assert!(
                        line == "POST /graphql HTTP/1.1",
                        "unexpected discovery request line"
                    );
                    assert!(query["query"].as_str().unwrap().contains("issues(states:"));
                    assert!(
                        query["variables"]
                            == serde_json::json!({"owner":"akiojin","repo":"gwt","after":null}),
                        "unexpected Issue discovery variables"
                    );
                    let listing = serde_json::json!({"data":{"repository":{"issues":connection(if comment {serde_json::json!([issue(&record.snapshot,false)])} else {serde_json::json!([])})}}});
                    let last_read = if comment {
                        respond(read_socket, 200, listing.clone(), read_end);
                        let (socket, line, query, end) = receive(
                            &listener,
                            &mut child.0,
                            &stage_path,
                            &failure_context,
                            "comments",
                        );
                        assert!(
                            line == "POST /graphql HTTP/1.1",
                            "unexpected discovery request line"
                        );
                        assert!(query["query"].as_str().unwrap().contains("comments(first:"));
                        assert!(
                            query["variables"]
                                == serde_json::json!({"owner":"akiojin","repo":"gwt","number":42,"after":null}),
                            "unexpected Comment discovery variables"
                        );
                        read_socket = socket;
                        read_end = end;
                        serde_json::json!({"data":{"repository":{"issue":{"comments":connection(serde_json::json!([]))}}}})
                    } else {
                        listing
                    };
                    if outcome == "cancel" {
                        assert_eq!(
                            store
                                .cancel_owned_attempt(&id, attempt, OwnedAutoReason::Cancelled)
                                .unwrap(),
                            OwnedCancelResult::Accepted
                        );
                    }
                    respond(read_socket, 200, last_read, read_end);
                    if outcome == "cancel" {
                        assert!(wait_child(&mut child.0).unwrap().success());
                        assert_eq!(
                            store
                                .read_auto(&id)
                                .unwrap()
                                .owned
                                .unwrap()
                                .attempt
                                .unwrap()
                                .phase,
                            OwnedAutoPhase::CancelAccepted
                        );
                    } else {
                        let (socket, line, body, end) = receive(
                            &listener,
                            &mut child.0,
                            &stage_path,
                            &failure_context,
                            "mutation",
                        );
                        assert!(
                            line == if comment {
                                "POST /repos/akiojin/gwt/issues/42/comments HTTP/1.1"
                            } else {
                                "POST /repos/akiojin/gwt/issues HTTP/1.1"
                            },
                            "unexpected mutation request line"
                        );
                        let expected = if comment {
                            serde_json::json!({"body":record.snapshot.render_issue()})
                        } else {
                            serde_json::json!({"title":record.snapshot.report().title(),"body":record.snapshot.render_issue(),"labels":[]})
                        };
                        assert!(
                            body == expected,
                            "POST differs from the allowlisted snapshot"
                        );
                        let committed = store.read_auto(&id).unwrap();
                        assert_eq!(committed.state, FeedbackState::Sending);
                        assert_eq!(
                            committed.owned.unwrap().attempt.unwrap().phase,
                            OwnedAutoPhase::DispatchCommitted
                        );
                        if outcome == "crash" {
                            // Complete expected POST observed; retain its socket without a response.
                            child.0.kill().unwrap();
                            assert!(!wait_child(&mut child.0).unwrap().success());
                            drop(socket);
                            // Voluntary fixture-owner release must not depend
                            // on being the final descriptor (e.g. parallel fork).
                            let owner_alias = owner.try_clone().unwrap();
                            fs2::FileExt::unlock(&owner).unwrap();
                            drop(owner);
                            let recovered = store.reconcile_owned_auto(&id).unwrap();
                            assert_eq!(recovered.state, FeedbackState::Uncertain);
                            assert_eq!(recovered.snapshot, record.snapshot);
                            let owned = recovered.owned.unwrap().attempt.unwrap();
                            assert_eq!(owned.id, attempt);
                            assert_eq!(owned.phase, OwnedAutoPhase::DispatchCommitted);
                            assert_eq!(owned.exit, OwnedExitObservation::Unknown);
                            assert_eq!(owned.cleanup, OwnedCleanupObservation::Unknown);
                            assert!(store.retry_owned_auto(&id, || Ok(())).is_err());
                            assert!(owned_operation_gate(&store, &id, attempt, true, &|| Ok(()))
                                .is_err());
                            assert_eq!(
                                store.reconcile_owned_auto(&id).unwrap().state,
                                FeedbackState::Uncertain
                            );
                            drop(owner_alias);
                        } else {
                            let comment_body = serde_json::json!({"id":7,"databaseId":7,"body":record.snapshot.render_issue(),"updated_at":"2026-10-31T00:00:00Z","updatedAt":"2026-10-31T00:00:00Z"});
                            let mut created = issue(&record.snapshot, true);
                            created["labels"] = serde_json::json!([]);
                            respond(
                                socket,
                                201,
                                if comment {
                                    comment_body.clone()
                                } else {
                                    created
                                },
                                end,
                            );
                            let (socket, line, query, end) = receive(
                                &listener,
                                &mut child.0,
                                &stage_path,
                                &failure_context,
                                "readback",
                            );
                            assert!(
                                line == "POST /graphql HTTP/1.1",
                                "unexpected readback request line"
                            );
                            assert!(
                                query["variables"]["number"] == 42,
                                "unexpected readback Issue number"
                            );
                            respond(
                                socket,
                                200,
                                if comment {
                                    serde_json::json!({"data":{"repository":{"issue":{"comments":connection(serde_json::json!([comment_body]))}}}})
                                } else {
                                    serde_json::json!({"data":{"repository":{"issue":issue(&record.snapshot,true)}}})
                                },
                                end,
                            );
                            assert!(wait_child(&mut child.0).unwrap().success());
                            let sent = store.read_auto(&id).unwrap();
                            assert_eq!(sent.state, FeedbackState::Sent);
                            assert_eq!(sent.issue_number, Some(42));
                        }
                    }
                    assert!(
                        matches!(listener.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock),
                        "unexpected second request after exact child exit/recovery"
                    );
                    eprintln!(
                        "real-loopback component: {} {outcome}; REST mutation POSTs={}",
                        if comment { "Comment" } else { "Issue" },
                        if outcome == "cancel" { 0 } else { 1 }
                    );
                }
            }
        }
    }

    #[test]
    fn owned_receipts_cannot_enter_the_public_direct_sender() {
        let dir = tempfile::tempdir().unwrap();
        let store = FeedbackStore::new(dir.path().join("feedback"));
        let (record, _) = store
            .submit_auto(
                SafeAutoReport::current(KnownOperation::VerifyRun),
                chrono::Utc::now(),
            )
            .unwrap();
        let path = store.path(&record.snapshot.id()).unwrap();
        let mut owned = serde_json::to_value(&record).unwrap();
        owned["owned"] = serde_json::json!({"version":"v1","attempt":null});
        std::fs::write(&path, serde_json::to_vec(&owned).unwrap()).unwrap();
        let pending = store.read_auto(&record.snapshot.id()).unwrap();
        let result = process_auto_with(
            &store,
            &record.snapshot.id(),
            || Ok(true),
            |_| panic!("owned receipt requires the private inherited sender binding"),
        );
        assert!(result.is_err());
        assert_eq!(store.read_auto(&record.snapshot.id()).unwrap(), pending);
    }

    #[test]
    fn auto_mutation_gate_covers_issue_and_comment_but_not_verified_readback() {
        let _guard = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        for comment in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let store = FeedbackStore::new(dir.path().join("feedback"));
            let (record, _) = store
                .submit_auto(
                    SafeAutoReport::current(KnownOperation::VerifyRun),
                    chrono::Utc::now(),
                )
                .unwrap();
            let client = FakeIssueClient::new();
            let issue = RepositoryIssue {
                repository: RepositoryIdentity::gwt_upstream(),
                number: IssueNumber(42),
                title: record.snapshot.report().title(),
                body: "\"safe_auto_v1\"".into(),
                labels: vec![],
                state: IssueState::Open,
                kind: RepositoryIssueKind::Plain,
                updated_at: UpdatedAt(chrono::Utc::now().to_rfc3339()),
            };
            if comment {
                client.seed_repository_issue(issue.clone());
            }
            let deadline = ResolutionDeadline::new(Duration::from_secs(5), Duration::from_secs(30));
            assert_eq!(
                send_auto_to_upstream_checked(
                    &client,
                    &record.snapshot,
                    &deadline,
                    &|| Ok(()),
                    &|| Err(AutoReason::SenderInterrupted),
                ),
                Err(AutoReason::SenderInterrupted)
            );
            assert_eq!(client.owner_mutation_count(), 0);
            client.seed_repository_issue(RepositoryIssue {
                body: record.snapshot.marker(),
                ..issue
            });
            assert_eq!(
                send_auto_to_upstream_checked(
                    &client,
                    &record.snapshot,
                    &deadline,
                    &|| Ok(()),
                    &|| panic!("verified marker readback requires no new POST"),
                )
                .unwrap(),
                42
            );
            assert_eq!(client.owner_mutation_count(), 0);
        }
    }

    #[test]
    fn late_read_return_cannot_bypass_cancellation_or_renew_the_original_budget() {
        use super::super::feedback_owner_bootstrap::{Bootstrap, Clock, CreationIdentity};
        use gwt_core::feedback::auto_report::{OwnedAutoPhase, OwnedAutoReason};
        use std::cell::Cell;
        let _guard = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        for cancel in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let store = FeedbackStore::new(dir.path().join("feedback"));
            let (record, _) = store
                .submit_owned_auto(
                    SafeAutoReport::current(KnownOperation::VerifyRun),
                    chrono::Utc::now(),
                )
                .unwrap();
            let id = record.snapshot.id();
            let attempt = uuid::Uuid::new_v4();
            let descriptor = Bootstrap::supervisor(
                uuid::Uuid::parse_str(&id).unwrap(),
                attempt,
                uuid::Uuid::new_v4(),
                Clock {
                    d0_ticks: 10_000,
                    frequency: 1_000,
                },
            );
            let sender = descriptor.sender(CreationIdentity { pid: 1, created: 1 });
            assert_eq!(sender.clock, descriptor.clock);
            let _owner = store.try_owner_lock(&id).unwrap();
            store.begin_owned_attempt(&id, attempt, || Ok(())).unwrap();
            let now = Cell::new(10_000);
            let authorize = || {
                sender
                    .clock
                    .remaining_work_at(now.get(), 1_000)
                    .map(|_| ())
                    .map_err(|_| AutoReason::PreSubmitFailed)
            };
            // Admission before auth and subsequent reads shares the same D0.
            owned_operation_gate(&store, &id, attempt, false, &authorize).unwrap();
            let read = || owned_operation_gate(&store, &id, attempt, false, &authorize);
            let commit = || {
                // The GET has returned; the cancellation/deadline wins before
                // its late result is allowed to enter the mutation gate.
                if cancel {
                    store
                        .cancel_owned_attempt(&id, attempt, OwnedAutoReason::Cancelled)
                        .unwrap();
                } else {
                    now.set(125_000);
                }
                owned_operation_gate(&store, &id, attempt, true, &authorize)
            };
            let client = FakeIssueClient::new();
            let outcome = send_auto_to_upstream_checked(
                &client,
                &record.snapshot,
                &ResolutionDeadline::new(Duration::from_secs(5), Duration::from_secs(30)),
                &read,
                &commit,
            );
            assert!(outcome.is_err());
            assert_eq!(client.owner_mutation_count(), 0);
            let finished = store.finish_owned_attempt(&id, attempt, outcome).unwrap();
            assert_eq!(finished.snapshot, record.snapshot);
            assert_eq!(
                finished.owned.unwrap().attempt.unwrap().phase,
                OwnedAutoPhase::CancelAccepted
            );
        }
    }

    #[test]
    fn owned_issue_and_comment_after_submit_failures_keep_the_commit_uncertain() {
        use gwt_core::feedback::auto_report::OwnedAutoPhase;
        let _guard = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        for comment in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let store = FeedbackStore::new(dir.path().join("feedback"));
            let (record, _) = store
                .submit_owned_auto(
                    SafeAutoReport::current(KnownOperation::VerifyPlan),
                    chrono::Utc::now(),
                )
                .unwrap();
            let id = record.snapshot.id();
            let attempt = uuid::Uuid::new_v4();
            let owner = store.try_owner_lock(&id).unwrap();
            store.begin_owned_attempt(&id, attempt, || Ok(())).unwrap();
            let client = FakeIssueClient::new();
            if comment {
                client.seed_repository_issue(RepositoryIssue {
                    repository: RepositoryIdentity::gwt_upstream(),
                    number: IssueNumber(42),
                    title: record.snapshot.report().title(),
                    body: "\"safe_auto_v1\"".into(),
                    labels: vec![],
                    state: IssueState::Open,
                    kind: RepositoryIssueKind::Plain,
                    updated_at: UpdatedAt(chrono::Utc::now().to_rfc3339()),
                });
            }
            client.fail_next_owner_operation(
                if comment {
                    OwnerRepositoryOperation::CreateComment
                } else {
                    OwnerRepositoryOperation::CreateIssue
                },
                OwnerRepositoryFaultTiming::AfterSubmit,
                ApiError::Unexpected("unmarked private remote failure".into()),
            );
            let outcome = send_auto_to_upstream_checked(
                &client,
                &record.snapshot,
                &ResolutionDeadline::new(Duration::from_secs(5), Duration::from_secs(30)),
                &|| owned_operation_gate(&store, &id, attempt, false, &|| Ok(())),
                &|| owned_operation_gate(&store, &id, attempt, true, &|| Ok(())),
            );
            assert_eq!(outcome, Err(AutoReason::RemoteOutcomeUnknown));
            assert_eq!(client.owner_mutation_count(), 1);
            let finished = store.finish_owned_attempt(&id, attempt, outcome).unwrap();
            assert_eq!(finished.state, FeedbackState::Uncertain);
            assert_eq!(finished.snapshot, record.snapshot);
            assert_eq!(
                finished.owned.unwrap().attempt.unwrap().phase,
                OwnedAutoPhase::DispatchCommitted
            );
            drop(owner);
            assert!(store.retry_owned_auto(&id, || Ok(())).is_err());
        }
    }

    #[test]
    fn auto_policy_revocation_and_unknown_authority_withhold_without_sending() {
        for policy in [Ok(false), Err(AutoReason::PolicyUnavailable)] {
            let dir = tempfile::tempdir().unwrap();
            let store = FeedbackStore::new(dir.path().join("feedback"));
            let (record, _) = store
                .submit_auto(
                    SafeAutoReport::current(KnownOperation::VerifyRun),
                    chrono::Utc::now(),
                )
                .unwrap();
            let result = process_auto_with(
                &store,
                &record.snapshot.id(),
                || policy,
                |_| panic!("revoked/unknown policy cannot send or request consent"),
            )
            .unwrap();
            assert_eq!(result.state, FeedbackState::Withheld);
            assert_eq!(
                result.error,
                Some(policy.err().unwrap_or(AutoReason::PolicyRevoked))
            );
            assert_eq!(result.snapshot, record.snapshot);
        }
    }

    #[test]
    fn auto_feedback_free_text_does_not_change_wire_storage_or_diagnostics() {
        use gwt_github::client::http::{FakeTransport, HttpIssueClient, HttpResponse};
        let _guard = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let (dir, _, sample) = report("https://github.com/acme/private.git");
        crate::save_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(dir.path()),
            &crate::IssueMonitorPrefs {
                autonomous_mode: true,
                ..Default::default()
            },
        )
        .unwrap();
        let store = FeedbackStore::new(
            gwt_core::paths::gwt_project_dir_for_repo_path(dir.path()).join("feedback"),
        );
        let trace_bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        #[derive(Clone)]
        struct Trace(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl io::Write for Trace {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        fn files(root: &Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
            let mut result = std::collections::BTreeMap::new();
            for entry in std::fs::read_dir(root).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    result.extend(files(&path));
                } else {
                    result.insert(path.clone(), std::fs::read(path).unwrap());
                }
            }
            result
        }
        let secrets = [
            "unmarked private sentence 98723",
            "未知の機密と未標識ソース",
            "cHJpdmF0ZS1wcm9qZWN0LXNlY3JldA==",
            "const hidden = proprietaryLogic();",
            "https://private.invalid/code C:\\private\\work",
            "ghp_dummy_known_token_12345678901234567890",
        ];
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer({
                let bytes = trace_bytes.clone();
                move || Trace(bytes.clone())
            })
            .finish();
        // Stabilize the optional, first-wins native envelope context before
        // varying caller text; another parallel test may have recorded it first.
        gwt_core::paths::record_operation_project_store(dir.path());
        tracing::subscriber::with_default(subscriber, || {
            let mut baseline = None;
            for secret in secrets {
                let mut input = sample.input.clone();
                for field in [
                    &mut input.title,
                    &mut input.refusal,
                    &mut input.reproduction,
                    &mut input.body,
                ] {
                    *field = secret.into();
                }
                input.workaround = Some(secret.into());
                input.error_record_id = Some(secret.into());
                input.labels = vec![secret.into()];
                let envelope = serde_json::json!({"schema_version":1,"operation":"feedback.create","params":input});
                let mut env = super::super::TestEnv::new(dir.path().join("cache"));
                env.repo_path = dir.path().to_path_buf();
                assert_eq!(
                    super::super::dispatch_json_input(&mut env, "gwtd", &envelope.to_string()),
                    0
                );
                let output: serde_json::Value = serde_json::from_slice(&env.stdout).unwrap();
                let receipt: serde_json::Value =
                    serde_json::from_str(output["output"].as_str().unwrap()).unwrap();
                let record = store
                    .read_auto(receipt["request_id"].as_str().unwrap())
                    .unwrap();
                let title = record.snapshot.report().title();
                let body = record.snapshot.render_issue();
                let transport = FakeTransport::new();
                let response = |value: serde_json::Value| HttpResponse {
                    status: 200,
                    headers: vec![],
                    body: value.to_string(),
                };
                transport.enqueue(response(serde_json::json!({"data":{"repository":{"issues":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}})));
                let mut created = response(
                    serde_json::json!({"number":1,"title":title,"body":body,"state":"open","updated_at":"2026-10-31T00:00:00Z","labels":[]}),
                );
                created.status = 201;
                transport.enqueue(created);
                transport.enqueue(response(serde_json::json!({"data":{"repository":{"issue":{"number":1,"title":title,"body":body,"state":"OPEN","updatedAt":"2026-10-31T00:00:00Z","labels":{"totalCount":0,"nodes":[]}}}}})));
                let client = HttpIssueClient::with_transport(
                    transport,
                    "dummy-auth".into(),
                    "akiojin",
                    "gwt",
                );
                assert_eq!(
                    send_auto_to_upstream_with_deadline(
                        &client,
                        &record.snapshot,
                        dir.path(),
                        &ResolutionDeadline::new(Duration::from_secs(5), Duration::from_secs(30))
                    )
                    .unwrap(),
                    1
                );
                let observed = (
                    env.stdout,
                    env.stderr,
                    files(store.path(&record.snapshot.id()).unwrap().parent().unwrap()),
                    client.transport().recorded(),
                );
                if let Some(expected) = &baseline {
                    assert_eq!(&observed, expected, "free text changed a sink");
                } else {
                    baseline = Some(observed);
                }
            }
        });
        let diagnostics = format!(
            "{}\n{:?}\n{:?}",
            String::from_utf8_lossy(&trace_bytes.lock().unwrap()),
            gwt_core::process_console::global().snapshot_all(),
            gwt_core::error_ledger::list_since(None).unwrap_or_default()
        );
        for secret in secrets {
            assert!(
                !diagnostics.contains(secret),
                "raw input reached diagnostics"
            );
        }
    }

    #[test]
    fn auto_sending_is_durable_and_unknown_outcomes_cannot_be_replayed() {
        for reason in [
            AutoReason::PreSubmitFailed,
            AutoReason::RemoteOutcomeUnknown,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let store = FeedbackStore::new(dir.path().join("feedback"));
            let (record, _) = store
                .submit_auto(
                    SafeAutoReport::current(KnownOperation::HookHealth),
                    chrono::Utc::now(),
                )
                .unwrap();
            let result = process_auto_with(
                &store,
                &record.snapshot.id(),
                || Ok(true),
                |snapshot| {
                    assert_eq!(snapshot, &record.snapshot);
                    assert_eq!(
                        store.read_auto(&snapshot.id()).unwrap().state,
                        FeedbackState::Sending
                    );
                    Err(reason)
                },
            )
            .unwrap();
            assert_eq!(
                result.state,
                if reason == AutoReason::RemoteOutcomeUnknown {
                    FeedbackState::Uncertain
                } else {
                    FeedbackState::Failed
                }
            );
            let repeated = process_auto_with(
                &store,
                &record.snapshot.id(),
                || Ok(true),
                |_| panic!("no automatic replay"),
            )
            .unwrap();
            assert_eq!(repeated, result);
            let (duplicate, fresh) = store
                .submit_auto(record.snapshot.report().clone(), chrono::Utc::now())
                .unwrap();
            assert!(!fresh);
            assert_eq!(duplicate, result);
        }
        let _guard = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let (dir, _, _) = report("https://github.com/acme/private.git");
        let storage = gwt_core::paths::gwt_project_dir_for_repo_path(dir.path()).join("feedback");
        let store = FeedbackStore::new(storage.clone());
        let (record, _) = store
            .submit_auto(
                SafeAutoReport::current(KnownOperation::Search),
                chrono::Utc::now(),
            )
            .unwrap();
        let receipt_path = store.path(&record.snapshot.id()).unwrap();
        let backup = dir.path().join("feedback-before-save-failure.json");
        let mut before_save_failure = None;
        assert!(process_auto_with(
            &store,
            &record.snapshot.id(),
            || Ok(true),
            |snapshot| {
                let sending = store.read_auto(&snapshot.id()).unwrap();
                assert_eq!(sending.state, FeedbackState::Sending);
                before_save_failure = Some(sending);
                std::fs::rename(&receipt_path, &backup)
                    .expect("back up the closed Sending receipt");
                std::fs::create_dir(&receipt_path).expect("obstruct the post-remote local save");
                Ok(42)
            },
        )
        .is_err());
        std::fs::remove_dir(&receipt_path).unwrap();
        std::fs::rename(&backup, &receipt_path).unwrap();
        let record = store.read_auto(&record.snapshot.id()).unwrap();
        assert_eq!(Some(&record), before_save_failure.as_ref());
        assert_eq!(record.state, FeedbackState::Sending);
        let mut env = super::super::TestEnv::new(dir.path().join("cache"));
        env.repo_path = dir.path().to_path_buf();
        let sender = store.try_sender_lock(&record.snapshot.id()).unwrap();
        let status = FeedbackCommand::Status {
            id: record.snapshot.id(),
        };
        run(&mut env, status.clone(), &mut String::new()).unwrap();
        assert_eq!(store.read_auto(&record.snapshot.id()).unwrap(), record);
        drop(sender);
        let mut output = String::new();
        run(&mut env, status, &mut output).unwrap();
        let interrupted = store.read_auto(&record.snapshot.id()).unwrap();
        assert_eq!(interrupted.state, FeedbackState::Uncertain);
        assert_eq!(interrupted.error, Some(AutoReason::SenderInterrupted));
        assert!(interrupted.issue_number.is_none());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&output).unwrap()["state"],
            "uncertain"
        );
        assert!(run(
            &mut env,
            FeedbackCommand::Retry {
                id: record.snapshot.id()
            },
            &mut String::new()
        )
        .is_err());
        assert!(env.feedback_worker_ids.is_empty());
        assert_eq!(env.client.owner_mutation_count(), 0);
    }

    #[test]
    fn auto_storage_and_worker_start_failures_keep_only_safe_results() {
        let _guard = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let (dir, _, sample) = report("https://github.com/acme/private.git");
        crate::save_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(dir.path()),
            &crate::IssueMonitorPrefs {
                autonomous_mode: true,
                ..Default::default()
            },
        )
        .unwrap();
        let mut env = super::super::TestEnv::new(dir.path().join("cache"));
        env.repo_path = dir.path().to_path_buf();
        let secret = "unmarked private caller and worker error phrase";
        let mut input = sample.input;
        input.title = secret.into();
        input.refusal = secret.into();
        input.body = secret.into();
        let storage = gwt_core::paths::gwt_project_dir_for_repo_path(dir.path()).join("feedback");
        std::fs::write(&storage, "blocked storage").unwrap();
        let mut output = String::new();
        let error = run(
            &mut env,
            FeedbackCommand::Create(Box::new(input.clone())),
            &mut output,
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("feedback storage unavailable or invalid"));
        assert!(!error.to_string().contains(secret));
        assert!(output.is_empty());
        assert!(env.feedback_worker_ids.is_empty());
        assert_eq!(env.client.owner_mutation_count(), 0);

        std::fs::remove_file(&storage).unwrap();
        env.feedback_worker_error = Some(secret.into());
        run(
            &mut env,
            FeedbackCommand::Create(Box::new(input)),
            &mut output,
        )
        .unwrap();
        let receipt: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(receipt["mode"], "auto");
        assert_eq!(
            receipt["state"],
            if cfg!(windows) { "uncertain" } else { "failed" }
        );
        assert_eq!(
            receipt["error"],
            if cfg!(windows) {
                "sender_interrupted"
            } else {
                "worker_unavailable"
            }
        );
        assert!(!output.contains(secret));
        assert!(
            !std::fs::read_to_string(receipt["receipt"].as_str().unwrap())
                .unwrap()
                .contains(secret)
        );
        assert_eq!(env.feedback_worker_ids.len(), 1);
        assert!(env.stderr.is_empty());
        assert_eq!(env.client.owner_mutation_count(), 0);
    }

    #[test]
    fn auto_remote_errors_and_retry_never_expose_raw_error_bodies() {
        let _guard = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let (dir, _, sample) = report("https://github.com/acme/private.git");
        crate::save_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(dir.path()),
            &crate::IssueMonitorPrefs {
                autonomous_mode: true,
                ..Default::default()
            },
        )
        .unwrap();
        let mut env = super::super::TestEnv::new(dir.path().join("cache"));
        env.repo_path = dir.path().to_path_buf();
        // Existing legacy Auto receipts keep their direct-process contract on
        // every OS; fresh Windows owned intake has separate native coverage.
        let store = FeedbackStore::new(
            gwt_core::paths::gwt_project_dir_for_repo_path(dir.path()).join("feedback"),
        );
        let (legacy, _) = store
            .submit_auto(
                SafeAutoReport::current(KnownOperation::classify(&sample.input.operation)),
                chrono::Utc::now(),
            )
            .unwrap();
        let mut created = String::new();
        render_auto_result(&store, &legacy, &mut created).unwrap();
        let receipt: serde_json::Value = serde_json::from_str(&created).unwrap();
        let id = receipt["request_id"].as_str().unwrap();
        let secret = "unmarked remote failure body 684672";
        env.client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            ApiError::Unexpected(secret.into()),
        );
        let mut output = String::new();
        run(
            &mut env,
            FeedbackCommand::Process { id: id.into() },
            &mut output,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&output).unwrap()["state"],
            "uncertain"
        );
        assert!(!output.contains(secret));
        let stored = std::fs::read_to_string(receipt["receipt"].as_str().unwrap()).unwrap();
        assert!(!stored.contains(secret));
        assert!(run(
            &mut env,
            FeedbackCommand::Retry { id: id.into() },
            &mut String::new()
        )
        .is_err());
        run(
            &mut env,
            FeedbackCommand::Process { id: id.into() },
            &mut String::new(),
        )
        .unwrap();
        assert_eq!(env.client.owner_mutation_count(), 1);
    }

    #[test]
    fn env_repository_and_capacity_auto_do_not_grant_feedback_auto_authority() {
        let _guard = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let _flag = gwt_core::test_support::ScopedEnvVar::set("GWT_AUTONOMOUS_EXECUTION", "1");
        let _mode = gwt_core::test_support::ScopedEnvVar::set("GWT_AUTONOMOUS_MODE", "true");
        let (dir, _, sample) = report("https://github.com/acme/private.git");
        std::fs::create_dir(dir.path().join(".gwt")).unwrap();
        std::fs::write(
            dir.path().join(".gwt/issue-monitor.json"),
            r#"{"autonomous_mode":true}"#,
        )
        .unwrap();
        crate::save_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(dir.path()),
            &crate::IssueMonitorPrefs {
                autonomous_mode: false,
                max_active_agents_mode: crate::issue_monitor::IssueMonitorMaxActiveMode::Auto,
                ..Default::default()
            },
        )
        .unwrap();
        let mut env = super::super::TestEnv::new(dir.path().join("cache"));
        env.repo_path = dir.path().to_path_buf();
        let mut output = String::new();
        run(
            &mut env,
            FeedbackCommand::Create(Box::new(sample.input)),
            &mut output,
        )
        .unwrap();
        let receipt: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(receipt["state"], "pending_approval");
        let store = FeedbackStore::new(
            gwt_core::paths::gwt_project_dir_for_repo_path(dir.path()).join("feedback"),
        );
        assert!(store.read(receipt["request_id"].as_str().unwrap()).is_ok());
        // Switching the preference never silently promotes that legacy raw job.
        crate::save_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(dir.path()),
            &crate::IssueMonitorPrefs {
                autonomous_mode: true,
                ..Default::default()
            },
        )
        .unwrap();
        let result = process_with(
            &store,
            receipt["request_id"].as_str().unwrap(),
            dir.path(),
            |_| Ok(false),
            |_| panic!("legacy raw report requires consent"),
        )
        .unwrap();
        assert_eq!(result.state, FeedbackState::Declined);

        // Unknown fresh mode retains Manual origin/preview semantics. It is
        // distinct from a known Auto job losing policy while being processed.
        for origin in [
            "https://github.com/akiojin/gwt.git",
            "https://github.com/acme/private.git",
        ] {
            let case_home = tempfile::tempdir().unwrap();
            let _case_home = gwt_core::test_support::ScopedGwtHome::set(case_home.path());
            let (dir, _, sample) = report(origin);
            let prefs = crate::issue_monitor_prefs_path_for_repo_path(dir.path());
            std::fs::create_dir_all(&prefs).unwrap();
            assert_eq!(auto_policy(dir.path()), Err(AutoReason::PolicyUnavailable));
            let original = sample.input;
            let mut env = super::super::TestEnv::new(dir.path().join("cache"));
            env.repo_path = dir.path().to_path_buf();
            let mut output = String::new();
            run(
                &mut env,
                FeedbackCommand::Create(Box::new(original.clone())),
                &mut output,
            )
            .unwrap();
            let receipt: serde_json::Value = serde_json::from_str(&output).unwrap();
            assert!(
                receipt.get("mode").is_none(),
                "unknown fresh mode must retain a Manual receipt"
            );
            let store = FeedbackStore::new(
                gwt_core::paths::gwt_project_dir_for_repo_path(dir.path()).join("feedback"),
            );
            let manual = store.read(receipt["request_id"].as_str().unwrap()).unwrap();
            assert!(manual.input == original, "Manual keeps its original input");
            let needs_approval = gwt_core::feedback::requires_approval(dir.path());
            assert_eq!(
                manual.state,
                if needs_approval {
                    FeedbackState::PendingApproval
                } else {
                    FeedbackState::Sent
                }
            );
            assert_eq!(env.feedback_worker_ids.len(), usize::from(needs_approval));
            assert_eq!(
                env.client.owner_mutation_count(),
                usize::from(!needs_approval)
            );
        }
    }

    #[test]
    fn concurrent_auto_workers_leave_the_owned_snapshot_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let store = FeedbackStore::new(dir.path().join("feedback"));
        let (record, _) = store
            .submit_auto(
                SafeAutoReport::current(KnownOperation::Search),
                chrono::Utc::now(),
            )
            .unwrap();
        let _sender = store.try_sender_lock(&record.snapshot.id()).unwrap();
        let observed = process_auto_with(
            &store,
            &record.snapshot.id(),
            || Ok(true),
            |_| panic!("only one sender can own a snapshot"),
        )
        .unwrap();
        assert_eq!(observed, record);
    }

    #[test]
    fn auto_retry_recovers_only_the_same_safe_snapshot_after_policy_is_restored() {
        let _guard = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let (dir, _, sample) = report("https://github.com/acme/private.git");
        let prefs = crate::issue_monitor_prefs_path_for_repo_path(dir.path());
        let auto = crate::IssueMonitorPrefs {
            autonomous_mode: true,
            ..Default::default()
        };
        crate::save_issue_monitor_prefs(&prefs, &auto).unwrap();
        let mut env = super::super::TestEnv::new(dir.path().join("cache"));
        env.repo_path = dir.path().to_path_buf();
        let mut output = String::new();
        let legacy_store = FeedbackStore::new(
            gwt_core::paths::gwt_project_dir_for_repo_path(dir.path()).join("feedback"),
        );
        let (legacy, _) = legacy_store
            .submit_auto(
                SafeAutoReport::current(KnownOperation::classify(&sample.input.operation)),
                chrono::Utc::now(),
            )
            .unwrap();
        let legacy = detach_auto(&mut env, &legacy_store, legacy).unwrap();
        render_auto_result(&legacy_store, &legacy, &mut output).unwrap();
        let receipt: serde_json::Value = serde_json::from_str(&output).unwrap();
        let id = receipt["request_id"].as_str().unwrap();
        let store = FeedbackStore::new(
            gwt_core::paths::gwt_project_dir_for_repo_path(dir.path()).join("feedback"),
        );
        let snapshot = store.read_auto(id).unwrap().snapshot;
        crate::save_issue_monitor_prefs(&prefs, &crate::IssueMonitorPrefs::default()).unwrap();
        run(
            &mut env,
            FeedbackCommand::Process { id: id.into() },
            &mut String::new(),
        )
        .unwrap();
        run(
            &mut env,
            FeedbackCommand::Retry { id: id.into() },
            &mut String::new(),
        )
        .unwrap();
        assert_eq!(store.read_auto(id).unwrap().state, FeedbackState::Withheld);
        assert_eq!(env.feedback_worker_ids.len(), 1);
        crate::save_issue_monitor_prefs(&prefs, &auto).unwrap();
        run(
            &mut env,
            FeedbackCommand::Retry { id: id.into() },
            &mut String::new(),
        )
        .unwrap();
        assert_eq!(env.feedback_worker_ids.len(), 2);
        run(
            &mut env,
            FeedbackCommand::Process { id: id.into() },
            &mut String::new(),
        )
        .unwrap();
        let sent = store.read_auto(id).unwrap();
        assert_eq!(sent.state, FeedbackState::Sent);
        assert_eq!(sent.snapshot, snapshot);

        let mut unknown = sample.input;
        unknown.operation = "unrecognized private action".into();
        let mut output = String::new();
        run(
            &mut env,
            FeedbackCommand::Create(Box::new(unknown)),
            &mut output,
        )
        .unwrap();
        let receipt: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(receipt["state"], "withheld");
        let retry = run(
            &mut env,
            FeedbackCommand::Retry {
                id: receipt["request_id"].as_str().unwrap().into(),
            },
            &mut String::new(),
        );
        if cfg!(windows) {
            assert!(
                retry.is_err(),
                "owned unknown facts cannot mint a new attempt"
            );
        } else {
            retry.unwrap();
        }
        assert_eq!(
            env.feedback_worker_ids.len(),
            2,
            "unknown facts never become a public report"
        );
    }

    #[test]
    fn canonical_auto_detaches_and_drops_all_free_text_before_persistence() {
        let _lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let (dir, _, sample) = report("https://github.com/akiojin/gwt.git");
        crate::save_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(dir.path()),
            &crate::IssueMonitorPrefs {
                autonomous_mode: true,
                ..Default::default()
            },
        )
        .unwrap();
        let mut env = super::super::TestEnv::new(dir.path().join("cache"));
        env.repo_path = dir.path().to_path_buf();
        let mut input = sample.input;
        input.title = "plain unmarked confidential phrase".into();
        input.refusal = "未知の機密を含む原文".into();
        input.reproduction = "https://private.example/source".into();
        input.body = "unmarked_private_source".into();
        input.labels = vec!["private_label".into()];
        let mut out = String::new();
        run(
            &mut env,
            FeedbackCommand::Create(Box::new(input.clone())),
            &mut out,
        )
        .unwrap();
        let response: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(response["state"], "queued", "Auto must never send inline");
        assert_eq!(env.feedback_worker_ids.len(), 1);
        assert_eq!(env.client.owner_mutation_count(), 0);
        let receipt = std::fs::read_to_string(response["receipt"].as_str().unwrap()).unwrap();
        for secret in [
            &input.title,
            &input.refusal,
            &input.reproduction,
            &input.body,
            &input.labels[0],
        ] {
            assert!(!receipt.contains(secret), "Auto receipt retained free text");
        }
        assert!(!receipt.contains("project_root"));
        assert!(!receipt.contains("active_agents"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&receipt).unwrap()["mode"],
            "auto"
        );

        input.title = "completely different secret".into();
        input.refusal = "another unrecognized confidential sentence".into();
        input.body = "const secret = different_code".into();
        input.labels.clear();
        let mut repeat = String::new();
        run(
            &mut env,
            FeedbackCommand::Create(Box::new(input)),
            &mut repeat,
        )
        .unwrap();
        assert_eq!(
            out, repeat,
            "free text must not affect fingerprint or status"
        );
        assert_eq!(env.feedback_worker_ids.len(), 1);
    }

    fn report(origin: &str) -> (tempfile::TempDir, FeedbackStore, FeedbackRecord) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(
            dir.path().join(".git/config"),
            format!("[remote \"origin\"]\nurl = {origin}\n"),
        )
        .unwrap();
        let store = FeedbackStore::new(dir.path().join("feedback"));
        let (record, _) = store
            .submit(
                FeedbackInput {
                    title: "codex hook trust hash mismatch".into(),
                    kind: FeedbackKind::Bug,
                    operation: "hook.health".into(),
                    refusal: "exact\nrefusal".into(),
                    reproduction: "unconfirmed".into(),
                    workaround: None,
                    body: String::new(),
                    error_record_id: None,
                    labels: vec![],
                },
                FeedbackContext {
                    project_root: dir.path().to_path_buf(),
                    project_hash: "source".into(),
                    gwt_version: "test".into(),
                    gwtd_version: "test".into(),
                    os: "test".into(),
                    active_agents: 2,
                },
                chrono::Utc::now(),
            )
            .unwrap();
        (dir, store, record)
    }

    #[test]
    fn public_preview_and_submission_share_masked_evidence_and_keep_local_original() {
        let (dir, store, mut record) = report("https://github.com/acme/private-app.git");
        let source = dir.path().join("src/private.rs").display().to_string();
        record.input.title = format!("Failure in {source}");
        record.input.refusal = format!(
            "  before\r\n{source}\nPRIVATE_SETTING=confidential-value\nBearer private-bearer-value\n  after  "
        );
        record.input.body = "```rust\nfn private_business_logic() {}\n```".into();
        record.input.reproduction = "Open `private_business_logic()` in src/private.rs".into();
        store.save(&record).unwrap();
        let preview = std::cell::RefCell::new(String::new());
        let client = FakeIssueClient::new();
        let result = process_with(
            &store,
            &record.id,
            dir.path(),
            |prompt| {
                for hidden in [
                    source.as_str(),
                    "confidential-value",
                    "private-bearer-value",
                    "private_business_logic",
                    "src/private.rs",
                ] {
                    assert!(!prompt.contains(hidden), "public preview exposed {hidden}");
                }
                assert!(prompt.contains("  before\r\n"));
                assert!(prompt.contains("  after  "));
                *preview.borrow_mut() = prompt.to_owned();
                Ok(true)
            },
            |published| send_to_upstream(&client, published),
        )
        .unwrap();
        let published = client
            .fetch_issue(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(result.issue_number.unwrap()),
                &ResolutionDeadline::new(Duration::from_secs(5), Duration::from_secs(120)),
            )
            .unwrap();
        assert!(preview
            .borrow()
            .ends_with(&format!("Title: {}\n\n{}", published.title, published.body)));
        assert_eq!(result.state, FeedbackState::Sent);
        assert_eq!(store.read(&record.id).unwrap().input, record.input);
    }

    #[test]
    fn retry_does_not_overwrite_a_receipt_owned_by_an_active_sender() {
        let _lock = gwt_core::test_support::env_lock();
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let (dir, _store, sample) = report("https://github.com/acme/app.git");
        let mut env = super::super::TestEnv::new(dir.path().join("cache"));
        env.repo_path = dir.path().to_path_buf();
        let mut out = String::new();
        run(
            &mut env,
            FeedbackCommand::Create(Box::new(sample.input)),
            &mut out,
        )
        .unwrap();
        let receipt: serde_json::Value = serde_json::from_str(&out).unwrap();
        let id = receipt["request_id"].as_str().unwrap();
        let store = FeedbackStore::new(
            gwt_core::paths::gwt_project_dir_for_repo_path(dir.path()).join("feedback"),
        );
        let mut failed = store.read(id).unwrap();
        failed.state = FeedbackState::Failed;
        store.save(&failed).unwrap();
        let _sender = store.try_sender_lock(id).unwrap();
        assert!(run(
            &mut env,
            FeedbackCommand::Retry { id: id.into() },
            &mut String::new()
        )
        .is_err());
        assert_eq!(store.read(id).unwrap(), failed);
        assert_eq!(env.feedback_worker_ids.len(), 1);
    }

    #[test]
    fn foreign_intake_returns_without_consent_and_automatically_links_source_evidence() {
        let _lock = gwt_core::test_support::env_lock();
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let (dir, _store, sample) = report("https://github.com/acme/app.git");
        let error = gwt_core::error_ledger::record(gwt_core::error_ledger::ErrorRecord::new(
            gwt_core::error_ledger::ErrorKind::OperationRefusal,
            format!("{}: {}", sample.input.operation, sample.input.refusal),
            gwt_core::error_ledger::ErrorTarget {
                project_root: Some(dir.path().display().to_string()),
                ..Default::default()
            },
        ))
        .unwrap();
        let mut env = super::super::TestEnv::new(dir.path().join("cache"));
        env.repo_path = dir.path().to_path_buf();
        let mut out = String::new();
        run(
            &mut env,
            FeedbackCommand::Create(Box::new(sample.input.clone())),
            &mut out,
        )
        .unwrap();
        let response: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(response["state"], "pending_approval");
        assert_eq!(env.feedback_worker_ids.len(), 1);
        assert_eq!(env.client.owner_mutation_count(), 0);
        let record: FeedbackRecord =
            serde_json::from_slice(&std::fs::read(response["receipt"].as_str().unwrap()).unwrap())
                .unwrap();
        assert_eq!(
            record.input.error_record_id.as_deref(),
            Some(error.id.as_str())
        );
        assert_eq!(
            record.context.project_hash,
            gwt_core::paths::project_scope_hash(dir.path()).to_string()
        );
        assert!(!record.context.os.is_empty());
        let mut repeat = String::new();
        run(
            &mut env,
            FeedbackCommand::Create(Box::new(sample.input)),
            &mut repeat,
        )
        .unwrap();
        assert_eq!(out, repeat);
        assert_eq!(env.feedback_worker_ids.len(), 1);
    }

    #[test]
    fn gwt_feedback_sends_once_without_requesting_consent() {
        let (dir, store, mut record) = report("https://github.com/akiojin/gwt.git");
        record.input.refusal = "PRIVATE_SETTING=exempt-secret".into();
        store.save(&record).unwrap();
        let client = FakeIssueClient::new();
        let mut send = |r: &PublicFeedback| send_to_upstream(&client, r);
        let sent = process_with(
            &store,
            &record.id,
            dir.path(),
            |_| panic!("gwt needs no approval"),
            &mut send,
        )
        .unwrap();
        assert_eq!(sent.state, FeedbackState::Sent);
        assert_eq!(
            sent.issue_url.as_deref(),
            Some("https://github.com/akiojin/gwt/issues/1")
        );
        process_with(
            &store,
            &record.id,
            dir.path(),
            |_| panic!("already sent"),
            &mut send,
        )
        .unwrap();
        assert_eq!(client.owner_mutation_count(), 1);
        assert_eq!(
            client.owner_mutation_call_log()[0].repository,
            RepositoryIdentity::gwt_upstream()
        );
        let published = client
            .fetch_issue(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(1),
                &ResolutionDeadline::new(Duration::from_secs(5), Duration::from_secs(120)),
            )
            .unwrap();
        assert!(!published.body.contains("exempt-secret"));
        assert_eq!(store.read(&record.id).unwrap().input, record.input);
    }

    #[test]
    fn foreign_feedback_requires_real_consent_and_preserves_declined_or_failed_reports() {
        for consent in [Ok(false), Err("No desktop display".into()), Ok(true)] {
            let (dir, store, record) = report("https://github.com/acme/app.git");
            let client = FakeIssueClient::new();
            let result = process_with(
                &store,
                &record.id,
                dir.path(),
                |prompt| {
                    assert!(prompt.contains("akiojin/gwt"));
                    assert!(prompt.contains(&record.input.refusal));
                    consent.clone()
                },
                |r| send_to_upstream(&client, r),
            )
            .unwrap();
            assert_eq!(
                client.owner_mutation_count(),
                usize::from(consent == Ok(true))
            );
            assert_eq!(store.read(&record.id).unwrap().input, record.input);
            assert_eq!(
                result.state,
                match consent {
                    Ok(true) => FeedbackState::Sent,
                    Ok(false) => FeedbackState::Declined,
                    Err(_) => FeedbackState::Failed,
                }
            );
        }
    }

    #[test]
    fn similar_closed_legacy_issue_receives_a_comment_and_its_number_is_returned() {
        let (dir, store, mut record) = report("https://github.com/akiojin/gwt.git");
        record.input.body = "PRIVATE_SETTING=duplicate-secret".into();
        let (record, fresh) = store
            .submit(record.input, record.context, record.created_at)
            .unwrap();
        assert!(fresh);
        let client = FakeIssueClient::new();
        client.seed_repository_issue(RepositoryIssue {
            repository: RepositoryIdentity::gwt_upstream(),
            number: IssueNumber(4918),
            title: "fix(launch): codex hook trust hash mismatch".into(),
            body: String::new(),
            labels: vec![],
            state: IssueState::Closed,
            kind: RepositoryIssueKind::Plain,
            updated_at: UpdatedAt("2026-10-01T00:00:00Z".into()),
        });
        let sent = process_with(
            &store,
            &record.id,
            dir.path(),
            |_| panic!("gwt needs no approval"),
            |public| send_to_upstream(&client, public),
        )
        .unwrap();
        assert_eq!(sent.issue_number, Some(4918));
        let comments = client
            .list_comments(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(4918),
                &ResolutionDeadline::new(Duration::from_secs(5), Duration::from_secs(120)),
            )
            .unwrap();
        assert_eq!(comments.items().len(), 1);
        assert_eq!(
            comments.items()[0].body,
            record.public_snapshot().render_issue()
        );
        assert!(!comments.items()[0].body.contains("duplicate-secret"));
        // The immutable request marker also reconciles a retry after an interrupted local save.
        assert_eq!(
            send_to_upstream(&client, &record.public_snapshot()).unwrap(),
            4918
        );
        assert_eq!(client.owner_mutation_count(), 1);
        assert_eq!(
            client.owner_mutation_call_log()[0].operation,
            OwnerRepositoryOperation::CreateComment
        );

        let environment = FeedbackContext {
            gwt_version: "2.0.0".into(),
            gwtd_version: "2.0.0".into(),
            os: "different-os".into(),
            ..record.context.clone()
        };
        let (additional, fresh) = store
            .submit(
                record.input.clone(),
                environment.clone(),
                record.created_at + chrono::TimeDelta::minutes(1),
            )
            .unwrap();
        assert!(fresh, "new environment must retain a distinct receipt");
        assert_ne!(additional.id, record.id);
        let additional_sent = process_with(
            &store,
            &additional.id,
            dir.path(),
            |_| panic!("gwt needs no approval"),
            |public| send_to_upstream(&client, public),
        )
        .unwrap();
        assert_eq!(additional_sent.issue_number, sent.issue_number);
        let comments = client
            .list_comments(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(4918),
                &ResolutionDeadline::new(Duration::from_secs(5), Duration::from_secs(120)),
            )
            .unwrap();
        assert_eq!(comments.items().len(), 2);
        assert_eq!(
            comments.items()[1].body,
            additional.public_snapshot().render_issue()
        );
        assert_eq!(store.read(&record.id).unwrap().input, record.input);
        let retained = store.read(&additional.id).unwrap();
        assert_eq!(retained.input, additional.input);
        assert_eq!(retained.context, environment);
        assert_eq!(
            send_to_upstream(&client, &additional.public_snapshot()).unwrap(),
            4918
        );
        assert_eq!(client.owner_mutation_count(), 2);
        assert!(client
            .owner_mutation_call_log()
            .iter()
            .all(|call| call.operation == OwnerRepositoryOperation::CreateComment));
    }

    #[test]
    fn similar_japanese_title_receives_a_comment_instead_of_a_duplicate_issue() {
        let (_dir, _store, mut record) = report("https://github.com/akiojin/gwt.git");
        record.input.title = "fix(hooks): フックの信頼検証が失敗している".into();
        let client = FakeIssueClient::new();
        client.seed_repository_issue(RepositoryIssue {
            repository: RepositoryIdentity::gwt_upstream(),
            number: IssueNumber(4918),
            title: "fix(hooks): フックの信頼検証が失敗する".into(),
            body: String::new(),
            labels: vec![],
            state: IssueState::Open,
            kind: RepositoryIssueKind::Plain,
            updated_at: UpdatedAt("2026-10-01T00:00:00Z".into()),
        });
        assert_eq!(
            send_to_upstream(&client, &record.public_snapshot()).unwrap(),
            4918
        );
        assert_eq!(client.owner_mutation_count(), 1);
        assert_eq!(
            client.owner_mutation_call_log()[0].operation,
            OwnerRepositoryOperation::CreateComment
        );
        assert!(
            similarity(
                "フックの信頼検証が失敗する",
                "設定画面からプロジェクトを選べない"
            ) < 0.8
        );
    }

    #[test]
    fn new_feedback_needs_no_label_permission_and_keeps_requested_labels_local() {
        let (_dir, store, mut record) = report("https://github.com/akiojin/gwt.git");
        record.input.labels = vec!["documentation".into()];
        store.save(&record).unwrap();
        let client = FakeIssueClient::new();
        let number = send_to_upstream(&client, &record.public_snapshot()).unwrap();
        let issue = client
            .fetch_issue(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(number),
                &ResolutionDeadline::new(Duration::from_secs(5), Duration::from_secs(120)),
            )
            .unwrap();
        assert!(issue.labels.is_empty());
        assert!(!issue.body.contains("\"requested_labels\""));
        assert!(!issue.body.contains("documentation"));
        assert_eq!(
            store.read(&record.id).unwrap().input.labels,
            record.input.labels
        );
    }

    #[test]
    fn unknown_send_outcome_is_preserved_and_never_automatically_retried() {
        let (dir, store, record) = report("https://github.com/akiojin/gwt.git");
        let client = FakeIssueClient::new();
        client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            ApiError::Network("connection lost after POST".into()),
        );
        let result = process_with(
            &store,
            &record.id,
            dir.path(),
            |_| panic!("gwt"),
            |r| send_to_upstream(&client, r),
        )
        .unwrap();
        assert_eq!(result.state, FeedbackState::Uncertain);
        assert!(result.error.unwrap().contains("akiojin/gwt"));
        process_with(
            &store,
            &record.id,
            dir.path(),
            |_| panic!("no retry"),
            |_| panic!("must reconcile first"),
        )
        .unwrap();
        assert_eq!(client.owner_mutation_count(), 1);
        assert_eq!(store.read(&record.id).unwrap().input, record.input);
    }

    #[test]
    fn upstream_dedupe_waits_for_the_host_lock_before_reading_issues() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let (_first_dir, _first_store, first) = report("https://github.com/acme/first.git");
        let (_second_dir, _second_store, second) = report("https://github.com/acme/second.git");
        let lock_dir = gwt_core::paths::gwt_home();
        std::fs::create_dir_all(&lock_dir).unwrap();
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_dir.join("feedback-upstream.lock"))
            .unwrap();
        fs2::FileExt::lock_exclusive(&lock).unwrap();
        let client = FakeIssueClient::new();
        let result = send_to_upstream_with_deadline(
            &client,
            &first.public_snapshot(),
            &ResolutionDeadline::new(Duration::from_secs(5), Duration::from_millis(100)),
        );
        assert!(
            matches!(
                result,
                Err(OwnerMutationError::PreSubmit(ApiError::Timeout { .. }))
            ),
            "a concurrent sender must finish before duplicate lookup: {result:?}"
        );
        assert!(client.owner_call_log().is_empty());
        drop(lock);

        let number = send_to_upstream(&client, &first.public_snapshot()).unwrap();
        assert_eq!(
            send_to_upstream(&client, &second.public_snapshot()).unwrap(),
            number
        );
        assert_eq!(
            client
                .owner_mutation_call_log()
                .iter()
                .filter(|call| call.operation == OwnerRepositoryOperation::CreateIssue)
                .count(),
            1
        );
    }
}
