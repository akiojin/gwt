//! `gwtd hook block-file-ops` — PreToolUse hook that forbids file-system
//! mutations outside the current worktree and temporary artifact directories.
//!
//! This recognizes shell file commands, not arbitrary interpreter I/O or output
//! redirections; it is a workflow guard, not an OS sandbox (Issue #4319).

use std::path::{Path, PathBuf};

use super::{segments::split_command_segments, HookError, HookEvent, HookOutput};

const FILE_OPS: &[&str] = &["mkdir", "rmdir", "rm", "touch", "cp", "mv"];

pub fn evaluate_bash_command(command: &str, worktree_root: &Path) -> Option<HookOutput> {
    let temporary_roots = temporary_roots();
    for segment in split_command_segments(command) {
        if segment_starts_with_file_op(&segment).is_none() {
            continue;
        }
        for file_path in extract_file_paths(&segment) {
            if file_path.is_empty() {
                continue;
            }
            if !is_allowed_path(&file_path, worktree_root, &temporary_roots) {
                return Some(HookOutput::pre_tool_use_permission(
                    "\u{1F6AB} File operations outside worktree or temporary artifacts are not allowed",
                    format!(
                        "Use the worktree or a child of an allowed temporary directory \
                         (including the session scratchpad). Temporary roots themselves are protected.\n\nWorktree \
                         root: {root}\nTarget path: {target}\nBlocked command: {command}\n\n\
                         Allowed temporary directories: {temporary}\n\
                         Preserve a temporary log with: cp \"{temp}/failure.log\" \"{temp}/failure.saved.log\"\n\
                         For artifacts elsewhere, use the tool's output option to write into the \
                         worktree or temporary scratchpad. Existing external artifacts require \
                         separately authorized access; do not bypass this guard with another syntax.",
                        root = worktree_root.display(),
                        target = file_path,
                        temporary = temporary_roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "),
                        temp = temporary_roots.first().map_or(worktree_root, PathBuf::as_path).display(),
                    ),
                ));
            }
        }
    }
    None
}

pub fn evaluate(event: &HookEvent, worktree_root: &Path) -> Result<HookOutput, HookError> {
    if event.tool_name.as_deref() != Some("Bash") {
        return Ok(HookOutput::Silent);
    }
    let Some(command) = event.command() else {
        return Ok(HookOutput::Silent);
    };
    Ok(evaluate_bash_command(command, worktree_root).unwrap_or(HookOutput::Silent))
}

pub fn handle() -> Result<HookOutput, HookError> {
    let Some(event) = HookEvent::read_from_stdin()? else {
        return Ok(HookOutput::Silent);
    };
    let root = crate::cli::hook::worktree::detect_worktree_root();
    evaluate(&event, &root)
}

fn segment_starts_with_file_op(segment: &str) -> Option<&'static str> {
    let first = segment.split_whitespace().next()?;
    FILE_OPS.iter().copied().find(|&op| first == op)
}

/// Preserve whole quoted operands, including spaces in artifact paths. Complex
/// shell expressions remain unresolved and do not gain a temporary exception.
fn extract_file_paths(segment: &str) -> Vec<String> {
    static WORDS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let copy_or_move = matches!(segment_starts_with_file_op(segment), Some("cp" | "mv"));
    WORDS
        .get_or_init(|| regex::Regex::new(r#"(?:[^\s'"]+|'[^']*'|"[^"]*")+"#).unwrap())
        .find_iter(segment)
        .map(|m| m.as_str())
        .skip(1)
        .filter_map(|raw| {
            let token = unquote(raw);
            if !token.starts_with('-') {
                return Some(raw.to_string());
            }
            // Attached destination options write files just like positional operands.
            if !copy_or_move {
                return None;
            }
            let target = if token.starts_with("--") {
                let (flag, value) = token.split_once('=')?;
                (flag.len() > 2 && "--target-directory".starts_with(flag)).then_some(value)?
            } else {
                // GNU short flags may be clustered, e.g. `cp -rt/destination`.
                token
                    .split_once('t')
                    .map(|(_, value)| value)
                    .filter(|value| !value.is_empty())?
            };
            Some(if raw.starts_with('\'') {
                format!("'{target}'")
            } else {
                target.to_string()
            })
        })
        .collect()
}

fn unquote(target: &str) -> &str {
    target
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .or_else(|| target.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
        .unwrap_or(target)
}

fn is_allowed_path(target: &str, worktree_root: &Path, temporary_roots: &[PathBuf]) -> bool {
    let single_quoted = target.starts_with('\'');
    let target = unquote(target);
    if target.is_empty() || target.starts_with('~') || target.contains(['\'', '"', '`']) {
        return false;
    }
    let mut target_path = PathBuf::from(target);
    if !single_quoted {
        for name in ["TMPDIR", "TMP", "TEMP"] {
            for prefix in [format!("${name}"), format!("${{{name}}}")] {
                if let Some(suffix) = target.strip_prefix(&prefix) {
                    if suffix.is_empty() || suffix.starts_with('/') {
                        let Some(value) = std::env::var_os(name) else {
                            return false;
                        };
                        target_path = PathBuf::from(value).join(suffix.trim_start_matches('/'));
                    }
                }
            }
        }
    }
    if target_path.to_string_lossy().contains('$') {
        return false;
    }
    let abs = if target_path.is_absolute() {
        target_path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(target_path)
    };
    let Some(abs) = resolve_path(&abs) else {
        return false;
    };
    if temporary_roots.contains(&abs) {
        return false;
    }
    if resolve_path(worktree_root).is_some_and(|root| abs.starts_with(root)) {
        return true;
    }
    // Do not grant a new temp exception to shell expressions that may expand
    // to symlinks or different paths. Existing worktree glob behavior is retained.
    if abs.to_string_lossy().contains(['*', '?', '[', '{']) {
        return false;
    }
    #[cfg(unix)]
    if target.contains('\\') {
        return false;
    }
    temporary_roots.iter().any(|root| abs.starts_with(root))
}

fn temporary_roots() -> Vec<PathBuf> {
    let mut candidates = vec![std::env::temp_dir()];
    if let Some(path) = std::env::var_os("TMPDIR") {
        candidates.push(path.into());
    }
    #[cfg(unix)]
    candidates.extend([PathBuf::from("/tmp"), PathBuf::from("/private/tmp")]);
    candidates
        .into_iter()
        .filter(|path| path.is_absolute())
        .filter_map(|path| dunce::canonicalize(path).ok())
        .filter(|path| path.parent().is_some() && path.is_dir())
        .collect()
}

/// Resolve existing ancestors before appending missing names. Never normalize
/// `..` before resolving a symlink, and fail closed on dangling links or I/O
/// errors. Missing descendants are normal for mkdir/touch/cp destinations.
fn resolve_path(path: &Path) -> Option<PathBuf> {
    match dunce::canonicalize(path) {
        Ok(path) => Some(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if std::fs::symlink_metadata(path).is_ok() {
                return None;
            }
            let parent = resolve_path(path.parent()?)?;
            Some(parent.join(path.file_name()?))
        }
        Err(_) => None,
    }
}
