use std::io;
use std::path::Path;

use gwt_agent::PendingDiscussionResume;

pub const DISCUSSION_RELATIVE_PATH: &str = ".gwt/discussion.md";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResumePromptSessionState {
    pub last_source_event: Option<String>,
    pub saw_session_start: bool,
    pub fallback_armed: bool,
    pub prompt_pending: bool,
    pub last_handled_proposal: Option<String>,
}

pub fn load_pending_resume(worktree: &Path) -> io::Result<Option<PendingDiscussionResume>> {
    let discussion_path = worktree.join(DISCUSSION_RELATIVE_PATH);
    if !discussion_path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(discussion_path)?;
    let proposals = parse_proposals(&content);
    Ok(select_pending_resume(&proposals))
}

pub fn park_pending_resume(worktree: &Path, pending: &PendingDiscussionResume) -> io::Result<bool> {
    let discussion_path = worktree.join(DISCUSSION_RELATIVE_PATH);
    if !discussion_path.exists() {
        return Ok(false);
    }

    let content = std::fs::read_to_string(&discussion_path)?;
    let proposals = parse_proposals(&content);
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
    let rewritten = lines.join("\n");
    let final_content = if content.ends_with('\n') {
        format!("{rewritten}\n")
    } else {
        rewritten
    };
    std::fs::write(discussion_path, final_content)?;
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
        "Use gwt-discussion to resume the unfinished discussion from `.gwt/discussion.md`.\nFocus on {} - {}.{}\nContinue the discussion before returning an Action Bundle.\n",
        pending.proposal_label, pending.proposal_title, next_question
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProposalStatus {
    Active,
    Parked,
    Rejected,
    Chosen,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedProposal {
    header_line_index: usize,
    label: String,
    title: String,
    status: ProposalStatus,
    next_question: Option<String>,
}

fn parse_proposals(content: &str) -> Vec<ParsedProposal> {
    let mut proposals: Vec<ParsedProposal> = Vec::new();

    for (index, raw_line) in content.lines().enumerate() {
        let trimmed = raw_line.trim();
        if !trimmed.starts_with("### Proposal ") {
            if let Some(current) = proposals.last_mut() {
                if let Some(question) = parse_field_value(trimmed, "Next Question") {
                    current.next_question = question;
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
        });
    }

    proposals
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_discussion() -> &'static str {
        r#"## Discussion TODO

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
}
