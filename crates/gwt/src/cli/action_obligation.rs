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
//! Follow-ups (dependent): assistant commitment scanner (T-243), action
//! bundle propagation into launches (T-246), completion-op rejection of
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
    if state
        .obligations
        .iter()
        .any(|entry| entry.prompt_digest == digest && entry.settled.is_none())
    {
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
    let Some(kind) = classify_prompt(prompt) else {
        return Ok(false);
    };
    match crate::cli::trusted_store::with_write_lease_wait(
        worktree,
        std::time::Duration::from_millis(300),
        || mark_locked(worktree, session_id, prompt, kind),
    ) {
        Err(err) if err.kind() == ErrorKind::WouldBlock => {
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
pub fn revive_deferred_best_effort(worktree: &Path, session_id: &str, kinds: &[ObligationKind]) {
    let result = crate::cli::trusted_store::with_write_lease(worktree, || {
        let Some(mut state) = load(worktree)? else {
            return Ok(());
        };
        if state.session_id != session_id || !integrity_ok(&state) {
            return Ok(());
        }
        let mut changed = false;
        for entry in &mut state.obligations {
            let deferred = entry
                .settled
                .as_ref()
                .is_some_and(|settlement| settlement.evidence.starts_with("deferred:"));
            if deferred && kinds.contains(&entry.kind) {
                entry.settled = None;
                changed = true;
            }
        }
        if changed {
            save(worktree, &state)?;
        }
        Ok(())
    });
    if let Err(error) = result {
        tracing::warn!(?error, "deferred obligation revival failed");
    }
}

/// T-243 core: classify assertive completion claims in assistant prose.
/// Deliberately narrow — only implemented/fixed and verified/tests-pass
/// claim forms (ja+en). PR/issue mentions are excluded: they appear in
/// historical summaries far too often to classify safely (T-243 full).
#[must_use]
pub fn classify_commitments(text: &str) -> Vec<ObligationKind> {
    let lower = text.to_lowercase();
    let mut kinds = Vec::new();
    const IMPLEMENTED_CLAIMS: &[&str] = &[
        "実装しました",
        "実装済み",
        "実装完了",
        "修正しました",
        "修正済み",
        "対応済み",
        "implemented",
        "fixed the",
        "has been fixed",
    ];
    const VERIFIED_CLAIMS: &[&str] = &[
        "検証しました",
        "検証済み",
        "テストは全て",
        "全テスト成功",
        "verified",
        "all tests pass",
        "tests pass",
        "tests are green",
    ];
    if IMPLEMENTED_CLAIMS.iter().any(|claim| lower.contains(claim)) {
        kinds.push(ObligationKind::Implementation);
    }
    if VERIFIED_CLAIMS.iter().any(|claim| lower.contains(claim)) {
        kinds.push(ObligationKind::Verification);
    }
    kinds
}

/// T-243 core: turn UNBACKED completion claims into open obligations. A
/// claim is backed when the session's ledger holds ANY entry of that kind
/// (open entries already block; settled entries prove the canonical
/// evidence ran). Sessions without a ledger are skipped entirely — the
/// scanner never invents context (fail-open for pre-P11 flows). Returns
/// the kinds it newly opened.
pub fn mark_unbacked_commitments(
    worktree: &Path,
    session_id: &str,
    claimed: &[ObligationKind],
) -> io::Result<Vec<ObligationKind>> {
    if claimed.is_empty() {
        return Ok(Vec::new());
    }
    let Some(state) = load(worktree)? else {
        return Ok(Vec::new());
    };
    if state.session_id != session_id || !integrity_ok(&state) {
        return Ok(Vec::new());
    }
    let mut opened = Vec::new();
    for kind in claimed {
        if state.obligations.iter().any(|entry| entry.kind == *kind) {
            continue;
        }
        let synthetic = format!("assistant-commitment:{}", kind.as_str());
        match crate::cli::trusted_store::with_write_lease_wait(
            worktree,
            std::time::Duration::from_millis(300),
            || mark_locked(worktree, session_id, &synthetic, *kind),
        ) {
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                mark_locked(worktree, session_id, &synthetic, *kind)?;
            }
            Err(err) => return Err(err),
            Ok(()) => {}
        }
        opened.push(*kind);
    }
    Ok(opened)
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

    // T-243 core: only assertive completion claims classify; requests,
    // status text, and historical PR mentions stay out.
    #[test]
    fn commitment_classifier_is_narrow() {
        assert_eq!(
            classify_commitments("バグを修正しました。全テスト成功です。"),
            vec![ObligationKind::Implementation, ObligationKind::Verification]
        );
        assert_eq!(
            classify_commitments("The fix has been fixed and all tests pass."),
            vec![ObligationKind::Implementation, ObligationKind::Verification]
        );
        assert!(classify_commitments("バグを修正してください").is_empty());
        assert!(classify_commitments("テストを実行して確認します").is_empty());
        assert!(classify_commitments("PR #3308 マージ済み / 関連 95 GREEN").is_empty());
        assert!(classify_commitments("fmt / clippy PASS").is_empty());
    }

    // T-243 core: unbacked claims open obligations; backed claims and
    // ledger-less sessions never do.
    #[test]
    fn unbacked_commitments_open_obligations_backed_ones_pass() {
        let dir = tempfile::tempdir().unwrap();
        // No ledger at all → the scanner invents nothing.
        assert!(
            mark_unbacked_commitments(dir.path(), "sess-1", &[ObligationKind::Implementation],)
                .unwrap()
                .is_empty()
        );

        // Ledger exists (producing prompt) and implementation evidence is
        // settled → an implementation claim is backed; a verification
        // claim is not and opens the obligation.
        mark_from_prompt(dir.path(), "sess-1", "バグを修正して").unwrap();
        settle_kinds_best_effort(
            dir.path(),
            "sess-1",
            &[ObligationKind::Implementation],
            "verify.run vr-x",
        );
        let opened = mark_unbacked_commitments(
            dir.path(),
            "sess-1",
            &[ObligationKind::Implementation, ObligationKind::Verification],
        )
        .unwrap();
        assert_eq!(opened, vec![ObligationKind::Verification]);
        assert_eq!(
            open_kinds(dir.path(), "sess-1"),
            vec![ObligationKind::Verification]
        );
        // Re-scanning does not stack duplicates.
        let opened =
            mark_unbacked_commitments(dir.path(), "sess-1", &[ObligationKind::Verification])
                .unwrap();
        assert!(opened.is_empty());
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
        let dir = tempfile::tempdir().unwrap();
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

        revive_deferred_best_effort(
            dir.path(),
            "sess-1",
            &[ObligationKind::IssueUpdate, ObligationKind::Pr],
        );
        assert_eq!(
            open_kinds(dir.path(), "sess-1"),
            vec![ObligationKind::IssueUpdate],
            "only the deferred issue update revives; evidence-settled implementation stays settled"
        );

        // Cross-session revival is a no-op.
        revive_deferred_best_effort(dir.path(), "sess-other", &[ObligationKind::IssueUpdate]);
        assert_eq!(
            open_kinds(dir.path(), "sess-1"),
            vec![ObligationKind::IssueUpdate]
        );
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
