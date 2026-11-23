//! PM reports are recorded on the project Board (Issue #7642, design decision 29).
//!
//! The report journal stays the durable source the PM writes to; the Board is
//! where people read the record. Every report is mirrored under a deterministic
//! Board identity, so publishing is idempotent and any report a previous attempt
//! failed to mirror is filled in by the next publish (write time, GUI report
//! read, or Board open). Each mirrored post keeps the report's own timestamp, so
//! the Board stays chronological even for backfilled reports.

use std::{collections::HashSet, path::Path};

use gwt_core::{
    coordination::{self, AuthorKind, BoardDeterministicOutcome, BoardEntry, BoardEntryKind},
    pm_report::{PmReport, PmReportKind},
    Result,
};

const AUTHOR: &str = "Project Manager";
const ID_PREFIX: &str = "pm-report-";

/// Board identity of the post that records `report`.
pub fn board_entry_id(report: &PmReport) -> String {
    format!("{ID_PREFIX}{}", report.id)
}

/// The Board post that records one PM report.
///
/// Blockers and Q&A are recorded as `status`: the Board is a record only
/// (design item 152), and a `blocked` post would open an escalation. Decisions
/// keep the `decision` kind and the report's exact text, so the Board record
/// matches what the decision view shows for the same report. No mentions or
/// audience are set: the post addresses nobody and notifies nobody.
pub fn board_entry_for_report(report: &PmReport) -> BoardEntry {
    let (kind, title) = match report.kind {
        PmReportKind::Progress => (BoardEntryKind::Status, "PM progress report"),
        PmReportKind::Decision => (BoardEntryKind::Decision, "PM decision"),
        PmReportKind::Blocker => (BoardEntryKind::Status, "PM blocker"),
        PmReportKind::Qa => (BoardEntryKind::Status, "PM Q&A"),
    };
    let mut body = match report.question.as_deref() {
        Some(question) if !question.trim().is_empty() => {
            format!("**Q:** {question}\n\n**A:** {}", report.body)
        }
        _ => report.body.clone(),
    };
    if let Some(details) = report
        .structured
        .as_ref()
        .map(|structured| structured.details.trim())
        .filter(|details| !details.is_empty() && *details != report.body.trim())
    {
        body.push_str("\n\n");
        body.push_str(details);
    }
    let mut entry = BoardEntry::new(
        AuthorKind::Agent,
        AUTHOR,
        kind,
        body,
        None,
        None,
        Vec::new(),
        Vec::new(),
    )
    .with_title(title);
    entry.id = board_entry_id(report);
    entry.created_at = report.created_at;
    entry.updated_at = report.created_at;
    entry
}

/// Mirror every PM report that is not yet on the Board. Returns how many posts
/// this call appended.
///
/// Only a Board that preserves caller-supplied identities (the local Board) is
/// written; a remote provider without that capability is left untouched rather
/// than receiving duplicates on every publish.
pub fn publish(repo: &Path) -> Result<usize> {
    if !crate::board_provider::supports_deterministic_identity(repo) {
        return Ok(0);
    }
    let reports = gwt_core::pm_report::load_reports(repo)?;
    let Some(oldest) = reports.iter().map(|report| report.created_at).min() else {
        return Ok(0);
    };
    let recorded: HashSet<String> =
        coordination::load_entries_since(repo, oldest - time::Duration::seconds(1))?
            .into_iter()
            .map(|entry| entry.id)
            .filter(|id| id.starts_with(ID_PREFIX))
            .collect();
    let mut appended = 0;
    for report in &reports {
        if recorded.contains(&board_entry_id(report)) {
            continue;
        }
        if let BoardDeterministicOutcome::Appended(_) =
            coordination::post_entry_deterministic(repo, board_entry_for_report(report))?
        {
            appended += 1;
        }
    }
    Ok(appended)
}

/// [`publish`] for callers whose own work already succeeded: a failure is
/// logged and repaired by the next publish.
pub fn publish_best_effort(repo: &Path) {
    if let Err(error) = publish(repo) {
        tracing::warn!(
            error = %gwt_core::error_ledger::sanitize_error_message(&error.to_string()),
            "PM reports could not be recorded on the Board; the next publish retries"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::{pm_report::post_report, test_support::ScopedGwtHome};

    fn board(repo: &Path) -> Vec<BoardEntry> {
        coordination::load_snapshot(repo)
            .unwrap()
            .board
            .entries
            .into_iter()
            .filter(|entry| entry.id.starts_with(ID_PREFIX))
            .collect()
    }

    #[test]
    fn every_saved_report_appears_on_the_board_in_report_order_exactly_once() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let progress = post_report(&repo, PmReportKind::Progress, "Work moved.", None).unwrap();
        let decision = post_report(&repo, PmReportKind::Decision, "Ship #7642?", None).unwrap();
        let qa = post_report(&repo, PmReportKind::Qa, "Yes.", Some("Is it done?")).unwrap();

        assert_eq!(publish(&repo).unwrap(), 3);
        assert_eq!(publish(&repo).unwrap(), 0, "publishing again adds nothing");

        let entries = board(&repo);
        let ids: Vec<_> = entries.iter().map(|entry| entry.id.clone()).collect();
        assert_eq!(
            ids,
            [&progress, &decision, &qa].map(board_entry_id).to_vec(),
            "reports are on the Board once each, in report order"
        );
        assert_eq!(entries[0].created_at, progress.created_at);
        // The decision record carries the same kind and text the decision view shows.
        assert_eq!(entries[1].kind, BoardEntryKind::Decision);
        assert_eq!(entries[1].body, decision.body);
        assert!(entries[1].mentions.is_empty() && entries[1].audience.is_empty());
        assert_eq!(entries[2].kind, BoardEntryKind::Status);
        assert!(entries[2].body.contains("Is it done?") && entries[2].body.contains("Yes."));
    }

    #[test]
    fn a_report_missed_by_an_earlier_publish_is_filled_in_later() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let first = post_report(&repo, PmReportKind::Progress, "First.", None).unwrap();
        publish(&repo).unwrap();
        // Saved without a publish, as when the Board write failed after the report.
        let missed = post_report(&repo, PmReportKind::Blocker, "Waiting on CI.", None).unwrap();
        let next = post_report(&repo, PmReportKind::Progress, "Next.", None).unwrap();

        assert_eq!(publish(&repo).unwrap(), 2);
        let ids: Vec<_> = board(&repo).into_iter().map(|entry| entry.id).collect();
        assert_eq!(ids, [&first, &missed, &next].map(board_entry_id).to_vec());
    }
}
