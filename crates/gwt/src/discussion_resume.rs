#![allow(dead_code)]

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use chrono::Local;

use gwt_agent::PendingDiscussionResume;

pub const DISCUSSION_RELATIVE_PATH: &str = "tasks/discussions.md";
pub const LEGACY_DISCUSSION_RELATIVE_PATH: &str = ".gwt/discussion.md";

const DEFAULT_DISCUSSIONS_HEADER: &str = "# Discussions\n\nThis file is the canonical gwt discussion log. Entries are updated in place while active and indexed by the `discussions` semantic scope.\n";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResumePromptSessionState {
    pub last_source_event: Option<String>,
    pub saw_session_start: bool,
    pub fallback_armed: bool,
    pub prompt_pending: bool,
    pub last_handled_proposal: Option<String>,
}

pub fn load_pending_resume(worktree: &Path) -> io::Result<Option<PendingDiscussionResume>> {
    if let Some(content) = read_canonical_discussion(worktree)? {
        let proposals = parse_active_discussion_proposals(&content);
        return Ok(select_pending_resume(&proposals));
    }

    if let Some(content) = read_legacy_discussion(worktree)? {
        let proposals = parse_proposals(&content);
        return Ok(select_pending_resume(&proposals));
    }

    Ok(None)
}

pub fn park_pending_resume(worktree: &Path, pending: &PendingDiscussionResume) -> io::Result<bool> {
    let Some((discussion_path, content)) =
        discussion_for_mutation(worktree, &pending.proposal_label)?
    else {
        return Ok(false);
    };
    let proposals = parse_active_discussion_proposals(&content);
    let Some(target) = proposals.into_iter().find(|proposal| {
        proposal.status == ProposalStatus::Active
            && proposal.label == pending.proposal_label
            && proposal.title == pending.proposal_title
    }) else {
        return Ok(false);
    };

    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    if let Some(line) = lines.get_mut(target.header_line_index) {
        *line = line.replacen("[active]", "[parked]", 1);
    }
    write_lines_preserving_trailing_newline(&discussion_path, &content, lines)?;
    Ok(true)
}

/// Set a proposal's status label (e.g. `[active]` → `[chosen]`) by its
/// label (e.g. `Proposal A`). Returns `Ok(true)` when the proposal was
/// found in an `[active]` state and rewritten; `Ok(false)` otherwise.
///
/// Used by the `gwtd discuss resolve|park|reject` CLI to let the LLM
/// explicitly exit the `gwt-discussion` skill so the Stop-block handler
/// (SPEC-1935 FR-014p) stays silent.
pub fn set_proposal_status_by_label(
    worktree: &Path,
    label: &str,
    new_status: &str,
) -> io::Result<bool> {
    let Some((discussion_path, content)) = discussion_for_mutation(worktree, label)? else {
        return Ok(false);
    };
    let proposals = parse_active_discussion_proposals(&content);
    let Some(target) = proposals
        .into_iter()
        .find(|p| p.status == ProposalStatus::Active && p.label.eq_ignore_ascii_case(label))
    else {
        return Ok(false);
    };

    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    if let Some(line) = lines.get_mut(target.header_line_index) {
        if let Some(rewritten) = replace_trailing_status_tag(line, new_status) {
            *line = rewritten;
        }
    }
    write_lines_preserving_trailing_newline(&discussion_path, &content, lines)?;
    Ok(true)
}

/// Rewrite only the terminal `[status]` tag on a `### Proposal ...` header
/// line. Mirrors the `rsplit_once('[')` parse contract used by
/// [`parse_proposals`] so titles that happen to contain a literal
/// `"[active]"` substring do not fool the replacement.
fn replace_trailing_status_tag(line: &str, new_status: &str) -> Option<String> {
    // Find the rightmost `[` and its matching `]` on the same line,
    // ignoring anything that appears before them (including a proposal
    // title that spuriously contains `[active]`).
    let trimmed_end = line.trim_end();
    if !trimmed_end.ends_with(']') {
        return None;
    }
    let last_open = trimmed_end.rfind('[')?;
    let trailing_whitespace_len = line.len() - trimmed_end.len();
    let prefix = &line[..last_open];
    let trailing = &line[line.len() - trailing_whitespace_len..];
    Some(format!("{prefix}[{new_status}]{trailing}"))
}

/// Clear the `Next Question:` line of the named `[active]` proposal.
/// Returns `Ok(true)` when the proposal was found and modified.
pub fn clear_proposal_next_question(worktree: &Path, label: &str) -> io::Result<bool> {
    let Some((discussion_path, content)) = discussion_for_mutation(worktree, label)? else {
        return Ok(false);
    };
    let proposals = parse_active_discussion_proposals(&content);
    let Some(target) = proposals
        .into_iter()
        .find(|p| p.status == ProposalStatus::Active && p.label.eq_ignore_ascii_case(label))
    else {
        return Ok(false);
    };

    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let start = target.header_line_index + 1;
    let mut modified = false;
    for line in lines.iter_mut().skip(start) {
        let leading_trim = line.trim_start();
        if leading_trim.starts_with("### Proposal ") || leading_trim.starts_with("## ") {
            break;
        }
        if leading_trim.starts_with("- Next Question:") {
            let indent_len = line.len() - leading_trim.len();
            let indent: String = line.chars().take(indent_len).collect();
            *line = format!("{indent}- Next Question:");
            modified = true;
            break;
        }
    }
    if !modified {
        return Ok(false);
    }
    write_lines_preserving_trailing_newline(&discussion_path, &content, lines)?;
    Ok(true)
}

pub fn build_resume_prompt(pending: &PendingDiscussionResume) -> String {
    let next_question = pending
        .next_question
        .as_deref()
        .filter(|question| !question.trim().is_empty())
        .map(|question| format!("\nNext question: {question}"))
        .unwrap_or_default();
    format!(
        "Use gwt-discussion to resume the unfinished discussion from `tasks/discussions.md`.\nFocus on {} - {}.{}\nContinue the discussion before returning an Action Bundle.\n",
        pending.proposal_label, pending.proposal_title, next_question
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalStatus {
    Active,
    Parked,
    Rejected,
    Chosen,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedProposal {
    pub(crate) header_line_index: usize,
    pub(crate) label: String,
    pub(crate) title: String,
    pub(crate) status: ProposalStatus,
    pub(crate) next_question: Option<String>,
    pub(crate) fields: Vec<(String, Option<String>)>,
}

pub fn parse_proposals(content: &str) -> Vec<ParsedProposal> {
    let mut proposals: Vec<ParsedProposal> = Vec::new();

    for (index, raw_line) in content.lines().enumerate() {
        let trimmed = raw_line.trim();
        if !trimmed.starts_with("### Proposal ") {
            if let Some(current) = proposals.last_mut() {
                if let Some(question) = parse_field_value(trimmed, "Next Question") {
                    current.next_question = question;
                }
                if let Some((field, value)) = parse_any_field_value(trimmed) {
                    current.fields.push((field, value));
                }
            }
            continue;
        }

        let Some((header, status)) = trimmed.rsplit_once('[') else {
            continue;
        };
        let status = status.trim_end_matches(']').trim();
        let Some(header) = header.trim().strip_prefix("### ") else {
            continue;
        };
        let Some((label, title)) = header.split_once(" - ") else {
            continue;
        };

        proposals.push(ParsedProposal {
            header_line_index: index,
            label: label.trim().to_string(),
            title: title.trim().to_string(),
            status: parse_status(status),
            next_question: None,
            fields: Vec::new(),
        });
    }

    proposals
}

fn parse_active_discussion_proposals(content: &str) -> Vec<ParsedProposal> {
    let lines: Vec<&str> = content.lines().collect();
    let mut proposals = Vec::new();

    for (start, end) in discussion_entry_ranges(&lines) {
        if !discussion_entry_is_active(&lines[start..end]) {
            continue;
        }

        let section = lines[start..end].join("\n");
        proposals.extend(parse_proposals(&section).into_iter().map(|mut proposal| {
            proposal.header_line_index += start;
            proposal
        }));
    }

    proposals
}

fn discussion_entry_ranges(lines: &[&str]) -> Vec<(usize, usize)> {
    let starts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.trim_start().starts_with("## ").then_some(index))
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(lines.len());
            (*start, end)
        })
        .collect()
}

fn discussion_entry_is_active(lines: &[&str]) -> bool {
    lines.iter().any(|line| {
        line.trim()
            .strip_prefix("Status:")
            .map(|status| status.trim().eq_ignore_ascii_case("active"))
            .unwrap_or(false)
    })
}

fn canonical_discussion_path(worktree: &Path) -> PathBuf {
    worktree.join(DISCUSSION_RELATIVE_PATH)
}

fn legacy_discussion_path(worktree: &Path) -> PathBuf {
    worktree.join(LEGACY_DISCUSSION_RELATIVE_PATH)
}

fn read_canonical_discussion(worktree: &Path) -> io::Result<Option<String>> {
    read_optional(canonical_discussion_path(worktree))
}

fn read_legacy_discussion(worktree: &Path) -> io::Result<Option<String>> {
    read_optional(legacy_discussion_path(worktree))
}

fn read_optional(path: PathBuf) -> io::Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    fs::read_to_string(path).map(Some)
}

fn discussion_for_mutation(
    worktree: &Path,
    proposal_label: &str,
) -> io::Result<Option<(PathBuf, String)>> {
    let canonical_path = canonical_discussion_path(worktree);
    if canonical_path.exists() {
        let content = fs::read_to_string(&canonical_path)?;
        return Ok(Some((canonical_path, content)));
    }

    if let Some(legacy_content) = read_legacy_discussion(worktree)? {
        if parse_proposals(&legacy_content).iter().any(|proposal| {
            proposal.status == ProposalStatus::Active
                && proposal.label.eq_ignore_ascii_case(proposal_label)
        }) {
            let path = canonicalize_legacy_discussion(worktree, &legacy_content)?;
            let content = fs::read_to_string(&path)?;
            return Ok(Some((path, content)));
        }
    }

    Ok(None)
}

fn canonicalize_legacy_discussion(worktree: &Path, legacy_content: &str) -> io::Result<PathBuf> {
    let path = canonical_discussion_path(worktree);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut content = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        DEFAULT_DISCUSSIONS_HEADER.to_string()
    };

    let date = Local::now().format("%Y-%m-%d");
    let legacy_body = normalize_legacy_body_for_canonical_entry(legacy_content);
    let entry = format!(
        "## {date} — Legacy gwt-discussion state\n\n\
         Status: active\n\
         Topics: gwt-discussion\n\
         Related SPECs: #1935\n\
         Related Works:\n\
         Promoted To:\n\n\
         Summary:\n\
         Migrated active gwt-discussion state from `.gwt/discussion.md`.\n\n\
         Decisions:\n\n\
         Open Questions:\n\n\
         Next:\n\
         Continue the migrated discussion.\n\n\
         {}\n",
        legacy_body.trim_end()
    );

    content = append_discussion_section(&content, &entry);
    fs::write(&path, content)?;
    Ok(path)
}

fn append_discussion_section(content: &str, entry: &str) -> String {
    let mut output = content.trim_end().to_string();
    if !output.is_empty() {
        output.push_str("\n\n");
    }
    output.push_str(entry.trim_end());
    output.push('\n');
    output
}

fn normalize_legacy_body_for_canonical_entry(content: &str) -> String {
    content
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if let Some(title) = trimmed.strip_prefix("## ") {
                let indent_len = line.len() - trimmed.len();
                let indent = &line[..indent_len];
                format!("{indent}### {title}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_lines_preserving_trailing_newline(
    path: &Path,
    original: &str,
    lines: Vec<String>,
) -> io::Result<()> {
    let rewritten = lines.join("\n");
    let final_content = if original.ends_with('\n') {
        format!("{rewritten}\n")
    } else {
        rewritten
    };
    fs::write(path, final_content)
}

fn parse_status(status: &str) -> ProposalStatus {
    match status {
        "active" => ProposalStatus::Active,
        "parked" => ProposalStatus::Parked,
        "rejected" => ProposalStatus::Rejected,
        "chosen" => ProposalStatus::Chosen,
        _ => ProposalStatus::Unknown,
    }
}

fn parse_field_value(line: &str, field: &str) -> Option<Option<String>> {
    let prefix = format!("- {field}:");
    let remainder = line.strip_prefix(&prefix)?;
    let value = remainder.trim();
    if value.is_empty() {
        Some(None)
    } else {
        Some(Some(value.to_string()))
    }
}

fn parse_any_field_value(line: &str) -> Option<(String, Option<String>)> {
    let remainder = line.strip_prefix("- ")?;
    let (field, value) = remainder.split_once(':')?;
    let field = field.trim();
    if field.is_empty() {
        return None;
    }
    let value = value.trim();
    Some((
        field.to_string(),
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        },
    ))
}

pub fn proposal_evidence_blocker_by_label(
    worktree: &Path,
    label: &str,
) -> io::Result<Option<String>> {
    if let Some(content) = read_canonical_discussion(worktree)? {
        let proposals = parse_active_discussion_proposals(&content);
        return Ok(proposals
            .iter()
            .find(|p| p.status == ProposalStatus::Active && p.label.eq_ignore_ascii_case(label))
            .and_then(evidence_gate_blocker));
    }

    if let Some(content) = read_legacy_discussion(worktree)? {
        let proposals = parse_proposals(&content);
        return Ok(proposals
            .iter()
            .find(|p| p.status == ProposalStatus::Active && p.label.eq_ignore_ascii_case(label))
            .and_then(evidence_gate_blocker));
    }

    Ok(None)
}

pub fn discussion_stop_blocker(worktree: &Path) -> io::Result<Option<PendingDiscussionResume>> {
    if let Some(content) = read_canonical_discussion(worktree)? {
        let proposals = parse_active_discussion_proposals(&content);
        return Ok(select_pending_discussion_blocker(&proposals));
    }

    if let Some(content) = read_legacy_discussion(worktree)? {
        let proposals = parse_proposals(&content);
        return Ok(select_pending_discussion_blocker(&proposals));
    }

    Ok(None)
}

fn select_pending_resume(proposals: &[ParsedProposal]) -> Option<PendingDiscussionResume> {
    proposals
        .iter()
        .find(|proposal| {
            proposal.status == ProposalStatus::Active && proposal.next_question.is_some()
        })
        .or_else(|| {
            proposals
                .iter()
                .find(|proposal| proposal.status == ProposalStatus::Active)
        })
        .map(|proposal| PendingDiscussionResume {
            proposal_label: proposal.label.clone(),
            proposal_title: proposal.title.clone(),
            next_question: proposal.next_question.clone(),
        })
}

fn select_pending_discussion_blocker(
    proposals: &[ParsedProposal],
) -> Option<PendingDiscussionResume> {
    proposals
        .iter()
        .find(|proposal| {
            proposal.status == ProposalStatus::Active && proposal.next_question.is_some()
        })
        .map(|proposal| PendingDiscussionResume {
            proposal_label: proposal.label.clone(),
            proposal_title: proposal.title.clone(),
            next_question: proposal.next_question.clone(),
        })
        .or_else(|| {
            proposals
                .iter()
                .filter(|proposal| proposal.status == ProposalStatus::Active)
                .find_map(|proposal| {
                    evidence_gate_blocker(proposal).map(|reason| PendingDiscussionResume {
                        proposal_label: proposal.label.clone(),
                        proposal_title: proposal.title.clone(),
                        next_question: Some(reason),
                    })
                })
        })
}

fn evidence_gate_blocker(proposal: &ParsedProposal) -> Option<String> {
    let exit_blockers = field_value(proposal, "Exit Blockers");
    if is_blocking_value(exit_blockers.as_deref()) {
        return Some(format!(
            "Exit Blockers remain unresolved: {}",
            exit_blockers.unwrap_or_default()
        ));
    }

    let required_fields = [
        "Implementation Proof",
        "SPEC/Issue Proof",
        "Gap Check Proof",
        "Official Docs Proof",
        "External Research Proof",
    ];
    for field in required_fields {
        let value = field_value(proposal, field);
        if !is_acceptable_proof(value.as_deref()) {
            return Some(format!("{field} is missing or incomplete"));
        }
    }

    match field_value(proposal, "Evidence Gate") {
        Some(value) if value.eq_ignore_ascii_case("complete") => None,
        Some(value) => Some(format!("Evidence Gate is not complete: {value}")),
        None => Some("Evidence Gate is missing".to_string()),
    }
}

fn field_value(proposal: &ParsedProposal, field: &str) -> Option<String> {
    proposal
        .fields
        .iter()
        .rev()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(field))
        .and_then(|(_, value)| value.clone())
}

fn is_acceptable_proof(value: Option<&str>) -> bool {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    let lower = value.to_ascii_lowercase();
    if let Some(reason) = lower.strip_prefix("not-applicable:") {
        return !reason.trim().is_empty();
    }
    !is_placeholder_value(value)
}

fn is_blocking_value(value: Option<&str>) -> bool {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    !matches!(
        value.to_ascii_lowercase().as_str(),
        "none" | "resolved" | "complete" | "closed" | "n/a" | "not-applicable"
    )
}

fn is_placeholder_value(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "tbd" | "todo" | "unknown" | "unverified" | "none" | "n/a" | "not-applicable"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_discussion() -> &'static str {
        r#"## Discussion TODO

Status: active

### Proposal A - Hook-driven resume [active]
- Summary: Keep unfinished discussion state in the local artifact.
- Open Questions: Whether Stop should drive the resume path.
- Dependency Checks: Hook events already exist.
- Deferred Decisions: Exact prompt copy.
- Next Question: Should SessionStart or UserPromptSubmit surface the resume proposal?
- Promotable Changes: Add runtime-state handoff.

### Proposal B - Manual follow-up only [parked]
- Summary: Keep resume entirely manual.
- Open Questions:
- Dependency Checks:
- Deferred Decisions:
- Next Question:
- Promotable Changes:
"#
    }

    fn sample_tasks_discussions() -> &'static str {
        r#"# Discussions

## 2026-05-27 — Hook-driven resume

Status: active
Topics: hooks
Related SPECs: #1935
Related Works:
Promoted To:

Summary:
Keep unfinished discussion state in the canonical repository discussion log.

Decisions:

Open Questions:
- Should SessionStart or UserPromptSubmit surface the resume proposal?

Next:
Continue the gwt-discussion workflow.

### Proposal A - Hook-driven resume [active]
- Summary: Keep unfinished discussion state in the canonical artifact.
- Open Questions: Whether Stop should drive the resume path.
- Dependency Checks: Hook events already exist.
- Deferred Decisions: Exact prompt copy.
- Next Question: Should SessionStart or UserPromptSubmit surface the resume proposal?
- Promotable Changes: Add runtime-state handoff.

### Proposal B - Manual follow-up only [parked]
- Summary: Keep resume entirely manual.
- Next Question:
"#
    }

    fn write_tasks_discussions(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        let path = dir.join("tasks/discussions.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        path
    }

    fn write_legacy_discussion(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        let path = dir.join(LEGACY_DISCUSSION_RELATIVE_PATH);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn load_pending_resume_reads_active_tasks_discussions_entry() {
        let dir = tempfile::tempdir().unwrap();
        write_tasks_discussions(dir.path(), sample_tasks_discussions());

        let pending = load_pending_resume(dir.path()).unwrap();

        assert_eq!(
            pending,
            Some(PendingDiscussionResume {
                proposal_label: "Proposal A".to_string(),
                proposal_title: "Hook-driven resume".to_string(),
                next_question: Some(
                    "Should SessionStart or UserPromptSubmit surface the resume proposal?"
                        .to_string()
                ),
            })
        );
    }

    #[test]
    fn discussion_stop_blocker_ignores_completed_tasks_discussions_entries() {
        let dir = tempfile::tempdir().unwrap();
        write_tasks_discussions(
            dir.path(),
            r#"# Discussions

## 2026-05-26 — Completed discussion

Status: completed
Topics: hooks
Related SPECs: #1935
Related Works:
Promoted To:

Summary:
Historical discussion.

Decisions:
- Done.

Open Questions:

Next:
None.

### Proposal A - Historical active-looking proposal [active]
- Next Question: This historical question must not block Stop.
"#,
        );
        write_legacy_discussion(
            dir.path(),
            "### Proposal B - Stale legacy question [active]\n\
             - Next Question: Completed canonical state must not revive this fallback.\n",
        );

        assert_eq!(discussion_stop_blocker(dir.path()).unwrap(), None);
    }

    #[test]
    fn load_pending_resume_reads_legacy_discussion_when_tasks_absent() {
        let dir = tempfile::tempdir().unwrap();
        write_legacy_discussion(dir.path(), sample_discussion());

        let pending = load_pending_resume(dir.path()).unwrap();

        assert_eq!(
            pending,
            Some(PendingDiscussionResume {
                proposal_label: "Proposal A".to_string(),
                proposal_title: "Hook-driven resume".to_string(),
                next_question: Some(
                    "Should SessionStart or UserPromptSubmit surface the resume proposal?"
                        .to_string()
                ),
            })
        );
    }

    #[test]
    fn set_proposal_status_canonicalizes_legacy_before_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = write_legacy_discussion(dir.path(), sample_discussion());

        let changed = set_proposal_status_by_label(dir.path(), "Proposal A", "chosen").unwrap();

        assert!(changed);
        let canonical = std::fs::read_to_string(dir.path().join(DISCUSSION_RELATIVE_PATH)).unwrap();
        assert!(canonical.contains("## "));
        assert!(canonical.contains("Status: active"));
        assert!(canonical.contains("### Proposal A - Hook-driven resume [chosen]"));
        let legacy = std::fs::read_to_string(legacy_path).unwrap();
        assert!(
            legacy.contains("### Proposal A - Hook-driven resume [active]"),
            "legacy fallback must remain read-only"
        );
    }

    #[test]
    fn legacy_canonicalization_preserves_proposals_under_old_h2_without_status() {
        let dir = tempfile::tempdir().unwrap();
        write_legacy_discussion(
            dir.path(),
            "## Discussion TODO\n\n\
             ### Proposal A - Legacy h2 state [active]\n\
             - Next Question: Can old discussion state still be exited?\n",
        );

        let changed = set_proposal_status_by_label(dir.path(), "Proposal A", "parked").unwrap();

        assert!(changed);
        let canonical = std::fs::read_to_string(dir.path().join(DISCUSSION_RELATIVE_PATH)).unwrap();
        assert!(canonical.contains("### Discussion TODO"));
        assert!(canonical.contains("### Proposal A - Legacy h2 state [parked]"));
    }

    #[test]
    fn load_pending_resume_prefers_active_proposal_with_next_question() {
        let dir = tempfile::tempdir().unwrap();
        let discussion_path = dir.path().join(DISCUSSION_RELATIVE_PATH);
        std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
        std::fs::write(&discussion_path, sample_discussion()).unwrap();

        let pending = load_pending_resume(dir.path()).unwrap();

        assert_eq!(
            pending,
            Some(PendingDiscussionResume {
                proposal_label: "Proposal A".to_string(),
                proposal_title: "Hook-driven resume".to_string(),
                next_question: Some(
                    "Should SessionStart or UserPromptSubmit surface the resume proposal?"
                        .to_string()
                ),
            })
        );
    }

    #[test]
    fn load_pending_resume_ignores_parked_proposals() {
        let dir = tempfile::tempdir().unwrap();
        let discussion_path = dir.path().join(DISCUSSION_RELATIVE_PATH);
        std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
        std::fs::write(
            &discussion_path,
            r#"## Discussion TODO

Status: active

### Proposal A - Hook-driven resume [parked]
- Summary: Keep unfinished discussion state in the local artifact.
- Open Questions:
- Dependency Checks:
- Deferred Decisions:
- Next Question: Should SessionStart surface a proposal?
- Promotable Changes:
"#,
        )
        .unwrap();

        let pending = load_pending_resume(dir.path()).unwrap();

        assert_eq!(pending, None);
    }

    #[test]
    fn park_pending_resume_updates_matching_active_proposal() {
        let dir = tempfile::tempdir().unwrap();
        let discussion_path = dir.path().join(DISCUSSION_RELATIVE_PATH);
        std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
        std::fs::write(&discussion_path, sample_discussion()).unwrap();

        let changed = park_pending_resume(
            dir.path(),
            &PendingDiscussionResume {
                proposal_label: "Proposal A".to_string(),
                proposal_title: "Hook-driven resume".to_string(),
                next_question: None,
            },
        )
        .unwrap();

        assert!(changed);
        let updated = std::fs::read_to_string(&discussion_path).unwrap();
        assert!(updated.contains("### Proposal A - Hook-driven resume [parked]"));
        assert!(!updated.contains("### Proposal A - Hook-driven resume [active]"));
    }

    #[test]
    fn build_resume_prompt_includes_focus_and_optional_question() {
        let prompt = build_resume_prompt(&PendingDiscussionResume {
            proposal_label: "Proposal A".to_string(),
            proposal_title: "Hook-driven resume".to_string(),
            next_question: Some("Which hook should surface the proposal?".to_string()),
        });
        assert!(prompt.contains("Use gwt-discussion"));
        assert!(prompt.contains("Proposal A - Hook-driven resume"));
        assert!(prompt.contains("Next question: Which hook should surface the proposal?"));

        let prompt_without_question = build_resume_prompt(&PendingDiscussionResume {
            proposal_label: "Proposal B".to_string(),
            proposal_title: "Manual follow-up only".to_string(),
            next_question: None,
        });
        assert!(!prompt_without_question.contains("Next question:"));
    }

    #[test]
    fn set_proposal_status_updates_active_to_chosen() {
        let dir = tempfile::tempdir().unwrap();
        let discussion_path = dir.path().join(DISCUSSION_RELATIVE_PATH);
        std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
        std::fs::write(&discussion_path, sample_discussion()).unwrap();

        let changed = set_proposal_status_by_label(dir.path(), "Proposal A", "chosen").unwrap();
        assert!(changed);
        let updated = std::fs::read_to_string(&discussion_path).unwrap();
        assert!(updated.contains("### Proposal A - Hook-driven resume [chosen]"));
        assert!(!updated.contains("### Proposal A - Hook-driven resume [active]"));
        // Other proposals remain untouched
        assert!(updated.contains("### Proposal B - Manual follow-up only [parked]"));
    }

    #[test]
    fn set_proposal_status_rewrites_only_trailing_status_tag_even_with_active_in_title() {
        // Regression: a proposal title that literally contains "[active]"
        // must NOT trick the setter into replacing the substring inside
        // the title. Only the terminal `[status]` tag should change.
        let dir = tempfile::tempdir().unwrap();
        let discussion_path = dir.path().join(DISCUSSION_RELATIVE_PATH);
        std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
        std::fs::write(
            &discussion_path,
            "## Discussion TODO\n\n\
             Status: active\n\n\
             ### Proposal A - Toggle [active] state review [active]\n\
             - Next Question: is this safe?\n",
        )
        .unwrap();

        let changed = set_proposal_status_by_label(dir.path(), "Proposal A", "chosen").unwrap();
        assert!(changed);
        let updated = std::fs::read_to_string(&discussion_path).unwrap();
        // Trailing tag flipped to [chosen]; the title substring untouched.
        assert!(
            updated.contains("### Proposal A - Toggle [active] state review [chosen]"),
            "trailing tag must be rewritten, title substring preserved; got: {updated}"
        );
        // And no stray "[active] state review [active]" remains.
        assert!(
            !updated.contains("[active] state review [active]"),
            "trailing [active] must be replaced: {updated}"
        );
    }

    #[test]
    fn set_proposal_status_returns_false_for_non_active_or_missing_label() {
        let dir = tempfile::tempdir().unwrap();
        let discussion_path = dir.path().join(DISCUSSION_RELATIVE_PATH);
        std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
        std::fs::write(&discussion_path, sample_discussion()).unwrap();

        // Already parked
        assert!(!set_proposal_status_by_label(dir.path(), "Proposal B", "chosen").unwrap());
        // Unknown label
        assert!(!set_proposal_status_by_label(dir.path(), "Proposal Z", "chosen").unwrap());
    }

    #[test]
    fn set_proposal_status_returns_false_when_discussion_md_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!set_proposal_status_by_label(dir.path(), "Proposal A", "chosen").unwrap());
    }

    #[test]
    fn clear_proposal_next_question_blanks_line_for_active_proposal() {
        let dir = tempfile::tempdir().unwrap();
        let discussion_path = dir.path().join(DISCUSSION_RELATIVE_PATH);
        std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
        std::fs::write(&discussion_path, sample_discussion()).unwrap();

        let changed = clear_proposal_next_question(dir.path(), "Proposal A").unwrap();
        assert!(changed);
        let updated = std::fs::read_to_string(&discussion_path).unwrap();
        assert!(updated.contains("- Next Question:\n"));
        assert!(!updated.contains(
            "- Next Question: Should SessionStart or UserPromptSubmit surface the resume proposal?"
        ));
    }

    #[test]
    fn parse_helpers_fall_back_to_active_without_question() {
        let proposals = parse_proposals(
            r#"## Discussion TODO

### Proposal C - Resume fallback [active]
- Summary: Keep the proposal active.
- Next Question:

### Proposal D - Already chosen [chosen]
- Summary: Done.
"#,
        );

        assert_eq!(parse_status("active"), ProposalStatus::Active);
        assert_eq!(parse_status("unexpected"), ProposalStatus::Unknown);
        assert_eq!(
            parse_field_value("- Next Question: clarify resume state", "Next Question"),
            Some(Some("clarify resume state".to_string()))
        );
        assert_eq!(
            parse_field_value("- Next Question:", "Next Question"),
            Some(None)
        );
        assert_eq!(
            parse_any_field_value("- Evidence Gate: complete"),
            Some(("Evidence Gate".to_string(), Some("complete".to_string())))
        );

        let pending = select_pending_resume(&proposals).expect("pending proposal");
        assert_eq!(pending.proposal_label, "Proposal C");
        assert_eq!(pending.next_question, None);

        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_pending_resume(dir.path()).unwrap(), None);
    }

    #[test]
    fn discussion_stop_blocker_reports_exit_blockers_without_next_question() {
        let dir = tempfile::tempdir().unwrap();
        let discussion_path = dir.path().join(DISCUSSION_RELATIVE_PATH);
        std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
        std::fs::write(
            &discussion_path,
            "## Discussion TODO\n\n\
             Status: active\n\n\
             ### Proposal A - Evidence gap [active]\n\
             - Summary: Root cause is still hypothetical.\n\
             - Implementation Proof: crates/gwt/src/foo.rs inspected\n\
             - SPEC/Issue Proof: SPEC-1935 checked\n\
             - Gap Check Proof: scope/integration/failure/migration/verification checked\n\
             - Official Docs Proof: not-applicable: local-only behavior\n\
             - External Research Proof: not-applicable: local-only behavior\n\
             - Exit Blockers: root cause has no reproducer yet\n\
             - Next Question:\n\
             - Evidence Gate: open\n",
        )
        .unwrap();

        let pending = discussion_stop_blocker(dir.path())
            .unwrap()
            .expect("exit blocker should keep discussion active");
        assert_eq!(pending.proposal_label, "Proposal A");
        assert!(pending
            .next_question
            .as_deref()
            .unwrap_or("")
            .contains("Exit Blockers remain unresolved"));
    }

    #[test]
    fn discussion_stop_blocker_is_silent_when_evidence_gate_is_complete() {
        let dir = tempfile::tempdir().unwrap();
        let discussion_path = dir.path().join(DISCUSSION_RELATIVE_PATH);
        std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
        std::fs::write(
            &discussion_path,
            "## Discussion TODO\n\n\
             Status: active\n\n\
             ### Proposal A - Evidence complete [active]\n\
             - Summary: Evidence is complete.\n\
             - Implementation Proof: crates/gwt/src/discussion_resume.rs inspected and focused tests run\n\
             - SPEC/Issue Proof: SPEC-1935 spec/plan/tasks checked\n\
             - Gap Check Proof: scope/integration/failure/migration/verification checked\n\
             - Official Docs Proof: Claude Code hooks docs checked\n\
             - External Research Proof: not-applicable: local-only behavior\n\
             - Exit Blockers: none\n\
             - Next Question:\n\
             - Evidence Gate: complete\n",
        )
        .unwrap();

        assert_eq!(discussion_stop_blocker(dir.path()).unwrap(), None);
        assert_eq!(
            proposal_evidence_blocker_by_label(dir.path(), "Proposal A").unwrap(),
            None
        );
    }

    #[test]
    fn discussion_stop_blocker_does_not_fall_back_to_legacy_when_tasks_has_active_proposal() {
        let dir = tempfile::tempdir().unwrap();
        write_tasks_discussions(
            dir.path(),
            "## Discussion TODO\n\n\
             Status: active\n\n\
             ### Proposal A - Evidence complete [active]\n\
             - Summary: Evidence is complete.\n\
             - Implementation Proof: crates/gwt/src/discussion_resume.rs inspected and focused tests run\n\
             - SPEC/Issue Proof: SPEC-1935 spec/plan/tasks checked\n\
             - Gap Check Proof: scope/integration/failure/migration/verification checked\n\
             - Official Docs Proof: not-applicable: local-only behavior\n\
             - External Research Proof: not-applicable: local-only behavior\n\
             - Exit Blockers: none\n\
             - Next Question:\n\
             - Evidence Gate: complete\n",
        );
        write_legacy_discussion(
            dir.path(),
            "### Proposal B - Stale legacy question [active]\n\
             - Next Question: This stale legacy state must not block.\n",
        );

        assert_eq!(discussion_stop_blocker(dir.path()).unwrap(), None);
    }

    #[test]
    fn proposal_evidence_blocker_reports_missing_proofs() {
        let dir = tempfile::tempdir().unwrap();
        let discussion_path = dir.path().join(DISCUSSION_RELATIVE_PATH);
        std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
        std::fs::write(
            &discussion_path,
            "## Discussion TODO\n\n\
             Status: active\n\n\
             ### Proposal A - Missing proof [active]\n\
             - Summary: Missing proof.\n\
             - Exit Blockers: none\n\
             - Next Question:\n\
             - Evidence Gate: complete\n",
        )
        .unwrap();

        let blocker = proposal_evidence_blocker_by_label(dir.path(), "Proposal A")
            .unwrap()
            .expect("missing proof should block resolve");
        assert!(blocker.contains("Implementation Proof"));
    }

    #[test]
    fn proposal_evidence_blocker_rejects_not_applicable_without_reason() {
        let dir = tempfile::tempdir().unwrap();
        let discussion_path = dir.path().join(DISCUSSION_RELATIVE_PATH);
        std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
        std::fs::write(
            &discussion_path,
            "## Discussion TODO\n\n\
             Status: active\n\n\
             ### Proposal A - Empty not applicable [active]\n\
             - Summary: Missing proof reason.\n\
             - Implementation Proof: crates/gwt/src/discussion_resume.rs inspected\n\
             - SPEC/Issue Proof: SPEC-1935 checked\n\
             - Gap Check Proof: scope/integration/failure/migration/verification checked\n\
             - Official Docs Proof: not-applicable:\n\
             - External Research Proof: not-applicable: local-only behavior\n\
             - Exit Blockers: none\n\
             - Next Question:\n\
             - Evidence Gate: complete\n",
        )
        .unwrap();

        let blocker = proposal_evidence_blocker_by_label(dir.path(), "Proposal A")
            .unwrap()
            .expect("empty not-applicable reason should block resolve");
        assert!(blocker.contains("Official Docs Proof"));
    }
}
