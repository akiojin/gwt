//! Versions tab — version history derived from git tags.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent};
use gwt_core::ai::{format_error_for_display, AIClient, AIError, ChatMessage};
use gwt_core::config::ProfilesConfig;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use semver::Version;

const MAX_VISIBLE_TAGS: usize = 10;
const MAX_SUBJECTS_FOR_CHANGELOG: usize = 400;
const MAX_SUBJECTS_FOR_AI: usize = 120;
const MAX_CHANGELOG_LINES_PER_GROUP: usize = 20;
const MAX_PROMPT_CHARS: usize = 12_000;

/// A single version entry shown in the Versions tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionTag {
    pub id: String,
    pub label: String,
    pub range_from: Option<String>,
    pub range_to: String,
    pub commit_count: u32,
    pub summary_preview: String,
}

impl VersionTag {
    fn range_label(&self) -> String {
        match &self.range_from {
            Some(from) => format!("{from}..{}", self.range_to),
            None => self.range_to.clone(),
        }
    }
}

/// State for the Versions screen.
#[derive(Debug, Default)]
pub struct VersionsState {
    pub tags: Vec<VersionTag>,
    pub selected: usize,
    pub scroll: usize,
    pub detail_mode: bool,
    pub detail_content: String,
    pub detail_scroll: usize,
}

impl VersionsState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Messages for the Versions screen.
#[derive(Debug)]
pub enum VersionsMessage {
    SelectPrev,
    SelectNext,
    OpenDetail,
    CloseDetail,
    ScrollDetailUp,
    ScrollDetailDown,
}

/// Handle key input for the Versions screen.
pub fn handle_key(state: &VersionsState, key: &KeyEvent) -> Option<VersionsMessage> {
    if state.detail_mode {
        match key.code {
            KeyCode::Esc => Some(VersionsMessage::CloseDetail),
            KeyCode::Up | KeyCode::Char('k') => Some(VersionsMessage::ScrollDetailUp),
            KeyCode::Down | KeyCode::Char('j') => Some(VersionsMessage::ScrollDetailDown),
            _ => None,
        }
    } else {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => Some(VersionsMessage::SelectPrev),
            KeyCode::Down | KeyCode::Char('j') => Some(VersionsMessage::SelectNext),
            KeyCode::Enter => Some(VersionsMessage::OpenDetail),
            _ => None,
        }
    }
}

/// Apply a VersionsMessage to state.
pub fn update(state: &mut VersionsState, msg: VersionsMessage) {
    match msg {
        VersionsMessage::SelectPrev => {
            state.selected = state.selected.saturating_sub(1);
        }
        VersionsMessage::SelectNext => {
            let max = state.tags.len().saturating_sub(1);
            if state.selected < max {
                state.selected += 1;
            }
        }
        VersionsMessage::OpenDetail => {
            if !state.tags.is_empty() {
                state.detail_mode = true;
                state.detail_scroll = 0;
            }
        }
        VersionsMessage::CloseDetail => {
            state.detail_mode = false;
            state.detail_content.clear();
            state.detail_scroll = 0;
        }
        VersionsMessage::ScrollDetailUp => {
            state.detail_scroll = state.detail_scroll.saturating_sub(1);
        }
        VersionsMessage::ScrollDetailDown => {
            state.detail_scroll = state.detail_scroll.saturating_add(1);
        }
    }
}

/// Render the Versions screen.
pub fn render(state: &VersionsState, buf: &mut Buffer, area: Rect) {
    if area.height < 3 {
        return;
    }

    if state.detail_mode {
        render_detail(state, buf, area);
        return;
    }

    let layout = Layout::vertical([
        Constraint::Length(1), // Header
        Constraint::Min(1),    // List
    ])
    .split(area);

    let count = state.tags.len();
    let header = format!(" Versions ({count})  [Enter] Open");
    let header_span = Span::styled(header, Style::default().fg(Color::Cyan).bold());
    buf.set_span(layout[0].x, layout[0].y, &header_span, layout[0].width);

    if state.tags.is_empty() {
        let msg = Paragraph::new("No version tags found")
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        let y = layout[1].y + layout[1].height / 2;
        let text_area = Rect::new(layout[1].x, y, layout[1].width, 1);
        ratatui::widgets::Widget::render(msg, text_area, buf);
        return;
    }

    let list_area = layout[1];
    let viewport_rows = list_area.height as usize;
    let rows_per_item = 2usize;
    let visible_items = (viewport_rows / rows_per_item.max(1)).max(1);

    let offset = if state.selected >= visible_items {
        state.selected - visible_items + 1
    } else {
        0
    };

    for (i, tag) in state
        .tags
        .iter()
        .skip(offset)
        .take(visible_items)
        .enumerate()
    {
        let item_index = i + offset;
        let base_y = list_area.y + (i * rows_per_item) as u16;
        let is_selected = item_index == state.selected;
        render_tag_row(tag, is_selected, buf, list_area.x, base_y, list_area.width);
    }
}

fn render_tag_row(
    tag: &VersionTag,
    is_selected: bool,
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
) {
    if is_selected {
        for row in y..y.saturating_add(2) {
            for col in x..x.saturating_add(width) {
                if let Some(cell) = buf.cell_mut((col, row)) {
                    cell.set_style(Style::default().bg(Color::Rgb(30, 45, 55)));
                    if cell.symbol().is_empty() {
                        cell.set_char(' ');
                    }
                }
            }
        }
    }

    let marker = if is_selected { ">" } else { " " };
    let headline = format!(
        " {marker} {:<12} {:<24} {:>3} {}",
        tag.label,
        tag.range_label(),
        format!("{}c", tag.commit_count),
        truncate_preview(&tag.summary_preview, width as usize, 46),
    );
    let headline_style = if is_selected {
        Style::default()
            .fg(Color::White)
            .bg(Color::Rgb(30, 45, 55))
            .bold()
    } else {
        Style::default().fg(Color::White)
    };
    buf.set_span(x, y, &Span::styled(headline, headline_style), width);

    let preview = format!(
        "   {}",
        truncate_preview(&tag.summary_preview, width as usize, width as usize)
    );
    let preview_style = if is_selected {
        Style::default().fg(Color::Gray).bg(Color::Rgb(30, 45, 55))
    } else {
        Style::default().fg(Color::DarkGray)
    };
    buf.set_span(
        x,
        y.saturating_add(1),
        &Span::styled(preview, preview_style),
        width,
    );
}

fn truncate_preview(text: &str, width: usize, max_len: usize) -> String {
    let limit = width.min(max_len).saturating_sub(4);
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(limit).collect();
        format!("{truncated}...")
    }
}

fn render_detail(state: &VersionsState, buf: &mut Buffer, area: Rect) {
    let tag_name = state
        .tags
        .get(state.selected)
        .map(|t| t.label.as_str())
        .unwrap_or("?");

    let layout = Layout::vertical([
        Constraint::Length(1), // Header
        Constraint::Min(1),    // Content
    ])
    .split(area);

    let header = format!(" {tag_name}  [Esc] Back");
    let header_span = Span::styled(header, Style::default().fg(Color::Cyan).bold());
    buf.set_span(layout[0].x, layout[0].y, &header_span, layout[0].width);

    crate::widgets::markdown::render_markdown(
        buf,
        layout[1],
        &state.detail_content,
        state.detail_scroll,
    );
}

/// Load version tags from git and build one-line previews from commit subjects.
pub fn load_tags(repo_root: &Path) -> Vec<VersionTag> {
    let tags =
        list_version_tags(repo_root, Some(MAX_VISIBLE_TAGS.saturating_add(1))).unwrap_or_default();
    let mut items = Vec::new();

    for (idx, tag) in tags.iter().take(MAX_VISIBLE_TAGS).enumerate() {
        let prev = tags.get(idx + 1).cloned();
        let commit_count = rev_list_count(repo_root, prev.as_deref(), tag).unwrap_or(0);
        let subjects = git_log_subjects(repo_root, prev.as_deref(), tag, 32).unwrap_or_default();
        let changelog = build_simple_changelog_markdown(&subjects, "en");
        items.push(VersionTag {
            id: tag.clone(),
            label: tag.clone(),
            range_from: prev,
            range_to: tag.clone(),
            commit_count,
            summary_preview: preview_from_changelog(&changelog),
        });
    }

    items
}

/// Load tag detail as Version History markdown.
pub fn load_tag_detail(repo_root: &Path, tag_name: &str) -> String {
    let (label, range_from, range_to) = match resolve_range_for_version(repo_root, tag_name) {
        Ok(range) => range,
        Err(err) => {
            tracing::warn!(
                message = "flow_failure",
                category = "ui",
                event = "load_version_detail",
                result = "failure",
                workspace = "default",
                tag = tag_name,
                error_code = "VERSION_RANGE_RESOLVE_FAILED",
                error_detail = %err,
            );
            return format!("## Summary\nFailed to resolve version: {err}");
        }
    };

    let commit_count = match rev_list_count(repo_root, range_from.as_deref(), &range_to) {
        Ok(count) => count,
        Err(err) => {
            tracing::warn!(
                message = "flow_failure",
                category = "ui",
                event = "load_version_detail",
                result = "failure",
                workspace = "default",
                tag = tag_name,
                error_code = "VERSION_COMMIT_COUNT_FAILED",
                error_detail = %err,
            );
            return format!("## Summary\nFailed to count commits: {err}");
        }
    };

    let subjects = match git_log_subjects(
        repo_root,
        range_from.as_deref(),
        &range_to,
        MAX_SUBJECTS_FOR_CHANGELOG,
    ) {
        Ok(subjects) => subjects,
        Err(err) => {
            tracing::warn!(
                message = "flow_failure",
                category = "ui",
                event = "load_version_detail",
                result = "failure",
                workspace = "default",
                tag = tag_name,
                error_code = "VERSION_GIT_LOG_FAILED",
                error_detail = %err,
            );
            return format!("## Summary\nFailed to read git history: {err}");
        }
    };

    let changelog_markdown = build_simple_changelog_markdown(&subjects, "en");
    let ai_input = build_ai_input(
        &label,
        range_from.as_deref(),
        &range_to,
        commit_count,
        &changelog_markdown,
        &subjects,
    );

    let summary_markdown = match load_ai_summary(&ai_input) {
        Ok(summary) => Some(summary),
        Err(err) => {
            tracing::warn!(
                message = "flow_failure",
                category = "ui",
                event = "generate_version_ai_summary",
                result = "failure",
                workspace = "default",
                tag = label.as_str(),
                error_code = "VERSION_AI_SUMMARY_FAILED",
                error_detail = %format_error_for_display(&err),
            );
            Some(format!(
                "## Summary\nAI summary unavailable. {}",
                format_error_for_display(&err)
            ))
        }
    };

    build_detail_markdown(
        &label,
        range_from.as_deref(),
        &range_to,
        commit_count,
        summary_markdown.as_deref(),
        &changelog_markdown,
    )
}

fn build_detail_markdown(
    label: &str,
    range_from: Option<&str>,
    range_to: &str,
    commit_count: u32,
    summary_markdown: Option<&str>,
    changelog_markdown: &str,
) -> String {
    let mut out = String::new();
    let range = match range_from {
        Some(from) => format!("{from}..{range_to}"),
        None => range_to.to_string(),
    };

    out.push_str("## Version\n");
    out.push_str(&format!("- Tag: `{label}`\n"));
    out.push_str(&format!("- Range: `{range}`\n"));
    out.push_str(&format!("- Commits: {commit_count}\n\n"));

    if let Some(summary) = summary_markdown {
        out.push_str(summary.trim());
        out.push_str("\n\n");
    } else {
        out.push_str("## Summary\nAI summary is disabled.\n\n");
    }

    out.push_str("## Changelog\n");
    out.push_str(changelog_markdown.trim());
    out
}

fn load_ai_summary(input: &str) -> Result<String, AIError> {
    let profiles = ProfilesConfig::load().map_err(|err| AIError::ConfigError(err.to_string()))?;
    let resolved = profiles.resolve_active_ai_settings();

    if !resolved.summary_enabled {
        return Ok("## Summary\nAI summary is disabled.\n".to_string());
    }

    let settings = resolved
        .resolved
        .ok_or_else(|| AIError::ConfigError("Active AI profile is not configured".to_string()))?;
    let client = AIClient::new(settings)?;
    generate_ai_summary(&client, input)
}

fn build_ai_input(
    label: &str,
    range_from: Option<&str>,
    range_to: &str,
    commit_count: u32,
    simple_changelog: &str,
    subjects: &[String],
) -> String {
    let range = match range_from {
        Some(from) => format!("{from}..{range_to}"),
        None => range_to.to_string(),
    };

    let mut raw_subjects = String::new();
    for subject in subjects.iter().take(MAX_SUBJECTS_FOR_AI) {
        let line = subject.trim();
        if line.is_empty() {
            continue;
        }
        raw_subjects.push_str("- ");
        raw_subjects.push_str(line);
        raw_subjects.push('\n');
    }

    let content = format!(
        "Version: {label}\nRange: {range}\nCommits: {commit_count}\n\nSimple Changelog:\n{simple_changelog}\n\nRaw Commit Subjects (sample):\n{raw_subjects}"
    );

    sample_text(&content, MAX_PROMPT_CHARS)
}

fn generate_ai_summary(client: &AIClient, input: &str) -> Result<String, AIError> {
    let system = [
        "You are a release notes assistant.",
        "Write concise English for end users.",
        "Do NOT list commit hashes or raw git commands.",
        "Do NOT copy commit subjects verbatim unless necessary.",
        "Output MUST be Markdown with these sections in this order:",
        "## Summary",
        "## Highlights",
        "Highlights MUST be 3-5 bullet points.",
        "Keep it short and practical.",
    ]
    .join("\n");

    let user = format!("Summarize the following project changes for this version.\n\n{input}\n");

    let out = client.create_response(vec![
        ChatMessage {
            role: "system".to_string(),
            content: system,
        },
        ChatMessage {
            role: "user".to_string(),
            content: user,
        },
    ])?;

    let markdown = normalize_ai_summary_markdown(out.trim());
    validate_ai_summary_markdown(&markdown)?;
    Ok(markdown)
}

fn validate_ai_summary_markdown(markdown: &str) -> Result<(), AIError> {
    let mut has_summary = false;
    let mut has_highlights = false;
    let mut highlight_bullets = 0usize;
    let mut in_highlights = false;

    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("## summary") {
            has_summary = true;
            in_highlights = false;
            continue;
        }
        if trimmed.eq_ignore_ascii_case("## highlights") {
            has_highlights = true;
            in_highlights = true;
            continue;
        }
        if trimmed.starts_with("## ") {
            in_highlights = false;
        }
        if in_highlights && is_bullet_line(trimmed) {
            highlight_bullets += 1;
        }
    }

    if has_summary && has_highlights && highlight_bullets >= 1 {
        Ok(())
    } else {
        Err(AIError::IncompleteSummary)
    }
}

fn normalize_ai_summary_markdown(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let lines: Vec<&str> = markdown.lines().collect();
    let last_idx = lines.len().saturating_sub(1);

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if let Some(title) = trimmed.strip_prefix("## ") {
            let title = title.trim();
            let title_lower = title.to_ascii_lowercase();
            if title_lower == "summary" {
                out.push_str("## Summary");
                if idx < last_idx {
                    out.push('\n');
                }
                continue;
            }
            if title_lower == "highlights" {
                out.push_str("## Highlights");
                if idx < last_idx {
                    out.push('\n');
                }
                continue;
            }
        }

        out.push_str(line);
        if idx < last_idx {
            out.push('\n');
        }
    }

    out
}

fn preview_from_changelog(changelog: &str) -> String {
    changelog
        .lines()
        .find_map(|line| line.strip_prefix("- "))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .unwrap_or("No commits")
        .to_string()
}

fn build_simple_changelog_markdown(subjects: &[String], language: &str) -> String {
    let want_ja = language == "ja";
    let mut groups: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    for subject in subjects {
        let subject = subject.trim();
        if subject.is_empty() {
            continue;
        }
        let group = classify_subject_group(subject);
        let entry = normalize_subject_for_changelog(subject);
        groups.entry(group).or_default().push(entry);
    }

    let order = [
        "Features",
        "Bug Fixes",
        "Documentation",
        "Performance",
        "Refactor",
        "Styling",
        "Testing",
        "Miscellaneous Tasks",
        "Other",
    ];

    let mut out = String::new();
    for name in order {
        let Some(entries) = groups.get(name) else {
            continue;
        };
        if entries.is_empty() {
            continue;
        }
        out.push_str("### ");
        out.push_str(if want_ja {
            translate_changelog_group(name)
        } else {
            name
        });
        out.push('\n');
        let mut shown = 0usize;
        for entry in entries.iter().take(MAX_CHANGELOG_LINES_PER_GROUP) {
            out.push_str("- ");
            out.push_str(entry);
            out.push('\n');
            shown += 1;
        }
        if entries.len() > shown {
            out.push_str(&format!("- (+{} more)\n", entries.len() - shown));
        }
        out.push('\n');
    }

    if out.trim().is_empty() {
        "(No commits)".to_string()
    } else {
        out.trim_end().to_string()
    }
}

fn translate_changelog_group(name: &str) -> &str {
    match name {
        "Features" => "機能",
        "Bug Fixes" => "バグ修正",
        "Documentation" => "ドキュメント",
        "Performance" => "パフォーマンス",
        "Refactor" => "リファクタ",
        "Styling" => "スタイル",
        "Testing" => "テスト",
        "Miscellaneous Tasks" => "その他タスク",
        "Other" => "その他",
        _ => name,
    }
}

fn classify_subject_group(subject: &str) -> &'static str {
    let lowered = subject.trim().to_ascii_lowercase();
    if lowered.starts_with("feat") {
        return "Features";
    }
    if lowered.starts_with("fix") {
        return "Bug Fixes";
    }
    if lowered.starts_with("docs") || lowered.starts_with("doc") {
        return "Documentation";
    }
    if lowered.starts_with("perf") {
        return "Performance";
    }
    if lowered.starts_with("refactor") {
        return "Refactor";
    }
    if lowered.starts_with("style") {
        return "Styling";
    }
    if lowered.starts_with("test") {
        return "Testing";
    }
    if lowered.starts_with("chore") {
        return "Miscellaneous Tasks";
    }
    "Other"
}

fn normalize_subject_for_changelog(subject: &str) -> String {
    let subject = subject.trim();
    let Some((prefix, rest)) = subject.split_once(':') else {
        return subject.to_string();
    };

    let msg = rest.trim();
    if msg.is_empty() {
        return subject.to_string();
    }

    let mut prefix = prefix.trim();
    if prefix.ends_with('!') {
        prefix = prefix.trim_end_matches('!');
    }

    let (typ, scope) = if let Some((typ, rest)) = prefix.split_once('(') {
        let scope = rest.trim_end_matches(')').trim();
        (
            typ.trim(),
            if scope.is_empty() { None } else { Some(scope) },
        )
    } else {
        (prefix, None)
    };

    let typ_lower = typ.to_ascii_lowercase();
    let known = matches!(
        typ_lower.as_str(),
        "feat" | "fix" | "docs" | "doc" | "perf" | "refactor" | "style" | "test" | "chore"
    );
    if !known {
        return subject.to_string();
    }

    if let Some(scope) = scope {
        format!("**{}:** {}", scope, msg)
    } else {
        msg.to_string()
    }
}

fn sample_text(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }

    let head_chars = max_chars * 2 / 5;
    let separator = "\n...[truncated]...\n";
    let tail_chars = max_chars.saturating_sub(head_chars + separator.len());
    let head: String = text.chars().take(head_chars).collect();
    let tail: String = text
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}{separator}{tail}")
}

fn resolve_range_for_version(
    repo_path: &Path,
    version_id: &str,
) -> Result<(String, Option<String>, String), String> {
    let tags = list_version_tags(repo_path, None)?;
    let idx = tags
        .iter()
        .position(|tag| tag == version_id)
        .ok_or_else(|| format!("Version tag not found: {version_id}"))?;
    let prev = tags.get(idx + 1).cloned();
    Ok((version_id.to_string(), prev, version_id.to_string()))
}

fn list_version_tags(repo_path: &Path, max: Option<usize>) -> Result<Vec<String>, String> {
    let out = git_output(
        repo_path,
        &["tag".to_string(), "--list".to_string(), "v*".to_string()],
    )?;
    let mut tags = parse_and_sort_version_tags(&out);
    if let Some(max) = max {
        tags.truncate(max);
    }
    Ok(tags)
}

fn parse_and_sort_version_tags(raw: &str) -> Vec<String> {
    let mut tags: Vec<String> = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|tag| {
            tag.starts_with('v') && tag.chars().nth(1).is_some_and(|ch| ch.is_ascii_digit())
        })
        .map(ToString::to_string)
        .collect();
    tags.sort_by(|a, b| compare_version_tag_desc(a, b));
    tags
}

fn compare_version_tag_desc(a: &str, b: &str) -> Ordering {
    match (parse_semver_tag(a), parse_semver_tag(b)) {
        (Some(ver_a), Some(ver_b)) => ver_b.cmp(&ver_a).then_with(|| b.cmp(a)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => b.cmp(a),
    }
}

fn parse_semver_tag(tag: &str) -> Option<Version> {
    let raw = tag.strip_prefix('v')?;
    Version::parse(raw).ok()
}

fn rev_list_count(repo_path: &Path, from: Option<&str>, to: &str) -> Result<u32, String> {
    let range = match from {
        Some(from) if !from.trim().is_empty() => format!("{from}..{to}"),
        _ => to.to_string(),
    };
    let out = git_output(
        repo_path,
        &["rev-list".to_string(), "--count".to_string(), range],
    )?;
    out.lines()
        .next()
        .unwrap_or("0")
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("Failed to parse rev-list count: {out}"))
}

fn git_log_subjects(
    repo_path: &Path,
    from: Option<&str>,
    to: &str,
    max: usize,
) -> Result<Vec<String>, String> {
    let range = match from {
        Some(from) if !from.trim().is_empty() => format!("{from}..{to}"),
        _ => to.to_string(),
    };
    let out = git_output(
        repo_path,
        &[
            "log".to_string(),
            "--no-merges".to_string(),
            "--pretty=format:%s".to_string(),
            "-n".to_string(),
            max.to_string(),
            range,
        ],
    )?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect())
}

fn git_output(repo_path: &Path, args: &[String]) -> Result<String, String> {
    let output = gwt_core::process::command("git")
        .args(args)
        .current_dir(repo_path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|err| format!("Failed to execute git: {err}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            "Git command failed".to_string()
        } else {
            stderr
        })
    }
}

fn is_bullet_line(line: &str) -> bool {
    line.starts_with("- ")
        || line.starts_with("* ")
        || line.starts_with('•')
        || strip_ordered_prefix(line).is_some()
}

fn strip_ordered_prefix(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == 0 || idx + 1 >= bytes.len() {
        return None;
    }
    if (bytes[idx] == b'.' || bytes[idx] == b')') && bytes[idx + 1] == b' ' {
        return Some(&line[idx + 2..]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tempfile::TempDir;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn sample_tags() -> Vec<VersionTag> {
        vec![
            VersionTag {
                id: "v2.0.0".into(),
                label: "v2.0.0".into(),
                range_from: Some("v1.1.0".into()),
                range_to: "v2.0.0".into(),
                commit_count: 4,
                summary_preview: "Major release".into(),
            },
            VersionTag {
                id: "v1.1.0".into(),
                label: "v1.1.0".into(),
                range_from: Some("v1.0.0".into()),
                range_to: "v1.1.0".into(),
                commit_count: 2,
                summary_preview: "Feature update".into(),
            },
            VersionTag {
                id: "v1.0.0".into(),
                label: "v1.0.0".into(),
                range_from: None,
                range_to: "v1.0.0".into(),
                commit_count: 1,
                summary_preview: "Initial release".into(),
            },
        ]
    }

    fn init_git_repo(path: &Path) {
        assert!(gwt_core::process::command("git")
            .args(["init"])
            .current_dir(path)
            .status()
            .unwrap()
            .success());
        let _ = gwt_core::process::command("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .status();
        let _ = gwt_core::process::command("git")
            .args(["config", "user.name", "Test"])
            .current_dir(path)
            .status();
    }

    fn commit_file(path: &Path, name: &str, content: &str, message: &str) {
        std::fs::write(path.join(name), content).unwrap();
        assert!(gwt_core::process::command("git")
            .args(["add", name])
            .current_dir(path)
            .status()
            .unwrap()
            .success());
        assert!(gwt_core::process::command("git")
            .args(["commit", "-m", message])
            .current_dir(path)
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn new_state_is_empty() {
        let state = VersionsState::new();
        assert!(state.tags.is_empty());
        assert_eq!(state.selected, 0);
        assert!(!state.detail_mode);
    }

    #[test]
    fn select_prev_next() {
        let mut state = VersionsState::new();
        state.tags = sample_tags();
        update(&mut state, VersionsMessage::SelectNext);
        assert_eq!(state.selected, 1);
        update(&mut state, VersionsMessage::SelectPrev);
        assert_eq!(state.selected, 0);
        state.selected = 2;
        update(&mut state, VersionsMessage::SelectNext);
        assert_eq!(state.selected, 2);
    }

    #[test]
    fn open_close_detail() {
        let mut state = VersionsState::new();
        state.tags = sample_tags();
        update(&mut state, VersionsMessage::OpenDetail);
        assert!(state.detail_mode);
        update(&mut state, VersionsMessage::CloseDetail);
        assert!(!state.detail_mode);
    }

    #[test]
    fn detail_scroll() {
        let mut state = VersionsState::new();
        state.tags = sample_tags();
        state.detail_mode = true;
        state.detail_content = "line1\nline2\nline3".to_string();
        update(&mut state, VersionsMessage::ScrollDetailDown);
        assert_eq!(state.detail_scroll, 1);
        update(&mut state, VersionsMessage::ScrollDetailUp);
        assert_eq!(state.detail_scroll, 0);
    }

    #[test]
    fn handle_key_list_mode() {
        let state = VersionsState::new();
        assert!(matches!(
            handle_key(&state, &key(KeyCode::Up)),
            Some(VersionsMessage::SelectPrev)
        ));
        assert!(matches!(
            handle_key(&state, &key(KeyCode::Down)),
            Some(VersionsMessage::SelectNext)
        ));
        assert!(matches!(
            handle_key(&state, &key(KeyCode::Enter)),
            Some(VersionsMessage::OpenDetail)
        ));
    }

    #[test]
    fn handle_key_detail_mode() {
        let mut state = VersionsState::new();
        state.detail_mode = true;
        assert!(matches!(
            handle_key(&state, &key(KeyCode::Esc)),
            Some(VersionsMessage::CloseDetail)
        ));
    }

    #[test]
    fn render_empty_no_panic() {
        let state = VersionsState::new();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&state, &mut buf, area);
    }

    #[test]
    fn render_with_tags_no_panic() {
        let mut state = VersionsState::new();
        state.tags = sample_tags();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&state, &mut buf, area);
    }

    #[test]
    fn render_detail_mode_no_panic() {
        let mut state = VersionsState::new();
        state.tags = sample_tags();
        state.detail_mode = true;
        state.detail_content = "## Summary\nReady\n\n## Changelog\n- Item".to_string();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&state, &mut buf, area);
    }

    #[test]
    fn parse_and_sort_version_tags_orders_semver_desc() {
        let parsed = parse_and_sort_version_tags("v1.2.0\nv1.10.0\nv2.0.0\nfoo\n");
        assert_eq!(parsed, vec!["v2.0.0", "v1.10.0", "v1.2.0"]);
    }

    #[test]
    fn build_simple_changelog_groups_subjects() {
        let changelog = build_simple_changelog_markdown(
            &[
                "feat(ui): add versions tab".to_string(),
                "fix: avoid crash".to_string(),
                "docs: update readme".to_string(),
            ],
            "en",
        );
        assert!(changelog.contains("### Features"));
        assert!(changelog.contains("**ui:** add versions tab"));
        assert!(changelog.contains("### Bug Fixes"));
        assert!(changelog.contains("avoid crash"));
    }

    #[test]
    fn load_tags_builds_ranges_commit_counts_and_preview() {
        let temp = TempDir::new().unwrap();
        init_git_repo(temp.path());

        commit_file(temp.path(), "file.txt", "one", "feat: initial release");
        assert!(gwt_core::process::command("git")
            .args(["tag", "v1.0.0"])
            .current_dir(temp.path())
            .status()
            .unwrap()
            .success());

        commit_file(temp.path(), "file.txt", "two", "fix: patch release");
        assert!(gwt_core::process::command("git")
            .args(["tag", "v1.1.0"])
            .current_dir(temp.path())
            .status()
            .unwrap()
            .success());

        commit_file(temp.path(), "file.txt", "three", "feat(core): big release");
        assert!(gwt_core::process::command("git")
            .args(["tag", "v2.0.0"])
            .current_dir(temp.path())
            .status()
            .unwrap()
            .success());

        let tags = load_tags(temp.path());
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[0].label, "v2.0.0");
        assert_eq!(tags[0].range_from.as_deref(), Some("v1.1.0"));
        assert_eq!(tags[0].commit_count, 1);
        assert!(tags[0].summary_preview.contains("big release"));
    }

    #[test]
    fn load_tags_limits_output_to_latest_ten_versions() {
        let temp = TempDir::new().unwrap();
        init_git_repo(temp.path());

        for patch in 0..12 {
            commit_file(
                temp.path(),
                "file.txt",
                &format!("content-{patch}"),
                &format!("fix: patch {patch}"),
            );
            assert!(gwt_core::process::command("git")
                .args(["tag", &format!("v1.0.{patch}")])
                .current_dir(temp.path())
                .status()
                .unwrap()
                .success());
        }

        let tags = load_tags(temp.path());
        assert_eq!(tags.len(), 10);
        assert_eq!(tags[0].label, "v1.0.11");
        assert_eq!(tags[9].label, "v1.0.2");
    }

    #[test]
    fn load_tag_detail_includes_version_metadata_and_changelog() {
        let markdown = build_detail_markdown(
            "v1.2.0",
            Some("v1.1.0"),
            "v1.2.0",
            3,
            Some("## Summary\nDone\n\n## Highlights\n- A"),
            "### Features\n- Added thing",
        );
        assert!(markdown.contains("## Version"));
        assert!(markdown.contains("- Tag: `v1.2.0`"));
        assert!(markdown.contains("## Summary"));
        assert!(markdown.contains("## Changelog"));
    }
}
