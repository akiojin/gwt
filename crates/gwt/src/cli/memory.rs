use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{Local, NaiveDate};
use gwt_github::{client::ApiError, SpecOpsError};

use super::{CliEnv, CliParseError};

const DEFAULT_MEMORY_HEADER: &str = "# Memory\n\nThis file is the canonical gwt memory log. New entries should use\n`Type`, `Context`, `Learning`, and `Future Action` fields.\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryCommand {
    Add(MemoryAddCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryAddCommand {
    pub date: Option<String>,
    pub memory_type: String,
    pub title: String,
    pub context: String,
    pub learning: String,
    pub future_action: String,
}

pub fn parse(args: &[String]) -> Result<MemoryCommand, CliParseError> {
    let (head, rest) = args.split_first().ok_or(CliParseError::Usage)?;
    match head.as_str() {
        "add" => parse_add(rest).map(MemoryCommand::Add),
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

fn parse_add(args: &[String]) -> Result<MemoryAddCommand, CliParseError> {
    let mut date = None;
    let mut memory_type = Some("lesson".to_string());
    let mut title = None;
    let mut context = None;
    let mut learning = None;
    let mut future_action = None;
    let mut i = 0;

    while i < args.len() {
        let flag = args[i].as_str();
        let value = args
            .get(i + 1)
            .ok_or(CliParseError::MissingFlag(flag_name(flag)?))?;
        match flag {
            "--date" => date = Some(non_empty("--date", value)?),
            "--type" => memory_type = Some(valid_memory_type(value)?),
            "--title" => title = Some(non_empty("--title", value)?),
            "--context" => context = Some(non_empty("--context", value)?),
            "--learning" => learning = Some(non_empty("--learning", value)?),
            "--future-action" => future_action = Some(non_empty("--future-action", value)?),
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 2;
    }

    if let Some(value) = &date {
        validate_date(value)?;
    }

    Ok(MemoryAddCommand {
        date,
        memory_type: memory_type.unwrap_or_else(|| "lesson".to_string()),
        title: title.ok_or(CliParseError::MissingFlag("--title"))?,
        context: context.ok_or(CliParseError::MissingFlag("--context"))?,
        learning: learning.ok_or(CliParseError::MissingFlag("--learning"))?,
        future_action: future_action.ok_or(CliParseError::MissingFlag("--future-action"))?,
    })
}

fn flag_name(flag: &str) -> Result<&'static str, CliParseError> {
    match flag {
        "--date" => Ok("--date"),
        "--type" => Ok("--type"),
        "--title" => Ok("--title"),
        "--context" => Ok("--context"),
        "--learning" => Ok("--learning"),
        "--future-action" => Ok("--future-action"),
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

fn valid_memory_type(value: &str) -> Result<String, CliParseError> {
    let value = non_empty("--type", value)?;
    match value.as_str() {
        "lesson" | "decision" | "workflow" | "failure-pattern" => Ok(value),
        _ => Err(CliParseError::InvalidValue {
            flag: "--type",
            reason: "must be lesson, decision, workflow, or failure-pattern",
        }),
    }
}

fn validate_date(value: &str) -> Result<(), CliParseError> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map(|_| ())
        .map_err(|_| CliParseError::InvalidValue {
            flag: "--date",
            reason: "must be YYYY-MM-DD",
        })
}

pub fn run<E: CliEnv>(
    env: &mut E,
    command: MemoryCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match command {
        MemoryCommand::Add(add) => {
            let path = append_memory_entry(env.repo_path(), &add).map_err(io_as_spec_error)?;
            out.push_str(&format!("memory updated: {}\n", path.display()));
            Ok(0)
        }
    }
}

/// Imports legacy memory sources (repo-local `.gwt/work/memory.md`,
/// `tasks/memory.md`, `tasks/lessons.md`) into the machine-local home
/// work-notes file when it does not yet exist. Idempotent — returns
/// `Ok(true)` only when an import happened.
///
/// SPEC-3214 (FR-007): project memory moved out of the git-tracked
/// repo-local `.gwt/work/` directory into the branch-independent home
/// scratch (`~/.gwt/projects/<repo-hash>/work-notes/`).
pub fn migrate_legacy_memory_file(repo_root: &Path) -> std::io::Result<bool> {
    crate::work_notes::migrate_memory_into_home(repo_root)
}

fn append_memory_entry(repo_root: &Path, add: &MemoryAddCommand) -> std::io::Result<PathBuf> {
    let path = gwt_core::paths::gwt_work_notes_memory_path(repo_root);
    crate::work_notes::with_work_notes_lock(repo_root, || {
        crate::work_notes::migrate_memory_into_home(repo_root)?;
        ensure_memory_file(&path)?;

        let date = add
            .date
            .clone()
            .unwrap_or_else(|| Local::now().format("%Y-%m-%d").to_string());
        let entry = format_memory_entry(&date, add);
        let mut file = OpenOptions::new().append(true).open(&path)?;
        file.write_all(entry.as_bytes())
    })?;
    Ok(path)
}

fn ensure_memory_file(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    fs::write(path, DEFAULT_MEMORY_HEADER)
}

fn format_memory_entry(date: &str, add: &MemoryAddCommand) -> String {
    format!(
        "\n## {date} — {title}\n\nType: {memory_type}\nContext: {context}\nLearning: {learning}\nFuture Action: {future_action}\n",
        title = add.title,
        memory_type = add.memory_type,
        context = add.context,
        learning = add.learning,
        future_action = add.future_action,
    )
}

fn io_as_spec_error(err: std::io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}
