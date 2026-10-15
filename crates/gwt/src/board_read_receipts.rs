//! Explicit Board reads, separate from automatic prompt injection (Issue #3713).

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    time::Instant,
};

use chrono::{DateTime, Duration, Utc};
use fs2::FileExt;
use gwt_core::coordination::{BoardAudienceScope, BoardEntry, BoardEntryKind, BoardProjection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cli::hook::HookOutput;

pub const READ_RECOVERY: &str =
    r#"{"schema_version":1,"operation":"board.show","params":{"all":true}}"#;

pub(crate) fn caller_session_from_env() -> Option<String> {
    // Session identities are exact; near-miss environment values own separate receipts.
    std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .ok()
        .filter(|session| !session.is_empty())
}

#[derive(Default, Deserialize, Serialize)]
struct ReadReceipt {
    session_id: String,
    entries: BTreeMap<String, String>,
    #[serde(default)]
    complete: Option<CompleteCoverage>,
}

#[derive(Deserialize, Serialize)]
struct CompleteCoverage {
    since: DateTime<Utc>,
    total_entries: usize,
    newest_entry_id: Option<String>,
}

pub(crate) fn project_store(repo: &Path) -> PathBuf {
    repo.ancestors()
        .find_map(crate::pm_registry::pm_worktree_store_dir)
        .or_else(|| {
            crate::pm_registry::pm_worktree_for_runtime_dir(repo)
                .and_then(|worktree| crate::pm_registry::pm_worktree_store_dir(&worktree))
        })
        .unwrap_or_else(|| gwt_core::paths::gwt_project_dir_for_repo_path(repo))
}

fn receipt_path(repo: &Path, session: &str) -> PathBuf {
    project_store(repo)
        .join("project-state/board-read-receipts")
        .join(format!(
            "{}.json",
            crate::pm_registry::pm_delivery_prompt_sha256(session)
        ))
}

fn load_receipt(repo: &Path, session: &str) -> io::Result<ReadReceipt> {
    match fs::read(receipt_path(repo, session)) {
        Ok(bytes) => {
            let receipt: ReadReceipt = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            if receipt.session_id != session {
                return Err(io::Error::other(
                    "Board read receipt Session does not match",
                ));
            }
            Ok(receipt)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(ReadReceipt {
            session_id: session.to_owned(),
            ..Default::default()
        }),
        Err(error) => Err(error),
    }
}

fn entry_fingerprint(entry: &BoardEntry) -> io::Result<String> {
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(entry).map_err(io::Error::other)?,
    )))
}

/// Call only after successfully rendering the entries actually returned by board.show.
pub fn record_returned_entries(
    repo: &Path,
    session: &str,
    entries: &[BoardEntry],
) -> io::Result<()> {
    record_read(repo, session, entries, None)
}

/// Only an unlimited, unfiltered all:true read can establish complete coverage.
pub fn record_complete_read(
    repo: &Path,
    session: &str,
    entries: &[BoardEntry],
    started_at: DateTime<Utc>,
    hot: &BoardProjection,
) -> io::Result<()> {
    let latest = latest_entries(entries.to_vec());
    let covered = hot.entries.iter().all(|entry| {
        latest
            .iter()
            .any(|returned| returned.id == entry.id && returned == entry)
    });
    let coverage = covered.then(|| CompleteCoverage {
        since: started_at,
        total_entries: hot.total_entries,
        newest_entry_id: hot.newest_entry_id.clone(),
    });
    record_read(repo, session, entries, coverage)
}

fn record_read(
    repo: &Path,
    session: &str,
    entries: &[BoardEntry],
    coverage: Option<CompleteCoverage>,
) -> io::Result<()> {
    if session.is_empty() || (entries.is_empty() && coverage.is_none()) {
        return Ok(());
    }
    let path = receipt_path(repo, session);
    fs::create_dir_all(path.parent().expect("receipt has a parent"))?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    lock.try_lock_exclusive()?;
    let mut receipt = load_receipt(repo, session)?;
    for entry in latest_entries(entries.to_vec()) {
        receipt
            .entries
            .insert(entry.id.clone(), entry_fingerprint(&entry)?);
    }
    if coverage.is_some() {
        receipt.complete = coverage;
    }
    gwt_github::cache::write_atomic(
        &path,
        &serde_json::to_vec(&receipt).map_err(io::Error::other)?,
    )
}

fn latest_entries(entries: Vec<BoardEntry>) -> Vec<BoardEntry> {
    let mut latest = BTreeMap::<String, BoardEntry>::new();
    for entry in entries {
        match latest.get(&entry.id) {
            Some(previous) if previous.updated_at > entry.updated_at => {}
            _ => {
                latest.insert(entry.id.clone(), entry);
            }
        }
    }
    let mut entries = latest.into_values().collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.created_at);
    entries
}

fn covered_hot_tail(hot: &BoardProjection, coverage: &CompleteCoverage) -> Option<Vec<BoardEntry>> {
    let appended = hot.total_entries.checked_sub(coverage.total_entries)?;
    if appended == 0 {
        return (hot.newest_entry_id == coverage.newest_entry_id).then(Vec::new);
    }
    if coverage.total_entries == 0 && appended == hot.entries.len() {
        return Some(hot.entries.clone());
    }
    let anchor = coverage.newest_entry_id.as_ref()?;
    let position = hot.entries.iter().position(|entry| &entry.id == anchor)?;
    let tail = &hot.entries[position + 1..];
    (tail.len() == appended).then(|| tail.to_vec())
}

fn observation_entries(
    repo: &Path,
    receipt: &ReadReceipt,
    since: DateTime<Utc>,
) -> io::Result<Vec<BoardEntry>> {
    if let Some(coverage) = &receipt.complete {
        let hot = crate::board_provider::load_snapshot(repo)
            .map_err(io::Error::other)?
            .board;
        local_history_is_current(repo, Some(hot.total_entries))?;
        if let Some(tail) = covered_hot_tail(&hot, coverage) {
            let mut entries =
                crate::board_provider::load_entries_since(repo, since.max(coverage.since))
                    .map_err(io::Error::other)?;
            // The append tail catches new posts with timestamps before the watermark.
            entries.extend(tail.into_iter().filter(|entry| entry.updated_at > since));
            return Ok(latest_entries(entries));
        }
    }
    let entries =
        crate::board_provider::load_entries_since(repo, since).map_err(io::Error::other)?;
    local_history_is_current(repo, None)?;
    Ok(latest_entries(entries))
}

fn local_history_is_current(repo: &Path, expected_total: Option<usize>) -> io::Result<()> {
    if crate::board_provider::routing_for(repo).provider != "local" {
        return Ok(());
    }
    #[derive(Deserialize)]
    struct Manifest {
        segments: Vec<Segment>,
    }
    #[derive(Deserialize)]
    struct Segment {
        file: String,
        entries: usize,
        bytes: u64,
    }
    let path = gwt_core::coordination::coordination_events_manifest_path(repo);
    let manifest: Manifest = serde_json::from_slice(&fs::read(&path)?).map_err(io::Error::other)?;
    let root = path.parent().expect("manifest has a parent").join("events");
    let physical = fs::read_dir(&root)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
        .count();
    let total: usize = manifest
        .segments
        .iter()
        .map(|segment| segment.entries)
        .sum();
    if physical != manifest.segments.len()
        || expected_total.is_some_and(|expected| expected != total)
    {
        return Err(io::Error::other("Board history changed outside the observed projection; complete observation is unavailable"));
    }
    for segment in manifest.segments {
        if fs::metadata(root.join(segment.file))?.len() != segment.bytes {
            return Err(io::Error::other(
                "Board append is in flight or its manifest is stale; unread state is unknown",
            ));
        }
    }
    Ok(())
}

fn unread_entries(
    repo: &Path,
    session: &str,
    scope: &BoardAudienceScope,
    since: DateTime<Utc>,
) -> io::Result<Vec<BoardEntry>> {
    gwt_core::operation_deadline::ensure_remaining("Board unread observation")?;
    let receipt = load_receipt(repo, session)?;
    let entries = observation_entries(repo, &receipt, since)?;
    let mut unread = Vec::new();
    for entry in entries {
        gwt_core::operation_deadline::ensure_remaining("Board unread observation")?;
        if gwt_core::coordination::board_entry_visible_for_scope(&entry, scope)
            && entry.origin_session_id.as_deref() != Some(session)
        {
            let already_read = match receipt.entries.get(&entry.id) {
                Some(fingerprint) => fingerprint == &entry_fingerprint(&entry)?,
                None => false,
            };
            if !already_read {
                unread.push(entry);
            }
        }
    }
    Ok(unread)
}

pub(crate) fn registered_pm(repo: &Path, session: &str) -> bool {
    crate::pm_registry::session_is_registered_pm(
        &project_store(repo).join("project-state/pm.json"),
        session,
    )
}

fn pm_unread(repo: &Path, session: &str) -> io::Result<Vec<BoardEntry>> {
    let scope = crate::board_audience::current_session_board_scope(repo, Some(session))
        .map_err(io::Error::other)?;
    unread_entries(repo, session, &scope, DateTime::<Utc>::MIN_UTC)
}

pub fn warning(error: impl std::fmt::Display) -> String {
    format!("Warning: Board read evidence unavailable ({error}); unread state is unknown. \
        This optional coordination check is fail-open. Read Board bodies and retry: {READ_RECOVERY}")
}

fn bounded_pm_prompt_context_with(
    deadline: Instant,
    observe: impl FnOnce() -> Option<String> + Send + 'static,
) -> Option<String> {
    use std::sync::mpsc;

    let expired = || Some(warning("PM Board observation deadline expired"));
    if Instant::now() >= deadline {
        return expired();
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    if let Err(error) = std::thread::Builder::new()
        .name("gwt-board-unread".into())
        .spawn(move || {
            let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(deadline);
            let _ = sender.send(observe());
        })
    {
        return Some(warning(error));
    }
    // Do not join the decoder: local receipt parsing or filesystem I/O can
    // still be busy after the cooperative event decoder observes its deadline.
    match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(context) if Instant::now() < deadline => context,
        Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => expired(),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Some(warning("PM Board observation worker disconnected"))
        }
    }
}

/// The observation never acknowledges reading, including when the count is zero.
pub fn pm_prompt_context(repo: &Path, session: &str) -> Option<String> {
    // Keep ordinary, retired and unregistered sessions silent even when the
    // shared prompt budget has already expired. Only this small identity read
    // stays on the parent; all receipt/history work below is deadline-bounded.
    if !registered_pm(repo, session) {
        return None;
    }
    match gwt_core::operation_deadline::current() {
        Some(deadline) => {
            let repo = repo.to_path_buf();
            let session = session.to_owned();
            bounded_pm_prompt_context_with(deadline, move || {
                pm_prompt_context_inline(&repo, &session)
            })
        }
        None => pm_prompt_context_inline(repo, session),
    }
}

fn pm_prompt_context_inline(repo: &Path, session: &str) -> Option<String> {
    Some(match pm_unread(repo, session) {
        Ok(entries) => {
            let mut counts = BTreeMap::<&str, usize>::new();
            for entry in &entries {
                *counts.entry(entry.kind.as_str()).or_default() += 1;
            }
            let kinds = counts
                .into_iter()
                .map(|(kind, count)| format!("{kind}={count}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Board unread: {} ({kinds}). Prompt injection is not an explicit read. \
                Read the actual bodies before deciding: {READ_RECOVERY}",
                entries.len()
            )
        }
        Err(error) => warning(error),
    })
}

pub fn read_required_message(operation: &str, entries: &[BoardEntry]) -> String {
    let ids = entries
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{operation} refused: unread Board evidence ({ids}). Read the returned bodies in \
        this caller Session, then retry. Run gwtd with this JSON envelope: {READ_RECOVERY}"
    )
}

/// Run before the resident-loop Stop check; its cadence/cap must not bypass unread work.
pub fn pm_stop_check(repo: &Path, session: Option<&str>) -> HookOutput {
    let Some(session) = session.filter(|session| registered_pm(repo, session)) else {
        return HookOutput::Silent;
    };
    match pm_unread(repo, session) {
        Ok(entries) => {
            let critical = entries
                .into_iter()
                .filter(|entry| {
                    matches!(
                        entry.kind,
                        BoardEntryKind::Blocked | BoardEntryKind::Handoff
                    )
                })
                .collect::<Vec<_>>();
            if critical.is_empty() {
                HookOutput::Silent
            } else {
                HookOutput::stop_block(read_required_message("PM Stop", &critical))
            }
        }
        Err(error) => {
            let warning = warning(error);
            tracing::warn!(%warning, "PM Stop Board observation failed open");
            HookOutput::system_message(warning)
        }
    }
}

/// Target identity comes from the actual pane/launch, never caller-supplied author text.
pub fn recent_owner_unread(
    repo: &Path,
    caller_session: &str,
    target_session: Option<&str>,
    target_owner: Option<u64>,
    now: DateTime<Utc>,
) -> io::Result<Vec<BoardEntry>> {
    if target_session.is_none() && target_owner.is_none() {
        return Ok(Vec::new());
    }
    let owner = target_owner.map(|number| number.to_string());
    Ok(unread_entries(
        repo,
        caller_session,
        &BoardAudienceScope::All,
        now - Duration::minutes(10),
    )?
    .into_iter()
    .filter(|entry| {
        entry.updated_at <= now
            && (target_session
                .is_some_and(|session| entry.origin_session_id.as_deref() == Some(session))
                || owner
                    .as_ref()
                    .is_some_and(|owner| entry.related_owners.contains(owner)))
    })
    .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::{coordination::AuthorKind, test_support::ScopedGwtHome};

    fn entry(kind: BoardEntryKind, owner: &str) -> BoardEntry {
        BoardEntry::new(
            AuthorKind::Agent,
            "Owner",
            kind,
            "actual evidence",
            None,
            None,
            vec![],
            vec![owner.into()],
        )
        .with_origin_session_id("owner-session")
    }

    fn register_pm(repo: &Path, session: &str) {
        crate::pm_registry::save_pm_prefs(
            &project_store(repo).join("project-state/pm.json"),
            &crate::pm_registry::PmPrefs {
                registration: Some(crate::pm_registry::PmRegistration {
                    session_id: session.into(),
                    agent_id: "codex".into(),
                    worktree_path: repo.display().to_string(),
                    created_at: None,
                    consecutive_crashes: 0,
                    next_not_before: None,
                }),
                ..Default::default()
            },
        )
        .unwrap();
    }

    #[test]
    fn issue_3713_receipts_are_session_scoped_and_revisions_need_another_read() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(tmp.path());
        let first = entry(BoardEntryKind::Handoff, "42");
        crate::board_provider::post_entry(tmp.path(), first.clone()).unwrap();
        record_returned_entries(tmp.path(), "reader-a", std::slice::from_ref(&first)).unwrap();
        assert!(unread_entries(
            tmp.path(),
            "reader-a",
            &BoardAudienceScope::All,
            DateTime::<Utc>::MIN_UTC
        )
        .unwrap()
        .is_empty());
        assert_eq!(
            unread_entries(
                tmp.path(),
                "reader-b",
                &BoardAudienceScope::All,
                DateTime::<Utc>::MIN_UTC
            )
            .unwrap()
            .len(),
            1
        );
        let mut revision = first.clone();
        revision.body = "changed ruling request".into();
        revision.updated_at += Duration::milliseconds(1);
        crate::board_provider::post_entry(tmp.path(), revision.clone()).unwrap();
        assert_eq!(
            unread_entries(
                tmp.path(),
                "reader-a",
                &BoardAudienceScope::All,
                DateTime::<Utc>::MIN_UTC
            )
            .unwrap()
            .len(),
            1
        );
        record_returned_entries(tmp.path(), "reader-a", &[revision]).unwrap();
        assert!(
            unread_entries(
                tmp.path(),
                "reader-a",
                &BoardAudienceScope::All,
                DateTime::<Utc>::MIN_UTC
            )
            .unwrap()
            .is_empty(),
            "reading the latest revision must not resurrect the old one"
        );
    }

    #[test]
    fn issue_3713_pm_stop_blocks_only_critical_unread_and_explicit_read_recovers() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(tmp.path());
        register_pm(tmp.path(), "pm-session");
        let blocked = entry(BoardEntryKind::Blocked, "42");
        let handoff = entry(BoardEntryKind::Handoff, "42");
        let status = entry(BoardEntryKind::Status, "42");
        for item in [&blocked, &handoff, &status] {
            crate::board_provider::post_entry(tmp.path(), item.clone()).unwrap();
        }
        assert_eq!(
            pm_stop_check(tmp.path(), Some("ordinary-session")),
            HookOutput::Silent
        );
        assert_eq!(pm_stop_check(tmp.path(), None), HookOutput::Silent);
        assert!(
            matches!(pm_stop_check(tmp.path(), Some("pm-session")), HookOutput::StopBlock { reason } if reason.contains("board.show") && reason.contains(&blocked.id) && reason.contains(&handoff.id))
        );
        record_returned_entries(
            tmp.path(),
            "other-session",
            &[blocked.clone(), handoff.clone()],
        )
        .unwrap();
        assert!(matches!(
            pm_stop_check(tmp.path(), Some("pm-session")),
            HookOutput::StopBlock { .. }
        ));
        record_returned_entries(tmp.path(), "pm-session", &[blocked, handoff]).unwrap();
        assert_eq!(
            pm_stop_check(tmp.path(), Some("pm-session")),
            HookOutput::Silent
        );
        assert!(pm_prompt_context(tmp.path(), "pm-session")
            .unwrap()
            .contains("status=1"));
    }

    #[test]
    fn issue_3713_scope_and_exact_target_evidence_exclude_unrelated_and_expired_posts() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(tmp.path());
        let now = Utc::now();
        let current = entry(BoardEntryKind::Handoff, "42").with_audience(vec!["workspace-a"]);
        let other = entry(BoardEntryKind::Blocked, "43")
            .with_audience(vec!["workspace-b"])
            .with_origin_session_id("other-session");
        let mut expired = entry(BoardEntryKind::Blocked, "42");
        expired.created_at = now - Duration::minutes(11);
        expired.updated_at = expired.created_at;
        for item in [&current, &other, &expired] {
            crate::board_provider::post_entry(tmp.path(), item.clone()).unwrap();
        }
        let scoped = unread_entries(
            tmp.path(),
            "reader",
            &BoardAudienceScope::Workspace("workspace-a".into()),
            DateTime::<Utc>::MIN_UTC,
        )
        .unwrap();
        assert_eq!(scoped.len(), 2);
        let recent = recent_owner_unread(
            tmp.path(),
            "reader",
            Some("owner-session"),
            Some(42),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(recent, vec![current.clone()]);
        record_returned_entries(tmp.path(), "reader", &[current]).unwrap();
        assert!(recent_owner_unread(
            tmp.path(),
            "reader",
            Some("owner-session"),
            Some(42),
            Utc::now()
        )
        .unwrap()
        .is_empty());
        assert!(
            recent_owner_unread(tmp.path(), "reader", None, None, Utc::now())
                .unwrap()
                .is_empty()
        );
        let newer = entry(BoardEntryKind::Status, "42");
        crate::board_provider::post_entry(tmp.path(), newer.clone()).unwrap();
        assert_eq!(
            recent_owner_unread(tmp.path(), "reader", None, Some(42), Utc::now()).unwrap(),
            vec![newer]
        );
    }

    #[test]
    fn issue_3713_corrupt_receipt_and_expired_prompt_budget_warn_without_false_zero_or_denial() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(tmp.path());
        register_pm(tmp.path(), "pm-session");
        let path = receipt_path(tmp.path(), "pm-session");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{corrupt").unwrap();
        let text = pm_prompt_context(tmp.path(), "pm-session").unwrap();
        assert!(text.contains("unknown") && !text.contains("Board unread: 0"));
        assert!(
            matches!(pm_stop_check(tmp.path(), Some("pm-session")), HookOutput::SystemMessage(text) if text.contains("fail-open"))
        );
        assert!(record_returned_entries(
            tmp.path(),
            "pm-session",
            &[entry(BoardEntryKind::Status, "42")]
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{corrupt");
        fs::remove_file(path).unwrap();
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() - std::time::Duration::from_secs(1),
        );
        assert!(pm_prompt_context(tmp.path(), "pm-session")
            .unwrap()
            .contains("unknown"));
    }

    #[test]
    fn issue_3713_pm_path_never_grants_an_unregistered_or_superseded_session_privilege() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(tmp.path());
        let worktree = crate::pm_registry::pm_worktree_path_for_repo_path(tmp.path());
        fs::create_dir_all(&worktree).unwrap();
        assert!(pm_prompt_context(&worktree, "old-session").is_none());
        register_pm(&worktree, "current-session");
        assert!(pm_prompt_context(&worktree, "old-session").is_none());
        assert_eq!(
            pm_stop_check(&worktree, Some("old-session")),
            HookOutput::Silent
        );
        assert!(pm_prompt_context(&worktree, "current-session").is_some());
    }

    #[test]
    fn issue_3713_complete_read_keeps_backdated_arrivals_unread_and_partial_reads_keep_coverage() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(tmp.path());
        let first = entry(BoardEntryKind::Status, "42");
        let hot = crate::board_provider::post_entry(tmp.path(), first.clone())
            .unwrap()
            .board;
        let started = Utc::now();
        record_complete_read(tmp.path(), "reader", &[first], started, &hot).unwrap();
        assert!(load_receipt(tmp.path(), "reader")
            .unwrap()
            .complete
            .is_some());
        let mut backdated = entry(BoardEntryKind::Handoff, "42");
        backdated.created_at = started - Duration::days(1);
        backdated.updated_at = backdated.created_at;
        crate::board_provider::post_entry(tmp.path(), backdated.clone()).unwrap();
        let unread = unread_entries(
            tmp.path(),
            "reader",
            &BoardAudienceScope::All,
            DateTime::<Utc>::MIN_UTC,
        )
        .unwrap();
        assert_eq!(unread, vec![backdated.clone()]);
        record_returned_entries(tmp.path(), "reader", &[backdated]).unwrap();
        assert_eq!(
            load_receipt(tmp.path(), "reader")
                .unwrap()
                .complete
                .unwrap()
                .since,
            started
        );
        assert!(unread_entries(
            tmp.path(),
            "reader",
            &BoardAudienceScope::All,
            DateTime::<Utc>::MIN_UTC
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn issue_3713_complete_read_overflow_and_inflight_append_never_fabricate_zero_unread() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(tmp.path());
        let first = entry(BoardEntryKind::Status, "42");
        let hot = crate::board_provider::post_entry(tmp.path(), first.clone())
            .unwrap()
            .board;
        let started = Utc::now();
        record_complete_read(tmp.path(), "reader", &[first], started, &hot).unwrap();
        let coverage = load_receipt(tmp.path(), "reader")
            .unwrap()
            .complete
            .unwrap();
        let mut overflow = hot.clone();
        overflow.total_entries += gwt_core::coordination::HOT_PROJECTION_ENTRY_LIMIT + 1;
        assert!(covered_hot_tail(&overflow, &coverage).is_none());
        let segments = gwt_core::coordination::coordination_events_segments_dir(tmp.path());
        let segment = fs::read_dir(segments)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        use std::io::Write;
        fs::OpenOptions::new()
            .append(true)
            .open(segment)
            .unwrap()
            .write_all(b"{inflight")
            .unwrap();
        assert!(unread_entries(
            tmp.path(),
            "reader",
            &BoardAudienceScope::All,
            DateTime::<Utc>::MIN_UTC
        )
        .is_err());
    }

    #[test]
    fn issue_3713_pm_prompt_deadline_returns_while_controlled_decoder_is_blocked() {
        use std::sync::mpsc;
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let deadline = Instant::now() + std::time::Duration::from_millis(100);
        let parent = std::thread::spawn(move || {
            let result = bounded_pm_prompt_context_with(deadline, move || {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Some("decoder completed".into())
            });
            result_tx.send(result).unwrap();
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let early_result = result_rx.recv_timeout(std::time::Duration::from_secs(5));
        // Always release/join before asserting so a RED run leaves no worker behind.
        release_tx.send(()).unwrap();
        parent.join().unwrap();
        let advisory = early_result
            .expect("parent must return while the decoder is still blocked")
            .expect("deadline must report unknown reading state");
        assert!(advisory.contains("unknown") && advisory.contains("fail-open"));
        assert!(advisory.contains("board.show"));
    }

    #[test]
    fn issue_3713_pm_prompt_child_inherits_deadline_and_preserves_completed_results() {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        assert_eq!(
            bounded_pm_prompt_context_with(deadline, move || {
                assert_eq!(gwt_core::operation_deadline::current(), Some(deadline));
                Some("Board unread: 1 (handoff=1)".into())
            }),
            Some("Board unread: 1 (handoff=1)".into())
        );
        assert_eq!(bounded_pm_prompt_context_with(deadline, || None), None);
    }

    #[test]
    fn issue_3713_expired_prompt_budget_is_silent_without_exact_pm_registration() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(tmp.path());
        let registered = tmp.path().join("registered");
        let unregistered = tmp.path().join("unregistered");
        fs::create_dir_all(&registered).unwrap();
        fs::create_dir_all(&unregistered).unwrap();
        register_pm(&registered, "current-pm");
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            Instant::now() - std::time::Duration::from_secs(1),
        );
        for (repo, session) in [
            (&registered, "ordinary-session"),
            (&registered, "old-pm"),
            (&registered, " current-pm"),
            (&registered, "current-pm "),
            (&unregistered, "current-pm"),
        ] {
            assert_eq!(pm_prompt_context(repo, session), None, "{session}");
        }
    }
}
