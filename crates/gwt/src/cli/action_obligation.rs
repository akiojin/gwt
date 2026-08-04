//! Action Obligation Records (SPEC-3248 P11 core, T-239/T-240/T-242 core).
//!
//! Every producing user prompt in an execution lane — a request-form ask to
//! implement, verify, update an Issue/SPEC, or drive a PR — creates a typed
//! obligation bound to the session. Canonical operations settle matching
//! kinds (`issue.comment` / `issue.spec.edit` → issue updates, an
//! all-passing `verify.run` → verification and implementation, `pr.create`
//! / `pr.edit` / `pr.ready` → PR work, `execution.blocked` → defers all
//! open obligations with the blocker reason), and the Stop gate refuses to
//! stop while producing obligations stay open — prose, Board posts, and PR
//! body text never settle anything (T-242).
//!
//! Non-producing prompts (status questions, design questions — no request
//! form) never arm the gate: the classifier is deliberately conservative
//! (FR-168 precedent) so over-blocking cannot creep in. Prompts are stored
//! as digests only — never raw text (no-secrets convention, T-124).
//!
//! State lives beside the other trusted records: worktree mirror at
//! `.gwt/skill-state/action-obligations.json`, authoritative copy in the
//! repo-scoped trusted store (P9b), writes under the owner write lease
//! (T-149), direct edits hook-blocked (T-120 extension).
//!
//! Follow-ups (dependent): action bundle propagation into launches (T-246),
//! completion-op rejection of
//! open obligations (T-247), gate-doctor recovery obligations (T-248),
//! evidence records with HEAD/revision binding (T-241 full).

use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Worktree-relative path of the obligation state mirror.
pub const ACTION_OBLIGATION_STATE_RELATIVE: &str = ".gwt/skill-state/action-obligations.json";
const ACTION_OBLIGATION_REVIVAL_STATE_RELATIVE: &str =
    ".gwt/skill-state/action-obligation-revival.json";
const ACTION_OBLIGATION_REVIVAL_FILE: &str = "action-obligation-revival.json";

/// Producing obligation kinds (T-240). Status/design questions are
/// non-producing and never persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationKind {
    IssueUpdate,
    Implementation,
    Verification,
    Pr,
}

impl ObligationKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IssueUpdate => "issue_update",
            Self::Implementation => "implementation",
            Self::Verification => "verification",
            Self::Pr => "pr",
        }
    }
}

/// How an obligation was settled (or deferred).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObligationSettlement {
    /// The canonical operation (or `deferred: <reason>`) that settled it.
    pub evidence: String,
    pub settled_at: DateTime<Utc>,
}

/// One typed obligation created by a producing prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionObligation {
    pub kind: ObligationKind,
    /// sha256 (16 hex) of the prompt — digests only, never raw text.
    pub prompt_digest: String,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled: Option<ObligationSettlement>,
}

/// Per-worktree obligation state. One session owns a worktree at a time; a
/// new session replaces the state (intake-outcome convention).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionObligationState {
    pub session_id: String,
    pub obligations: Vec<ActionObligation>,
    /// Integrity hash (P9a convention).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ObligationRevivalOutcome {
    Revived { kinds: Vec<ObligationKind> },
    Deferred { reason: String },
    PersistFailed { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObligationRevivalRecord {
    pub session_id: String,
    pub result: ObligationRevivalOutcome,
    pub recorded_at: DateTime<Utc>,
    pub content_hash: String,
}

fn revival_record_hash(record: &ObligationRevivalRecord) -> String {
    let mut canonical = record.clone();
    canonical.content_hash.clear();
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&canonical).unwrap_or_default())
    )
}

#[cfg(test)]
std::thread_local! {
    static REVIVAL_RECORD_WRITE_FAILURES: std::cell::Cell<u8> =
        const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn set_revival_record_write_failure() {
    set_revival_record_write_failures(1);
}

#[cfg(test)]
pub(crate) fn set_revival_record_write_failures(count: u8) {
    REVIVAL_RECORD_WRITE_FAILURES.with(|slot| {
        assert!(
            slot.replace(count) == 0,
            "revival record failure injection must not be nested"
        );
    });
}

fn save_revival_record(
    worktree: &Path,
    session_id: &str,
    result: &ObligationRevivalOutcome,
) -> io::Result<()> {
    let mut record = ObligationRevivalRecord {
        session_id: session_id.to_string(),
        result: result.clone(),
        recorded_at: Utc::now(),
        content_hash: String::new(),
    };
    record.content_hash = revival_record_hash(&record);
    let serialized = serde_json::to_vec_pretty(&record)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    #[cfg(test)]
    REVIVAL_RECORD_WRITE_FAILURES.with(|slot| {
        let remaining = slot.get();
        if remaining > 0 {
            slot.set(remaining - 1);
            return Err(io::Error::other(
                "injected obligation revival record write failure",
            ));
        }
        Ok(())
    })?;
    crate::cli::trusted_store::write_with_mirror(
        worktree,
        ACTION_OBLIGATION_REVIVAL_FILE,
        &worktree.join(ACTION_OBLIGATION_REVIVAL_STATE_RELATIVE),
        &serialized,
    )
}

pub fn load_revival_record(
    worktree: &Path,
    session_id: &str,
) -> io::Result<Option<ObligationRevivalRecord>> {
    let contents = match crate::cli::trusted_store::read(worktree, ACTION_OBLIGATION_REVIVAL_FILE)?
    {
        Some(contents) => contents,
        None => return Ok(None),
    };
    let record = serde_json::from_str::<ObligationRevivalRecord>(&contents)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    if record.session_id != session_id
        || record.content_hash.is_empty()
        || record.content_hash != revival_record_hash(&record)
    {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "action obligation revival record failed identity/integrity validation",
        ));
    }
    Ok(Some(record))
}

/// Compute the integrity hash (content with the hash field emptied).
#[must_use]
pub fn compute_content_hash(state: &ActionObligationState) -> String {
    let mut canonical = state.clone();
    canonical.content_hash = String::new();
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    format!("{:x}", Sha256::digest(&bytes))
}

/// True when the stored hash matches (or the state is legacy-empty).
#[must_use]
pub fn integrity_ok(state: &ActionObligationState) -> bool {
    state.content_hash.is_empty() || state.content_hash == compute_content_hash(state)
}

/// Resolve the mirror path for a worktree.
#[must_use]
pub fn state_path(worktree: &Path) -> PathBuf {
    worktree.join(ACTION_OBLIGATION_STATE_RELATIVE)
}

/// Load the state (trusted copy authoritative, mirror fallback — P9b
/// conventions shared with the other records).
pub fn load(worktree: &Path) -> io::Result<Option<ActionObligationState>> {
    let contents = match crate::cli::trusted_store::read(worktree, "action-obligations.json")? {
        Some(contents) => contents,
        None if crate::cli::trusted_store::under_trusted_management(worktree) => return Ok(None),
        None => match fs::read_to_string(state_path(worktree)) {
            Ok(contents) => contents,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        },
    };
    let state = serde_json::from_str::<ActionObligationState>(&contents)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    Ok(Some(state))
}

/// Persist the state (trusted authoritative + mirror, fresh hash).
pub fn save(worktree: &Path, state: &ActionObligationState) -> io::Result<()> {
    let mut state = state.clone();
    state.content_hash = compute_content_hash(&state);
    let serialized = serde_json::to_vec_pretty(&state)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    crate::cli::trusted_store::write_with_mirror(
        worktree,
        "action-obligations.json",
        &state_path(worktree),
        &serialized,
    )
}

/// True when `word` (ASCII) appears with word boundaries — substring hits
/// inside longer words ("print" for "pr", "latest" for "test") must not
/// classify (P11 review fix).
fn contains_word(lower: &str, word: &str) -> bool {
    debug_assert!(word.is_ascii());
    let bytes = lower.as_bytes();
    let mut start = 0;
    while let Some(pos) = lower[start..].find(word) {
        let begin = start + pos;
        let end = begin + word.len();
        let before_ok = begin == 0 || !bytes[begin - 1].is_ascii_alphanumeric();
        let after_ok = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = end;
    }
    false
}

/// T-240 core classifier: a prompt arms an obligation only when it carries
/// an explicit WORK verb (imperative/te-form). Polite forms alone
/// ("ください", "お願い", "please", "can you") are not sufficient — polite
/// questions, courtesy closings, and explanation asks ("教えてください")
/// must never arm (P11 review fix; conservative bias: under-arming is
/// acceptable, over-blocking is not). Kind markers use word-boundary
/// matching so "print"/"previous" never read as PR work.
#[must_use]
pub fn classify_prompt(prompt: &str) -> Option<ObligationKind> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    const WORK_FORMS: &[&str] = &[
        "登録して",
        "作成して",
        "作って",
        "更新して",
        "修正して",
        "実装して",
        "追加して",
        "記録して",
        "直して",
        "検証して",
        "実行して",
        "やって",
        "進めて",
        "してほしい",
        "fix ",
        "implement ",
        "create ",
        "update ",
    ];
    if !WORK_FORMS.iter().any(|form| lower.contains(form)) {
        return None;
    }
    if lower.contains("pull request") || lower.contains("プルリク") || contains_word(&lower, "pr")
    {
        return Some(ObligationKind::Pr);
    }
    if lower.contains("検証")
        || lower.contains("テスト")
        || contains_word(&lower, "test")
        || contains_word(&lower, "tests")
        || contains_word(&lower, "verify")
    {
        return Some(ObligationKind::Verification);
    }
    if lower.contains("イシュー")
        || lower.contains("起票")
        || contains_word(&lower, "issue")
        || contains_word(&lower, "issues")
        || contains_word(&lower, "spec")
        || contains_word(&lower, "specs")
    {
        return Some(ObligationKind::IssueUpdate);
    }
    Some(ObligationKind::Implementation)
}

fn prompt_digest(prompt: &str) -> String {
    format!("{:x}", Sha256::digest(prompt.trim().as_bytes()))[..16].to_string()
}

/// A bare continuation request after an integrity-valid Completed execution
/// means "finish the handoff" rather than "start more implementation". Keep
/// this exact and contextual so explicit new implementation requests retain
/// their ordinary Implementation obligation.
fn classify_prompt_for_worktree(
    worktree: &Path,
    prompt: &str,
    kind: ObligationKind,
) -> ObligationKind {
    if kind == ObligationKind::Implementation
        && prompt.trim() == "進めて"
        && crate::cli::execution_state::is_completed(worktree)
    {
        return ObligationKind::Pr;
    }
    kind
}

fn mark_locked(
    worktree: &Path,
    session_id: &str,
    prompt: &str,
    kind: ObligationKind,
) -> io::Result<()> {
    let digest = prompt_digest(prompt);
    let mut state = match load(worktree) {
        Ok(Some(state)) if state.session_id == session_id && integrity_ok(&state) => state,
        // New session (or tampered/unreadable state) starts a fresh ledger.
        _ => ActionObligationState {
            session_id: session_id.to_string(),
            obligations: Vec::new(),
            content_hash: String::new(),
        },
    };
    // Re-submitting the same prompt refreshes the open obligation instead
    // of stacking duplicates.
    if let Some(entry) = state
        .obligations
        .iter_mut()
        .find(|entry| entry.prompt_digest == digest && entry.settled.is_none())
    {
        if entry.kind == ObligationKind::Implementation && kind == ObligationKind::Pr {
            entry.kind = ObligationKind::Pr;
            save(worktree, &state)?;
        }
        return Ok(());
    }
    state.obligations.push(ActionObligation {
        kind,
        prompt_digest: digest,
        created_at: Utc::now(),
        settled: None,
    });
    save(worktree, &state)
}

/// UserPromptSubmit entry: arm the gate for producing prompts. Same lease
/// posture as the intake marker — short bounded wait, unleased fallback,
/// because a dropped arming fails OPEN (T-149 review convention).
pub fn mark_from_prompt(worktree: &Path, session_id: &str, prompt: &str) -> io::Result<bool> {
    let Some(base_kind) = classify_prompt(prompt) else {
        return Ok(false);
    };
    match crate::cli::trusted_store::with_write_lease_wait(
        worktree,
        std::time::Duration::from_millis(300),
        || {
            let kind = classify_prompt_for_worktree(worktree, prompt, base_kind);
            mark_locked(worktree, session_id, prompt, kind)
        },
    ) {
        Err(err) if err.kind() == ErrorKind::WouldBlock => {
            // This fail-open path cannot share the lease with execution state,
            // but it must still avoid reusing a classification sampled before
            // the bounded wait.
            let kind = classify_prompt_for_worktree(worktree, prompt, base_kind);
            mark_locked(worktree, session_id, prompt, kind)?;
            Ok(true)
        }
        Err(err) => Err(err),
        Ok(()) => Ok(true),
    }
}

/// Settle every open obligation of the given kinds with the named canonical
/// evidence. Best-effort from operation success paths: a bookkeeping
/// failure must not fail the verified operation.
pub fn settle_kinds_best_effort(
    worktree: &Path,
    session_id: &str,
    kinds: &[ObligationKind],
    evidence: &str,
) {
    let result = crate::cli::trusted_store::with_write_lease(worktree, || {
        let Some(mut state) = load(worktree)? else {
            return Ok(());
        };
        if state.session_id != session_id || !integrity_ok(&state) {
            return Ok(());
        }
        let mut changed = false;
        for entry in &mut state.obligations {
            if entry.settled.is_none() && kinds.contains(&entry.kind) {
                entry.settled = Some(ObligationSettlement {
                    evidence: evidence.to_string(),
                    settled_at: Utc::now(),
                });
                changed = true;
            }
        }
        if changed {
            save(worktree, &state)?;
        }
        Ok(())
    });
    if let Err(error) = result {
        tracing::warn!(?error, "action obligation settlement failed");
    }
}

/// `execution.blocked` defers every open obligation with the blocker
/// reason — blocked is the honest terminal path (AS-26 analog).
pub fn defer_all_best_effort(worktree: &Path, session_id: &str, reason: &str) {
    settle_kinds_best_effort(
        worktree,
        session_id,
        &[
            ObligationKind::IssueUpdate,
            ObligationKind::Implementation,
            ObligationKind::Verification,
            ObligationKind::Pr,
        ],
        &format!("deferred: execution.blocked ({reason})"),
    );
}

/// T-248 absorbed core: a successful `execution.reopen` revives the
/// obligations that `execution.blocked` deferred, for the kinds the
/// recovery evidence does NOT cover (issue updates and PR work — the
/// reopen contract already proves fresh implementation/verification
/// evidence). Recovery therefore re-owes exactly the work the block
/// parked, through the existing obligation gate — no separate Gate Doctor
/// surface (user decision, 2026-07-28).
pub fn revive_deferred(
    worktree: &Path,
    session_id: &str,
    kinds: &[ObligationKind],
) -> ObligationRevivalOutcome {
    let result = crate::cli::trusted_store::with_write_lease(worktree, || {
        let Some(mut state) = load(worktree)? else {
            return Ok(ObligationRevivalOutcome::Deferred {
                reason: "obligation_state_missing".to_string(),
            });
        };
        if state.session_id != session_id {
            return Ok(ObligationRevivalOutcome::Deferred {
                reason: "obligation_session_mismatch".to_string(),
            });
        }
        if !integrity_ok(&state) {
            return Ok(ObligationRevivalOutcome::Deferred {
                reason: "obligation_integrity_failure".to_string(),
            });
        }
        let mut revived = Vec::new();
        for entry in &mut state.obligations {
            let deferred = entry
                .settled
                .as_ref()
                .is_some_and(|settlement| settlement.evidence.starts_with("deferred:"));
            if deferred && kinds.contains(&entry.kind) {
                entry.settled = None;
                if !revived.contains(&entry.kind) {
                    revived.push(entry.kind);
                }
            }
        }
        if !revived.is_empty() {
            save(worktree, &state)?;
            return Ok(ObligationRevivalOutcome::Revived { kinds: revived });
        }
        Ok(ObligationRevivalOutcome::Deferred {
            reason: "no_matching_deferred_obligation".to_string(),
        })
    });
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::warn!(?error, "deferred obligation revival failed");
            ObligationRevivalOutcome::PersistFailed {
                error: error.to_string(),
            }
        }
    };
    match save_revival_record(worktree, session_id, &outcome) {
        Ok(()) => outcome,
        Err(error) => {
            tracing::warn!(?error, "obligation revival outcome persistence failed");
            let persist_failed = ObligationRevivalOutcome::PersistFailed {
                error: format!("obligation revival outcome persistence failed: {error}"),
            };
            if let Err(fallback_error) = save_revival_record(worktree, session_id, &persist_failed)
            {
                tracing::warn!(
                    ?fallback_error,
                    "obligation revival persistence-failure audit could not be saved"
                );
            }
            persist_failed
        }
    }
}

/// T-247: completion/ready operations refuse while producing obligations
/// stay open. `excluding` lets self-settling operations skip their own
/// kind (a PR mutation settles `pr` on success).
pub fn open_obligation_refusal(
    worktree: &Path,
    session_id: &str,
    excluding: &[ObligationKind],
) -> Option<String> {
    let open: Vec<ObligationKind> = open_kinds(worktree, session_id)
        .into_iter()
        .filter(|kind| !excluding.contains(kind))
        .collect();
    if open.is_empty() {
        return None;
    }
    let kinds = open
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "open action obligations [{kinds}] from this session's prompts are unsettled (T-247). \
         Settle them first — `issue.comment` / `issue.spec.edit` for issue_update, a plan-covering \
         all-passing `verify.run` for implementation/verification, `pr.create` / `pr.edit` / `pr.ready` \
         for pr — or defer them with `execution.blocked` and a non-empty `params.reason`."
    ))
}

/// Open (unsettled) obligation kinds for the session — the Stop gate input.
pub fn open_kinds(worktree: &Path, session_id: &str) -> Vec<ObligationKind> {
    match load(worktree) {
        Ok(Some(state)) if state.session_id == session_id && integrity_ok(&state) => {
            let mut kinds: Vec<ObligationKind> = Vec::new();
            for entry in &state.obligations {
                if entry.settled.is_none() && !kinds.contains(&entry.kind) {
                    kinds.push(entry.kind);
                }
            }
            kinds
        }
        // Missing, cross-session, tampered, or unreadable state fails open
        // for the hook reader (FR-014u; tamper enforcement lives with the
        // execution-control gate).
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-240 core: request-form producing prompts arm typed kinds; status
    // and design questions never arm.
    #[test]
    fn classifier_types_producing_prompts_and_ignores_questions() {
        assert_eq!(
            classify_prompt("PR を作成してください"),
            Some(ObligationKind::Pr)
        );
        assert_eq!(
            classify_prompt("テストを実行して"),
            Some(ObligationKind::Verification)
        );
        assert_eq!(
            classify_prompt("Issue #42 にコメントを追加して"),
            Some(ObligationKind::IssueUpdate)
        );
        assert_eq!(
            classify_prompt("バグを修正して"),
            Some(ObligationKind::Implementation)
        );
        assert_eq!(
            classify_prompt("進めて"),
            Some(ObligationKind::Implementation)
        );
        assert_eq!(
            classify_prompt("please fix the login bug"),
            Some(ObligationKind::Implementation)
        );

        assert_eq!(classify_prompt("any updates?"), None);
        assert_eq!(classify_prompt("登録済みですか？"), None);
        assert_eq!(classify_prompt("この設計はなぜこうなっていますか？"), None);
        assert_eq!(classify_prompt("what is the current status"), None);
        assert_eq!(classify_prompt(""), None);

        // P11 review fixes — substring over-matching must not misroute:
        // "print"/"prompt"/"previous" are not PR work, and code-comment asks
        // are not issue updates.
        assert_eq!(
            classify_prompt("please fix the print bug"),
            Some(ObligationKind::Implementation)
        );
        assert_eq!(
            classify_prompt("update the prompt handling"),
            Some(ObligationKind::Implementation)
        );
        assert_eq!(
            classify_prompt("この関数にコメントを追加して"),
            Some(ObligationKind::Implementation)
        );
        // Polite forms alone (ください / お願い / please / can you) never arm:
        // polite questions and courtesy closings are non-producing.
        assert_eq!(classify_prompt("現状を教えてください"), None);
        assert_eq!(classify_prompt("よろしくお願いします"), None);
        assert_eq!(
            classify_prompt("can you explain why the previous run failed?"),
            None
        );
        assert_eq!(
            classify_prompt("調査の進捗はどうですか？よろしくお願いします"),
            None
        );
    }

    fn materialize_execution(
        worktree: &Path,
        owner_number: u64,
        session_id: &str,
        completed: bool,
    ) {
        crate::cli::trusted_store::init_git_repo_with_origin(worktree);
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: owner_number,
        };
        crate::cli::execution_state::materialize_at_launch(
            worktree,
            owner.kind,
            owner.number,
            session_id,
            "gwt-execute",
            false,
        )
        .expect("materialize action-obligation execution");
        crate::cli::execution_state::ensure_generation_ledger(
            worktree,
            owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("materialize action-obligation generation ledger");
        if completed {
            assert!(matches!(
                crate::cli::execution_state::settle(
                    worktree,
                    session_id,
                    crate::cli::execution_state::ExecutionSettlement::Completed,
                )
                .expect("complete action-obligation execution"),
                crate::cli::execution_state::SettleResult::Settled(_)
            ));
        }
    }

    #[test]
    fn prompt_classification_respects_completed_handoff_context() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());

        let active = tempfile::tempdir().expect("Active obligation repository");
        materialize_execution(active.path(), 3442, "session-active-handoff", false);
        mark_from_prompt(active.path(), "session-active-handoff", "進めて").unwrap();
        assert_eq!(
            open_kinds(active.path(), "session-active-handoff"),
            vec![ObligationKind::Implementation],
        );

        let completed = tempfile::tempdir().expect("Completed obligation repository");
        materialize_execution(completed.path(), 3443, "session-completed-handoff", true);
        mark_from_prompt(completed.path(), "session-completed-handoff", "進めて").unwrap();
        assert_eq!(
            open_kinds(completed.path(), "session-completed-handoff"),
            vec![ObligationKind::Pr],
            "an ambiguous continuation in Completed is PR handoff work",
        );

        mark_from_prompt(
            completed.path(),
            "session-completed-handoff",
            "バグを修正して",
        )
        .unwrap();
        assert_eq!(
            open_kinds(completed.path(), "session-completed-handoff"),
            vec![ObligationKind::Pr, ObligationKind::Implementation],
            "an explicit new implementation request must not be consumed by PR handoff",
        );

        let unmanaged = tempfile::tempdir().expect("unmanaged obligation directory");
        mark_from_prompt(unmanaged.path(), "session-unmanaged", "進めて").unwrap();
        assert_eq!(
            open_kinds(unmanaged.path(), "session-unmanaged"),
            vec![ObligationKind::Implementation],
        );
    }

    #[test]
    fn completed_handoff_classification_is_rechecked_under_the_write_lease() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let worktree = tempfile::tempdir().expect("transitioning obligation repository");
        crate::cli::trusted_store::init_git_repo_with_origin(worktree.path());
        let session_id = "session-transitioning-handoff";
        crate::cli::execution_state::materialize_at_launch(
            worktree.path(),
            crate::cli::execution_state::ExecutionOwnerKind::Issue,
            3442,
            session_id,
            "gwt-execute",
            false,
        )
        .expect("materialize Active execution before prompt classification");

        let transitioning_worktree = worktree.path().to_path_buf();
        crate::cli::trusted_store::set_write_lease_acquired_hook(move || {
            let mut execution = crate::cli::execution_state::load(&transitioning_worktree)
                .expect("load execution during classification race")
                .expect("execution exists during classification race");
            execution.status = crate::cli::execution_state::ExecutionControlStatus::Completed;
            execution.settled_at = Some(Utc::now());
            crate::cli::execution_state::save(&transitioning_worktree, &execution)
                .expect("complete flat execution while the prompt writer owns the lease");
        });

        mark_from_prompt(worktree.path(), session_id, "進めて")
            .expect("classify prompt after acquiring the write lease");
        assert_eq!(
            open_kinds(worktree.path(), session_id),
            vec![ObligationKind::Pr],
            "classification must use the execution state protected by the write lease",
        );
    }

    #[test]
    fn ambiguous_handoff_does_not_trust_tampered_completed_authority() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let worktree = tempfile::tempdir().expect("tampered Completed obligation repository");
        let session_id = "session-tampered-completed-handoff";
        materialize_execution(worktree.path(), 3442, session_id, true);

        let mut tampered = crate::cli::execution_state::load(worktree.path())
            .expect("load Completed execution before tampering")
            .expect("Completed execution exists");
        tampered.content_hash = "tampered-completed-authority".to_string();
        let trusted = crate::cli::trusted_store::trusted_dir_for_worktree(worktree.path())
            .expect("trusted worktree directory")
            .join("execution-control.json");
        std::fs::write(
            trusted,
            serde_json::to_vec_pretty(&tampered).expect("serialize tampered authority"),
        )
        .expect("tamper Completed authority fixture");

        mark_from_prompt(worktree.path(), session_id, "進めて")
            .expect("classify prompt with tampered Completed authority");
        assert_eq!(
            open_kinds(worktree.path(), session_id),
            vec![ObligationKind::Implementation],
            "only integrity-valid Completed authority may reclassify an ambiguous prompt",
        );
    }

    #[test]
    fn completed_handoff_reclassifies_same_digest_open_implementation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let completed = tempfile::tempdir().expect("Completed obligation repository");
        let session_id = "session-completed-retry";
        materialize_execution(completed.path(), 3442, session_id, true);

        mark_locked(
            completed.path(),
            session_id,
            "進めて",
            ObligationKind::Implementation,
        )
        .expect("seed pre-fix open Implementation obligation");
        mark_from_prompt(completed.path(), session_id, "進めて")
            .expect("retry Completed handoff prompt");

        let state = load(completed.path()).unwrap().unwrap();
        assert_eq!(
            state.obligations.len(),
            1,
            "retry must not stack a duplicate"
        );
        assert_eq!(
            open_kinds(completed.path(), session_id),
            vec![ObligationKind::Pr]
        );
    }

    // T-239/T-242 core: producing prompts open obligations; matching
    // canonical evidence settles them; prose settles nothing.
    #[test]
    fn obligations_open_and_settle_by_kind() {
        let dir = tempfile::tempdir().unwrap();
        assert!(mark_from_prompt(dir.path(), "sess-1", "バグを修正して").unwrap());
        assert!(mark_from_prompt(dir.path(), "sess-1", "PR を作成して").unwrap());
        // Duplicate prompt refreshes, not stacks.
        assert!(mark_from_prompt(dir.path(), "sess-1", "バグを修正して").unwrap());
        let state = load(dir.path()).unwrap().unwrap();
        assert_eq!(state.obligations.len(), 2);
        assert!(integrity_ok(&state));

        assert_eq!(
            open_kinds(dir.path(), "sess-1"),
            vec![ObligationKind::Implementation, ObligationKind::Pr]
        );
        // Cross-session reads fail open.
        assert!(open_kinds(dir.path(), "sess-other").is_empty());

        // An all-passing verify.run settles implementation, not PR.
        settle_kinds_best_effort(
            dir.path(),
            "sess-1",
            &[ObligationKind::Verification, ObligationKind::Implementation],
            "verify.run vr-1",
        );
        assert_eq!(open_kinds(dir.path(), "sess-1"), vec![ObligationKind::Pr]);

        settle_kinds_best_effort(dir.path(), "sess-1", &[ObligationKind::Pr], "pr.ready #1");
        assert!(open_kinds(dir.path(), "sess-1").is_empty());
    }

    // A new session replaces the previous session's ledger; execution.blocked
    // defers everything open.
    #[test]
    fn new_session_replaces_and_blocked_defers() {
        let dir = tempfile::tempdir().unwrap();
        mark_from_prompt(dir.path(), "sess-1", "実装して").unwrap();
        mark_from_prompt(dir.path(), "sess-2", "検証して").unwrap();
        let state = load(dir.path()).unwrap().unwrap();
        assert_eq!(state.session_id, "sess-2");
        assert_eq!(state.obligations.len(), 1);

        defer_all_best_effort(dir.path(), "sess-2", "runner unavailable");
        assert!(open_kinds(dir.path(), "sess-2").is_empty());
        let state = load(dir.path()).unwrap().unwrap();
        assert!(state.obligations[0]
            .settled
            .as_ref()
            .unwrap()
            .evidence
            .contains("deferred: execution.blocked"));
    }

    // T-247: the refusal helper lists open kinds and honors exclusions.
    #[test]
    fn open_obligation_refusal_lists_kinds_and_honors_exclusions() {
        let dir = tempfile::tempdir().unwrap();
        mark_from_prompt(dir.path(), "sess-1", "PR を作成して").unwrap();
        mark_from_prompt(dir.path(), "sess-1", "Issue #1 にコメントを追加して").unwrap();

        let refusal = open_obligation_refusal(dir.path(), "sess-1", &[]).unwrap();
        assert!(refusal.contains("pr"), "{refusal}");
        assert!(refusal.contains("issue_update"), "{refusal}");

        // A PR mutation settles its own kind — excluding it leaves only the
        // issue update in the way.
        let refusal = open_obligation_refusal(dir.path(), "sess-1", &[ObligationKind::Pr]).unwrap();
        assert!(refusal.contains("issue_update"), "{refusal}");
        assert!(!refusal.contains("[pr"), "{refusal}");

        settle_kinds_best_effort(
            dir.path(),
            "sess-1",
            &[ObligationKind::IssueUpdate],
            "issue.comment",
        );
        assert!(open_obligation_refusal(dir.path(), "sess-1", &[ObligationKind::Pr]).is_none());
    }

    // T-248 absorbed core: revival reopens only DEFERRED entries of the
    // requested kinds; evidence-settled entries and other sessions stay
    // untouched.
    #[test]
    fn revive_deferred_reopens_only_deferred_kinds() {
        // The revival record round-trips through the HOME-scoped trusted-store
        // mirror; a parallel test swapping HOME mid-test loses the record
        // (issue #3411). Hold the env lock and pin a private HOME.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        mark_from_prompt(dir.path(), "sess-1", "Issue #1 にコメントを追加して").unwrap();
        mark_from_prompt(dir.path(), "sess-1", "バグを修正して").unwrap();
        // Implementation settles with real evidence; the issue update is
        // deferred by execution.blocked.
        settle_kinds_best_effort(
            dir.path(),
            "sess-1",
            &[ObligationKind::Implementation],
            "verify.run vr-1",
        );
        settle_kinds_best_effort(
            dir.path(),
            "sess-1",
            &[ObligationKind::IssueUpdate],
            "deferred: execution.blocked (cannot comment)",
        );
        assert!(open_kinds(dir.path(), "sess-1").is_empty());

        let outcome = revive_deferred(
            dir.path(),
            "sess-1",
            &[ObligationKind::IssueUpdate, ObligationKind::Pr],
        );
        assert_eq!(
            outcome,
            ObligationRevivalOutcome::Revived {
                kinds: vec![ObligationKind::IssueUpdate]
            }
        );
        assert_eq!(
            open_kinds(dir.path(), "sess-1"),
            vec![ObligationKind::IssueUpdate],
            "only the deferred issue update revives; evidence-settled implementation stays settled"
        );

        // Cross-session revival is a no-op.
        assert_eq!(
            revive_deferred(dir.path(), "sess-other", &[ObligationKind::IssueUpdate]),
            ObligationRevivalOutcome::Deferred {
                reason: "obligation_session_mismatch".to_string()
            }
        );
        assert_eq!(
            open_kinds(dir.path(), "sess-1"),
            vec![ObligationKind::IssueUpdate]
        );
        let recorded = load_revival_record(dir.path(), "sess-other")
            .unwrap()
            .unwrap();
        assert_eq!(
            recorded.result,
            ObligationRevivalOutcome::Deferred {
                reason: "obligation_session_mismatch".to_string()
            }
        );
    }

    #[test]
    fn revive_deferred_reports_persist_failed_truthfully() {
        // Same trusted-store mirror isolation as
        // `revive_deferred_reopens_only_deferred_kinds` (issue #3411).
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        fs::create_dir_all(state_path(dir.path()).parent().unwrap()).unwrap();
        fs::write(state_path(dir.path()), "{corrupt").unwrap();

        let outcome = revive_deferred(dir.path(), "sess-corrupt", &[ObligationKind::IssueUpdate]);

        assert!(matches!(
            outcome,
            ObligationRevivalOutcome::PersistFailed { .. }
        ));
        let recorded = load_revival_record(dir.path(), "sess-corrupt")
            .unwrap()
            .unwrap();
        assert!(matches!(
            recorded.result,
            ObligationRevivalOutcome::PersistFailed { .. }
        ));
    }

    #[test]
    fn revival_record_write_failure_cannot_report_revived() {
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        mark_from_prompt(dir.path(), "sess-1", "Issue #1 にコメントを追加して").unwrap();
        settle_kinds_best_effort(
            dir.path(),
            "sess-1",
            &[ObligationKind::IssueUpdate],
            "deferred: execution.blocked (offline)",
        );
        set_revival_record_write_failure();

        let outcome = revive_deferred(dir.path(), "sess-1", &[ObligationKind::IssueUpdate]);

        assert!(
            matches!(outcome, ObligationRevivalOutcome::PersistFailed { .. }),
            "a missing durable revival outcome must never be reported as revived: {outcome:?}"
        );
        assert_eq!(
            open_kinds(dir.path(), "sess-1"),
            vec![ObligationKind::IssueUpdate],
            "the state mutation may be visible, but the caller receives a truthful persistence failure"
        );
        let record = load_revival_record(dir.path(), "sess-1")
            .unwrap()
            .expect("the retry records the persistence failure for execution.status");
        assert!(matches!(
            record.result,
            ObligationRevivalOutcome::PersistFailed { .. }
        ));
    }

    // No-secrets: raw prompts never persist, only digests.
    #[test]
    fn prompts_persist_as_digests_only() {
        let dir = tempfile::tempdir().unwrap();
        mark_from_prompt(dir.path(), "sess-1", "token ghp_SECRET123 を使って修正して").unwrap();
        let raw = fs::read_to_string(state_path(dir.path())).unwrap();
        assert!(!raw.contains("ghp_SECRET123"), "{raw}");
    }
}
