use gwt_core::feedback::{
    FeedbackContext, FeedbackInput, FeedbackKind, FeedbackRecord, FeedbackState, FeedbackStore,
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

pub fn run<E: super::CliEnv>(
    env: &mut E,
    command: FeedbackCommand,
    out: &mut String,
) -> Result<i32, gwt_github::SpecOpsError> {
    let root = env.repo_path().to_path_buf();
    let store =
        FeedbackStore::new(gwt_core::paths::gwt_project_dir_for_repo_path(&root).join("feedback"));
    let result = (|| -> io::Result<FeedbackRecord> {
        match command {
            FeedbackCommand::Status { id } => store.read(&id),
            FeedbackCommand::Process { id } => process_with(
                &store,
                &id,
                &root,
                |prompt| {
                    crate::provider_reset::native::confirm_with(
                        "GWT — Send feedback to akiojin/gwt",
                        "Send feedback",
                        prompt,
                    )
                },
                |record| env.deliver_feedback(record),
            ),
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
    let mut child = gwt_core::process::hidden_command(std::env::current_exe()?)
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
    mut send: impl FnMut(&FeedbackRecord) -> Result<u64, OwnerMutationError>,
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
        let prompt = format!("Send this report to the public GitHub repository akiojin/gwt?\nPrivacy masks are applied below; the complete original stays local.\nCancel leaves it saved locally and your project work can continue.\n\nTitle: {}\n\n{}", public.input.title, public.render_issue());
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
    record: &FeedbackRecord,
) -> Result<u64, OwnerMutationError> {
    send_to_upstream_with_deadline(
        client,
        record,
        &ResolutionDeadline::new(Duration::from_secs(5), Duration::from_secs(120)),
    )
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
    record: &FeedbackRecord,
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
        let mut send = |r: &FeedbackRecord| send_to_upstream(&client, r);
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
        store.save(&record).unwrap();
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
        assert!(!comments.items()[0].body.contains("duplicate-secret"));
        // The immutable request marker also reconciles a retry after an interrupted local save.
        assert_eq!(send_to_upstream(&client, &record).unwrap(), 4918);
        assert_eq!(client.owner_mutation_count(), 1);
        assert_eq!(
            client.owner_mutation_call_log()[0].operation,
            OwnerRepositoryOperation::CreateComment
        );
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
        assert_eq!(send_to_upstream(&client, &record).unwrap(), 4918);
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
    fn new_feedback_needs_no_label_permission_and_preserves_requested_labels() {
        let (_dir, _store, mut record) = report("https://github.com/akiojin/gwt.git");
        record.input.labels = vec!["documentation".into()];
        let client = FakeIssueClient::new();
        let number = send_to_upstream(&client, &record).unwrap();
        let issue = client
            .fetch_issue(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(number),
                &ResolutionDeadline::new(Duration::from_secs(5), Duration::from_secs(120)),
            )
            .unwrap();
        assert!(issue.labels.is_empty());
        assert!(issue.body.contains("\"requested_labels\""));
        assert!(issue.body.contains("documentation"));
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
            &first,
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

        let number = send_to_upstream(&client, &first).unwrap();
        assert_eq!(send_to_upstream(&client, &second).unwrap(), number);
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
