use std::{
    fs,
    path::{Path, PathBuf},
};

use chrono::{Local, NaiveDate, Utc};
use gwt_core::{
    coordination::{
        post_entry_idempotent, AuthorKind, BoardEntryDraft, BoardEntryDraftError, BoardEntryKind,
        BoardOrigin, PostEntryIdempotentError,
    },
    recovery::{
        BoardMilestoneIntent, RecoveryAttachmentRef, RecoveryRecord, RecoverySessionKind,
        RecoveryStore, RecoveryStoreError, SemanticCheckpoint, VisibleDiscussionItem,
    },
};
use gwt_github::{client::ApiError, SpecOpsError};
use sha2::{Digest, Sha256};

use super::{CliEnv, CliParseError};

const DEFAULT_DISCUSSIONS_HEADER: &str = "# Discussions\n\nThis file is the canonical gwt discussion log. Entries are updated in place while active and indexed by the `discussions` semantic scope.\n";
const MAX_CHECKPOINT_VISIBLE_ITEMS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscussionCommand {
    Update(DiscussionUpdateCommand),
    IntakeCheckpointShow,
    IntakeCheckpointUpdate(IntakeCheckpointUpdateCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscussionUpdateCommand {
    pub date: Option<String>,
    pub title: String,
    pub status: String,
    pub topics: Vec<String>,
    pub related_specs: Vec<u64>,
    pub related_works: Vec<String>,
    pub promoted_to: Vec<String>,
    pub summary: String,
    pub decisions: Vec<String>,
    pub open_questions: Vec<String>,
    pub next: String,
}

/// Session-scoped, complete-replacement Intake checkpoint update.
///
/// Unlike [`DiscussionUpdateCommand`], this operation exposes the Recovery
/// Store CAS revision explicitly and never writes arbitrary transcript data to
/// work-notes. Only the allowlisted, completed visible items parsed by the JSON
/// envelope are accepted here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntakeCheckpointUpdateCommand {
    pub expected_revision: u64,
    pub title: String,
    pub related_specs: Vec<u64>,
    pub summary: String,
    pub decisions: Vec<String>,
    pub open_questions: Vec<String>,
    pub next: String,
    pub visible_items: Vec<VisibleDiscussionItem>,
    /// Content-addressed references copied by an earlier checkpoint and
    /// explicitly retained by a complete-replacement CAS update.
    pub retained_attachment_refs: Vec<RecoveryAttachmentRef>,
    pub attachment_paths: Vec<PathBuf>,
}

pub fn parse(args: &[String]) -> Result<DiscussionCommand, CliParseError> {
    let (head, rest) = args.split_first().ok_or(CliParseError::Usage)?;
    match head.as_str() {
        "update" => parse_update(rest).map(DiscussionCommand::Update),
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

fn parse_update(args: &[String]) -> Result<DiscussionUpdateCommand, CliParseError> {
    let mut date = None;
    let mut title = None;
    let mut status = Some("active".to_string());
    let mut topics = Vec::new();
    let mut related_specs = Vec::new();
    let mut related_works = Vec::new();
    let mut promoted_to = Vec::new();
    let mut summary = None;
    let mut decisions = Vec::new();
    let mut open_questions = Vec::new();
    let mut next = None;
    let mut i = 0;

    while i < args.len() {
        let flag = args[i].as_str();
        let value = args
            .get(i + 1)
            .ok_or(CliParseError::MissingFlag(flag_name(flag)?))?;
        match flag {
            "--date" => date = Some(valid_date(value)?),
            "--title" => title = Some(non_empty("--title", value)?),
            "--status" => status = Some(valid_status(value)?),
            "--topic" => topics.push(non_empty("--topic", value)?),
            "--related-spec" => related_specs.push(parse_spec(value)?),
            "--related-work" => related_works.push(non_empty("--related-work", value)?),
            "--promoted-to" => promoted_to.push(non_empty("--promoted-to", value)?),
            "--summary" => summary = Some(non_empty("--summary", value)?),
            "--decision" => decisions.push(non_empty("--decision", value)?),
            "--open-question" => open_questions.push(non_empty("--open-question", value)?),
            "--next" => next = Some(non_empty("--next", value)?),
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 2;
    }

    Ok(DiscussionUpdateCommand {
        date,
        title: title.ok_or(CliParseError::MissingFlag("--title"))?,
        status: status.unwrap_or_else(|| "active".to_string()),
        topics,
        related_specs,
        related_works,
        promoted_to,
        summary: summary.ok_or(CliParseError::MissingFlag("--summary"))?,
        decisions,
        open_questions,
        next: next.ok_or(CliParseError::MissingFlag("--next"))?,
    })
}

fn flag_name(flag: &str) -> Result<&'static str, CliParseError> {
    match flag {
        "--date" => Ok("--date"),
        "--title" => Ok("--title"),
        "--status" => Ok("--status"),
        "--topic" => Ok("--topic"),
        "--related-spec" => Ok("--related-spec"),
        "--related-work" => Ok("--related-work"),
        "--promoted-to" => Ok("--promoted-to"),
        "--summary" => Ok("--summary"),
        "--decision" => Ok("--decision"),
        "--open-question" => Ok("--open-question"),
        "--next" => Ok("--next"),
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

fn non_empty(flag: &'static str, value: &str) -> Result<String, CliParseError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CliParseError::InvalidValue {
            flag,
            reason: "must not be empty",
        });
    }
    Ok(trimmed.to_string())
}

fn valid_date(value: &str) -> Result<String, CliParseError> {
    let value = non_empty("--date", value)?;
    NaiveDate::parse_from_str(&value, "%Y-%m-%d")
        .map(|_| value)
        .map_err(|_| CliParseError::InvalidValue {
            flag: "--date",
            reason: "must be YYYY-MM-DD",
        })
}

pub(crate) fn valid_status(value: &str) -> Result<String, CliParseError> {
    let value = non_empty("--status", value)?;
    match value.as_str() {
        "active" | "suspended" | "completed" | "promoted" => Ok(value),
        _ => Err(CliParseError::InvalidValue {
            flag: "--status",
            reason: "must be active, suspended, completed, or promoted",
        }),
    }
}

fn parse_spec(value: &str) -> Result<u64, CliParseError> {
    let value = non_empty("--related-spec", value)?;
    value
        .trim_start_matches('#')
        .trim_start_matches("SPEC-")
        .trim_start_matches("spec-")
        .parse::<u64>()
        .map_err(|_| CliParseError::InvalidNumber(value))
}

pub fn run<E: CliEnv>(
    env: &mut E,
    command: DiscussionCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match command {
        DiscussionCommand::Update(update) => {
            // Resolve and authorize a managed Intake before touching either
            // projection. A child/ambiguous caller must not leave a memo that
            // looks durable while its Recovery checkpoint was rejected.
            let current_intake = resolve_current_intake(env.repo_path(), false)
                .map_err(discussion_recovery_as_spec_error)?;
            if let Some(current) = current_intake.as_ref() {
                require_root_provider_caller(&current.record)
                    .map_err(discussion_recovery_as_spec_error)?;
            }
            let projection = current_intake
                .as_ref()
                .map(|current| discussion_projection_plan(&current.record, &update));
            let path = update_discussion_entry(env.repo_path(), &update, projection.as_ref())
                .map_err(io_as_spec_error)?;
            out.push_str(&format!("discussion updated: {}\n", path.display()));
            if let (Some(current), Some(projection)) = (current_intake, projection) {
                project_discussion_update(env.repo_path(), current, &update, &projection, out)
                    .map_err(discussion_recovery_as_spec_error)?;
            }
            Ok(0)
        }
        DiscussionCommand::IntakeCheckpointShow => run_intake_checkpoint_show(env, out),
        DiscussionCommand::IntakeCheckpointUpdate(update) => {
            run_intake_checkpoint_update(env, &update, out)
        }
    }
}

/// Imports legacy discussion sources (repo-local `.gwt/work/discussions.md`,
/// `tasks/discussions.md`) into the machine-local home work-notes file when
/// it does not yet exist. Idempotent — returns `Ok(true)` only when an
/// import happened.
///
/// SPEC-3214 (FR-007): the discussion log moved out of the git-tracked
/// repo-local `.gwt/work/` directory into the branch-independent home
/// scratch (`~/.gwt/projects/<repo-hash>/work-notes/`).
pub fn migrate_legacy_discussions_file(repo_root: &Path) -> std::io::Result<bool> {
    crate::work_notes::migrate_discussions_into_home(repo_root)
}

fn update_discussion_entry(
    repo_root: &Path,
    update: &DiscussionUpdateCommand,
    projection: Option<&DiscussionProjectionPlan>,
) -> std::io::Result<PathBuf> {
    let path = gwt_core::paths::gwt_work_notes_discussions_path(repo_root);
    crate::work_notes::with_work_notes_lock(repo_root, || {
        crate::work_notes::migrate_discussions_into_home(repo_root)?;
        ensure_discussions_file(&path)?;

        let mut content = fs::read_to_string(&path)?;
        let date = update
            .date
            .clone()
            .unwrap_or_else(|| Local::now().format("%Y-%m-%d").to_string());
        let heading = format!("## {date} — {}", update.title);
        let entry = format_discussion_entry(&date, update, projection);
        content = replace_or_append_section(&content, &heading, &entry);
        if let Some(projection) = projection {
            content = upsert_current_intake_operation_marker(&content, projection);
        }
        fs::write(&path, content)
    })?;
    Ok(path)
}

fn ensure_discussions_file(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    fs::write(path, DEFAULT_DISCUSSIONS_HEADER)
}

fn format_discussion_entry(
    date: &str,
    update: &DiscussionUpdateCommand,
    projection: Option<&DiscussionProjectionPlan>,
) -> String {
    let related_specs = if update.related_specs.is_empty() {
        String::new()
    } else {
        update
            .related_specs
            .iter()
            .map(|number| format!("#{number}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let checkpoint_operation = projection
        .map(|projection| {
            format!(
                "Checkpoint Operation: {}\n",
                projection.checkpoint_operation_id
            )
        })
        .unwrap_or_default();
    format!(
        "## {date} — {title}\n\nStatus: {status}\nTopics: {topics}\nRelated SPECs: {related_specs}\nRelated Works: {related_works}\nPromoted To: {promoted_to}\n{checkpoint_operation}\nSummary:\n{summary}\n\nDecisions:\n{decisions}\n\nOpen Questions:\n{open_questions}\n\nNext:\n{next}\n",
        title = update.title,
        status = update.status,
        topics = update.topics.join(", "),
        related_specs = related_specs,
        related_works = update.related_works.join(", "),
        promoted_to = update.promoted_to.join(", "),
        summary = update.summary,
        decisions = format_bullets(&update.decisions),
        open_questions = format_bullets(&update.open_questions),
        next = update.next,
    )
}

fn format_bullets(items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn replace_or_append_section(content: &str, heading: &str, entry: &str) -> String {
    let mut ranges = Vec::new();
    let mut offset = 0;
    for line in content.split_inclusive('\n') {
        if line.trim_end() == heading {
            ranges.push(offset);
        }
        offset += line.len();
    }
    let Some(start) = ranges.first().copied() else {
        let mut output = content.trim_end().to_string();
        output.push_str("\n\n");
        output.push_str(entry.trim_end());
        output.push('\n');
        return output;
    };
    let tail = &content[start + heading.len()..];
    let next = tail
        .find("\n## ")
        .map(|index| start + heading.len() + index + 1)
        .unwrap_or(content.len());
    let mut output = String::new();
    output.push_str(content[..start].trim_end());
    output.push_str("\n\n");
    output.push_str(entry.trim_end());
    output.push('\n');
    output.push_str(content[next..].trim_start_matches('\n'));
    output
}

fn current_intake_marker_key(recovery_id: &str) -> String {
    let digest = Sha256::digest(recovery_id.as_bytes());
    hex::encode(&digest[..12])
}

fn current_intake_marker_prefix(recovery_id: &str) -> String {
    format!(
        "<!-- gwt-intake-current {} ",
        current_intake_marker_key(recovery_id)
    )
}

fn current_intake_marker_line(projection: &DiscussionProjectionPlan) -> String {
    format!(
        "{}{} -->",
        current_intake_marker_prefix(&projection.recovery_id),
        projection.checkpoint_operation_id
    )
}

/// Update the current per-Recovery operation pointer in the canonical memo.
/// The pointer and its section-level `Checkpoint Operation` field are written
/// in the same locked file write, so a later Stop can detect a checkpoint that
/// did not cross the memo -> Recovery crash boundary.
fn upsert_current_intake_operation_marker(
    content: &str,
    projection: &DiscussionProjectionPlan,
) -> String {
    let marker_prefix = current_intake_marker_prefix(&projection.recovery_id);
    let marker_line = current_intake_marker_line(projection);
    let mut cleaned = content
        .lines()
        .filter(|line| !line.starts_with(&marker_prefix))
        .collect::<Vec<_>>()
        .join("\n");
    if content.ends_with('\n') {
        cleaned.push('\n');
    }
    let insertion = cleaned.find("\n## ").unwrap_or(cleaned.len());
    let header = cleaned[..insertion].trim_end();
    let discussions = cleaned[insertion..].trim_start_matches('\n').trim_end();
    let mut output = String::new();
    if !header.is_empty() {
        output.push_str(header);
        output.push_str("\n\n");
    }
    output.push_str(&marker_line);
    if !discussions.is_empty() {
        output.push_str("\n\n");
        output.push_str(discussions);
    }
    output.push('\n');
    output
}

fn current_memo_checkpoint_operation(
    repo_root: &Path,
    recovery_id: &str,
) -> Result<Option<String>, String> {
    let path = gwt_core::paths::gwt_work_notes_discussions_path(repo_root);
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read canonical discussion memo: {error}")),
    };
    let marker_prefix = current_intake_marker_prefix(recovery_id);
    let mut operations = content
        .lines()
        .filter_map(|line| {
            line.strip_prefix(&marker_prefix)
                .and_then(|tail| tail.strip_suffix(" -->"))
                .map(str::trim)
                .filter(|operation| {
                    !operation.is_empty() && !operation.contains(char::is_whitespace)
                })
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    operations.sort();
    operations.dedup();
    let operation = match operations.as_slice() {
        [] => return Ok(None),
        [operation] => operation.clone(),
        _ => {
            return Err(
                "canonical discussion memo contains conflicting current operation markers"
                    .to_string(),
            )
        }
    };
    let section_marker = format!("Checkpoint Operation: {operation}");
    if !content.lines().any(|line| line == section_marker) {
        return Err(
            "canonical discussion memo current marker has no matching structured section"
                .to_string(),
        );
    }
    Ok(Some(operation))
}

fn io_as_spec_error(err: std::io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum DiscussionRecoveryError {
    #[error(transparent)]
    Recovery(#[from] RecoveryStoreError),
    #[error(transparent)]
    BoardDraft(#[from] BoardEntryDraftError),
    #[error(transparent)]
    BoardPost(#[from] PostEntryIdempotentError),
    #[error(
        "recovery {recovery_id} has no authoritative provider root and remains in Attention; confirm the recorded candidate in Recovery Center (gwt will not backfill private transcript or Board text)"
    )]
    MissingProviderRoot { recovery_id: String },
    #[error("{name} is required to prove the root provider caller for recovery {recovery_id}")]
    MissingProviderIdentity {
        recovery_id: String,
        name: &'static str,
    },
    #[error(
        "recovery {recovery_id} provider root is {expected}, not caller provider identity {actual}"
    )]
    ProviderIdentityMismatch {
        recovery_id: String,
        expected: String,
        actual: String,
    },
    #[error(
        "recovery {recovery_id} root-owned checkpoint rejected caller role {role}; unproven roots remain in Recovery Center Attention and are never backfilled from private content"
    )]
    ProviderCallerRoleRejected { recovery_id: String, role: String },
    #[error("recovery {recovery_id} provider {provider} has no trusted root caller identity")]
    UnsupportedProviderIdentity {
        recovery_id: String,
        provider: String,
    },
    #[error("{name} is required for a session-scoped Intake checkpoint update")]
    MissingSessionEnvironment { name: &'static str },
    #[error("invalid current Session ledger identity {session_id:?}")]
    InvalidSessionIdentity { session_id: String },
    #[error(
        "current Session {session_id} has no readable ledger entry; Recovery remains in Attention and private discussion content was not backfilled"
    )]
    MissingSessionLedger { session_id: String },
    #[error(
        "current Session ledger {session_id} contains owner {actual}; Recovery remains in Attention"
    )]
    SessionLedgerMismatch { session_id: String, actual: String },
    #[error(
        "current Session {session_id} cannot be resolved to exactly one Intake recovery; Recovery remains in Attention: {reason}"
    )]
    LegacyIntakeResolution { session_id: String, reason: String },
    #[error(
        "recovery {recovery_id} discussion projection changed concurrently: {reason}; retry discussion.update so memo and checkpoint converge"
    )]
    ConcurrentDiscussionProjection { recovery_id: String, reason: String },
    #[error("recovery {recovery_id} belongs to session {expected}, not caller session {actual}")]
    SessionMismatch {
        recovery_id: String,
        expected: String,
        actual: String,
    },
    #[error("recovery {recovery_id} is not an Intake session")]
    NotIntake { recovery_id: String },
    #[error(
        "recovery {recovery_id} cannot retain attachment {content_id} because it is not in the current checkpoint"
    )]
    UnknownRetainedAttachment {
        recovery_id: String,
        content_id: String,
    },
    #[error(
        "recovery {recovery_id} Board outbox remains pending: selected {provider} provider cannot preserve caller-supplied Board entry IDs for idempotent crash replay"
    )]
    UnsupportedBoardOutboxProvider {
        recovery_id: String,
        provider: String,
    },
}

#[derive(Debug, Clone)]
struct CurrentIntakeRecovery {
    store: RecoveryStore,
    record: RecoveryRecord,
    caller_session: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscussionProjectionPlan {
    recovery_id: String,
    expected_checkpoint_revision: u64,
    expected_as_of_turn_id: Option<String>,
    title: String,
    body: String,
    /// Recovery-scoped digest of every structured public milestone field.
    /// This is both the deterministic Board id and the memo/checkpoint link.
    checkpoint_operation_id: String,
}

/// Resolve the current managed Intake without looking at provider transcript
/// or Board text. For a pre-upgrade Session that lacks `GWT_RECOVERY_ID`, the
/// exact Session ledger gates the existing bounded legacy metadata importer.
///
/// `required=false` preserves standalone `discussion.update` and Execution
/// behavior when no Intake environment is present. Once either managed
/// identity is present, however, an ambiguous/missing Intake owner fails
/// closed instead of silently producing a non-recoverable discussion memo.
fn resolve_current_intake(
    repo_root: &Path,
    required: bool,
) -> Result<Option<CurrentIntakeRecovery>, DiscussionRecoveryError> {
    let caller_session = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let requested_recovery = std::env::var(gwt_agent::GWT_RECOVERY_ID_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if caller_session.is_none() && requested_recovery.is_none() && !required {
        return Ok(None);
    }
    let caller_session =
        caller_session.ok_or(DiscussionRecoveryError::MissingSessionEnvironment {
            name: gwt_agent::GWT_SESSION_ID_ENV,
        })?;
    let project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(repo_root);
    let store = RecoveryStore::for_project_dir(project_dir);

    if let Some(recovery_id) = requested_recovery {
        let record = store
            .load(&recovery_id)?
            .ok_or_else(|| RecoveryStoreError::NotFound(recovery_id.clone()))?;
        if record.session_id != caller_session {
            return Err(DiscussionRecoveryError::SessionMismatch {
                recovery_id: record.recovery_id,
                expected: record.session_id,
                actual: caller_session,
            });
        }
        if record.session_kind != RecoverySessionKind::Intake {
            if required {
                return Err(DiscussionRecoveryError::NotIntake {
                    recovery_id: record.recovery_id,
                });
            }
            return Ok(None);
        }
        return Ok(Some(CurrentIntakeRecovery {
            store,
            record,
            caller_session,
        }));
    }

    validate_session_ledger_identity(&caller_session)?;
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let session_path = sessions_dir.join(format!("{caller_session}.toml"));
    let session = gwt_agent::Session::load(&session_path).map_err(|_| {
        DiscussionRecoveryError::MissingSessionLedger {
            session_id: caller_session.clone(),
        }
    })?;
    if session.id != caller_session {
        return Err(DiscussionRecoveryError::SessionLedgerMismatch {
            session_id: caller_session,
            actual: session.id,
        });
    }
    if !legacy_session_is_intake_candidate(&session) {
        if required {
            return Err(DiscussionRecoveryError::LegacyIntakeResolution {
                session_id: caller_session,
                reason: "the Session ledger does not prove the Intake lane".to_string(),
            });
        }
        return Ok(None);
    }

    let expected_repo_id = gwt_core::paths::project_scope_hash(repo_root).to_string();
    let existing = session
        .recovery_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|recovery_id| store.load(recovery_id).ok().flatten());
    if existing.is_none() {
        // This importer scans only metadata with global byte/file/entry
        // budgets. The exact current Session was checked above before the
        // scan is permitted, so arbitrary sessions cannot trigger a private
        // history reconstruction path.
        let report =
            gwt_agent::import_legacy_recovery_sessions(&sessions_dir, &store, &expected_repo_id);
        let current_failed = report
            .errors
            .iter()
            .any(|error| error.session_id.as_deref() == Some(caller_session.as_str()));
        if current_failed {
            return Err(DiscussionRecoveryError::LegacyIntakeResolution {
                session_id: caller_session,
                reason: "the bounded metadata import could not validate the Session owner"
                    .to_string(),
            });
        }
    }

    let synced = gwt_agent::Session::load(&session_path).map_err(|_| {
        DiscussionRecoveryError::MissingSessionLedger {
            session_id: caller_session.clone(),
        }
    })?;
    if synced.id != caller_session {
        return Err(DiscussionRecoveryError::SessionLedgerMismatch {
            session_id: caller_session,
            actual: synced.id,
        });
    }
    let mut candidates = store
        .list()?
        .into_iter()
        .filter(|record| {
            record.session_id == caller_session
                && record.repo_id == expected_repo_id
                && record.session_kind == RecoverySessionKind::Intake
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.recovery_id.cmp(&right.recovery_id));
    candidates.dedup_by(|left, right| left.recovery_id == right.recovery_id);
    if candidates.len() != 1 {
        return Err(DiscussionRecoveryError::LegacyIntakeResolution {
            session_id: caller_session,
            reason: format!(
                "bounded metadata import found {} matching candidates; choose the authoritative candidate in Recovery Center",
                candidates.len()
            ),
        });
    }
    let record = candidates.pop().expect("one candidate checked above");
    Ok(Some(CurrentIntakeRecovery {
        store,
        record,
        caller_session,
    }))
}

fn validate_session_ledger_identity(session_id: &str) -> Result<(), DiscussionRecoveryError> {
    let valid = !session_id.is_empty()
        && session_id.len() <= 160
        && session_id != "."
        && session_id != ".."
        && session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(DiscussionRecoveryError::InvalidSessionIdentity {
            session_id: session_id.to_string(),
        })
    }
}

fn legacy_session_is_intake_candidate(session: &gwt_agent::Session) -> bool {
    session.session_kind == Some(gwt_skills::SessionKind::Intake)
        || session.is_ephemeral
        || session
            .worktree_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == ".intake" || name.starts_with(".intake-"))
}

fn discussion_projection_plan(
    record: &RecoveryRecord,
    update: &DiscussionUpdateCommand,
) -> DiscussionProjectionPlan {
    let title = format!("Intake checkpoint: {}", update.title);
    let body = format_board_milestone(update);
    let checkpoint_operation_id = discussion_board_entry_id(&record.recovery_id, &title, &body);
    DiscussionProjectionPlan {
        recovery_id: record.recovery_id.clone(),
        expected_checkpoint_revision: record.checkpoint_revision,
        expected_as_of_turn_id: record
            .latest_root_input
            .as_ref()
            .map(|input| input.turn_id.clone()),
        title,
        body,
        checkpoint_operation_id,
    }
}

fn project_discussion_update(
    repo_root: &Path,
    current: CurrentIntakeRecovery,
    update: &DiscussionUpdateCommand,
    projection: &DiscussionProjectionPlan,
    out: &mut String,
) -> Result<(), DiscussionRecoveryError> {
    let CurrentIntakeRecovery {
        store,
        record,
        caller_session,
    } = current;
    let mut delivery_error = flush_recovery_board_outbox(repo_root, &store, &record.recovery_id)
        .err()
        .map(|error| error.to_string());
    let record = store
        .load(&record.recovery_id)?
        .ok_or_else(|| RecoveryStoreError::NotFound(record.recovery_id.clone()))?;
    let record = require_intake_session_owner(record, &caller_session)?;
    require_root_provider_caller(&record)?;
    let root = authoritative_root(&record)?;
    if record.recovery_id != projection.recovery_id {
        return Err(DiscussionRecoveryError::LegacyIntakeResolution {
            session_id: caller_session,
            reason: "the Recovery owner changed after the discussion memo write".to_string(),
        });
    }
    let as_of_turn_id = record
        .latest_root_input
        .as_ref()
        .map(|input| input.turn_id.clone());
    if as_of_turn_id != projection.expected_as_of_turn_id {
        return Err(DiscussionRecoveryError::ConcurrentDiscussionProjection {
            recovery_id: record.recovery_id,
            reason: "the latest root turn changed after the discussion memo write".to_string(),
        });
    }

    if discussion_projection_matches(
        &record,
        update,
        &projection.checkpoint_operation_id,
        as_of_turn_id.as_deref(),
    ) {
        append_checkpoint_delivery_outcome(out, &record, delivery_error.as_deref());
        return Ok(());
    }

    if record.checkpoint_revision != projection.expected_checkpoint_revision {
        return Err(RecoveryStoreError::RevisionMismatch {
            expected: projection.expected_checkpoint_revision,
            actual: record.checkpoint_revision,
        }
        .into());
    }

    let queued_at = existing_board_intent(&record, &projection.checkpoint_operation_id)
        .map(|intent| intent.queued_at)
        .unwrap_or_else(Utc::now);
    let mut visible_items = record
        .checkpoint
        .as_ref()
        .map(|checkpoint| {
            checkpoint
                .visible_items
                .iter()
                .filter(|item| item.kind != "discussion_checkpoint")
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if visible_items.len() >= MAX_CHECKPOINT_VISIBLE_ITEMS {
        let remove = visible_items
            .len()
            .saturating_sub(MAX_CHECKPOINT_VISIBLE_ITEMS - 1);
        visible_items.drain(..remove);
    }
    visible_items.push(VisibleDiscussionItem {
        role: "assistant".to_string(),
        kind: "discussion_checkpoint".to_string(),
        text: projection.body.clone(),
        partial: false,
    });
    let attachment_refs = record
        .checkpoint
        .as_ref()
        .map(|checkpoint| checkpoint.attachment_refs.clone())
        .unwrap_or_default();
    let checkpoint = SemanticCheckpoint {
        summary: update.summary.clone(),
        confirmed_decisions: update.decisions.clone(),
        open_questions: update.open_questions.clone(),
        next_action: Some(update.next.clone()),
        as_of_turn_id,
        visible_items,
        attachment_refs,
        board_intents: vec![BoardMilestoneIntent {
            entry_id: projection.checkpoint_operation_id.clone(),
            title: projection.title.clone(),
            body: projection.body.clone(),
            queued_at,
        }],
    };
    let updated = store.replace_checkpoint(
        &record.recovery_id,
        &root.root_id,
        projection.expected_checkpoint_revision,
        checkpoint,
        format!(
            "discussion-checkpoint-{}-{}",
            projection.expected_checkpoint_revision.saturating_add(1),
            projection.checkpoint_operation_id
        ),
    )?;
    if let Err(error) = flush_recovery_board_outbox(repo_root, &store, &updated.recovery_id) {
        delivery_error = Some(error.to_string());
    }
    let current = store
        .load(&updated.recovery_id)?
        .ok_or_else(|| RecoveryStoreError::NotFound(updated.recovery_id.clone()))?;
    append_checkpoint_delivery_outcome(out, &current, delivery_error.as_deref());
    Ok(())
}

fn discussion_projection_matches(
    record: &RecoveryRecord,
    update: &DiscussionUpdateCommand,
    entry_id: &str,
    as_of_turn_id: Option<&str>,
) -> bool {
    let Some(checkpoint) = record.checkpoint.as_ref() else {
        return false;
    };
    checkpoint.summary == update.summary
        && checkpoint.confirmed_decisions == update.decisions
        && checkpoint.open_questions == update.open_questions
        && checkpoint.next_action.as_deref() == Some(update.next.as_str())
        && checkpoint.as_of_turn_id.as_deref() == as_of_turn_id
        && checkpoint
            .board_intents
            .iter()
            .any(|intent| intent.entry_id == entry_id)
}

fn existing_board_intent<'a>(
    record: &'a RecoveryRecord,
    entry_id: &str,
) -> Option<&'a BoardMilestoneIntent> {
    record
        .checkpoint
        .as_ref()
        .and_then(|checkpoint| {
            checkpoint
                .board_intents
                .iter()
                .find(|intent| intent.entry_id == entry_id)
        })
        .or_else(|| {
            record
                .board_outbox
                .iter()
                .find(|intent| intent.entry_id == entry_id)
        })
}

fn authoritative_root(
    record: &RecoveryRecord,
) -> Result<&gwt_core::recovery::ProviderRootBinding, DiscussionRecoveryError> {
    record
        .provider_root
        .as_ref()
        .filter(|binding| binding.quality.is_authoritative())
        .ok_or_else(|| DiscussionRecoveryError::MissingProviderRoot {
            recovery_id: record.recovery_id.clone(),
        })
}

fn append_checkpoint_delivery_outcome(
    out: &mut String,
    record: &RecoveryRecord,
    fallback_delivery_error: Option<&str>,
) {
    let delivery_error = record
        .board_delivery_error
        .as_deref()
        .or(fallback_delivery_error)
        .map(|error| {
            error
                .chars()
                .take(gwt_core::recovery::MAX_BOARD_DELIVERY_ERROR_CHARS)
                .collect::<String>()
        });
    out.push_str(&format!(
        "intake checkpoint durable: revision {} board_pending={} board_pending_count={}",
        record.checkpoint_revision,
        !record.board_outbox.is_empty(),
        record.board_outbox.len()
    ));
    if let Some(error) = delivery_error {
        out.push_str(" delivery_error=");
        out.push_str(
            &serde_json::to_string(&error)
                .unwrap_or_else(|_| "\"Board delivery failed\"".to_string()),
        );
    }
    out.push('\n');
}

fn run_intake_checkpoint_update<E: CliEnv>(
    env: &mut E,
    update: &IntakeCheckpointUpdateCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let current = resolve_current_intake(env.repo_path(), true)
        .map_err(discussion_recovery_as_spec_error)?
        .expect("required current Intake resolution");
    let CurrentIntakeRecovery {
        store,
        record,
        caller_session,
    } = current;
    require_root_provider_caller(&record).map_err(discussion_recovery_as_spec_error)?;

    // A previous invocation may have committed the checkpoint but crashed or
    // returned while publishing its Board milestone. Opportunistically drain
    // that durable outbox before evaluating the caller's checkpoint CAS. Board
    // delivery is deliberately non-blocking: a provider outage or a provider
    // without caller-ID idempotency remains visible as pending evidence while
    // the semantic checkpoint revision is still allowed to advance.
    let mut delivery_error =
        flush_recovery_board_outbox(env.repo_path(), &store, &record.recovery_id)
            .err()
            .map(|error| error.to_string());
    let record = store
        .load(&record.recovery_id)
        .map_err(DiscussionRecoveryError::from)
        .map_err(discussion_recovery_as_spec_error)?
        .ok_or_else(|| RecoveryStoreError::NotFound(record.recovery_id.clone()))
        .map_err(DiscussionRecoveryError::from)
        .map_err(discussion_recovery_as_spec_error)?;
    let record = require_intake_session_owner(record, &caller_session)
        .map_err(discussion_recovery_as_spec_error)?;
    require_root_provider_caller(&record).map_err(discussion_recovery_as_spec_error)?;
    let is_same_revision_retry = match record.checkpoint_revision {
        actual if actual == update.expected_revision => false,
        actual if actual == update.expected_revision.saturating_add(1) => true,
        actual => {
            return Err(discussion_recovery_as_spec_error(
                RecoveryStoreError::RevisionMismatch {
                    expected: update.expected_revision,
                    actual,
                }
                .into(),
            ));
        }
    };
    let attachment_paths = update
        .attachment_paths
        .iter()
        .map(|source| {
            if source.is_absolute() {
                source.clone()
            } else {
                env.repo_path().join(source)
            }
        })
        .collect::<Vec<_>>();
    let current_attachment_refs = record
        .checkpoint
        .as_ref()
        .map(|checkpoint| checkpoint.attachment_refs.as_slice())
        .unwrap_or_default();
    let mut retained_attachment_refs = Vec::new();
    for attachment in &update.retained_attachment_refs {
        if !current_attachment_refs.contains(attachment) {
            return Err(discussion_recovery_as_spec_error(
                DiscussionRecoveryError::UnknownRetainedAttachment {
                    recovery_id: record.recovery_id.clone(),
                    content_id: attachment.content_id.clone(),
                },
            ));
        }
        store
            .verify_attachment(attachment)
            .map_err(DiscussionRecoveryError::from)
            .map_err(discussion_recovery_as_spec_error)?;
        if !retained_attachment_refs.contains(attachment) {
            retained_attachment_refs.push(attachment.clone());
        }
    }
    let root = authoritative_root(&record).map_err(discussion_recovery_as_spec_error)?;
    let requested_title = format!("Intake checkpoint: {}", update.title);
    let existing_structured_intent = record
        .checkpoint
        .as_ref()
        .filter(|checkpoint| checkpoint_matches_explicit_update(checkpoint, update))
        .and_then(|checkpoint| {
            checkpoint.board_intents.iter().find(|intent| {
                intent.title == requested_title
                    && board_intent_related_specs_match(intent, &update.related_specs)
            })
        });
    // `intake.checkpoint.update` supplements automatic discussion durability
    // with attachments/completed visible items. Reuse the automatic intent
    // verbatim when the structured fields match, including its terminal
    // status, so the supplemental CAS cannot create a second Board milestone.
    let (title, body, entry_id, queued_at) = if let Some(intent) = existing_structured_intent {
        (
            intent.title.clone(),
            intent.body.clone(),
            intent.entry_id.clone(),
            intent.queued_at,
        )
    } else {
        let body = format_intake_checkpoint_board_milestone(update);
        let entry_id = discussion_board_entry_id(&record.recovery_id, &requested_title, &body);
        let queued_at = is_same_revision_retry
            .then(|| existing_board_intent(&record, &entry_id).map(|intent| intent.queued_at))
            .flatten()
            .unwrap_or_else(Utc::now);
        (requested_title, body, entry_id, queued_at)
    };
    let mut visible_items = update.visible_items.clone();
    if visible_items.is_empty() {
        visible_items.push(VisibleDiscussionItem {
            role: "assistant".to_string(),
            kind: "discussion_checkpoint".to_string(),
            text: body.clone(),
            partial: false,
        });
    }
    let checkpoint = SemanticCheckpoint {
        summary: update.summary.clone(),
        confirmed_decisions: update.decisions.clone(),
        open_questions: update.open_questions.clone(),
        next_action: Some(update.next.clone()),
        as_of_turn_id: record
            .latest_root_input
            .as_ref()
            .map(|input| input.turn_id.clone()),
        visible_items,
        attachment_refs: retained_attachment_refs,
        board_intents: vec![BoardMilestoneIntent {
            entry_id: entry_id.clone(),
            title,
            body,
            queued_at,
        }],
    };
    store
        .replace_checkpoint_with_attachments(
            &record.recovery_id,
            &root.root_id,
            update.expected_revision,
            checkpoint,
            &attachment_paths,
            format!(
                "intake-checkpoint-{}-{entry_id}",
                update.expected_revision.saturating_add(1)
            ),
        )
        .map_err(DiscussionRecoveryError::from)
        .map_err(discussion_recovery_as_spec_error)?;
    if let Err(error) = flush_recovery_board_outbox(env.repo_path(), &store, &record.recovery_id) {
        delivery_error = Some(error.to_string());
    }
    let current = store
        .load(&record.recovery_id)
        .map_err(DiscussionRecoveryError::from)
        .map_err(discussion_recovery_as_spec_error)?
        .ok_or_else(|| RecoveryStoreError::NotFound(record.recovery_id.clone()))
        .map_err(DiscussionRecoveryError::from)
        .map_err(discussion_recovery_as_spec_error)?;
    let board_pending = !current.board_outbox.is_empty();
    let delivery_error = current
        .board_delivery_error
        .or(delivery_error)
        .map(|error| {
            error
                .chars()
                .take(gwt_core::recovery::MAX_BOARD_DELIVERY_ERROR_CHARS)
                .collect::<String>()
        });
    out.push_str(&format!(
        "intake checkpoint updated: revision {} board_pending={} board_pending_count={}",
        current.checkpoint_revision,
        board_pending,
        current.board_outbox.len()
    ));
    if let Some(error) = delivery_error {
        out.push_str(" delivery_error=");
        out.push_str(
            &serde_json::to_string(&error)
                .unwrap_or_else(|_| "\"Board delivery failed\"".to_string()),
        );
    }
    out.push('\n');
    Ok(0)
}

fn run_intake_checkpoint_show<E: CliEnv>(
    env: &mut E,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let current = resolve_current_intake(env.repo_path(), true)
        .map_err(discussion_recovery_as_spec_error)?
        .expect("required current Intake resolution");
    require_intake_session_owner(current.record, &current.caller_session)
        .and_then(|record| {
            require_root_provider_caller(&record)?;
            Ok(record)
        })
        .map_err(discussion_recovery_as_spec_error)
        .map(|record| {
            let checkpoint = record.checkpoint.as_ref().map(|checkpoint| {
                serde_json::json!({
                    "summary": checkpoint.summary,
                    "confirmed_decisions": checkpoint.confirmed_decisions,
                    "open_questions": checkpoint.open_questions,
                    "next_action": checkpoint.next_action,
                    "as_of_turn_id": checkpoint.as_of_turn_id,
                    "visible_items": checkpoint.visible_items,
                    "attachment_refs": checkpoint.attachment_refs,
                })
            });
            let state = serde_json::json!({
                "recovery_id": record.recovery_id,
                "revision": record.checkpoint_revision,
                "coverage": record.checkpoint_coverage,
                "checkpoint": checkpoint,
                "latest_root_input": record.latest_root_input,
                "board_pending": !record.board_outbox.is_empty(),
                "board_pending_count": record.board_outbox.len(),
            });
            out.push_str(&format!(
                "intake checkpoint current: recovery_id={} revision={} coverage={:?} state={}\n",
                record.recovery_id,
                record.checkpoint_revision,
                record.checkpoint_coverage,
                serde_json::to_string(&state)
                    .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".to_string())
            ));
            0
        })
}

fn require_intake_session_owner(
    record: RecoveryRecord,
    caller_session: &str,
) -> Result<RecoveryRecord, DiscussionRecoveryError> {
    if record.session_id != caller_session {
        return Err(DiscussionRecoveryError::SessionMismatch {
            recovery_id: record.recovery_id,
            expected: record.session_id,
            actual: caller_session.to_string(),
        });
    }
    if record.session_kind != gwt_core::recovery::RecoverySessionKind::Intake {
        return Err(DiscussionRecoveryError::NotIntake {
            recovery_id: record.recovery_id,
        });
    }
    Ok(record)
}

fn require_root_provider_caller(record: &RecoveryRecord) -> Result<(), DiscussionRecoveryError> {
    if record.root_role != gwt_core::recovery::ProviderRootRole::Root {
        return Err(DiscussionRecoveryError::ProviderCallerRoleRejected {
            recovery_id: record.recovery_id.clone(),
            role: format!("{:?}", record.root_role),
        });
    }
    let root = record
        .provider_root
        .as_ref()
        .filter(|binding| binding.quality.is_authoritative())
        .ok_or_else(|| DiscussionRecoveryError::MissingProviderRoot {
            recovery_id: record.recovery_id.clone(),
        })?;
    let provider = record.provider.to_ascii_lowercase();
    if provider.contains("codex") {
        let identity_env = "CODEX_THREAD_ID";
        let caller_identity = required_provider_identity(identity_env, record)?;
        if caller_identity != root.root_id {
            return Err(DiscussionRecoveryError::ProviderIdentityMismatch {
                recovery_id: record.recovery_id.clone(),
                expected: root.root_id.clone(),
                actual: format!("{identity_env}:{caller_identity}"),
            });
        }
    } else if provider.contains("claude") {
        // Claude exposes the provider session id to hooks, but does not
        // guarantee a session-id environment variable in Bash subprocesses.
        // Root/child authority is therefore enforced at the managed
        // PreToolUse boundary (which carries agent_id/agent_type for a
        // subagent), while this CLI independently requires the Store's
        // authoritative root binding and root role above.
        if claude_subagent_environment_present() {
            return Err(DiscussionRecoveryError::ProviderCallerRoleRejected {
                recovery_id: record.recovery_id.clone(),
                role: "claude_subagent".to_string(),
            });
        }
    } else {
        return Err(DiscussionRecoveryError::UnsupportedProviderIdentity {
            recovery_id: record.recovery_id.clone(),
            provider: record.provider.clone(),
        });
    }
    Ok(())
}

/// Read-only Stop/phase-boundary guard for the current managed Intake.
///
/// It never derives a checkpoint from the hook payload and never posts Board.
/// A crash turn whose latest root input has not crossed a structured
/// durability boundary therefore stays recoverably incomplete instead of
/// being guessed into a public milestone.
pub(crate) fn current_intake_durability_blocker(
    repo_root: &Path,
) -> Result<Option<String>, DiscussionRecoveryError> {
    let Some(current) = resolve_current_intake(repo_root, false)? else {
        return Ok(None);
    };
    require_root_provider_caller(&current.record)?;
    let record = current.record;
    let Some(checkpoint) = record.checkpoint.as_ref() else {
        return Ok(Some(format!(
            "Current Intake {} has no semantic discussion checkpoint. Run the JSON operation `discussion.update` with the completed public-safe summary, decisions, open questions, and next action before ending this phase. The Stop hook will not infer or post the incomplete turn.",
            record.recovery_id
        )));
    };
    let structurally_complete = !checkpoint.summary.trim().is_empty()
        && checkpoint
            .next_action
            .as_deref()
            .is_some_and(|next| !next.trim().is_empty())
        && !checkpoint.board_intents.is_empty();
    if !structurally_complete {
        return Ok(Some(format!(
            "Current Intake {} has an incomplete structured checkpoint. Run `discussion.update` before ending this phase; attachments or partial visible items alone are not a discussion durability boundary, and no Board post will be guessed from them.",
            record.recovery_id
        )));
    }
    let memo_operation = match current_memo_checkpoint_operation(repo_root, &record.recovery_id) {
        Ok(Some(operation)) => operation,
        Ok(None) => {
            return Ok(Some(format!(
                "Current Intake {} has no canonical memo/checkpoint operation marker. Run `discussion.update` before ending this phase; an older checkpoint cannot prove that the latest memo crossed the Recovery durability boundary.",
                record.recovery_id
            )))
        }
        Err(reason) => {
            return Ok(Some(format!(
                "Current Intake {} has an invalid canonical memo/checkpoint operation marker: {reason}. Retry `discussion.update`; the Stop hook will not treat a partially written memo as durable.",
                record.recovery_id
            )))
        }
    };
    if !checkpoint
        .board_intents
        .iter()
        .any(|intent| intent.entry_id == memo_operation)
    {
        return Ok(Some(format!(
            "Current Intake {} memo/checkpoint operation mismatch: memo {} is newer or different from the durable Recovery checkpoint. Retry `discussion.update` so both projections converge; the Stop hook will not guess-post the incomplete milestone.",
            record.recovery_id, memo_operation
        )));
    }
    if let Some(latest) = record.latest_root_input.as_ref() {
        if checkpoint.as_of_turn_id.as_deref() != Some(latest.turn_id.as_str()) {
            return Ok(Some(format!(
                "Current Intake {} has an uncheckpointed root turn {}. Run `discussion.update` after applying that turn before ending this phase. The Stop hook will not reconstruct private input or guess-post an incomplete Board milestone.",
                record.recovery_id, latest.turn_id
            )));
        }
    }
    Ok(None)
}

fn required_provider_identity(
    name: &'static str,
    record: &RecoveryRecord,
) -> Result<String, DiscussionRecoveryError> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DiscussionRecoveryError::MissingProviderIdentity {
            recovery_id: record.recovery_id.clone(),
            name,
        })
}

fn claude_subagent_environment_present() -> bool {
    ["CLAUDE_CODE_AGENT_ID", "CLAUDE_AGENT_ID"]
        .iter()
        .any(|name| {
            std::env::var(name)
                .ok()
                .is_some_and(|value| !value.trim().is_empty())
        })
        || std::env::var("CLAUDE_CODE_IS_SIDECHAIN")
            .ok()
            .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "True"))
}

/// Replay pending Board posts after a crash. The Board append is idempotent by
/// entry ID; the recovery acknowledgement happens only after that append is
/// confirmed, so a crash at either boundary is safe to retry.
pub(crate) fn flush_recovery_board_outbox(
    repo_root: &Path,
    store: &RecoveryStore,
    recovery_id: &str,
) -> Result<(), DiscussionRecoveryError> {
    let record = store
        .load(recovery_id)?
        .ok_or_else(|| RecoveryStoreError::NotFound(recovery_id.to_string()))?;
    if record.board_outbox.is_empty() {
        if record.board_delivery_error.is_some() {
            store.set_board_delivery_error(recovery_id, None)?;
        }
        return Ok(());
    }
    let routing = crate::board_provider::routing_for(repo_root);
    if routing.provider != "local" {
        // Slack and Teams currently allocate their own post IDs. A retry
        // after remote success but before the recovery ack could therefore
        // create a duplicate. Keep the durable intent pending until the
        // provider contract supports caller-supplied idempotency keys.
        let error = DiscussionRecoveryError::UnsupportedBoardOutboxProvider {
            recovery_id: recovery_id.to_string(),
            provider: routing.provider,
        };
        return Err(record_board_delivery_error(store, recovery_id, error));
    }
    for intent in record.board_outbox.clone() {
        let mut draft = BoardEntryDraft::new(
            AuthorKind::Agent,
            "gwt-discussion",
            BoardEntryKind::Status,
            &intent.body,
        );
        draft.title = Some(intent.title.clone());
        draft.origin = board_origin_for_recovery(&record);
        let mut entry = match draft.finalize() {
            Ok(entry) => entry,
            Err(error) => {
                return Err(record_board_delivery_error(
                    store,
                    recovery_id,
                    error.into(),
                ));
            }
        };
        entry.id = intent.entry_id.clone();
        let snapshot = match post_entry_idempotent(repo_root, entry) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Err(record_board_delivery_error(
                    store,
                    recovery_id,
                    error.into(),
                ));
            }
        };
        super::board::publish_board_change(repo_root, snapshot.board.entries.len());
        if let Err(error) = store.ack_board_entry(
            recovery_id,
            &intent.entry_id,
            format!("board-ack-{}", intent.entry_id),
        ) {
            return Err(record_board_delivery_error(
                store,
                recovery_id,
                error.into(),
            ));
        }
    }
    Ok(())
}

fn record_board_delivery_error(
    store: &RecoveryStore,
    recovery_id: &str,
    error: DiscussionRecoveryError,
) -> DiscussionRecoveryError {
    let message = error.to_string();
    if let Err(store_error) = store.set_board_delivery_error(recovery_id, Some(&message)) {
        return store_error.into();
    }
    error
}

fn board_origin_for_recovery(record: &RecoveryRecord) -> BoardOrigin {
    BoardOrigin::new("", &record.session_id, &record.provider)
        .with_session_kind(match record.session_kind {
            gwt_core::recovery::RecoverySessionKind::Intake => "intake",
            gwt_core::recovery::RecoverySessionKind::Execution => "execution",
        })
        .with_recovery_id(&record.recovery_id)
}

fn format_board_milestone(update: &DiscussionUpdateCommand) -> String {
    let decisions = format_bullets(&update.decisions);
    let open_questions = format_bullets(&update.open_questions);
    let related_specs = update
        .related_specs
        .iter()
        .map(|number| format!("#{number}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Status: {status}\nTopics: {topics}\nRelated SPECs: {related_specs}\nRelated Works: {related_works}\nPromoted To: {promoted_to}\n\nSummary:\n{summary}\n\nDecisions:\n{decisions}\n\nOpen Questions:\n{open_questions}\n\nNext:\n{next}",
        status = update.status,
        topics = update.topics.join(", "),
        related_works = update.related_works.join(", "),
        promoted_to = update.promoted_to.join(", "),
        summary = update.summary,
        next = update.next,
    )
}

fn format_intake_checkpoint_board_milestone(update: &IntakeCheckpointUpdateCommand) -> String {
    let projection = DiscussionUpdateCommand {
        date: None,
        title: update.title.clone(),
        status: "active".to_string(),
        topics: Vec::new(),
        related_specs: update.related_specs.clone(),
        related_works: Vec::new(),
        promoted_to: Vec::new(),
        summary: update.summary.clone(),
        decisions: update.decisions.clone(),
        open_questions: update.open_questions.clone(),
        next: update.next.clone(),
    };
    format_board_milestone(&projection)
}

fn checkpoint_matches_explicit_update(
    checkpoint: &SemanticCheckpoint,
    update: &IntakeCheckpointUpdateCommand,
) -> bool {
    checkpoint.summary == update.summary
        && checkpoint.confirmed_decisions == update.decisions
        && checkpoint.open_questions == update.open_questions
        && checkpoint.next_action.as_deref() == Some(update.next.as_str())
}

fn board_intent_related_specs_match(intent: &BoardMilestoneIntent, related_specs: &[u64]) -> bool {
    let expected = related_specs
        .iter()
        .map(|number| format!("#{number}"))
        .collect::<Vec<_>>()
        .join(", ");
    intent
        .body
        .lines()
        .find_map(|line| line.strip_prefix("Related SPECs: "))
        == Some(expected.as_str())
}

fn discussion_board_entry_id(recovery_id: &str, title: &str, body: &str) -> String {
    let digest = Sha256::digest(format!("{recovery_id}\0{title}\0{body}").as_bytes());
    format!("discussion-{}", &hex::encode(digest)[..24])
}

fn discussion_recovery_as_spec_error(err: DiscussionRecoveryError) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

#[cfg(test)]
mod durability_boundary_tests {
    use super::*;
    use gwt_core::{
        recovery::{BindingQuality, CreateRecovery, ProviderRootBinding, RecoverySessionKind},
        test_support::ScopedEnvVar,
    };

    const RECOVERY_ID: &str = "discussion-memo-boundary";
    const SESSION_ID: &str = "discussion-memo-session";
    const ROOT_ID: &str = "discussion-memo-root";

    fn update(title: &str, summary: &str) -> DiscussionUpdateCommand {
        DiscussionUpdateCommand {
            date: Some("2026-07-16".to_string()),
            title: title.to_string(),
            status: "active".to_string(),
            topics: vec!["intake".to_string(), "durability".to_string()],
            related_specs: vec![3214],
            related_works: Vec::new(),
            promoted_to: Vec::new(),
            summary: summary.to_string(),
            decisions: vec!["Keep memo and Recovery linked by one digest.".to_string()],
            open_questions: Vec::new(),
            next: "Cross the next durability boundary.".to_string(),
        }
    }

    fn create_store(repo: &Path, home: &Path) -> RecoveryStore {
        let repo_id = gwt_core::paths::project_scope_hash(repo).to_string();
        let store =
            RecoveryStore::for_project_dir(home.join(".gwt").join("projects").join(&repo_id));
        store
            .create(
                CreateRecovery {
                    recovery_id: RECOVERY_ID.to_string(),
                    session_id: SESSION_ID.to_string(),
                    repo_id,
                    session_kind: RecoverySessionKind::Intake,
                    worktree_path: repo.to_path_buf(),
                    launch_base_ref: None,
                    launch_base_oid: "c".repeat(40),
                    launch_head_oid: "c".repeat(40),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "test memo/checkpoint boundary".to_string(),
                    created_at: Utc::now(),
                },
                "discussion-memo-create",
            )
            .unwrap();
        store
            .bind_root(
                RECOVERY_ID,
                ProviderRootBinding {
                    root_id: ROOT_ID.to_string(),
                    session_tree_id: None,
                    quality: BindingQuality::Verified,
                    bound_at: Utc::now(),
                },
                "discussion-memo-bind",
            )
            .unwrap();
        store
            .record_root_input(
                RECOVERY_ID,
                ROOT_ID,
                "turn-memo-boundary",
                "private input is only a staleness boundary",
                "discussion-memo-root-input",
            )
            .unwrap();
        store
    }

    fn current(store: &RecoveryStore) -> CurrentIntakeRecovery {
        CurrentIntakeRecovery {
            store: store.clone(),
            record: store.load(RECOVERY_ID).unwrap().unwrap(),
            caller_session: SESSION_ID.to_string(),
        }
    }

    fn project(
        repo: &Path,
        current: CurrentIntakeRecovery,
        update: &DiscussionUpdateCommand,
        plan: &DiscussionProjectionPlan,
    ) -> Result<(), DiscussionRecoveryError> {
        let mut output = String::new();
        project_discussion_update(repo, current, update, plan, &mut output)
    }

    #[test]
    fn stop_blocks_crash_after_memo_before_checkpoint_until_retry_converges() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let home = temp.path().join("home");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&home).unwrap();
        let _home = ScopedEnvVar::set("HOME", &home);
        let _userprofile = ScopedEnvVar::set("USERPROFILE", &home);
        let _recovery = ScopedEnvVar::set(gwt_agent::GWT_RECOVERY_ID_ENV, RECOVERY_ID);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, SESSION_ID);
        let _provider = ScopedEnvVar::set("CODEX_THREAD_ID", ROOT_ID);
        let store = create_store(&repo, &home);

        let a = update("Boundary A", "Checkpoint A is durable.");
        let current_a = current(&store);
        let plan_a = discussion_projection_plan(&current_a.record, &a);
        update_discussion_entry(&repo, &a, Some(&plan_a)).unwrap();
        project(&repo, current_a, &a, &plan_a).unwrap();
        assert_eq!(current_intake_durability_blocker(&repo).unwrap(), None);

        let b = update("Boundary B", "Memo B reached disk before checkpoint B.");
        let current_b = current(&store);
        let plan_b = discussion_projection_plan(&current_b.record, &b);
        update_discussion_entry(&repo, &b, Some(&plan_b)).unwrap();

        let blocked = current_intake_durability_blocker(&repo)
            .unwrap()
            .expect("memo-only crash must block Stop");
        assert!(
            blocked.contains("memo/checkpoint operation mismatch"),
            "{blocked}"
        );
        let crashed = store.load(RECOVERY_ID).unwrap().unwrap();
        assert_eq!(crashed.checkpoint_revision, 1);
        assert_eq!(crashed.board_entry_ids.len(), 1);

        project(&repo, current_b, &b, &plan_b).unwrap();
        assert_eq!(current_intake_durability_blocker(&repo).unwrap(), None);
        let converged = store.load(RECOVERY_ID).unwrap().unwrap();
        assert_eq!(converged.checkpoint_revision, 2);
        assert_eq!(converged.board_entry_ids.len(), 2);
    }

    #[test]
    fn concurrent_projection_fails_stale_cas_and_retry_is_idempotent() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let home = temp.path().join("home");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&home).unwrap();
        let _home = ScopedEnvVar::set("HOME", &home);
        let _userprofile = ScopedEnvVar::set("USERPROFILE", &home);
        let _recovery = ScopedEnvVar::set(gwt_agent::GWT_RECOVERY_ID_ENV, RECOVERY_ID);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, SESSION_ID);
        let _provider = ScopedEnvVar::set("CODEX_THREAD_ID", ROOT_ID);
        let store = create_store(&repo, &home);

        let b = update("Concurrent B", "Concurrent checkpoint B wins the CAS.");
        let c = update("Concurrent C", "Concurrent checkpoint C must retry.");
        let stale_b = current(&store);
        let stale_c = stale_b.clone();
        let plan_b = discussion_projection_plan(&stale_b.record, &b);
        let stale_plan_c = discussion_projection_plan(&stale_c.record, &c);

        update_discussion_entry(&repo, &b, Some(&plan_b)).unwrap();
        project(&repo, stale_b, &b, &plan_b).unwrap();
        update_discussion_entry(&repo, &c, Some(&stale_plan_c)).unwrap();
        let error = project(&repo, stale_c, &c, &stale_plan_c)
            .expect_err("concurrent stale projection must fail CAS");
        assert!(
            matches!(
                error,
                DiscussionRecoveryError::Recovery(RecoveryStoreError::RevisionMismatch {
                    expected: 0,
                    actual: 1
                })
            ),
            "{error}"
        );
        let blocked = current_intake_durability_blocker(&repo)
            .unwrap()
            .expect("losing memo must remain visibly incomplete");
        assert!(
            blocked.contains("memo/checkpoint operation mismatch"),
            "{blocked}"
        );

        let fresh_c = current(&store);
        let retry_plan_c = discussion_projection_plan(&fresh_c.record, &c);
        update_discussion_entry(&repo, &c, Some(&retry_plan_c)).unwrap();
        project(&repo, fresh_c, &c, &retry_plan_c).unwrap();
        assert_eq!(current_intake_durability_blocker(&repo).unwrap(), None);
        let after_retry = store.load(RECOVERY_ID).unwrap().unwrap();
        assert_eq!(after_retry.checkpoint_revision, 2);
        assert_eq!(after_retry.board_entry_ids.len(), 2);

        let exact_retry = current(&store);
        let exact_retry_plan = discussion_projection_plan(&exact_retry.record, &c);
        update_discussion_entry(&repo, &c, Some(&exact_retry_plan)).unwrap();
        project(&repo, exact_retry, &c, &exact_retry_plan).unwrap();
        let idempotent = store.load(RECOVERY_ID).unwrap().unwrap();
        assert_eq!(idempotent.checkpoint_revision, 2);
        assert_eq!(idempotent.board_entry_ids.len(), 2);
    }
}
