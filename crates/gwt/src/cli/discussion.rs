use std::{
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
            let path =
                update_discussion_entry(env.repo_path(), &update).map_err(io_as_spec_error)?;
            out.push_str(&format!("discussion updated: {}\n", path.display()));
            Ok(0)
        }
    }
}

/// Moves a legacy `tasks/discussions.md` into the repo-local
/// `.gwt/work/discussions.md` location when the new file does not yet exist.
/// Idempotent — returns `Ok(true)` only when a move happened.
///
/// SPEC-2359 Phase W-12: the discussion log moved out of the untracked
/// `tasks/` directory into the git-tracked `.gwt/work/` directory.
pub fn migrate_legacy_discussions_file(repo_root: &Path) -> std::io::Result<bool> {
    let new_path = gwt_core::paths::gwt_repo_local_discussions_path(repo_root);
    if new_path.exists() {
        return Ok(false);
    }
    let legacy = repo_root.join("tasks").join("discussions.md");
    if !legacy.exists() {
        return Ok(false);
    }
    if let Some(parent) = new_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(&legacy, &new_path)?;
    Ok(true)
}

fn update_discussion_entry(
    repo_root: &Path,
    update: &DiscussionUpdateCommand,
) -> std::io::Result<PathBuf> {
    migrate_legacy_discussions_file(repo_root)?;
    let path = gwt_core::paths::gwt_repo_local_discussions_path(repo_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    ensure_discussions_file(&path)?;

    let mut content = fs::read_to_string(&path)?;
    let date = update
        .date
        .clone()
        .unwrap_or_else(|| Local::now().format("%Y-%m-%d").to_string());
    let heading = format!("## {date} — {}", update.title);
    let entry = format_discussion_entry(&date, update);
    content = replace_or_append_section(&content, &heading, &entry);
    fs::write(&path, content)?;
    Ok(path)
}

fn ensure_discussions_file(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    fs::write(path, DEFAULT_DISCUSSIONS_HEADER)
}

fn format_discussion_entry(date: &str, update: &DiscussionUpdateCommand) -> String {
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
    format!(
        "## {date} — {title}\n\nStatus: {status}\nTopics: {topics}\nRelated SPECs: {related_specs}\nRelated Works: {related_works}\nPromoted To: {promoted_to}\n\nSummary:\n{summary}\n\nDecisions:\n{decisions}\n\nOpen Questions:\n{open_questions}\n\nNext:\n{next}\n",
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

fn io_as_spec_error(err: std::io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}
