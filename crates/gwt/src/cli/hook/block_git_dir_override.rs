//! `gwtd hook block-git-dir-override` — PreToolUse hook that forbids
//! overriding Git selection or writing repository identity configuration
//! through an agent's shell command.
//!
//! Ported from the retired external git-dir policy hook.

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use regex::Regex;

use super::{HookError, HookEvent, HookOutput};

pub fn evaluate_bash_command(command: &str) -> Option<HookOutput> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    evaluate_bash_command_in(command, &cwd)
}

pub(super) fn evaluate_bash_command_in(command: &str, worktree_root: &Path) -> Option<HookOutput> {
    if re_git_dir().is_match(command) {
        return Some(HookOutput::pre_tool_use_permission(
            "\u{1F6AB} GIT_DIR environment variable override is not allowed",
            format!(
                "Modifying GIT_DIR in a worktree environment can cause unintended repository \
                 operations.\n\nBlocked command: {command}\n\nWorktrees have their own .git file \
                 pointing to the main repository worktree directory. Overriding GIT_DIR may \
                 break this relationship and cause git commands to operate on the wrong \
                 repository."
            ),
        ));
    }
    if re_git_work_tree().is_match(command) {
        return Some(HookOutput::pre_tool_use_permission(
            "\u{1F6AB} GIT_WORK_TREE environment variable override is not allowed",
            format!(
                "Modifying GIT_WORK_TREE in a worktree environment can cause unintended \
                 repository operations.\n\nBlocked command: {command}\n\nWorktrees have their \
                 own working directory configuration. Overriding GIT_WORK_TREE may cause git \
                 commands to operate on the wrong directory."
            ),
        ));
    }
    for segment in super::segments::split_command_segments(command) {
        let Ok(words) = crate::cli::verification_record::split_command_line(&segment) else {
            continue;
        };
        // Match the existing policy's leading shell keywords, env wrapper,
        // and assignments after quote-aware tokenization.
        let start = words
            .iter()
            .position(|word| {
                !matches!(word.as_str(), "do" | "then" | "env")
                    && !word.split_once('=').is_some_and(|(key, _)| {
                        !key.is_empty()
                            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    })
            })
            .unwrap_or(words.len());
        let words = &words[start..];
        if let Some(body) = shell_body(words) {
            if let Some(decision) = evaluate_bash_command_in(body, worktree_root) {
                return Some(decision);
            }
        }
        if writes_repository_identity(words, worktree_root) {
            return Some(HookOutput::pre_tool_use_permission(
                "Repository identity configuration writes are not allowed",
                format!(
                    "Changing core.bare or core.worktree can corrupt the shared repository and affect every worktree.\n\n\
Read the current configuration with `git config --show-origin --get core.bare` and `git config --show-origin --get core.worktree`. \
Use `git -C <worktree>` for ordinary Git operations, or `git config --file <fixture-config>` for isolated test configuration. \
If the shared configuration is already damaged, stop Git writes and report the diagnostic to the PM using JSON operation `board.post`; request explicit repository repair or migration before retrying.\n\n\
Blocked command: {command}"
                ),
            ));
        }
    }
    None
}

pub fn evaluate(event: &HookEvent) -> Result<HookOutput, HookError> {
    if !matches!(
        event.tool_name.as_deref(),
        Some("Bash" | "exec_command" | "functions.exec_command")
    ) {
        return Ok(HookOutput::Silent);
    }
    let Some(command) = event.command() else {
        return Ok(HookOutput::Silent);
    };
    let fallback = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    Ok(
        evaluate_bash_command_in(command, &event.command_cwd(&fallback))
            .unwrap_or(HookOutput::Silent),
    )
}

fn command_name(word: &str) -> &str {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    name.strip_suffix(".exe").unwrap_or(name)
}

/// Inspect an actual shell wrapper, never a quoted argument to `echo`.
fn shell_body(words: &[String]) -> Option<&str> {
    let mut shell = 0;
    if command_name(words.first()?) == "wsl" {
        shell = words
            .iter()
            .position(|word| matches!(word.as_str(), "-e" | "--exec"))?
            + 1;
    }
    if !matches!(command_name(words.get(shell)?), "bash" | "sh" | "zsh") {
        return None;
    }
    let flag = words.get(shell + 1)?;
    if matches!(flag.as_str(), "-c" | "-lc" | "-cl") {
        words.get(shell + 2).map(String::as_str)
    } else {
        None
    }
}

fn writes_repository_identity(words: &[String], root: &Path) -> bool {
    if words.first().is_none_or(|word| command_name(word) != "git") {
        return false;
    }
    let mut cwd = root.to_path_buf();
    let mut git_dir = None;
    let mut index = 1;
    while let Some(word) = words.get(index) {
        match word.as_str() {
            "config" => break,
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--config-env" => {
                let Some(value) = words.get(index + 1) else {
                    return false;
                };
                if word == "-C" {
                    cwd = cwd.join(value);
                } else if word == "--git-dir" {
                    git_dir = Some(cwd.join(value));
                }
                index += 2;
            }
            _ if word.starts_with("--git-dir=") => {
                git_dir = Some(cwd.join(&word[10..]));
                index += 1;
            }
            _ if word.starts_with('-') => index += 1,
            _ => return false,
        }
    }
    if words.get(index).map(String::as_str) != Some("config") {
        return false;
    }

    let mut file = None;
    let mut positional = Vec::new();
    let mut mutating = false;
    index += 1;
    while let Some(word) = words.get(index) {
        match word.as_str() {
            "get" | "list" if positional.is_empty() => return false,
            "set" | "unset" if positional.is_empty() => mutating = true,
            "--get" | "--get-all" | "--get-regexp" | "--get-urlmatch" | "--get-color"
            | "--get-colorbool" | "--list" | "-l" => return false,
            "--add" | "--replace-all" | "--unset" | "--unset-all" => mutating = true,
            "--file" | "-f" | "--type" | "--default" | "--comment" | "--value" => {
                let Some(value) = words.get(index + 1) else {
                    return false;
                };
                if matches!(word.as_str(), "--file" | "-f") {
                    file = Some(cwd.join(value));
                }
                index += 1;
            }
            _ if word.starts_with("--file=") => file = Some(cwd.join(&word[7..])),
            _ if word.starts_with('-') => {}
            _ => positional.push(word.as_str()),
        }
        index += 1;
    }
    if !positional.first().is_some_and(|key| {
        key.eq_ignore_ascii_case("core.bare") || key.eq_ignore_ascii_case("core.worktree")
    }) || (!mutating && positional.len() < 2)
    {
        return false;
    }
    file.is_none_or(|file| is_repository_config(&file, git_dir.as_deref().unwrap_or(&cwd)))
}

fn is_repository_config(file: &Path, root: &Path) -> bool {
    let canonical = |path: &Path| dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let file = canonical(file);
    if gwt_core::repo_hash::repository_common_dir(root)
        .is_some_and(|common| file == canonical(&common.join("config")))
    {
        return true;
    }
    // An explicitly selected external bare repository is shared as well.
    file.file_name().is_some_and(|name| name == "config")
        && file
            .parent()
            .is_some_and(|parent| parent.join("HEAD").is_file() && parent.join("refs").is_dir())
}

pub fn handle() -> Result<HookOutput, HookError> {
    let Some(event) = HookEvent::read_from_stdin()? else {
        return Ok(HookOutput::Silent);
    };
    evaluate(&event)
}

fn re_git_dir() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| {
        Regex::new(
            r"(^|[;&|]|\s)(export\s+)?GIT_DIR\s*=|env\s+[^;]*GIT_DIR\s*=|declare\s+-x\s+GIT_DIR\s*=",
        )
        .unwrap()
    })
}

fn re_git_work_tree() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| {
        Regex::new(
            r"(^|[;&|]|\s)(export\s+)?GIT_WORK_TREE\s*=|env\s+[^;]*GIT_WORK_TREE\s*=|declare\s+-x\s+GIT_WORK_TREE\s*=",
        )
        .unwrap()
    })
}
