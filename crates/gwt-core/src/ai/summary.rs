//! Session summary generation and cache.

use super::client::{AIClient, AIError, ChatMessage};
use super::session_parser::{MessageRole, ParsedSession, SessionMessage};
use std::collections::{BTreeSet, HashMap};
use std::time::SystemTime;

pub const SESSION_SYSTEM_PROMPT_BASE: &str = "You are a helpful assistant summarizing a coding agent session so the user can remember the original request and latest instruction.\nReturn Markdown only with the following format and headings, in this exact order:\n\n## <Purpose heading in the user's language>\n<1 sentence: the worktree/branch objective (why) + key constraints + explicit exclusions>\n\n## <Summary heading in the user's language>\n<1-2 sentences: current status (use a clear status word) + the latest user instruction; mention if blocked>\n\n## <Highlights heading in the user's language>\n- <Original request: ...>\n- <Latest instruction: ...>\n- <Decisions/constraints: ...>\n- <Exclusions/not doing: ...>\n- <Status: ...>\n- <Progress: ...>\n- <Recent meaningful actions (last 1-3): ...>\n- <Needs user input (as a direct question): ...>\n- <Key words (3 items): ...>\n\nAdd more bullets if there are additional important items, but keep the list concise.\nIf there was no progress, say so and why.\nIf waiting for user input, state the exact question needed.\nDo not guess; if something is unknown, say so explicitly in the user's language.\nUse short labels followed by \":\" for each bullet and translate the labels to the user's language.\nDetect the response language from the session content and respond in that language.\nIf the session contains multiple languages, use the language used by the user messages.\nAll headings and all content must be in the user's language.\nDo not output JSON, code fences, or any extra text.\n\nPurpose writing rule:\n- The Purpose section must describe the intended outcome of the worktree/branch.\n- Treat PR/MR creation, PR template filling, merge/push, and status checks as means (how), not purpose (why).\n\nIgnore operational workflow chatter from this session except for content that changes direction:\n- PR/MR creation, branch operations, test/build/CI activity, and short status updates.\n- Keep summaries focused on user intent, decisions, constraints, outcomes, blockers, or pending actions.\n- Ignore one-line acknowledgements unless they contain a blocking issue or design decision.\nPrioritize substantive conversation over command history.\n\nWhen the session language is Japanese, headings must be exactly in this order:\n- ## 目的\n- ## 要約\n- ## ハイライト\nWhen the session language is English, headings must be exactly in this order:\n- ## Purpose\n- ## Summary\n- ## Highlights.";

const SESSION_SYSTEM_PROMPT_EN: &str = "You are a helpful assistant summarizing a coding agent session so the user can remember the original request and latest instruction.\nRespond in English.\nReturn Markdown only with the following format and headings, in this exact order:\n\n## Purpose\n<1 sentence: the worktree/branch objective (why) + key constraints + explicit exclusions>\n\n## Summary\n<1-2 sentences: current status (use a clear status word) + the latest user instruction; mention if blocked>\n\n## Highlights\n- <Original request: ...>\n- <Latest instruction: ...>\n- <Decisions/constraints: ...>\n- <Exclusions/not doing: ...>\n- <Status: ...>\n- <Progress: ...>\n- <Recent meaningful actions (last 1-3): ...>\n- <Needs user input (as a direct question): ...>\n- <Key words (3 items): ...>\n\nAdd more bullets if there are additional important items, but keep the list concise.\nIf there was no progress, say so and why.\nIf waiting for user input, state the exact question needed.\nDo not guess; if something is unknown, say so explicitly in English.\nUse short labels followed by \":\" for each bullet.\nAll headings and all content must be in English.\nDo not output JSON, code fences, or any extra text.\n\nPurpose writing rule:\n- The Purpose section must describe the intended outcome of the worktree/branch.\n- Treat PR/MR creation, PR template filling, merge/push, and status checks as means (how), not purpose (why).\n\nIgnore operational workflow chatter from this session except for content that changes direction:\n- PR/MR creation, branch operations, test/build/CI activity, and short status updates.\n- Keep summaries focused on user intent, decisions, constraints, outcomes, blockers, or pending actions.\n- Ignore one-line acknowledgements unless they contain a blocking issue or design decision.\nPrioritize substantive conversation over command history.";

const SESSION_SYSTEM_PROMPT_JA: &str = "You are a helpful assistant summarizing a coding agent session so the user can remember the original request and latest instruction.\nRespond in Japanese.\nReturn Markdown only with the following format and headings, in this exact order:\n\n## 目的\n<1文: Worktree/ブランチで達成する成果（Why） + 重要な制約 + 明示された除外>\n\n## 要約\n<1-2文: 現在ステータス（明確な状態語を使う）+ 最新のユーザー指示。ブロックされているなら明記>\n\n## ハイライト\n- <元の依頼: ...>\n- <最新指示: ...>\n- <決定事項/制約: ...>\n- <除外/やらないこと: ...>\n- <ステータス: ...>\n- <進捗: ...>\n- <直近の意味のある行動（1-3件）: ...>\n- <ユーザーに必要な入力（質問として）: ...>\n- <キーワード（3つ）: ...>\n\n重要な項目があれば箇条書きを追加してよいが、簡潔にすること。\n進捗がない場合は、その旨と理由を書くこと。\nユーザー入力待ちの場合は、必要な質問をそのまま書くこと。\n推測しない。不明な点は不明と明記すること。\n各箇条書きは短いラベル + \":\" で始め、ラベルも日本語にすること。\n見出しと本文はすべて日本語にすること。\nJSONやコードフェンス、余計なテキストを出力しないこと。\n\n目的の記述ルール:\n- 目的にはWorktree/ブランチの達成成果（Why）を書く。\n- PR/MR作成、PR本文テンプレート記入、merge/push、ステータス確認は手段（How）として扱い、目的にしない。\n\n以下の運用的なやり取りは、方向性が変わる内容を除き無視すること:\n- PR/MR作成、ブランチ操作、テスト/ビルド/CI、短いステータス更新\n- 要約はユーザー意図、決定事項、制約、結果、ブロッカー、未完了作業に集中する\n- ブロッキングや設計判断を含まない1行の相槌は無視する\n会話の中身を優先し、コマンド履歴に引っ張られないこと。";

const MAX_MESSAGE_CHARS: usize = 220;
const MAX_PROMPT_CHARS: usize = 16000;
const MAX_PURPOSE_TEXT_CHARS: usize = 180;
const MAX_ROLLING_SCROLLBACK_UPDATES: usize = 5;

const PURPOSE_KEYWORDS: &[&str] = &[
    "目的",
    "目標",
    "狙い",
    "goal",
    "purpose",
    "objective",
    "intent",
];

const OUTCOME_KEYWORDS: &[&str] = &[
    "実装",
    "改善",
    "修正",
    "追加",
    "達成",
    "対応",
    "統合",
    "機能",
    "反映",
    "deliver",
    "implement",
    "improve",
    "fix",
    "add",
    "build",
    "ship",
    "support",
    "enable",
    "complete",
];

const OPERATIONAL_KEYWORDS: &[&str] = &[
    "pr",
    "mr",
    "template",
    "テンプレ",
    "マージ",
    "merge",
    "push",
    "commit",
    "branch operation",
    "ブランチ操作",
    "ci",
    "check",
    "status",
    "url",
    "gh pr",
    "rebase",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SummaryLanguage {
    Ja,
    En,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PurposeSource {
    Explicit,
    Inferred,
}

#[derive(Debug, Clone)]
struct DerivedPurpose {
    text: String,
    source: PurposeSource,
}

impl DerivedPurpose {
    fn explicit(text: String) -> Self {
        Self {
            text,
            source: PurposeSource::Explicit,
        }
    }

    fn inferred(text: String) -> Self {
        Self {
            text,
            source: PurposeSource::Inferred,
        }
    }

    fn is_inferred(&self) -> bool {
        matches!(self.source, PurposeSource::Inferred)
    }

    fn confidence_label(&self) -> &'static str {
        match self.source {
            PurposeSource::Explicit => "explicit",
            PurposeSource::Inferred => "inferred",
        }
    }

    fn render_for_output(&self, lang: SummaryLanguage) -> String {
        let text = clip_chars(self.text.trim(), MAX_PURPOSE_TEXT_CHARS);
        if self.is_inferred() {
            match lang {
                SummaryLanguage::Ja => format!("（推定）{text}"),
                SummaryLanguage::En => format!("(Inferred) {text}"),
            }
        } else {
            text
        }
    }
}

fn session_system_prompt(language: &str) -> &'static str {
    match language.trim() {
        "ja" => SESSION_SYSTEM_PROMPT_JA,
        "auto" => SESSION_SYSTEM_PROMPT_BASE,
        _ => SESSION_SYSTEM_PROMPT_EN,
    }
}

fn contains_japanese(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(
            c,
            '\u{3040}'..='\u{309F}' // Hiragana
                | '\u{30A0}'..='\u{30FF}' // Katakana
                | '\u{4E00}'..='\u{9FFF}' // CJK Unified Ideographs
        )
    })
}

#[derive(Debug, Clone, Default)]
pub struct SessionSummary {
    pub task_overview: Option<String>,
    pub short_summary: Option<String>,
    pub bullet_points: Vec<String>,
    pub markdown: Option<String>,
    pub metrics: SessionMetrics,
    pub last_updated: Option<SystemTime>,
}

#[derive(Debug, Clone, Default)]
pub struct SessionMetrics {
    pub token_count: Option<usize>,
    pub tool_execution_count: usize,
    pub elapsed_seconds: Option<u64>,
    pub turn_count: usize,
}

#[derive(Debug, Default, Clone)]
pub struct SessionSummaryCache {
    cache: HashMap<String, SessionSummary>,
    last_modified: HashMap<String, SystemTime>,
    session_ids: HashMap<String, String>,
    tool_ids: HashMap<String, String>,
    languages: HashMap<String, String>,
    scrollback_inputs: HashMap<String, String>,
    scrollback_update_counts: HashMap<String, usize>,
}

impl SessionSummaryCache {
    pub fn get(&self, branch: &str) -> Option<&SessionSummary> {
        self.cache.get(branch)
    }

    pub fn input_mtime(&self, branch: &str) -> Option<SystemTime> {
        self.last_modified.get(branch).copied()
    }

    pub fn tool_id(&self, branch: &str) -> Option<&str> {
        self.tool_ids.get(branch).map(|s| s.as_str())
    }

    pub fn session_id(&self, branch: &str) -> Option<&str> {
        self.session_ids.get(branch).map(|s| s.as_str())
    }

    pub fn language(&self, branch: &str) -> Option<&str> {
        self.languages.get(branch).map(|s| s.as_str())
    }

    pub fn scrollback_input(&self, branch: &str) -> Option<&str> {
        self.scrollback_inputs.get(branch).map(|s| s.as_str())
    }

    pub fn scrollback_update_count(&self, branch: &str) -> usize {
        self.scrollback_update_counts
            .get(branch)
            .copied()
            .unwrap_or(0)
    }

    pub fn set(
        &mut self,
        branch: String,
        tool_id: String,
        session_id: String,
        language: String,
        summary: SessionSummary,
        mtime: SystemTime,
    ) {
        let branch_key = branch.clone();
        self.cache.insert(branch.clone(), summary);
        self.last_modified.insert(branch.clone(), mtime);
        self.session_ids.insert(branch.clone(), session_id);
        self.tool_ids.insert(branch.clone(), tool_id);
        self.languages.insert(branch, language);
        self.scrollback_inputs.remove(&branch_key);
        self.scrollback_update_counts.remove(&branch_key);
    }

    pub fn set_scrollback(
        &mut self,
        branch: String,
        tool_id: String,
        session_id: String,
        language: String,
        summary: SessionSummary,
        mtime: SystemTime,
        normalized_input: String,
        rolling_update_count: usize,
    ) {
        let branch_key = branch.clone();
        self.set(branch, tool_id, session_id, language, summary, mtime);
        self.scrollback_inputs
            .insert(branch_key.clone(), normalized_input);
        self.scrollback_update_counts
            .insert(branch_key, rolling_update_count);
    }

    pub fn is_stale(
        &self,
        branch: &str,
        session_id: &str,
        language: &str,
        current_mtime: SystemTime,
    ) -> bool {
        if let Some(cached_session_id) = self.session_ids.get(branch) {
            if cached_session_id != session_id {
                return true;
            }
        } else {
            return true;
        }

        if let Some(cached_language) = self.languages.get(branch) {
            if cached_language != language {
                return true;
            }
        } else {
            return true;
        }

        self.last_modified
            .get(branch)
            .map(|&cached| cached < current_mtime)
            .unwrap_or(true)
    }
}

#[derive(Debug, Default)]
struct SessionSummaryFields {
    task_overview: Option<String>,
    short_summary: Option<String>,
    bullet_points: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScrollbackSummaryMode {
    FullShort,
    IncrementalLong,
    FullRebuild,
}

#[derive(Debug, Clone, Default)]
pub struct ScrollbackRollingContext {
    pub session_id: String,
    pub previous_markdown: Option<String>,
    pub previous_normalized_input: Option<String>,
    pub rolling_update_count: usize,
}

#[derive(Debug, Clone)]
pub struct ScrollbackSummaryBuild {
    pub summary: SessionSummary,
    pub normalized_input: String,
    pub mode: ScrollbackSummaryMode,
    pub next_rolling_update_count: usize,
}

pub fn build_session_prompt(parsed: &ParsedSession, language: &str) -> Vec<ChatMessage> {
    build_session_prompt_with_context(parsed, language, None, None)
}

fn build_session_prompt_with_context(
    parsed: &ParsedSession,
    language: &str,
    branch_name: Option<&str>,
    derived_purpose: Option<&DerivedPurpose>,
) -> Vec<ChatMessage> {
    let mut lines = Vec::new();
    lines.push(format!(
        "Agent: {} (session_id: {})",
        parsed.agent_type.display_name(),
        parsed.session_id
    ));
    if let Some(branch) = branch_name {
        lines.push(format!("Branch: {branch}"));
    }
    if let Some(purpose) = derived_purpose {
        lines.push(format!(
            "Derived worktree purpose ({}): {}",
            purpose.confidence_label(),
            purpose.text
        ));
    }
    lines.push(
        "Purpose guidance: In the Purpose section, describe the worktree objective (why). \
Treat PR/MR creation, template filling, merge/push, and status checks as means (how), not purpose."
            .to_string(),
    );

    if parsed.messages.is_empty() {
        lines.push("No messages recorded.".to_string());
    } else {
        lines.push("Messages (sampled):".to_string());
        let mut used_chars = lines.join("\n").chars().count();
        let mut truncated = false;
        let mut included_messages = 0usize;
        for message in parsed.messages.iter() {
            let role = match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
            };
            if matches!(message.role, MessageRole::Assistant)
                && is_operational_status_noise(message.content.trim())
            {
                continue;
            }

            let mut content = message.content.trim().to_string();
            if content.chars().count() > MAX_MESSAGE_CHARS {
                content = format!(
                    "{}...",
                    content
                        .chars()
                        .take(MAX_MESSAGE_CHARS - 3)
                        .collect::<String>()
                );
            }
            let line = format!("{}. {}: {}", included_messages + 1, role, content);
            let line_len = line.chars().count() + 1; // +1 for newline
            if used_chars + line_len > MAX_PROMPT_CHARS {
                truncated = true;
                break;
            }
            lines.push(line);
            used_chars += line_len;
            included_messages += 1;
        }

        if included_messages == 0 {
            lines.push(
                "No substantive conversation content found; focus was operational updates only."
                    .to_string(),
            );
        }

        if truncated {
            let notice = "Messages truncated due to length.";
            if used_chars + notice.chars().count() < MAX_PROMPT_CHARS {
                lines.push(notice.to_string());
            }
        }
    }

    let mut user_prompt = lines.join("\n");
    if user_prompt.chars().count() > MAX_PROMPT_CHARS {
        user_prompt = user_prompt
            .chars()
            .take(MAX_PROMPT_CHARS)
            .collect::<String>();
    }

    vec![
        ChatMessage {
            role: "system".to_string(),
            content: session_system_prompt(language).to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_prompt,
        },
    ]
}

fn derive_worktree_purpose_from_messages(
    messages: &[SessionMessage],
    branch_name: &str,
    fallback_language: SummaryLanguage,
) -> DerivedPurpose {
    let mut explicit: Option<String> = None;
    let mut inferred: Option<String> = None;

    for message in messages {
        if !matches!(message.role, MessageRole::User) {
            continue;
        }
        for fragment in split_text_fragments(&message.content) {
            let cleaned = normalize_purpose_fragment(&fragment);
            if cleaned.is_empty() {
                continue;
            }

            if contains_purpose_keyword(&cleaned) {
                let candidate = extract_purpose_payload(&cleaned);
                if is_meaningful_purpose(&candidate) {
                    explicit = Some(candidate);
                }
                continue;
            }

            if is_meaningful_purpose(&cleaned) {
                inferred = Some(cleaned);
            }
        }
    }

    if let Some(text) = explicit {
        return DerivedPurpose::explicit(text);
    }
    if let Some(text) = inferred {
        return DerivedPurpose::inferred(text);
    }

    DerivedPurpose::inferred(infer_purpose_from_branch(branch_name, fallback_language))
}

fn derive_worktree_purpose_from_scrollback(
    text: &str,
    branch_name: &str,
    fallback_language: SummaryLanguage,
) -> DerivedPurpose {
    let mut explicit: Option<String> = None;
    let mut inferred: Option<String> = None;

    for fragment in split_text_fragments(text) {
        let cleaned = normalize_purpose_fragment(&fragment);
        if cleaned.is_empty() {
            continue;
        }

        if contains_purpose_keyword(&cleaned) {
            let candidate = extract_purpose_payload(&cleaned);
            if is_meaningful_purpose(&candidate) {
                explicit = Some(candidate);
            }
            continue;
        }

        if is_meaningful_purpose(&cleaned) {
            inferred = Some(cleaned);
        }
    }

    if let Some(text) = explicit {
        return DerivedPurpose::explicit(text);
    }
    if let Some(text) = inferred {
        return DerivedPurpose::inferred(text);
    }

    DerivedPurpose::inferred(infer_purpose_from_branch(branch_name, fallback_language))
}

fn split_text_fragments(text: &str) -> Vec<String> {
    let normalized = text
        .replace('\r', "\n")
        .replace("。", "。\n")
        .replace(". ", ".\n")
        .replace("? ", "?\n")
        .replace("？", "？\n");

    normalized
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect()
}

fn normalize_purpose_fragment(text: &str) -> String {
    let trimmed = text.trim();
    let trimmed = if let Some(rest) = trimmed.strip_prefix("- ") {
        rest.trim()
    } else if let Some(rest) = trimmed.strip_prefix("* ") {
        rest.trim()
    } else if let Some(rest) = strip_ordered_prefix(trimmed) {
        rest.trim()
    } else {
        trimmed
    };
    clip_chars(
        trimmed
            .trim_matches(|c| c == '"' || c == '\'' || c == '“' || c == '”')
            .trim(),
        MAX_PURPOSE_TEXT_CHARS,
    )
}

fn contains_purpose_keyword(text: &str) -> bool {
    contains_any_keyword(text, PURPOSE_KEYWORDS)
}

fn contains_any_keyword(text: &str, keywords: &[&str]) -> bool {
    let lowered = text.to_lowercase();
    keywords.iter().any(|keyword| {
        if keyword.is_ascii() {
            let needle = keyword.to_lowercase();
            if needle.len() <= 3 && needle.chars().all(|c| c.is_ascii_alphanumeric()) {
                contains_ascii_word(&lowered, &needle)
            } else {
                lowered.contains(&needle)
            }
        } else {
            text.contains(keyword)
        }
    })
}

fn contains_ascii_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| !token.is_empty() && token == word)
}

fn extract_purpose_payload(text: &str) -> String {
    let trimmed = text.trim();

    for prefix in ["目的は", "目的:", "目的：", "目標は", "目標:", "目標："] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let candidate = rest.trim();
            if !candidate.is_empty() {
                return clip_chars(candidate, MAX_PURPOSE_TEXT_CHARS);
            }
        }
    }

    let lowered = trimmed.to_lowercase();
    for prefix in [
        "goal:",
        "goal is",
        "purpose:",
        "purpose is",
        "objective:",
        "objective is",
        "intent:",
        "intent is",
    ] {
        if lowered.starts_with(prefix) {
            let candidate = trimmed[prefix.len()..].trim();
            if !candidate.is_empty() {
                return clip_chars(candidate, MAX_PURPOSE_TEXT_CHARS);
            }
        }
    }

    if let Some((left, right)) = trimmed.split_once(':') {
        if contains_purpose_keyword(left) {
            let candidate = right.trim();
            if !candidate.is_empty() {
                return clip_chars(candidate, MAX_PURPOSE_TEXT_CHARS);
            }
        }
    }
    if let Some((left, right)) = trimmed.split_once('：') {
        if contains_purpose_keyword(left) {
            let candidate = right.trim();
            if !candidate.is_empty() {
                return clip_chars(candidate, MAX_PURPOSE_TEXT_CHARS);
            }
        }
    }

    clip_chars(trimmed, MAX_PURPOSE_TEXT_CHARS)
}

fn is_meaningful_purpose(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.chars().count() < 6 {
        return false;
    }
    if trimmed.ends_with('?') || trimmed.ends_with('？') {
        return false;
    }
    if is_unknown_placeholder(trimmed) {
        return false;
    }

    !is_operational_only(trimmed)
}

fn is_operational_only(text: &str) -> bool {
    let has_operational = contains_any_keyword(text, OPERATIONAL_KEYWORDS);
    if !has_operational {
        return false;
    }
    let has_outcome =
        contains_any_keyword(text, OUTCOME_KEYWORDS) || contains_purpose_keyword(text);
    !has_outcome
}

fn is_unknown_placeholder(text: &str) -> bool {
    matches!(
        text.trim(),
        "(不明)" | "不明" | "(Not available)" | "Not available" | "(Unknown)" | "Unknown"
    )
}

fn infer_purpose_from_branch(branch_name: &str, lang: SummaryLanguage) -> String {
    let topic = branch_name
        .split('/')
        .next_back()
        .unwrap_or(branch_name)
        .replace(['-', '_'], " ");
    match lang {
        SummaryLanguage::Ja => {
            if topic.trim().is_empty() {
                "このWorktreeで進めている成果を達成すること".to_string()
            } else {
                format!("{topic} に関する成果をこのWorktreeで達成すること")
            }
        }
        SummaryLanguage::En => {
            if topic.trim().is_empty() {
                "Deliver the primary outcome for this worktree".to_string()
            } else {
                format!("Deliver the outcome intended by branch '{branch_name}'")
            }
        }
    }
}

fn clip_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut clipped = text
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    clipped.push('…');
    clipped
}

fn is_operational_status_noise(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }

    if trimmed.starts_with('$') {
        return true;
    }

    if trimmed.contains('\n') {
        return false;
    }

    if should_preserve_short_status_message(trimmed) {
        return false;
    }

    let trimmed_len = trimmed.chars().count();
    if trimmed_len <= 12 {
        return true;
    }

    if trimmed_len <= 30 {
        let word_count = trimmed.split_whitespace().count();
        if word_count <= 4 {
            return true;
        }
    }

    false
}

fn should_preserve_short_status_message(text: &str) -> bool {
    if text.ends_with('?') || text.ends_with('？') {
        return true;
    }

    let lowered = text.to_lowercase();
    const IMPORTANT_KEYWORDS: &[&str] = &[
        "error",
        "errors",
        "failed",
        "fail",
        "failure",
        "blocked",
        "blocking",
        "blocker",
        "unable",
        "cannot",
        "can't",
        "need",
        "required",
        "attention",
        "please",
        "warn",
        "warning",
        "stop",
        "wait",
        "update",
    ];

    IMPORTANT_KEYWORDS
        .iter()
        .any(|keyword| lowered.contains(keyword))
}

/// Filters noise from terminal scrollback text before AI summarization.
///
/// Applies the following line-level filters:
/// - Removes AI thinking indicator lines (e.g. "Musing...", "Thinking...")
/// - Removes progress bar / spinner lines
/// - Removes terminal chrome lines (pane headers, UI hints, symbol-heavy status rows)
/// - Collapses 3+ consecutive identical non-blank lines into 1 + `[repeated N times]`
/// - Collapses 2+ consecutive blank lines into 1
/// - Compresses 10+ consecutive build-output lines (Compiling, Downloading, etc.)
fn filter_scrollback_noise(text: &str) -> String {
    let normalized = normalize_scrollback_text(text);
    let lines: Vec<&str> = normalized.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    // First pass: remove single-line noise
    let filtered: Vec<&str> = lines
        .into_iter()
        .filter(|line| {
            let trimmed = line.trim();
            !is_ai_thinking_indicator(trimmed)
                && !is_progress_spinner_line(trimmed)
                && !is_terminal_chrome_line(trimmed)
        })
        .collect();

    // Second pass: deduplicate consecutive identical non-blank lines
    let mut deduped: Vec<String> = Vec::with_capacity(filtered.len());
    let mut i = 0;
    while i < filtered.len() {
        let current = filtered[i];
        // Skip blank lines here; pass 3 collapses them silently.
        if current.trim().is_empty() {
            deduped.push(current.to_string());
            i += 1;
            continue;
        }
        let mut count = 1usize;
        while i + count < filtered.len() && filtered[i + count] == current {
            count += 1;
        }
        deduped.push(current.to_string());
        if count >= 3 {
            deduped.push(format!("[repeated {} times]", count));
        } else {
            for _ in 1..count {
                deduped.push(current.to_string());
            }
        }
        i += count;
    }

    // Third pass: collapse consecutive blank lines (3+ → 1)
    let mut collapsed: Vec<String> = Vec::with_capacity(deduped.len());
    let mut blank_run = 0usize;
    for line in &deduped {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run <= 1 {
                collapsed.push(line.clone());
            }
        } else {
            blank_run = 0;
            collapsed.push(line.clone());
        }
    }

    // Fourth pass: compress build output runs (10+ lines → first + [...N lines...] + last)
    let mut result: Vec<String> = Vec::with_capacity(collapsed.len());
    let mut j = 0;
    while j < collapsed.len() {
        if is_build_output_line(collapsed[j].trim()) {
            let run_start = j;
            while j < collapsed.len() && is_build_output_line(collapsed[j].trim()) {
                j += 1;
            }
            let run_len = j - run_start;
            if run_len >= 10 {
                result.push(collapsed[run_start].clone());
                result.push(format!("[...{} lines...]", run_len - 2));
                result.push(collapsed[j - 1].clone());
            } else {
                for item in collapsed.iter().take(j).skip(run_start) {
                    result.push(item.clone());
                }
            }
        } else {
            result.push(collapsed[j].clone());
            j += 1;
        }
    }

    result.join("\n")
}

fn is_ai_thinking_indicator(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Short single-line status with trailing ellipsis
    if trimmed.chars().count() > 40 {
        return false;
    }
    let lowered = trimmed.to_lowercase();
    const INDICATORS: &[&str] = &[
        "musing",
        "thinking",
        "reasoning",
        "pondering",
        "analyzing",
        "considering",
        "processing",
        "generating",
        "searching",
        "planning",
        "reflecting",
        "evaluating",
    ];
    INDICATORS.iter().any(|ind| {
        lowered.starts_with(ind)
            && (lowered.ends_with("...") || lowered.ends_with('…') || lowered == *ind)
    })
}

fn is_progress_spinner_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Common spinner characters
    const SPINNERS: &[char] = &[
        '⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏', '|', '/', '-', '\\',
    ];
    if trimmed.chars().count() <= 3 && trimmed.chars().all(|c| SPINNERS.contains(&c) || c == ' ') {
        return true;
    }
    // Progress bar patterns like [=====>    ] 50%  or  ██████░░░░
    if (trimmed.contains('[')
        && trimmed.contains(']')
        && (trimmed.contains('=') || trimmed.contains('#')))
        && trimmed.chars().count() < 80
        && (trimmed.contains('%')
            || trimmed.matches('=').count() + trimmed.matches('#').count() > 5)
    {
        return true;
    }
    false
}

fn is_build_output_line(line: &str) -> bool {
    let trimmed = line.trim();
    const BUILD_PREFIXES: &[&str] = &[
        "Compiling ",
        "Downloading ",
        "Downloaded ",
        "Fetching ",
        "Installing ",
        "Resolving ",
        "Updating ",
        "Building ",
        "Linking ",
        "Finished ",
        "warning: unused",
    ];
    BUILD_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

fn normalize_scrollback_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn is_terminal_chrome_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }

    if is_scrollback_summary_heading(trimmed) {
        return true;
    }

    let lowered = trimmed.to_lowercase();
    if trimmed == "--- Status Bar ---"
        || trimmed.starts_with("--- Pane:")
        || trimmed.starts_with("────────────────")
        || trimmed.starts_with("────────")
        || lowered.starts_with("source: live (scrollback)")
        || lowered.starts_with("working (")
        || lowered == "working"
        || lowered == "plan mode"
        || lowered.contains("ctrl+o to expand")
        || lowered.contains("esc to interrupt")
        || lowered.contains("press enter to confirm")
        || lowered.contains("tokens left")
        || lowered.contains("bypass permissions")
        || lowered.contains("voice: unavailable")
        || lowered.contains("usebtw to ask")
        || lowered.contains("without interrupting")
        || lowered == "goodbye!"
    {
        return true;
    }

    is_symbol_heavy_status_line(trimmed)
}

fn is_scrollback_summary_heading(line: &str) -> bool {
    matches!(
        line,
        "## Purpose" | "## Summary" | "## Highlights" | "## 目的" | "## 要約" | "## ハイライト"
    )
}

fn is_symbol_heavy_status_line(line: &str) -> bool {
    let chars = line
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<Vec<_>>();
    if chars.len() < 12 {
        return false;
    }

    let alpha_count = chars.iter().filter(|c| c.is_alphabetic()).count();
    let digit_count = chars.iter().filter(|c| c.is_ascii_digit()).count();
    let symbol_count = chars
        .iter()
        .filter(|c| {
            matches!(
                c,
                '•' | '·'
                    | '✶'
                    | '✻'
                    | '✽'
                    | '✢'
                    | '─'
                    | '│'
                    | '┌'
                    | '┐'
                    | '└'
                    | '┘'
                    | '├'
                    | '┤'
                    | '┬'
                    | '┴'
                    | '┼'
                    | '═'
                    | '║'
                    | '╔'
                    | '╗'
                    | '╚'
                    | '╝'
                    | '╠'
                    | '╣'
                    | '╦'
                    | '╩'
                    | '╬'
                    | '⏵'
                    | '⎿'
                    | '*'
                    | '.'
                    | '_'
            )
        })
        .count();

    alpha_count == 0 && digit_count <= 2 && symbol_count * 4 >= chars.len() * 3
}

fn build_scrollback_prompt_prefix(branch_name: &str, derived_purpose: &DerivedPurpose) -> String {
    format!(
        "Branch: {branch_name}\nDerived worktree purpose ({}): {}\nPurpose guidance: \
In the Purpose section, describe the worktree objective (why). \
Treat PR/MR creation, template filling, merge/push, and status checks as means (how), not purpose.\n\nTerminal session output:\n",
        derived_purpose.confidence_label(),
        derived_purpose.text
    )
}

fn build_incremental_scrollback_prompt_prefix(
    branch_name: &str,
    derived_purpose: &DerivedPurpose,
) -> String {
    format!(
        "Branch: {branch_name}\nDerived worktree purpose ({}): {}\nPurpose guidance: \
Update the previous summary using only the new terminal output delta. \
Keep still-correct facts, replace anything contradicted by the delta, and do not repeat terminal chrome or spinner noise.\n\nPrevious summary:\n",
        derived_purpose.confidence_label(),
        derived_purpose.text
    )
}

fn build_incremental_scrollback_user_prompt(
    branch_name: &str,
    derived_purpose: &DerivedPurpose,
    previous_summary: &str,
    delta: &str,
) -> String {
    let prefix = build_incremental_scrollback_prompt_prefix(branch_name, derived_purpose);
    let delta_label = "\n\nNew terminal output since the previous summary:\n";
    let remaining_budget = MAX_PROMPT_CHARS
        .saturating_sub(prefix.chars().count())
        .saturating_sub(delta_label.chars().count());
    let previous_budget = remaining_budget / 3;
    let delta_budget = remaining_budget.saturating_sub(previous_budget);
    let previous_text = clip_text_to_budget_by_line(previous_summary, previous_budget);
    let delta_text = compress_scrollback_text(delta, delta_budget);
    format!("{prefix}{previous_text}{delta_label}{delta_text}")
}

fn clip_text_to_budget_by_line(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let lines = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return clip_chars(text, max_chars);
    }

    take_scrollback_lines_within_budget(&lines, max_chars)
}

fn determine_scrollback_summary_mode(
    normalized_input: &str,
    filtered_input: &str,
    body_budget: usize,
    current_session_id: &str,
    context: Option<&ScrollbackRollingContext>,
) -> ScrollbackSummaryMode {
    if filtered_input.chars().count() <= body_budget {
        return ScrollbackSummaryMode::FullShort;
    }

    let Some(context) = context else {
        return ScrollbackSummaryMode::FullRebuild;
    };
    let Some(previous_markdown) = context.previous_markdown.as_deref() else {
        return ScrollbackSummaryMode::FullRebuild;
    };
    let Some(previous_input) = context.previous_normalized_input.as_deref() else {
        return ScrollbackSummaryMode::FullRebuild;
    };
    if previous_markdown.trim().is_empty()
        || previous_input.trim().is_empty()
        || context.session_id.trim() != current_session_id
        || context.rolling_update_count >= MAX_ROLLING_SCROLLBACK_UPDATES
        || !normalized_input.starts_with(previous_input)
    {
        return ScrollbackSummaryMode::FullRebuild;
    }

    let delta = &normalized_input[previous_input.len()..];
    if delta.trim().is_empty() || delta.chars().count() > body_budget {
        return ScrollbackSummaryMode::FullRebuild;
    }

    ScrollbackSummaryMode::IncrementalLong
}

fn build_scrollback_user_prompt(
    normalized_input: &str,
    filtered_input: &str,
    branch_name: &str,
    derived_purpose: &DerivedPurpose,
    current_session_id: &str,
    context: Option<&ScrollbackRollingContext>,
) -> (String, ScrollbackSummaryMode, usize) {
    let full_prefix = build_scrollback_prompt_prefix(branch_name, derived_purpose);
    let body_budget = MAX_PROMPT_CHARS.saturating_sub(full_prefix.chars().count());
    let mode = determine_scrollback_summary_mode(
        normalized_input,
        filtered_input,
        body_budget,
        current_session_id,
        context,
    );

    match mode {
        ScrollbackSummaryMode::FullShort => (
            format!("{full_prefix}{filtered_input}"),
            ScrollbackSummaryMode::FullShort,
            0,
        ),
        ScrollbackSummaryMode::IncrementalLong => {
            let context = context.expect("incremental mode requires context");
            let previous_input = context
                .previous_normalized_input
                .as_deref()
                .expect("incremental mode requires previous input");
            let previous_markdown = context
                .previous_markdown
                .as_deref()
                .expect("incremental mode requires previous markdown");
            let delta = &normalized_input[previous_input.len()..];
            (
                build_incremental_scrollback_user_prompt(
                    branch_name,
                    derived_purpose,
                    previous_markdown,
                    delta,
                ),
                ScrollbackSummaryMode::IncrementalLong,
                context.rolling_update_count + 1,
            )
        }
        ScrollbackSummaryMode::FullRebuild => (
            format!(
                "{full_prefix}{}",
                compress_scrollback_text(normalized_input, body_budget)
            ),
            ScrollbackSummaryMode::FullRebuild,
            0,
        ),
    }
}

fn compress_scrollback_text(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let filtered = filter_scrollback_noise(text);
    if filtered.chars().count() <= max_chars {
        return filtered;
    }

    let lines = filtered
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return clip_chars(&filtered, max_chars);
    }

    let selected = select_prioritized_scrollback_lines(&lines, max_chars);
    let rendered = render_scrollback_selection(&lines, &selected);
    if !rendered.is_empty() {
        return rendered;
    }

    take_scrollback_lines_within_budget(&lines, max_chars)
}

fn select_prioritized_scrollback_lines(lines: &[&str], max_chars: usize) -> BTreeSet<usize> {
    let mut ranked = lines
        .iter()
        .enumerate()
        .map(|(idx, line)| (idx, scrollback_line_priority(line)))
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut selected = BTreeSet::new();
    for threshold in [95, 80, 65, 45, 20] {
        for (idx, score) in &ranked {
            if *score < threshold {
                continue;
            }
            let _ = try_insert_scrollback_line(lines, &mut selected, *idx, max_chars);
        }
        if render_scrollback_selection(lines, &selected)
            .chars()
            .count()
            >= max_chars * 9 / 10
        {
            break;
        }
    }

    selected
}

fn try_insert_scrollback_line(
    lines: &[&str],
    selected: &mut BTreeSet<usize>,
    idx: usize,
    max_chars: usize,
) -> bool {
    if selected.contains(&idx) {
        return true;
    }

    selected.insert(idx);
    let rendered = render_scrollback_selection(lines, selected);
    if rendered.chars().count() <= max_chars {
        true
    } else {
        selected.remove(&idx);
        false
    }
}

fn render_scrollback_selection(lines: &[&str], selected: &BTreeSet<usize>) -> String {
    const OMITTED_MARKER: &str = "[...omitted...]";

    let mut out = String::new();
    let mut previous: Option<usize> = None;
    for idx in selected {
        if let Some(prev) = previous {
            if *idx > prev + 1 {
                out.push('\n');
                out.push_str(OMITTED_MARKER);
                out.push('\n');
            } else {
                out.push('\n');
            }
        }
        out.push_str(lines[*idx]);
        previous = Some(*idx);
    }
    out
}

fn take_scrollback_lines_within_budget(lines: &[&str], max_chars: usize) -> String {
    let mut out = String::new();
    for line in lines {
        let addition_len = if out.is_empty() {
            line.chars().count()
        } else {
            1 + line.chars().count()
        };
        if !out.is_empty() && out.chars().count() + addition_len > max_chars {
            break;
        }
        if out.is_empty() && line.chars().count() > max_chars {
            return clip_chars(line, max_chars);
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    out
}

fn scrollback_line_priority(line: &str) -> u8 {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return 0;
    }
    if is_ai_thinking_indicator(trimmed)
        || is_progress_spinner_line(trimmed)
        || is_terminal_chrome_line(trimmed)
    {
        return 0;
    }

    let lowered = trimmed.to_lowercase();
    let mut score = 20u8;

    if looks_like_error_or_blocker_line(trimmed, &lowered) {
        score = score.max(100);
    }
    if looks_like_instruction_or_decision_line(trimmed, &lowered) {
        score = score.max(95);
    }
    if looks_like_result_or_progress_line(trimmed, &lowered) {
        score = score.max(80);
    }
    if looks_like_test_or_verification_line(trimmed, &lowered) {
        score = score.max(70);
    }
    if looks_like_command_line(trimmed) {
        score = score.max(45);
    }

    if is_probable_code_fragment(trimmed) && score < 95 {
        score = score.min(25);
    }
    if is_build_output_line(trimmed) && score < 95 {
        score = score.min(10);
    }
    if trimmed.chars().count() > MAX_MESSAGE_CHARS && score < 95 {
        score = score.min(55);
    }

    score
}

fn looks_like_error_or_blocker_line(text: &str, lowered: &str) -> bool {
    contains_any_keyword(
        text,
        &[
            "エラー",
            "失敗",
            "失敗した",
            "不具合",
            "原因",
            "ブロック",
            "確認が必要",
            "warning",
            "error",
            "failed",
            "failure",
            "panic",
            "exception",
            "blocked",
            "blocking",
            "timeout",
        ],
    ) || lowered.contains("need confirmation")
}

fn looks_like_instruction_or_decision_line(text: &str, lowered: &str) -> bool {
    contains_any_keyword(
        text,
        &[
            "依頼",
            "指示",
            "最新指示",
            "元の依頼",
            "決定事項",
            "制約",
            "やらないこと",
            "必要な入力",
            "original request",
            "latest instruction",
            "decision",
            "constraint",
            "needs user input",
        ],
    ) || lowered.starts_with("user:")
}

fn looks_like_result_or_progress_line(text: &str, lowered: &str) -> bool {
    contains_any_keyword(
        text,
        &[
            "進捗",
            "ステータス",
            "完了",
            "修正",
            "対応",
            "実装",
            "追加",
            "検証",
            "progress",
            "status",
            "done",
            "fixed",
            "resolved",
            "implemented",
            "added",
            "updated",
            "verification",
        ],
    ) || lowered.starts_with("summary:")
}

fn looks_like_test_or_verification_line(text: &str, lowered: &str) -> bool {
    contains_any_keyword(
        text,
        &[
            "テスト",
            "検証",
            "確認",
            "passed",
            "test",
            "tests",
            "clippy",
            "lint",
            "fmt",
            "markdownlint",
        ],
    ) || lowered.contains("0 failed")
        || lowered.contains("0 failures")
}

fn looks_like_command_line(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with('$')
        || trimmed.starts_with("Bash(")
        || trimmed.starts_with("● Bash(")
        || trimmed.starts_with("gh ")
        || trimmed.starts_with("git ")
        || trimmed.starts_with("cargo ")
        || trimmed.starts_with("pnpm ")
        || trimmed.starts_with("python ")
}

fn is_probable_code_fragment(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("//!")
        || trimmed.starts_with("///")
        || trimmed.starts_with("fn ")
        || trimmed.starts_with("pub ")
        || trimmed.starts_with("let ")
        || trimmed.starts_with("if ")
        || trimmed.starts_with("match ")
        || trimmed.starts_with("impl ")
        || trimmed.contains("→//!")
        || trimmed.contains("::")
}

/// Summarizes a terminal scrollback as plain text, bypassing session parsers.
///
/// The scrollback text should already have ANSI sequences stripped.
/// Large texts are compressed by prioritizing meaningful lines to fit within MAX_PROMPT_CHARS.
pub fn summarize_scrollback(
    client: &AIClient,
    scrollback_text: &str,
    branch_name: &str,
    language: &str,
    current_session_id: &str,
    context: Option<&ScrollbackRollingContext>,
) -> Result<ScrollbackSummaryBuild, AIError> {
    let normalized = normalize_scrollback_text(scrollback_text);
    let filtered = filter_scrollback_noise(&normalized);
    let fallback_language = fallback_purpose_language_for_text(language, &filtered);
    let derived_purpose =
        derive_worktree_purpose_from_scrollback(&filtered, branch_name, fallback_language);
    let (mut user_prompt, mode, next_rolling_update_count) = build_scrollback_user_prompt(
        &normalized,
        &filtered,
        branch_name,
        &derived_purpose,
        current_session_id,
        context,
    );
    if user_prompt.chars().count() > MAX_PROMPT_CHARS {
        user_prompt = clip_chars(&user_prompt, MAX_PROMPT_CHARS);
    }
    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: session_system_prompt(language).to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_prompt,
        },
    ];
    let content = client.create_response(messages)?;
    let fields = parse_session_summary_fields(&content).unwrap_or_default();
    let markdown = normalize_session_summary_markdown(&content, &fields, language)?;
    let summary_lang = target_summary_language(language, &markdown);
    let markdown = enforce_worktree_purpose_in_markdown(
        &markdown,
        summary_lang,
        branch_name,
        &derived_purpose,
    );
    validate_session_summary_markdown(&markdown)?;
    let task_overview = extract_purpose_body(&markdown);

    let token_count = scrollback_text.chars().count() / 4;
    let metrics = SessionMetrics {
        token_count: if token_count > 0 {
            Some(token_count)
        } else {
            None
        },
        tool_execution_count: 0,
        elapsed_seconds: None,
        turn_count: 0,
    };

    Ok(ScrollbackSummaryBuild {
        summary: SessionSummary {
            task_overview: task_overview.or(fields.task_overview),
            short_summary: fields.short_summary,
            bullet_points: fields.bullet_points,
            markdown: Some(markdown),
            metrics,
            last_updated: Some(SystemTime::now()),
        },
        normalized_input: normalized,
        mode,
        next_rolling_update_count,
    })
}

pub fn summarize_session(
    client: &AIClient,
    parsed: &ParsedSession,
    branch_name: &str,
    language: &str,
) -> Result<SessionSummary, AIError> {
    let fallback_language = fallback_purpose_language_for_messages(language, &parsed.messages);
    let derived_purpose =
        derive_worktree_purpose_from_messages(&parsed.messages, branch_name, fallback_language);
    let messages = build_session_prompt_with_context(
        parsed,
        language,
        Some(branch_name),
        Some(&derived_purpose),
    );
    let content = client.create_response(messages)?;
    let fields = parse_session_summary_fields(&content).unwrap_or_default();
    let markdown = normalize_session_summary_markdown(&content, &fields, language)?;
    let summary_lang = target_summary_language(language, &markdown);
    let markdown = enforce_worktree_purpose_in_markdown(
        &markdown,
        summary_lang,
        branch_name,
        &derived_purpose,
    );
    validate_session_summary_markdown(&markdown)?;
    let task_overview = extract_purpose_body(&markdown);

    let metrics = build_metrics(parsed);

    Ok(SessionSummary {
        task_overview: task_overview.or(fields.task_overview),
        short_summary: fields.short_summary,
        bullet_points: fields.bullet_points,
        markdown: Some(markdown),
        metrics,
        last_updated: Some(SystemTime::now()),
    })
}

fn build_metrics(parsed: &ParsedSession) -> SessionMetrics {
    let token_count = estimate_token_count(&parsed.messages);
    let elapsed_seconds = match (parsed.started_at, parsed.last_updated_at) {
        (Some(start), Some(end)) => {
            let duration = end.signed_duration_since(start);
            duration.num_seconds().max(0) as u64
        }
        _ => 0,
    };

    SessionMetrics {
        token_count: if token_count > 0 {
            Some(token_count)
        } else {
            None
        },
        tool_execution_count: parsed.tool_executions.len(),
        elapsed_seconds: if elapsed_seconds > 0 {
            Some(elapsed_seconds)
        } else {
            None
        },
        turn_count: if parsed.total_turns > 0 {
            parsed.total_turns
        } else {
            parsed.messages.len()
        },
    }
}

fn estimate_token_count(messages: &[SessionMessage]) -> usize {
    let total_chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
    if total_chars == 0 {
        return 0;
    }
    (total_chars / 4).max(1)
}

fn parse_session_summary_fields(content: &str) -> Result<SessionSummaryFields, AIError> {
    if let Some(fields) = parse_json_summary(content) {
        return Ok(fields);
    }

    let bullet_points = parse_summary_lines(content).unwrap_or_default();
    let short_summary = bullet_points
        .first()
        .map(|line| line.trim_start_matches("- ").to_string());

    Ok(SessionSummaryFields {
        task_overview: None,
        short_summary,
        bullet_points,
    })
}

fn normalize_session_summary_markdown(
    content: &str,
    fields: &SessionSummaryFields,
    language: &str,
) -> Result<String, AIError> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(AIError::ParseError("Empty summary".to_string()));
    }

    let target = target_summary_language(language, trimmed);

    if parse_json_summary(trimmed).is_some() {
        return Ok(build_markdown_from_fields(fields, target));
    }

    if looks_like_markdown(trimmed) {
        return Ok(normalize_summary_headings(trimmed, target));
    }

    Ok(build_markdown_from_fields(fields, target))
}

fn validate_session_summary_markdown(markdown: &str) -> Result<(), AIError> {
    let mut stage = SummaryStage::Start;
    let mut lang: Option<SummaryLanguage> = None;
    let mut has_bullet = false;

    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("## ") {
            let title = title.trim();
            match stage {
                SummaryStage::Start => {
                    if heading_matches(title, &["目的"]) {
                        lang = Some(SummaryLanguage::Ja);
                        stage = SummaryStage::Purpose;
                        continue;
                    }
                    if heading_matches(title, &["Purpose", "purpose"]) {
                        lang = Some(SummaryLanguage::En);
                        stage = SummaryStage::Purpose;
                        continue;
                    }
                }
                SummaryStage::Purpose => match lang.unwrap_or(SummaryLanguage::Ja) {
                    SummaryLanguage::Ja => {
                        if heading_matches(title, &["要約", "概要"]) {
                            stage = SummaryStage::Summary;
                            continue;
                        }
                    }
                    SummaryLanguage::En => {
                        if heading_matches(title, &["Summary", "summary"]) {
                            stage = SummaryStage::Summary;
                            continue;
                        }
                    }
                },
                SummaryStage::Summary => match lang.unwrap_or(SummaryLanguage::Ja) {
                    SummaryLanguage::Ja => {
                        if heading_matches(title, &["ハイライト"]) {
                            stage = SummaryStage::Highlight;
                            continue;
                        }
                    }
                    SummaryLanguage::En => {
                        if heading_matches(title, &["Highlights", "highlights"]) {
                            stage = SummaryStage::Highlight;
                            continue;
                        }
                    }
                },
                SummaryStage::Highlight => stage = SummaryStage::Done,
                SummaryStage::Done => {}
            }
            if stage == SummaryStage::Highlight {
                stage = SummaryStage::Done;
            }
            continue;
        }

        if stage == SummaryStage::Highlight && is_bullet_line(trimmed) {
            has_bullet = true;
        }
    }

    if (stage == SummaryStage::Highlight || stage == SummaryStage::Done) && has_bullet {
        return Ok(());
    }

    Err(AIError::IncompleteSummary)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeadingKind {
    Purpose,
    Summary,
    Highlights,
}

fn classify_heading(title: &str) -> Option<HeadingKind> {
    if heading_matches(title, &["目的", "Purpose", "purpose"]) {
        return Some(HeadingKind::Purpose);
    }
    if heading_matches(title, &["要約", "概要", "Summary", "summary"]) {
        return Some(HeadingKind::Summary);
    }
    if heading_matches(title, &["ハイライト", "Highlights", "highlights"]) {
        return Some(HeadingKind::Highlights);
    }
    None
}

fn canonical_heading(kind: HeadingKind, lang: SummaryLanguage) -> &'static str {
    match (kind, lang) {
        (HeadingKind::Purpose, SummaryLanguage::Ja) => "目的",
        (HeadingKind::Purpose, SummaryLanguage::En) => "Purpose",
        (HeadingKind::Summary, SummaryLanguage::Ja) => "要約",
        (HeadingKind::Summary, SummaryLanguage::En) => "Summary",
        (HeadingKind::Highlights, SummaryLanguage::Ja) => "ハイライト",
        (HeadingKind::Highlights, SummaryLanguage::En) => "Highlights",
    }
}

fn normalize_summary_headings(markdown: &str, lang: SummaryLanguage) -> String {
    let mut out = String::with_capacity(markdown.len());
    let lines: Vec<&str> = markdown.lines().collect();
    let last_idx = lines.len().saturating_sub(1);
    for (idx, line) in lines.iter().enumerate() {
        let line = *line;
        if let Some(title) = line.trim_start().strip_prefix("## ") {
            let normalized_title = classify_heading(title)
                .map(|kind| canonical_heading(kind, lang).to_string())
                .unwrap_or_else(|| title.trim().to_string());
            out.push_str("## ");
            out.push_str(&normalized_title);
            if idx < last_idx {
                out.push('\n');
            }
            continue;
        }
        out.push_str(line);
        if idx < last_idx {
            out.push('\n');
        }
    }
    out
}

fn enforce_worktree_purpose_in_markdown(
    markdown: &str,
    lang: SummaryLanguage,
    branch_name: &str,
    derived_purpose: &DerivedPurpose,
) -> String {
    let lines: Vec<&str> = markdown.lines().collect();
    if lines.is_empty() {
        return markdown.to_string();
    }

    let mut purpose_idx: Option<usize> = None;
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let Some(title) = trimmed.strip_prefix("## ") else {
            continue;
        };
        if matches!(classify_heading(title.trim()), Some(HeadingKind::Purpose)) {
            purpose_idx = Some(idx);
            break;
        }
    }

    let Some(start) = purpose_idx else {
        return markdown.to_string();
    };

    let mut next_heading = lines.len();
    for (idx, line) in lines.iter().enumerate().skip(start + 1) {
        if line.trim_start().starts_with("## ") {
            next_heading = idx;
            break;
        }
    }

    let existing_body = lines[start + 1..next_heading].join("\n");
    let existing_trimmed = existing_body.trim();
    let desired = derived_purpose.render_for_output(lang);
    let existing_has_inferred_marker = has_inferred_marker(existing_trimmed, lang);
    let should_replace = matches!(derived_purpose.source, PurposeSource::Explicit)
        || existing_trimmed.is_empty()
        || is_unknown_placeholder(existing_trimmed)
        || is_operational_only(existing_trimmed);

    if !should_replace {
        if derived_purpose.is_inferred() && !existing_has_inferred_marker {
            let annotated = annotate_as_inferred(existing_trimmed, lang);
            let prefix = lines[..=start].join("\n");
            if next_heading >= lines.len() {
                return format!("{prefix}\n{annotated}");
            }
            let suffix = lines[next_heading..].join("\n");
            return format!("{prefix}\n{annotated}\n\n{suffix}");
        }
        return markdown.to_string();
    }

    let desired_body = if desired.trim().is_empty() {
        match lang {
            SummaryLanguage::Ja => {
                format!(
                    "（推定）{}",
                    infer_purpose_from_branch(branch_name, SummaryLanguage::Ja)
                )
            }
            SummaryLanguage::En => format!(
                "(Inferred) {}",
                infer_purpose_from_branch(branch_name, SummaryLanguage::En)
            ),
        }
    } else {
        desired
    };

    let prefix = lines[..=start].join("\n");
    if next_heading >= lines.len() {
        return format!("{prefix}\n{desired_body}");
    }
    let suffix = lines[next_heading..].join("\n");
    format!("{prefix}\n{desired_body}\n\n{suffix}")
}

fn has_inferred_marker(text: &str, lang: SummaryLanguage) -> bool {
    match lang {
        SummaryLanguage::Ja => text.trim_start().starts_with("（推定）"),
        SummaryLanguage::En => text.trim_start().starts_with("(Inferred)"),
    }
}

fn annotate_as_inferred(text: &str, lang: SummaryLanguage) -> String {
    let trimmed = text.trim();
    match lang {
        SummaryLanguage::Ja => format!("（推定）{trimmed}"),
        SummaryLanguage::En => format!("(Inferred) {trimmed}"),
    }
}

fn extract_purpose_body(markdown: &str) -> Option<String> {
    let lines: Vec<&str> = markdown.lines().collect();
    if lines.is_empty() {
        return None;
    }

    let mut start: Option<usize> = None;
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let Some(title) = trimmed.strip_prefix("## ") else {
            continue;
        };
        if matches!(classify_heading(title.trim()), Some(HeadingKind::Purpose)) {
            start = Some(idx + 1);
            break;
        }
    }

    let start = start?;
    let mut end = lines.len();
    for (idx, line) in lines.iter().enumerate().skip(start) {
        if line.trim_start().starts_with("## ") {
            end = idx;
            break;
        }
    }

    let body = lines[start..end].join("\n").trim().to_string();
    if body.is_empty() {
        None
    } else {
        Some(body)
    }
}

fn heading_matches(title: &str, expected: &[&str]) -> bool {
    let trimmed = title.trim();
    expected.iter().any(|exp| {
        if trimmed == *exp {
            return true;
        }
        if let Some(rest) = trimmed.strip_prefix(exp) {
            let rest = rest.trim_start();
            rest.is_empty()
                || rest.starts_with('(')
                || rest.starts_with('（')
                || rest.starts_with(':')
                || rest.starts_with('：')
        } else {
            false
        }
    })
}

fn is_bullet_line(line: &str) -> bool {
    if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("•") {
        return true;
    }
    strip_ordered_prefix(line).is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SummaryStage {
    Start,
    Purpose,
    Summary,
    Highlight,
    Done,
}

fn looks_like_markdown(content: &str) -> bool {
    content.contains("## ")
        || content.contains("\n- ")
        || content.contains("\n* ")
        || content.contains("\n1.")
        || content.contains("\n1)")
}

fn build_markdown_from_fields(fields: &SessionSummaryFields, lang: SummaryLanguage) -> String {
    let purpose = fields
        .task_overview
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(match lang {
            SummaryLanguage::Ja => "(不明)",
            SummaryLanguage::En => "(Not available)",
        });
    let summary = fields
        .short_summary
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(match lang {
            SummaryLanguage::Ja => "(不明)",
            SummaryLanguage::En => "(Not available)",
        });

    let mut out = String::new();
    out.push_str("## ");
    out.push_str(canonical_heading(HeadingKind::Purpose, lang));
    out.push('\n');
    out.push_str(purpose);
    out.push_str("\n\n## ");
    out.push_str(canonical_heading(HeadingKind::Summary, lang));
    out.push('\n');
    out.push_str(summary);
    out.push_str("\n\n## ");
    out.push_str(canonical_heading(HeadingKind::Highlights, lang));
    out.push('\n');
    if fields.bullet_points.is_empty() {
        match lang {
            SummaryLanguage::Ja => out.push_str("- (ハイライトなし)\n"),
            SummaryLanguage::En => out.push_str("- (No highlights)\n"),
        }
    } else {
        for bullet in fields.bullet_points.iter().take(3) {
            let line = bullet.trim_start_matches("- ").trim();
            out.push_str("- ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn target_summary_language(requested: &str, content: &str) -> SummaryLanguage {
    match requested.trim() {
        "ja" => SummaryLanguage::Ja,
        "en" => SummaryLanguage::En,
        "auto" => detect_summary_language(content).unwrap_or_else(|| {
            if contains_japanese(content) {
                SummaryLanguage::Ja
            } else {
                SummaryLanguage::En
            }
        }),
        _ => SummaryLanguage::En,
    }
}

fn fallback_purpose_language_for_messages(
    requested: &str,
    messages: &[SessionMessage],
) -> SummaryLanguage {
    match requested.trim() {
        "ja" => SummaryLanguage::Ja,
        "en" => SummaryLanguage::En,
        "auto" => {
            if messages
                .iter()
                .any(|message| contains_japanese(&message.content))
            {
                SummaryLanguage::Ja
            } else {
                SummaryLanguage::En
            }
        }
        _ => SummaryLanguage::En,
    }
}

fn fallback_purpose_language_for_text(requested: &str, text: &str) -> SummaryLanguage {
    match requested.trim() {
        "ja" => SummaryLanguage::Ja,
        "en" => SummaryLanguage::En,
        "auto" => {
            if contains_japanese(text) {
                SummaryLanguage::Ja
            } else {
                SummaryLanguage::En
            }
        }
        _ => SummaryLanguage::En,
    }
}

fn detect_summary_language(content: &str) -> Option<SummaryLanguage> {
    for line in content.lines() {
        let trimmed = line.trim();
        let Some(title) = trimmed.strip_prefix("## ") else {
            continue;
        };
        let title = title.trim();
        if heading_matches(title, &["目的", "要約", "概要", "ハイライト"]) {
            return Some(SummaryLanguage::Ja);
        }
        if heading_matches(
            title,
            &[
                "Purpose",
                "purpose",
                "Summary",
                "summary",
                "Highlights",
                "highlights",
            ],
        ) {
            return Some(SummaryLanguage::En);
        }
    }
    None
}

fn parse_json_summary(content: &str) -> Option<SessionSummaryFields> {
    let candidate = content.trim();
    if candidate.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(candidate) {
            return extract_fields_from_json(&value);
        }
    }

    if let Some((start, end)) = find_json_bounds(candidate) {
        let slice = &candidate[start..=end];
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(slice) {
            return extract_fields_from_json(&value);
        }
    }

    None
}

fn extract_fields_from_json(value: &serde_json::Value) -> Option<SessionSummaryFields> {
    let task_overview = value
        .get("task_overview")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let short_summary = value
        .get("short_summary")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let bullets_value = value
        .get("bullets")
        .or_else(|| value.get("bullet_points"))
        .or_else(|| value.get("bulletPoints"));

    let mut bullet_points = Vec::new();
    if let Some(bullets) = bullets_value {
        if let Some(arr) = bullets.as_array() {
            for item in arr {
                if let Some(text) = item.as_str() {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        bullet_points.push(normalize_bullet(trimmed));
                    }
                }
            }
        } else if let Some(text) = bullets.as_str() {
            if let Ok(lines) = parse_summary_lines(text) {
                bullet_points = lines;
            }
        }
    }

    Some(SessionSummaryFields {
        task_overview,
        short_summary,
        bullet_points,
    })
}

fn normalize_bullet(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with("- ") {
        trimmed.to_string()
    } else {
        format!("- {}", trimmed)
    }
}

fn find_json_bounds(value: &str) -> Option<(usize, usize)> {
    let start = value.find('{')?;
    let end = value.rfind('}')?;
    if start < end {
        Some((start, end))
    } else {
        None
    }
}

pub fn parse_summary_lines(content: &str) -> Result<Vec<String>, AIError> {
    let mut lines: Vec<String> = content.lines().filter_map(normalize_line).collect();

    if lines.is_empty() {
        let cleaned = content.trim();
        if cleaned.is_empty() {
            return Err(AIError::ParseError("Empty summary".to_string()));
        }
        for sentence in cleaned.split_terminator(". ") {
            let trimmed = sentence.trim();
            if trimmed.is_empty() {
                continue;
            }
            lines.push(format!("- {}", trimmed.trim_end_matches('.')));
        }
    }

    if lines.is_empty() {
        return Err(AIError::ParseError("No summary lines".to_string()));
    }

    Ok(lines)
}

fn normalize_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let trimmed = if let Some(rest) = trimmed.strip_prefix("- ") {
        rest.trim()
    } else if let Some(rest) = trimmed.strip_prefix('-') {
        rest.trim()
    } else if let Some(rest) = trimmed.strip_prefix("* ") {
        rest.trim()
    } else if let Some(rest) = trimmed.strip_prefix('*') {
        rest.trim()
    } else if let Some(rest) = trimmed.strip_prefix("•") {
        rest.trim()
    } else if let Some(rest) = strip_ordered_prefix(trimmed) {
        rest.trim()
    } else {
        trimmed
    };

    if trimmed.is_empty() {
        return None;
    }

    Some(format!("- {}", trimmed))
}

fn strip_ordered_prefix(value: &str) -> Option<&str> {
    let mut chars = value.chars();
    let mut digit_count = 0usize;
    while let Some(ch) = chars.next() {
        if ch.is_ascii_digit() {
            digit_count += 1;
            continue;
        }
        if digit_count > 0 && (ch == '.' || ch == ')') {
            return Some(chars.as_str().trim_start());
        }
        break;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_summary_lines_bullets() {
        let content = "- Added login\n- Fixed bug\n- Updated docs";
        let lines = parse_summary_lines(content).expect("should parse");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "- Added login");
    }

    #[test]
    fn test_parse_summary_lines_allows_more_than_three() {
        let content = "- A\n- B\n- C\n- D\n- E";
        let lines = parse_summary_lines(content).expect("should parse");
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[3], "- D");
    }

    #[test]
    fn test_parse_summary_lines_ordered() {
        let content = "1. Added login\n2) Fixed bug";
        let lines = parse_summary_lines(content).expect("should parse");
        assert_eq!(lines[0], "- Added login");
        assert_eq!(lines[1], "- Fixed bug");
    }

    #[test]
    fn test_parse_session_summary_fields_keeps_all_bullets_from_json() {
        let content =
            r#"{"task_overview":"目的","short_summary":"要約","bullets":["A","B","C","D"]}"#;
        let fields = parse_session_summary_fields(content).expect("parse fields");
        assert_eq!(fields.bullet_points.len(), 4);
        assert_eq!(fields.bullet_points[3], "- D");
    }

    #[test]
    fn test_session_summary_cache_stale_session_id() {
        let mut cache = SessionSummaryCache::default();
        let summary = SessionSummary::default();
        let now = SystemTime::now();
        cache.set(
            "main".to_string(),
            "codex-cli".to_string(),
            "sess-1".to_string(),
            "en".to_string(),
            summary,
            now,
        );
        assert!(cache.is_stale("main", "sess-2", "en", now));
        assert!(cache.is_stale("main", "sess-1", "ja", now));
        assert!(!cache.is_stale("main", "sess-1", "en", now));
    }

    #[test]
    fn test_session_summary_cache_plain_set_clears_scrollback_context() {
        let mut cache = SessionSummaryCache::default();
        let summary = SessionSummary::default();
        let now = SystemTime::now();

        cache.set_scrollback(
            "main".to_string(),
            "codex-cli".to_string(),
            "pane:123".to_string(),
            "en".to_string(),
            summary.clone(),
            now,
            "first\nsecond".to_string(),
            2,
        );
        assert_eq!(cache.scrollback_input("main"), Some("first\nsecond"));
        assert_eq!(cache.scrollback_update_count("main"), 2);

        cache.set(
            "main".to_string(),
            "codex-cli".to_string(),
            "sess-2".to_string(),
            "en".to_string(),
            summary,
            now,
        );
        assert!(cache.scrollback_input("main").is_none());
        assert_eq!(cache.scrollback_update_count("main"), 0);
    }

    #[test]
    fn test_build_session_prompt_caps_length() {
        let long_text = "a".repeat(2000);
        let messages = (0..200)
            .map(|_| SessionMessage {
                role: MessageRole::User,
                content: long_text.clone(),
                timestamp: None,
            })
            .collect::<Vec<_>>();
        let parsed = ParsedSession {
            session_id: "sess-1".to_string(),
            agent_type: crate::ai::AgentType::CodexCli,
            messages,
            tool_executions: vec![],
            started_at: None,
            last_updated_at: None,
            total_turns: 200,
        };

        let prompt = build_session_prompt(&parsed, "auto");
        let user_prompt = prompt
            .iter()
            .find(|msg| msg.role == "user")
            .expect("user prompt")
            .content
            .clone();

        assert!(user_prompt.chars().count() <= MAX_PROMPT_CHARS);
    }

    #[test]
    fn test_build_session_prompt_filters_assistant_operational_noise() {
        let messages = vec![
            SessionMessage {
                role: MessageRole::User,
                content: "AI要約の本質を改善したいです。".to_string(),
                timestamp: None,
            },
            SessionMessage {
                role: MessageRole::Assistant,
                content: "$gh-pr".to_string(),
                timestamp: None,
            },
            SessionMessage {
                role: MessageRole::Assistant,
                content: "Implemented the plan to compress summary input and keep only meaningful messages.".to_string(),
                timestamp: None,
            },
            SessionMessage {
                role: MessageRole::Assistant,
                content: "Implemented.".to_string(),
                timestamp: None,
            },
        ];
        let parsed = ParsedSession {
            session_id: "sess-noise".to_string(),
            agent_type: crate::ai::AgentType::CodexCli,
            messages,
            tool_executions: vec![],
            started_at: None,
            last_updated_at: None,
            total_turns: 3,
        };

        let prompt = build_session_prompt(&parsed, "auto");
        let user_prompt = prompt
            .iter()
            .find(|msg| msg.role == "user")
            .expect("user prompt")
            .content
            .clone();

        assert!(user_prompt.contains("AI要約の本質を改善したいです。"));
        assert!(!user_prompt.contains("$gh-pr"));
        assert!(user_prompt.contains(
            "Implemented the plan to compress summary input and keep only meaningful messages."
        ));
        assert!(!user_prompt.contains("Implemented."));
    }

    #[test]
    fn test_build_session_prompt_keeps_short_assistant_blocker_or_question() {
        let messages = vec![
            SessionMessage {
                role: MessageRole::User,
                content: "テストを続けます。".to_string(),
                timestamp: None,
            },
            SessionMessage {
                role: MessageRole::Assistant,
                content: "Error: build blocked.".to_string(),
                timestamp: None,
            },
            SessionMessage {
                role: MessageRole::Assistant,
                content: "Need confirmation?".to_string(),
                timestamp: None,
            },
            SessionMessage {
                role: MessageRole::Assistant,
                content: "go".to_string(),
                timestamp: None,
            },
        ];

        let parsed = ParsedSession {
            session_id: "sess-blocker".to_string(),
            agent_type: crate::ai::AgentType::CodexCli,
            messages,
            tool_executions: vec![],
            started_at: None,
            last_updated_at: None,
            total_turns: 4,
        };

        let prompt = build_session_prompt(&parsed, "auto");
        let user_prompt = prompt
            .iter()
            .find(|msg| msg.role == "user")
            .expect("user prompt")
            .content
            .clone();

        assert!(user_prompt.contains("Error: build blocked."));
        assert!(user_prompt.contains("Need confirmation?"));
        assert!(!user_prompt.contains("go"));
    }

    #[test]
    fn test_derive_worktree_purpose_prefers_latest_explicit_user_message() {
        let messages = vec![
            SessionMessage {
                role: MessageRole::User,
                content: "目的: 古い目的".to_string(),
                timestamp: None,
            },
            SessionMessage {
                role: MessageRole::Assistant,
                content: "了解です".to_string(),
                timestamp: None,
            },
            SessionMessage {
                role: MessageRole::User,
                content:
                    "違います。目的はfeature/project-modeで作った成果をdevelopに取り込むことです。"
                        .to_string(),
                timestamp: None,
            },
        ];

        let derived = derive_worktree_purpose_from_messages(
            &messages,
            "feature/project-mode",
            SummaryLanguage::Ja,
        );
        assert_eq!(derived.source, PurposeSource::Explicit);
        assert!(derived.text.contains("feature/project-mode"));
    }

    #[test]
    fn test_derive_worktree_purpose_falls_back_to_inferred_branch_goal() {
        let messages = vec![
            SessionMessage {
                role: MessageRole::User,
                content: "PR本文テンプレートを埋めてください。".to_string(),
                timestamp: None,
            },
            SessionMessage {
                role: MessageRole::User,
                content: "PRを作成してURLを確認".to_string(),
                timestamp: None,
            },
        ];

        let derived = derive_worktree_purpose_from_messages(
            &messages,
            "feature/project-mode",
            SummaryLanguage::En,
        );
        assert_eq!(derived.source, PurposeSource::Inferred);
        assert!(derived.text.contains("feature/project-mode"));
    }

    #[test]
    fn test_derive_worktree_purpose_falls_back_to_requested_japanese_goal() {
        let messages = vec![SessionMessage {
            role: MessageRole::User,
            content: "PR本文テンプレートを埋めてください。".to_string(),
            timestamp: None,
        }];

        let derived = derive_worktree_purpose_from_messages(
            &messages,
            "feature/project-mode",
            SummaryLanguage::Ja,
        );
        assert_eq!(derived.source, PurposeSource::Inferred);
        assert!(derived.text.contains("成果をこのWorktreeで達成すること"));
        assert!(!derived.text.contains("Deliver the outcome intended"));
    }

    #[test]
    fn test_enforce_worktree_purpose_replaces_operational_purpose_text() {
        let markdown = "## 目的\nPR本文テンプレートを埋めてPRを作ること\n\n## 要約\n進行中\n\n## ハイライト\n- 元の依頼: ...";
        let derived = DerivedPurpose::explicit(
            "feature/project-mode の成果を develop に取り込むこと".to_string(),
        );
        let enforced = enforce_worktree_purpose_in_markdown(
            markdown,
            SummaryLanguage::Ja,
            "feature/project-mode",
            &derived,
        );

        assert!(enforced.contains("feature/project-mode の成果を develop に取り込むこと"));
        assert!(!enforced.contains("PR本文テンプレートを埋めてPRを作ること"));
    }

    #[test]
    fn test_enforce_worktree_purpose_keeps_existing_when_inferred_and_valid() {
        let markdown =
            "## 目的\nIssue タブの改善成果を取り込むこと\n\n## 要約\n進行中\n\n## ハイライト\n- 元の依頼: ...";
        let derived = DerivedPurpose::inferred(
            "Deliver the outcome intended by branch 'feature/issue-tab'".to_string(),
        );

        let enforced = enforce_worktree_purpose_in_markdown(
            markdown,
            SummaryLanguage::Ja,
            "feature/issue-tab",
            &derived,
        );

        assert!(enforced.contains("## 目的\n（推定）Issue タブの改善成果を取り込むこと"));
    }

    #[test]
    fn test_enforce_worktree_purpose_rewrites_pr_template_as_means() {
        let markdown = "## 目的\nfeature/project-mode ブランチから develop への PR を作成し、PR 本文テンプレートを埋めること\n\n## 要約\n完了\n\n## ハイライト\n- Original request: feature/project-mode ブランチから develop への PR を作成\n- Latest instruction: PR 本文テンプレートを埋める";
        let derived = DerivedPurpose::explicit(
            "feature/project-mode で達成した成果を develop に取り込むこと".to_string(),
        );

        let enforced = enforce_worktree_purpose_in_markdown(
            markdown,
            SummaryLanguage::Ja,
            "feature/project-mode",
            &derived,
        );

        assert!(enforced.contains("feature/project-mode で達成した成果を develop に取り込むこと"));
        assert!(!enforced.contains("PR 本文テンプレートを埋めること"));
    }

    #[test]
    fn test_normalize_session_summary_markdown_from_json() {
        let content =
            r#"{"task_overview":"目的文","short_summary":"要約文","bullets":["項目1","項目2"]}"#;
        let fields = parse_session_summary_fields(content).expect("parse fields");
        let markdown =
            normalize_session_summary_markdown(content, &fields, "ja").expect("markdown");
        assert!(markdown.contains("## 目的"));
        assert!(markdown.contains("目的文"));
        assert!(markdown.contains("## 要約"));
        assert!(markdown.contains("要約文"));
        assert!(markdown.contains("## ハイライト"));
        assert!(markdown.contains("- 項目1"));
    }

    #[test]
    fn test_normalize_session_summary_markdown_from_json_english_headings() {
        let content = r#"{"task_overview":"Purpose text","short_summary":"Summary text","bullets":["A","B"]}"#;
        let fields = parse_session_summary_fields(content).expect("parse fields");
        let markdown =
            normalize_session_summary_markdown(content, &fields, "en").expect("markdown");
        assert!(markdown.contains("## Purpose"));
        assert!(markdown.contains("## Summary"));
        assert!(markdown.contains("## Highlights"));
    }

    #[test]
    fn test_normalize_session_summary_markdown_passthrough() {
        let content = "## 目的\nA\n\n## 要約\nB\n\n## ハイライト\n- C";
        let fields = SessionSummaryFields::default();
        let markdown =
            normalize_session_summary_markdown(content, &fields, "ja").expect("markdown");
        assert_eq!(markdown, content);
    }

    #[test]
    fn test_normalize_session_summary_markdown_normalizes_alternative_summary_heading() {
        let content = "## 目的\nA\n\n## 概要\nB\n\n## ハイライト\n- C";
        let fields = SessionSummaryFields::default();
        let markdown =
            normalize_session_summary_markdown(content, &fields, "ja").expect("markdown");
        assert_eq!(markdown, "## 目的\nA\n\n## 要約\nB\n\n## ハイライト\n- C");
    }

    #[test]
    fn test_normalize_session_summary_markdown_normalizes_english_headings() {
        let content = "## Purpose\nA\n\n## Summary\nB\n\n## Highlights\n- C";
        let fields = SessionSummaryFields::default();
        let markdown =
            normalize_session_summary_markdown(content, &fields, "ja").expect("markdown");
        assert_eq!(markdown, "## 目的\nA\n\n## 要約\nB\n\n## ハイライト\n- C");
    }

    #[test]
    fn test_validate_session_summary_markdown_accepts_complete() {
        let content = "## 目的\nA\n\n## 要約\nB\n\n## ハイライト\n- C";
        assert!(validate_session_summary_markdown(content).is_ok());
    }

    #[test]
    fn test_validate_session_summary_markdown_accepts_english_headings() {
        let content = "## Purpose\nA\n\n## Summary\nB\n\n## Highlights\n- C";
        assert!(validate_session_summary_markdown(content).is_ok());
    }

    #[test]
    fn test_validate_session_summary_markdown_accepts_overview_heading_alias() {
        let content = "## 目的\nA\n\n## 概要\nB\n\n## ハイライト\n- C";
        assert!(validate_session_summary_markdown(content).is_ok());
    }

    #[test]
    fn test_validate_session_summary_markdown_accepts_many_bullets() {
        let content = "## 目的\nA\n\n## 要約\nB\n\n## ハイライト\n- C\n- D\n- E\n- F";
        assert!(validate_session_summary_markdown(content).is_ok());
    }

    #[test]
    fn test_validate_session_summary_markdown_rejects_missing_highlight() {
        let content = "## 目的\nA\n\n## 要約\nB\n";
        let result = validate_session_summary_markdown(content);
        assert!(matches!(result, Err(AIError::IncompleteSummary)));
    }

    #[test]
    fn test_validate_session_summary_markdown_rejects_missing_bullets() {
        let content = "## 目的\nA\n\n## 要約\nB\n\n## ハイライト\n";
        let result = validate_session_summary_markdown(content);
        assert!(matches!(result, Err(AIError::IncompleteSummary)));
    }

    // --- normalize_scrollback_text / compress_scrollback_text tests ---

    #[test]
    fn test_normalize_scrollback_text_normalizes_carriage_returns() {
        let input = "line1\rline2\r\nline3";
        let result = normalize_scrollback_text(input);
        assert_eq!(result, "line1\nline2\nline3");
    }

    #[test]
    fn test_compress_scrollback_text_keeps_line_boundaries() {
        let input = (0..24)
            .map(|idx| format!("line-{idx:02}-{}", "x".repeat(40)))
            .collect::<Vec<_>>()
            .join("\n");
        let original_lines = input.lines().collect::<Vec<_>>();

        let result = compress_scrollback_text(&input, 220);
        assert!(result.chars().count() <= 220);
        for line in result.lines() {
            assert!(
                line == "[...omitted...]" || original_lines.contains(&line),
                "unexpected line in compressed output: {line}"
            );
        }
    }

    #[test]
    fn test_compress_scrollback_text_prioritizes_meaningful_lines() {
        let input = [
            "--- Status Bar ---",
            "Working (30s • esc to interrupt)",
            "ctrl+o to expand",
            "Latest instruction: improve summary quality",
            "error[E0308]: mismatched types",
            "3→//! This command tree is intentionally hidden from --help.",
            "Progress: implemented backend filtering",
            "pnpm test",
        ]
        .join("\n");

        let result = compress_scrollback_text(&input, 160);
        assert!(result.contains("Latest instruction: improve summary quality"));
        assert!(result.contains("error[E0308]: mismatched types"));
        assert!(result.contains("Progress: implemented backend filtering"));
        assert!(!result.contains("Status Bar"));
        assert!(!result.contains("Working (30s"));
        assert!(!result.contains("ctrl+o to expand"));
        assert!(!result.contains("3→//!"));
    }

    #[test]
    fn test_build_scrollback_user_prompt_full_short_uses_filtered_text() {
        let normalized = normalize_scrollback_text(
            "Working (30s • esc to interrupt)\nLatest instruction: improve summary quality",
        );
        let filtered = filter_scrollback_noise(&normalized);
        let derived =
            derive_worktree_purpose_from_scrollback(&filtered, "main", SummaryLanguage::En);

        let (prompt, mode, _) =
            build_scrollback_user_prompt(&normalized, &filtered, "main", &derived, "pane:1", None);

        assert_eq!(mode, ScrollbackSummaryMode::FullShort);
        assert!(prompt.contains("Latest instruction: improve summary quality"));
        assert!(!prompt.contains("Working (30s"));
    }

    #[test]
    fn test_derive_scrollback_purpose_from_filtered_text_ignores_terminal_chrome() {
        let normalized = normalize_scrollback_text(
            "Working (30s • esc to interrupt)\nStatus: updating\nFix broken summary generation",
        );
        let filtered = filter_scrollback_noise(&normalized);
        let derived = derive_worktree_purpose_from_scrollback(
            &filtered,
            "bugfix/issue-1600",
            SummaryLanguage::En,
        );

        assert!(!derived.text.contains("Working"));
        assert!(derived.text.contains("Fix broken summary generation"));
    }

    #[test]
    fn test_determine_scrollback_summary_mode_full_short_when_budget_allows() {
        let mode = determine_scrollback_summary_mode(
            "short\nscrollback",
            "short\nscrollback",
            200,
            "pane:abc",
            Some(&ScrollbackRollingContext {
                session_id: "pane:abc".to_string(),
                previous_markdown: Some("## Summary\nold".to_string()),
                previous_normalized_input: Some("short".to_string()),
                rolling_update_count: 1,
            }),
        );
        assert_eq!(mode, ScrollbackSummaryMode::FullShort);
    }

    #[test]
    fn test_determine_scrollback_summary_mode_incremental_for_append_only_growth() {
        let mode = determine_scrollback_summary_mode(
            "line1\nline2\nline3",
            "line1\nline2\nline3",
            8,
            "pane:abc",
            Some(&ScrollbackRollingContext {
                session_id: "pane:abc".to_string(),
                previous_markdown: Some("## Summary\nold".to_string()),
                previous_normalized_input: Some("line1\nline2".to_string()),
                rolling_update_count: 2,
            }),
        );
        assert_eq!(mode, ScrollbackSummaryMode::IncrementalLong);
    }

    #[test]
    fn test_determine_scrollback_summary_mode_rebuilds_on_session_change() {
        let mode = determine_scrollback_summary_mode(
            "line1\nline2\nline3",
            "line1\nline2\nline3",
            8,
            "pane:new",
            Some(&ScrollbackRollingContext {
                session_id: "pane:old".to_string(),
                previous_markdown: Some("## Summary\nold".to_string()),
                previous_normalized_input: Some("line1\nline2".to_string()),
                rolling_update_count: 0,
            }),
        );
        assert_eq!(mode, ScrollbackSummaryMode::FullRebuild);
    }

    // --- filter_scrollback_noise tests ---

    #[test]
    fn test_filter_scrollback_noise_removes_musing_lines() {
        let input = "user request here\nMusing...\nActual response";
        let result = filter_scrollback_noise(input);
        assert!(!result.contains("Musing"));
        assert!(result.contains("user request here"));
        assert!(result.contains("Actual response"));
    }

    #[test]
    fn test_filter_scrollback_noise_removes_thinking_lines() {
        let input = "start\nThinking...\nThinking…\nend";
        let result = filter_scrollback_noise(input);
        assert!(!result.contains("Thinking"));
        assert!(result.contains("start"));
        assert!(result.contains("end"));
    }

    #[test]
    fn test_filter_scrollback_noise_preserves_substantive_lines() {
        let input = "Implement the authentication module\nAdd error handling for edge cases";
        let result = filter_scrollback_noise(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_filter_scrollback_noise_normalizes_carriage_returns_and_drops_terminal_chrome() {
        let input =
            "start\rWorking (0s • esc to interrupt)\rActual response\r--- Status Bar ---\rend";
        let result = filter_scrollback_noise(input);
        assert!(result.contains("start"));
        assert!(result.contains("Actual response"));
        assert!(result.contains("end"));
        assert!(!result.contains("Working (0s"));
        assert!(!result.contains("Status Bar"));
    }

    #[test]
    fn test_filter_scrollback_noise_deduplicates_repeated_lines() {
        let mut lines = vec!["header"];
        lines.extend(std::iter::repeat_n("same line", 5));
        lines.push("footer");
        let input = lines.join("\n");
        let result = filter_scrollback_noise(&input);
        assert!(result.contains("same line"));
        assert!(result.contains("[repeated 5 times]"));
        assert!(result.contains("header"));
        assert!(result.contains("footer"));
        // Should only have one occurrence of the actual line
        assert_eq!(result.matches("same line").count(), 1);
    }

    #[test]
    fn test_filter_scrollback_noise_collapses_blank_runs() {
        let input = "line1\n\n\n\n\nline2";
        let result = filter_scrollback_noise(input);
        // Should have at most 1 blank line between line1 and line2
        let blank_count = result.lines().filter(|l| l.trim().is_empty()).count();
        assert!(blank_count <= 1);
        assert!(result.contains("line1"));
        assert!(result.contains("line2"));
    }

    #[test]
    fn test_filter_scrollback_noise_compresses_build_output() {
        let mut lines = vec!["start".to_string()];
        for i in 0..15 {
            lines.push(format!("Compiling crate_{i} v0.1.0"));
        }
        lines.push("end".to_string());
        let input = lines.join("\n");
        let result = filter_scrollback_noise(&input);
        assert!(result.contains("Compiling crate_0"));
        assert!(result.contains("Compiling crate_14"));
        assert!(result.contains("[...13 lines...]"));
        assert!(!result.contains("Compiling crate_7"));
    }

    #[test]
    fn test_filter_scrollback_noise_empty_input() {
        assert_eq!(filter_scrollback_noise(""), "");
    }

    #[test]
    fn test_filter_scrollback_noise_preserves_error_messages() {
        let input =
            "Musing...\nerror[E0308]: mismatched types\nThinking...\nwarning: unused variable";
        let result = filter_scrollback_noise(input);
        assert!(result.contains("error[E0308]: mismatched types"));
        assert!(!result.contains("Musing"));
        assert!(!result.contains("Thinking"));
    }
}
