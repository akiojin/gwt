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
        match auto_policy(&root) {
            Ok(false) => {}
            policy => {
                let report = SafeAutoReport::current(KnownOperation::classify(&input.operation));
                return run_auto_create(env, &store, report, policy.err(), out);
            }
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

fn auto_policy(root: &Path) -> Result<bool, AutoReason> {
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
    let value = serde_json::json!({
        "request_id": id, "mode": "auto", "state": record.state,
        "issue_number": record.issue_number,
        "issue_url": record.issue_number.map(|number| format!("https://github.com/akiojin/gwt/issues/{number}")),
        "receipt": receipt, "error": record.error,
    });
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
    withheld: Option<AutoReason>,
    out: &mut String,
) -> Result<i32, gwt_github::SpecOpsError> {
    let now = chrono::Utc::now();
    let submitted = match withheld {
        Some(reason) => store.submit_auto_withheld(report, now, reason),
        None => store.submit_auto(report, now),
    };
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

fn reconcile_auto_status(store: &FeedbackStore, id: &str) -> io::Result<AutoFeedbackRecord> {
    let _lock = match store.try_sender_lock(id) {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return store.read_auto(id),
        Err(error) => return Err(error),
    };
    let mut record = store.read_auto(id)?;
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
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return store.read_auto(id),
        Err(error) => return Err(error),
    };
    let mut record = store.read_auto(id)?;
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
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return store.read(id),
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
    require_auto(&|| auto_policy(root))?;
    let _lock = acquire_upstream_lock(deadline).map_err(auto_send_error)?;
    require_auto(&|| auto_policy(root))?;
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
            require_auto(&|| auto_policy(root))?;
            client
                .create_owner_comment(&repository, issue.number, &body, deadline)
                .map_err(auto_send_error)?;
        }
        Ok(issue.number.0)
    } else {
        require_auto(&|| auto_policy(root))?;
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
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
            {
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
        let backup = storage.with_file_name("feedback-before-save-failure");
        assert!(process_auto_with(
            &store,
            &record.snapshot.id(),
            || Ok(true),
            |snapshot| {
                assert_eq!(
                    store.read_auto(&snapshot.id()).unwrap().state,
                    FeedbackState::Sending
                );
                std::fs::rename(&storage, &backup).unwrap();
                std::fs::write(&storage, "blocked storage").unwrap();
                Ok(42)
            },
        )
        .is_err());
        std::fs::remove_file(&storage).unwrap();
        std::fs::rename(&backup, &storage).unwrap();
        let record = store.read_auto(&record.snapshot.id()).unwrap();
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
        assert_eq!(receipt["state"], "failed");
        assert_eq!(receipt["error"], "worker_unavailable");
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
        let mut created = String::new();
        run(
            &mut env,
            FeedbackCommand::Create(Box::new(sample.input)),
            &mut created,
        )
        .unwrap();
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
        run(
            &mut env,
            FeedbackCommand::Create(Box::new(sample.input.clone())),
            &mut output,
        )
        .unwrap();
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
        run(
            &mut env,
            FeedbackCommand::Retry {
                id: receipt["request_id"].as_str().unwrap().into(),
            },
            &mut String::new(),
        )
        .unwrap();
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
