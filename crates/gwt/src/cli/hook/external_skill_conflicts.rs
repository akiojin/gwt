//! Bounded, read-only diagnosis of external instructions (Issue #3713).
//! Provenance and conflicts are advisory; neither is authorization evidence.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::Value;

use super::{block_bash_policy, HookEvent};

const ROOTS_FILE: &str = ".gwt/external-skill-roots.json";
const MAX_ROOTS: usize = 32;
const MAX_ENTRIES: usize = 512;
const MAX_FILES: usize = 64;
const MAX_FILE_BYTES: usize = 128 * 1024;
const MAX_TOTAL_BYTES: usize = 2 * 1024 * 1024;
const MAX_FINDINGS: usize = 64;
const MAX_DEPTH: usize = 6;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    Complete,
    Partial,
    Unavailable,
    #[default]
    NotConfigured,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictFinding {
    pub file: String,
    pub line: usize,
    pub command: String,
    pub alternative: String,
    pub plugin: String,
    pub skill: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ConflictReport {
    pub status: ScanStatus,
    pub inspected_files: usize,
    pub conflict_count: usize,
    pub files: Vec<String>,
    pub findings: Vec<ConflictFinding>,
    pub diagnostics: Vec<String>,
    #[serde(skip)]
    pub(crate) watched_paths: Vec<PathBuf>,
}

#[derive(Debug)]
struct ScanRoot {
    path: PathBuf,
    plugin: String,
}

/// Files that choose the inventory. The health cache also watches visited
/// directories and documents, so new/changed command files invalidate it.
pub(crate) fn discovery_paths(worktree: &Path) -> Vec<PathBuf> {
    let mut paths = vec![
        worktree.join(ROOTS_FILE),
        worktree.join(".claude/settings.json"),
        worktree.join(".claude/settings.local.json"),
    ];
    if let Some(home) = gwt_config::home::gwt_home_override().or_else(dirs::home_dir) {
        paths.push(home.join(".claude/settings.json"));
        paths.push(home.join(".claude/plugins/installed_plugins.json"));
    }
    paths
}

fn read_bounded(path: &Path, limit: usize) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("{}: symlink or non-file skipped", path.display()));
    }
    if metadata.len() > limit as u64 {
        return Err(format!("{}: byte limit reached", path.display()));
    }
    let mut text = String::new();
    fs::File::open(path)
        .and_then(|file| file.take(limit as u64 + 1).read_to_string(&mut text))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if text.len() > limit {
        return Err(format!("{}: byte limit reached", path.display()));
    }
    Ok(text)
}

fn read_json(path: &Path, limit: usize) -> Result<Value, String> {
    serde_json::from_str(&read_bounded(path, limit)?)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn discover_roots(worktree: &Path, report: &mut ConflictReport) -> Vec<ScanRoot> {
    let explicit = worktree.join(ROOTS_FILE);
    if fs::symlink_metadata(&explicit).is_ok() {
        let parsed = read_json(&explicit, MAX_FILE_BYTES).and_then(|v| {
            serde_json::from_value::<Vec<PathBuf>>(v)
                .map_err(|e| format!("{}: {e}", explicit.display()))
        });
        return match parsed {
            Ok(paths) => {
                if paths.len() > MAX_ROOTS {
                    report
                        .diagnostics
                        .push("Explicit root limit reached".into());
                }
                paths
                    .into_iter()
                    .take(MAX_ROOTS)
                    .map(|path| {
                        let path = worktree.join(path);
                        let plugin = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        ScanRoot { path, plugin }
                    })
                    .collect()
            }
            Err(error) => {
                report.diagnostics.push(error);
                Vec::new()
            }
        };
    }
    // Only enabled plugins and their effective install locations are read.
    // Never sweep the cache, which contains old/inactive plugin versions.
    let Some(home) = gwt_config::home::gwt_home_override().or_else(dirs::home_dir) else {
        return Vec::new();
    };
    let mut enabled = BTreeMap::new();
    for settings in [
        home.join(".claude/settings.json"),
        worktree.join(".claude/settings.json"),
        worktree.join(".claude/settings.local.json"),
    ] {
        if !settings.exists() {
            continue;
        }
        match read_json(&settings, MAX_FILE_BYTES) {
            Ok(value) => {
                if let Some(plugins) = value.get("enabledPlugins").and_then(Value::as_object) {
                    for (name, flag) in plugins {
                        enabled.insert(name.clone(), flag.as_bool().unwrap_or(false));
                    }
                }
            }
            Err(error) => report.diagnostics.push(error),
        }
    }
    enabled.retain(|_, enabled| *enabled);
    if enabled.is_empty() {
        return Vec::new();
    }
    let manifest = match read_json(
        &home.join(".claude/plugins/installed_plugins.json"),
        MAX_TOTAL_BYTES,
    ) {
        Ok(value) => value,
        Err(error) => {
            report.diagnostics.push(error);
            return Vec::new();
        }
    };
    let mut roots = Vec::new();
    for name in enabled.keys().take(MAX_ROOTS) {
        let entries = manifest
            .get("plugins")
            .and_then(|v| v.get(name))
            .and_then(Value::as_array);
        let selected = entries
            .into_iter()
            .flatten()
            .filter(|entry| {
                entry["scope"] == "user"
                    || entry
                        .get("projectPath")
                        .and_then(Value::as_str)
                        .is_some_and(|path| Path::new(path) == worktree)
            })
            .max_by_key(|entry| {
                (
                    entry["scope"] != "user",
                    entry["lastUpdated"].as_str().unwrap_or(""),
                )
            });
        if let Some(path) = selected
            .and_then(|entry| entry.get("installPath"))
            .and_then(Value::as_str)
        {
            let root = Path::new(path);
            // Keep absent instruction directories observable too: installing
            // a command later need not rewrite the plugin manifest.
            report.watched_paths.push(root.to_path_buf());
            for subdir in ["skills", "commands"] {
                let path = root.join(subdir);
                report.watched_paths.push(path.clone());
                if fs::symlink_metadata(&path).is_ok() {
                    roots.push(ScanRoot {
                        path,
                        plugin: name.clone(),
                    });
                }
            }
        } else {
            report.diagnostics.push(format!(
                "Enabled plugin {name}: install location unavailable"
            ));
        }
    }
    if enabled.len() > MAX_ROOTS {
        report
            .diagnostics
            .push("Enabled plugin limit reached".into());
    }
    roots
}

pub fn scan_worktree(worktree: &Path) -> ConflictReport {
    let mut report = ConflictReport {
        watched_paths: discovery_paths(worktree),
        ..ConflictReport::default()
    };
    let roots = discover_roots(worktree, &mut report);
    scan_roots(roots, report)
}

fn scan_roots(roots: Vec<ScanRoot>, mut report: ConflictReport) -> ConflictReport {
    let configured = !roots.is_empty();
    let mut visited = BTreeSet::new();
    let mut remaining_entries = MAX_ENTRIES;
    let mut remaining_bytes = MAX_TOTAL_BYTES;
    for root in roots {
        walk(
            &root.path,
            &root.plugin,
            0,
            &mut report,
            &mut visited,
            &mut remaining_entries,
            &mut remaining_bytes,
        );
    }
    report.files = report
        .findings
        .iter()
        .map(|f| f.file.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    report.conflict_count = report.findings.len();
    report.status = if !report.diagnostics.is_empty() {
        if report.inspected_files == 0 {
            ScanStatus::Unavailable
        } else {
            ScanStatus::Partial
        }
    } else if configured {
        ScanStatus::Complete
    } else {
        ScanStatus::NotConfigured
    };
    // A skipped symlink is a partial scan even when no regular file exists.
    if report.diagnostics.iter().any(|d| d.contains("symlink")) {
        report.status = ScanStatus::Partial;
    }
    report
}

fn walk(
    path: &Path,
    plugin: &str,
    depth: usize,
    report: &mut ConflictReport,
    visited: &mut BTreeSet<PathBuf>,
    remaining_entries: &mut usize,
    remaining_bytes: &mut usize,
) {
    if !visited.insert(path.to_path_buf()) {
        return;
    }
    report.watched_paths.push(path.to_path_buf());
    if *remaining_entries == 0 {
        report
            .diagnostics
            .push("Directory entry limit reached".into());
        return;
    }
    *remaining_entries -= 1;
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            report.diagnostics.push(format!("{}: {e}", path.display()));
            return;
        }
    };
    if metadata.file_type().is_symlink() {
        report
            .diagnostics
            .push(format!("{}: symlink skipped", path.display()));
        return;
    }
    if metadata.is_dir() {
        if depth >= MAX_DEPTH {
            report
                .diagnostics
                .push(format!("{}: depth limit reached", path.display()));
            return;
        }
        match fs::read_dir(path) {
            Ok(entries) => {
                let mut children = Vec::new();
                for entry in entries.take(remaining_entries.saturating_add(1)) {
                    match entry {
                        Ok(e) => children.push(e.path()),
                        Err(e) => report.diagnostics.push(format!("{}: {e}", path.display())),
                    }
                }
                children.sort();
                for child in children {
                    if *remaining_entries == 0 {
                        report
                            .diagnostics
                            .push("Directory entry limit reached".into());
                        break;
                    }
                    walk(
                        &child,
                        plugin,
                        depth + 1,
                        report,
                        visited,
                        remaining_entries,
                        remaining_bytes,
                    );
                }
            }
            Err(e) => report.diagnostics.push(format!("{}: {e}", path.display())),
        }
        return;
    }
    let skill_file = path.file_name().is_some_and(|name| name == "SKILL.md");
    let command_file = path.extension().is_some_and(|ext| ext == "md")
        && path
            .components()
            .any(|component| component.as_os_str() == "commands");
    if !metadata.is_file() || !(skill_file || command_file) {
        return;
    }
    if report.inspected_files >= MAX_FILES {
        report
            .diagnostics
            .push("Skill/command file limit reached".into());
        return;
    }
    let text = match read_bounded(path, MAX_FILE_BYTES.min(*remaining_bytes)) {
        Ok(text) => text,
        Err(e) => {
            report.diagnostics.push(e);
            return;
        }
    };
    *remaining_bytes -= text.len();
    report.inspected_files += 1;
    scan_document(path, plugin, &text, report);
}

fn prohibited(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "do not",
        "don't",
        "never ",
        "forbidden",
        "prohibited",
        "must not",
        "avoid ",
        "禁止",
        "not allowed",
    ]
    .iter()
    .any(|word| text.contains(word))
}

fn incomplete_shell_statement(text: &str) -> bool {
    let mut quote = None;
    let mut escaped = false;
    for ch in text.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, ch) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('\''), _) => {}
            (_, '\\') => escaped = true,
            (None, '\'' | '"') => quote = Some(ch),
            _ => {}
        }
    }
    quote.is_some() || escaped
}

fn scan_document(path: &Path, plugin: &str, text: &str, report: &mut ConflictReport) {
    let skill = if path.file_name().is_some_and(|name| name == "SKILL.md") {
        path.parent().and_then(Path::file_name)
    } else {
        path.file_stem()
    }
    .unwrap_or_default()
    .to_string_lossy()
    .into_owned();
    let mut fence = false;
    let mut shell_fence = false;
    let mut prohibited_section = false;
    let mut prior_prohibited = false;
    let mut prohibited_fence = false;
    let mut pending_shell: Option<(usize, String)> = None;
    let mut skip_fence = false;
    for (line_index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if !fence && trimmed.starts_with('#') {
            prohibited_section = prohibited(trimmed);
        }
        if let Some(language) = trimmed
            .strip_prefix("```")
            .or_else(|| trimmed.strip_prefix("~~~"))
        {
            if let Some((line, _)) = pending_shell.take() {
                report.diagnostics.push(format!(
                    "{}:{line}: unterminated shell statement",
                    path.display()
                ));
            }
            fence = !fence;
            skip_fence = false;
            prohibited_fence = fence && (prior_prohibited || prohibited_section);
            if !fence {
                prior_prohibited = false;
            }
            shell_fence = fence
                && matches!(
                    language.trim(),
                    "" | "sh" | "bash" | "shell" | "zsh" | "powershell" | "pwsh"
                );
            continue;
        }
        // Prohibitions describe surrounding prose, not words inside a
        // command's arguments (e.g. a PR titled "forbidden"). Preserve the
        // context over its fenced example, without suppressing later commands.
        let prose = line.split('`').step_by(2).collect::<Vec<_>>().join(" ");
        let plain_command = trimmed.starts_with("gh ")
            || trimmed.starts_with("curl ")
            || trimmed.starts_with("git ");
        let line_prohibited = !fence && !plain_command && prohibited(&prose);
        if !fence && !trimmed.is_empty() {
            prior_prohibited = line_prohibited;
        }
        if prohibited_section || line_prohibited || prohibited_fence || skip_fence {
            continue;
        }
        let mut candidates = Vec::new();
        if shell_fence {
            let (start, mut command) = pending_shell
                .take()
                .unwrap_or((line_index + 1, String::new()));
            if !command.is_empty() {
                if command.ends_with('\\') {
                    command.pop();
                    command.push(' ');
                } else {
                    command.push('\n');
                }
            }
            command.push_str(trimmed.trim_start_matches("$ "));
            if command.len() > 2048 {
                report.diagnostics.push(format!(
                    "{}:{start}: command byte limit reached; remaining fence skipped",
                    path.display()
                ));
                skip_fence = true;
                continue;
            }
            if incomplete_shell_statement(&command) {
                pending_shell = Some((start, command));
                continue;
            }
            candidates.push((start, command));
        }
        if !fence {
            candidates.extend(
                line.split('`')
                    .enumerate()
                    .filter(|(index, _)| index % 2 == 1)
                    .map(|(_, text)| (line_index + 1, text.to_string())),
            );
            if trimmed.starts_with("gh ")
                || trimmed.starts_with("curl ")
                || trimmed.starts_with("git ")
            {
                candidates.push((line_index + 1, trimmed.to_string()));
            }
            if trimmed.starts_with("allowed-tools:") {
                for fragment in line.split("Bash(").skip(1) {
                    if let Some((command, _)) = fragment.split_once(')') {
                        candidates
                            .push((line_index + 1, command.trim_end_matches(":*").to_string()));
                    }
                }
            }
        }
        for (command_line, command) in candidates {
            if command.len() > 2048 {
                report.diagnostics.push(format!(
                    "{}:{}: command byte limit reached",
                    path.display(),
                    command_line
                ));
                continue;
            }
            let Some(decision) = block_bash_policy::github_mutation_policy(&command) else {
                continue;
            };
            if report.findings.len() >= MAX_FINDINGS {
                report
                    .diagnostics
                    .push("Conflict finding limit reached".into());
                return;
            }
            if report.findings.iter().any(|f| {
                f.file == path.to_string_lossy() && f.line == command_line && f.command == command
            }) {
                continue;
            }
            report.findings.push(ConflictFinding {
                file: path.to_string_lossy().into_owned(), line: command_line, command,
                alternative: format!("{}\nNon-PM Issue registration: post the proposal to PM with board.post; do not call issue.create.", decision.detail()),
                plugin: plugin.into(), skill: skill.clone(),
            });
        }
    }
    if let Some((line, _)) = pending_shell {
        report.diagnostics.push(format!(
            "{}:{line}: unterminated shell statement",
            path.display()
        ));
    }
}

/// Dispatcher integration: SessionStart inventory warning and targeted Skill
/// PreToolUse context. Malformed optional metadata never blocks a tool.
pub fn advisory_context(event_name: &str, input: &str, worktree: &Path) -> Option<String> {
    let event = HookEvent::read_from_str(input).ok().flatten()?;
    let selected = if event_name == "PreToolUse" {
        if !matches!(event.tool_name.as_deref(), Some("Skill" | "skill")) {
            return None;
        }
        event
            .active_skill
            .as_deref()
            .or_else(|| event.tool_input.as_ref()?.get("skill")?.as_str())
    } else if event_name == "SessionStart" {
        None
    } else {
        return None;
    };
    let mut report = scan_worktree(worktree);
    if let Some(skill) = selected {
        let name = skill.rsplit(':').next().unwrap_or(skill);
        report
            .findings
            .retain(|finding| finding.skill == name || finding.file == skill);
    }
    if let Some(plugin) = event.active_plugin.as_deref() {
        report.findings.retain(|finding| {
            finding.plugin == plugin || finding.plugin.split('@').next() == Some(plugin)
        });
    }
    if report.findings.is_empty() && report.diagnostics.is_empty() {
        return None;
    }
    let mut context = format!("[gwt] External skill policy advisory: {} conflicting command(s); scan {:?}. Identity is advisory only. Use canonical gwtd JSON operations instead of these GitHub writes.\n", report.findings.len(), report.status);
    for finding in report.findings.iter().take(8) {
        context.push_str(&format!(
            "File {:?}:{}; command {:?}\n{}\n",
            finding.file, finding.line, finding.command, finding.alternative
        ));
    }
    if report.findings.len() > 8 {
        context.push_str("More findings are available in JSON operation hook.health.\n");
    }
    for diagnostic in report.diagnostics.iter().take(4) {
        context.push_str(&format!("Scan incomplete: {diagnostic:?}\n"));
    }
    Some(context)
}
