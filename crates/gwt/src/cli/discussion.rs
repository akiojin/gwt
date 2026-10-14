use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use chrono::{Local, NaiveDate};
use gwt_github::{client::ApiError, SpecOpsError};

use super::{CliEnv, CliParseError};

const DEFAULT_DISCUSSIONS_HEADER: &str = "# Discussions\n\nThis file is the canonical gwt discussion log. Entries are updated in place while active and indexed by the `discussions` semantic scope.\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscussionCommand {
    Update(DiscussionUpdateCommand),
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

fn valid_status(value: &str) -> Result<String, CliParseError> {
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
            // Issue #3465: stamp the owning session at the edge so the
            // project-scoped discussion log stays shareable while the Stop
            // gate can tell whose discussion an entry is.
            let origin_session = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
                .ok()
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty());
            let path = update_discussion_entry(env.repo_path(), &update, origin_session.as_deref())
                .map_err(io_as_spec_error)?;
            out.push_str(&format!("discussion updated: {}\n", path.display()));
            Ok(0)
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
    origin_session: Option<&str>,
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
        let entry = format_discussion_entry(&date, update, origin_session);
        content = replace_or_append_section(&content, &heading, &entry);
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
    origin_session: Option<&str>,
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
    let origin_session = origin_session
        .map(|id| {
            format!(
                "{field}: {id}\n",
                field = crate::discussion_resume::ORIGIN_SESSION_FIELD
            )
        })
        .unwrap_or_default();
    format!(
        "## {date} — {title}\n\nStatus: {status}\n{origin_session}Topics: {topics}\nRelated SPECs: {related_specs}\nRelated Works: {related_works}\nPromoted To: {promoted_to}\n\nSummary:\n{summary}\n\nDecisions:\n{decisions}\n\nOpen Questions:\n{open_questions}\n\nNext:\n{next}\n",
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
    let lines: Vec<&str> = content.lines().collect();
    let Some(start_line) = lines.iter().position(|line| line.trim_end() == heading) else {
        let mut output = content.trim_end().to_string();
        output.push_str("\n\n");
        output.push_str(entry.trim_end());
        output.push('\n');
        return output;
    };
    // Issue #5075: the entry ends at the next discussion entry heading, not at
    // any `## ` line. A summary may carry its own `## Discussion TODO`, and
    // splitting there left the old proposals behind as stale duplicates.
    let end_line = crate::discussion_resume::discussion_entry_heading_indices(&lines)
        .into_iter()
        .find(|index| *index > start_line)
        .unwrap_or(lines.len());
    let line_offset =
        |line: usize| -> usize { content.split_inclusive('\n').take(line).map(str::len).sum() };
    let start = line_offset(start_line);
    let next = line_offset(end_line);
    // Issue #4434: proposals are `### Proposal ...` blocks stored *inside*
    // the entry being replaced. Carry them over, or the update silently
    // drops them along with the fields it is refreshing.
    let entry = preserve_existing_proposal_blocks(entry, &content[start..next]);
    let mut output = String::new();
    output.push_str(content[..start].trim_end());
    output.push_str("\n\n");
    output.push_str(entry.trim_end());
    output.push('\n');
    output.push_str(content[next..].trim_start_matches('\n'));
    output
}

/// Re-attaches the proposal blocks of the entry being replaced to the freshly
/// formatted entry, so they keep following their own discussion fields.
///
/// Issue #5075: a proposal is identified by its label and title. One that the
/// new entry re-sends replaces its stored block but keeps the stored status
/// (status changes belong to `discuss.*`), and stored copies of one proposal
/// collapse into the first.
fn preserve_existing_proposal_blocks(entry: &str, existing_section: &str) -> String {
    let Some(blocks) = proposal_blocks(existing_section) else {
        return entry.to_string();
    };
    let mut stored_status = HashMap::new();
    let mut kept = Vec::new();
    for block in split_proposal_blocks(blocks) {
        match proposal_identity(block) {
            Some((key, status)) => {
                if !stored_status.contains_key(&key) {
                    stored_status.insert(key.clone(), status);
                    kept.push((Some(key), block));
                }
            }
            None => kept.push((None, block)),
        }
    }
    let mut resent = HashSet::new();
    let mut output: String = entry
        .split_inclusive('\n')
        .map(|line| {
            let Some((key, _)) = proposal_identity(line) else {
                return line.to_string();
            };
            let status = stored_status.get(&key).copied();
            resent.insert(key);
            status
                .and_then(|status| {
                    crate::discussion_resume::replace_trailing_status_tag(line, status)
                })
                .unwrap_or_else(|| line.to_string())
        })
        .collect();
    output.truncate(output.trim_end().len());
    for (key, block) in kept {
        if key.is_some_and(|key| resent.contains(&key)) {
            continue;
        }
        output.push_str("\n\n");
        output.push_str(block.trim());
    }
    output.push('\n');
    output
}

/// Returns the tail of `section` starting at its first `### Proposal ` heading.
fn proposal_blocks(section: &str) -> Option<&str> {
    let mut offset = 0;
    for line in section.split_inclusive('\n') {
        if line.trim_start().starts_with("### Proposal ") {
            return Some(&section[offset..]);
        }
        offset += line.len();
    }
    None
}

/// Splits proposal blocks so that each one starts at its own header.
fn split_proposal_blocks(blocks: &str) -> Vec<&str> {
    let mut starts = Vec::new();
    let mut offset = 0;
    for line in blocks.split_inclusive('\n') {
        if line.trim_start().starts_with("### Proposal ") {
            starts.push(offset);
        }
        offset += line.len();
    }
    let ends = starts.iter().skip(1).copied().chain([blocks.len()]);
    starts
        .iter()
        .zip(ends)
        .map(|(start, end)| &blocks[*start..end])
        .collect()
}

/// `(label + title, status)` of the `### Proposal <label> - <title> [status]`
/// header on the first line of `text`.
fn proposal_identity(text: &str) -> Option<(String, &str)> {
    let header = text.lines().next()?.trim();
    let head = header.strip_prefix("### ")?;
    if !head.starts_with("Proposal ") {
        return None;
    }
    let (head, status) = head.rsplit_once('[')?;
    let status = status.strip_suffix(']')?.trim();
    let (label, title) = head.trim().split_once(" - ")?;
    let key = format!("{}\n{}", label.trim().to_ascii_lowercase(), title.trim());
    Some((key, status))
}

fn io_as_spec_error(err: std::io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedGwtHome;

    fn update_command(summary: &str) -> DiscussionUpdateCommand {
        DiscussionUpdateCommand {
            date: Some("2026-09-16".to_string()),
            title: "Proposal survival across updates".to_string(),
            status: "active".to_string(),
            topics: Vec::new(),
            related_specs: Vec::new(),
            related_works: Vec::new(),
            promoted_to: Vec::new(),
            summary: summary.to_string(),
            decisions: Vec::new(),
            open_questions: Vec::new(),
            next: "Continue".to_string(),
        }
    }

    /// Issue #4434 (AC-3): a proposal that survives the entry replacement has
    /// to stay where `discussion_resume` looks for it, so the resume prompt
    /// still picks it up after the update.
    #[test]
    fn preserved_proposal_stays_visible_to_discussion_resume() {
        let repo = tempfile::tempdir().expect("repo");
        let _home = ScopedGwtHome::set(repo.path().join("gwt-home"));

        let path = update_discussion_entry(repo.path(), &update_command("Initial summary"), None)
            .expect("first update");
        let mut seeded = fs::read_to_string(&path).expect("read discussions");
        seeded.push_str(
            "\n### Proposal A - Preserve proposals on update [active]\n\
             - Implementation Proof: pending\n\
             - Next Question: Keep this proposal while refreshing the summary?\n",
        );
        fs::write(&path, seeded).expect("append proposal");

        update_discussion_entry(repo.path(), &update_command("Updated summary"), None)
            .expect("second update");

        let pending = crate::discussion_resume::load_pending_resume(repo.path())
            .expect("load pending resume")
            .expect("preserved proposal must still be a resume candidate");
        assert_eq!(pending.proposal_label, "Proposal A");
        assert_eq!(pending.proposal_title, "Preserve proposals on update");
        assert_eq!(
            pending.next_question.as_deref(),
            Some("Keep this proposal while refreshing the summary?")
        );
    }

    const RESENT_SUMMARY: &str = "Width slice

## Discussion TODO

### Proposal A - Preserve the minimum [active]
- Implementation Proof: crates/gwt/web/app.js:10
- Exit Blockers: none
";

    fn proposal_headers(content: &str, header: &str) -> usize {
        content
            .lines()
            .filter(|line| line.trim_start().starts_with(header))
            .count()
    }

    /// Issue #5075 (AC-1/AC-2): re-sending the same proposal in the summary
    /// replaces it instead of stacking copies, so a title + origin resolve
    /// still finds exactly one candidate.
    #[test]
    fn resending_a_proposal_in_the_summary_does_not_duplicate_it() {
        let repo = tempfile::tempdir().expect("repo");
        let _home = ScopedGwtHome::set(repo.path().join("gwt-home"));

        update_discussion_entry(repo.path(), &update_command(RESENT_SUMMARY), Some("s1"))
            .expect("first update");
        let resent = RESENT_SUMMARY.replace("app.js:10", "app.js:42");
        let path = update_discussion_entry(repo.path(), &update_command(&resent), Some("s1"))
            .expect("second update");
        update_discussion_entry(repo.path(), &update_command(&resent), Some("s1"))
            .expect("third update");

        let content = fs::read_to_string(&path).expect("read discussions");
        assert_eq!(
            proposal_headers(&content, "### Proposal A - Preserve the minimum"),
            1,
            "{content}"
        );
        assert!(content.contains("app.js:42"), "{content}");
        assert!(!content.contains("app.js:10"), "{content}");

        let target = crate::discussion_resume::ProposalTarget {
            title: "Preserve the minimum".to_string(),
            origin_session: Some("s1".to_string()),
        };
        let updated = crate::discussion_resume::set_proposal_status_by_label(
            repo.path(),
            "Proposal A",
            "parked",
            Some("s1"),
            Some(&target),
        )
        .expect("resolve must not be ambiguous");
        assert!(updated.is_some());
    }

    /// Issue #5075 (AC-3): a summary update keeps other sessions' entries,
    /// other proposals, and the resolved state of a re-sent proposal.
    #[test]
    fn summary_update_keeps_other_entries_proposals_and_resolved_state() {
        let repo = tempfile::tempdir().expect("repo");
        let _home = ScopedGwtHome::set(repo.path().join("gwt-home"));

        let path =
            update_discussion_entry(repo.path(), &update_command(RESENT_SUMMARY), Some("s1"))
                .expect("first update");
        let seeded = fs::read_to_string(&path)
            .expect("read discussions")
            .replace(
                "Preserve the minimum [active]",
                "Preserve the minimum [chosen]",
            )
            + "
### Proposal B - Other option [parked]
- Implementation Proof: other.rs:1

## 2026-09-17 — Foreign entry

Status: active
Origin Session: s2

### Proposal A - Preserve the minimum [active]
- Implementation Proof: foreign.rs:1
";
        fs::write(&path, seeded).expect("seed");

        update_discussion_entry(repo.path(), &update_command(RESENT_SUMMARY), Some("s1"))
            .expect("second update");

        let content = fs::read_to_string(&path).expect("read discussions");
        assert_eq!(
            proposal_headers(&content, "### Proposal A - Preserve the minimum [chosen]"),
            1,
            "{content}"
        );
        assert_eq!(
            proposal_headers(&content, "### Proposal A - Preserve the minimum [active]"),
            1,
            "foreign entry proposal must survive: {content}"
        );
        assert!(
            content.contains(
                "### Proposal B - Other option [parked]
- Implementation Proof: other.rs:1"
            ),
            "{content}"
        );
        assert!(content.contains("Origin Session: s2"), "{content}");
        assert!(content.contains("foreign.rs:1"), "{content}");
    }

    /// Issue #5075 (AC-4): an entry already holding duplicate copies of the
    /// same proposal is normalized to one copy on its next update.
    #[test]
    fn summary_update_normalizes_existing_duplicate_proposals() {
        let repo = tempfile::tempdir().expect("repo");
        let _home = ScopedGwtHome::set(repo.path().join("gwt-home"));

        let path = update_discussion_entry(repo.path(), &update_command("Plain"), Some("s1"))
            .expect("first update");
        let block = "
### Proposal A - Preserve the minimum [active]
- Implementation Proof: a.rs:1
";
        let seeded = fs::read_to_string(&path).expect("read") + block + block + block;
        fs::write(&path, seeded).expect("seed duplicates");

        update_discussion_entry(repo.path(), &update_command("Plain again"), Some("s1"))
            .expect("second update");

        let content = fs::read_to_string(&path).expect("read discussions");
        assert_eq!(
            proposal_headers(&content, "### Proposal A - Preserve the minimum [active]"),
            1,
            "{content}"
        );
        assert!(content.contains("a.rs:1"), "{content}");
    }
}
