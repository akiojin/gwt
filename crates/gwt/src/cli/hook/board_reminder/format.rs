//! Entry-line formatting and self-target detection for `board-reminder`.
//!
//! These helpers turn a list of [`BoardEntry`] values into the text body
//! that gets injected via `additionalContext`. The `[for-you]`-equivalent
//! marker (FR-041 / FR-043) is sourced from [`super::texts::FOR_YOU_MARKER`]
//! so that the entry-line prefix and reminder body never share verbatim
//! substrings.

use gwt_core::coordination::BoardEntry;

use super::texts::{FOR_YOU_MARKER, INJECTION_HEADER, SESSION_START_HEADER, USER_PROMPT_REMINDER};

pub(super) fn filter_and_cap_latest(
    mut entries: Vec<BoardEntry>,
    self_session_id: &str,
    cap: usize,
) -> Vec<BoardEntry> {
    entries.retain(|entry| entry.origin_session_id.as_deref() != Some(self_session_id));
    if entries.len() > cap {
        let start = entries.len() - cap;
        entries.drain(..start);
    }
    entries
}

pub(super) fn injection_text(entries: &[BoardEntry], match_keys: &[String]) -> String {
    let mut out = String::from(INJECTION_HEADER);
    for entry in entries {
        out.push_str(&format_entry_line(entry, match_keys));
    }
    out
}

pub(super) fn session_start_text(entries: &[BoardEntry], match_keys: &[String]) -> String {
    let mut out = String::from(SESSION_START_HEADER);
    if entries.is_empty() {
        out.push_str("- (no recent posts from other Agents)\n");
    } else {
        for entry in entries {
            out.push_str(&format_entry_line(entry, match_keys));
        }
    }
    out.push('\n');
    out.push_str(USER_PROMPT_REMINDER);
    out
}

pub(super) fn entry_targets_self(entry: &BoardEntry, match_keys: &[String]) -> bool {
    !entry.target_owners.is_empty()
        && entry
            .target_owners
            .iter()
            .any(|t| match_keys.iter().any(|k| k == t))
}

pub(super) fn format_entry_line(entry: &BoardEntry, match_keys: &[String]) -> String {
    let branch = entry.origin_branch.as_deref().unwrap_or("-");
    let session_id = entry.origin_session_id.as_deref().unwrap_or("-");
    let prefix = if entry_targets_self(entry, match_keys) {
        FOR_YOU_MARKER
    } else {
        ""
    };
    format!(
        "- {prefix}[{author} @ {branch} / {session}] ({kind}) {body}\n",
        prefix = prefix,
        author = entry.author,
        branch = branch,
        session = session_id,
        kind = entry.kind.as_str(),
        body = entry.body,
    )
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use gwt_core::coordination::{AuthorKind, BoardEntry, BoardEntryKind};

    use super::*;

    fn make_entry(target_owners: Vec<String>) -> BoardEntry {
        BoardEntry::new(
            AuthorKind::Agent,
            "OtherAgent",
            BoardEntryKind::Status,
            "test body",
            None,
            None,
            vec![],
            vec![],
        )
        .with_origin_branch("feature/other")
        .with_origin_session_id("sess-other")
        .with_target_owners(target_owners)
    }

    #[test]
    fn format_entry_line_uses_for_you_marker_constant_for_self_targeted_entry() {
        let entry = make_entry(vec!["sess-1".into()]);
        let line = format_entry_line(&entry, &["sess-1".into()]);
        assert!(
            line.contains(FOR_YOU_MARKER),
            "expected FOR_YOU_MARKER prefix, got: {line}"
        );
        assert!(
            line.starts_with("- "),
            "entry line should start with bullet: {line}"
        );
    }

    #[test]
    fn format_entry_line_no_marker_for_broadcast_entry() {
        let entry = make_entry(vec![]);
        let line = format_entry_line(&entry, &["sess-1".into(), "feature/me".into()]);
        assert!(
            !line.contains(FOR_YOU_MARKER),
            "broadcast entry must not be highlighted, got: {line}"
        );
    }

    #[test]
    fn entry_targets_self_or_match_with_session_id() {
        let entry = make_entry(vec!["sess-1".into()]);
        assert!(entry_targets_self(&entry, &["sess-1".into()]));
        assert!(!entry_targets_self(&entry, &["sess-other".into()]));
    }

    #[test]
    fn entry_targets_self_or_match_with_branch() {
        let entry = make_entry(vec!["feature/me".into()]);
        assert!(entry_targets_self(
            &entry,
            &["sess-1".into(), "feature/me".into()]
        ));
    }

    #[test]
    fn entry_targets_self_returns_false_when_target_owners_empty() {
        let entry = make_entry(vec![]);
        assert!(!entry_targets_self(
            &entry,
            &["sess-1".into(), "feature/me".into()]
        ));
    }

    #[test]
    fn filter_and_cap_latest_excludes_self_session_id() {
        let mut self_entry = BoardEntry::new(
            AuthorKind::Agent,
            "Me",
            BoardEntryKind::Status,
            "self",
            None,
            None,
            vec![],
            vec![],
        )
        .with_origin_session_id("sess-self");
        self_entry.created_at = Utc::now();
        self_entry.updated_at = self_entry.created_at;
        let other_entry = BoardEntry::new(
            AuthorKind::Agent,
            "Other",
            BoardEntryKind::Status,
            "other",
            None,
            None,
            vec![],
            vec![],
        )
        .with_origin_session_id("sess-other");

        let filtered = filter_and_cap_latest(vec![self_entry, other_entry], "sess-self", 20);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].author, "Other");
    }

    #[test]
    fn injection_text_starts_with_header_and_renders_lines() {
        let entry = make_entry(vec![]);
        let text = injection_text(&[entry], &[]);
        assert!(text.starts_with("# Recent Board updates"));
        assert!(text.contains("[OtherAgent @ feature/other / sess-other]"));
    }

    #[test]
    fn session_start_text_appends_user_prompt_reminder() {
        let text = session_start_text(&[], &[]);
        assert!(text.contains("(no recent posts from other Agents)"));
        assert!(text.contains("Board Post Reminder"));
    }
}
