//! Session summary generation and cache.

use super::client::{AIClient, AIError, ChatMessage};
use super::session_parser::{MessageRole, ParsedSession, SessionMessage};
use std::collections::HashMap;
use std::time::SystemTime;

pub const SESSION_SYSTEM_PROMPT_BASE: &str = "You are a helpful assistant summarizing a coding agent session so the user can remember the original request and latest instruction.\nReturn Markdown only with the following format and headings, in this exact order:\n\n## <Purpose heading in the user's language>\n<1 sentence: the worktree/branch objective (why) + key constraints + explicit exclusions>\n\n## <Summary heading in the user's language>\n<1-2 sentences: current status (use a clear status word) + the latest user instruction; mention if blocked>\n\n## <Highlights heading in the user's language>\n- <Original request: ...>\n- <Latest instruction: ...>\n- <Decisions/constraints: ...>\n- <Exclusions/not doing: ...>\n- <Status: ...>\n- <Progress: ...>\n- <Recent meaningful actions (last 1-3): ...>\n- <Needs user input (as a direct question): ...>\n- <Key words (3 items): ...>\n\nAdd more bullets if there are additional important items, but keep the list concise.\nIf there was no progress, say so and why.\nIf waiting for user input, state the exact question needed.\nDo not guess; if something is unknown, say so explicitly in the user's language.\nUse short labels followed by \":\" for each bullet and translate the labels to the user's language.\nDetect the response language from the session content and respond in that language.\nIf the session contains multiple languages, use the language used by the user messages.\nAll headings and all content must be in the user's language.\nDo not output JSON, code fences, or any extra text.\n\nPurpose writing rule:\n- The Purpose section must describe the intended outcome of the worktree/branch.\n- Treat PR/MR creation, PR template filling, merge/push, and status checks as means (how), not purpose (why).\n\nIgnore operational workflow chatter from this session except for content that changes direction:\n- PR/MR creation, branch operations, test/build/CI activity, and short status updates.\n- Keep summaries focused on user intent, decisions, constraints, outcomes, blockers, or pending actions.\n- Ignore one-line acknowledgements unless they contain a blocking issue or design decision.\nPrioritize substantive conversation over command history.\n\nWhen the session language is Japanese, headings must be exactly in this order:\n- ## 目的\n- ## 要約\n- ## ハイライト\nWhen the session language is English, headings must be exactly in this order:\n- ## Purpose\n- ## Summary\n- ## Highlights.";

const SESSION_SYSTEM_PROMPT_EN: &str = "You are a helpful assistant summarizing a coding agent session so the user can remember the original request and latest instruction.\nRespond in English.\nReturn Markdown only with the following format and headings, in this exact order:\n\n## Purpose\n<1 sentence: the worktree/branch objective (why) + key constraints + explicit exclusions>\n\n## Summary\n<1-2 sentences: current status (use a clear status word) + the latest user instruction; mention if blocked>\n\n## Highlights\n- <Original request: ...>\n- <Latest instruction: ...>\n- <Decisions/constraints: ...>\n- <Exclusions/not doing: ...>\n- <Status: ...>\n- <Progress: ...>\n- <Recent meaningful actions (last 1-3): ...>\n- <Needs user input (as a direct question): ...>\n- <Key words (3 items): ...>\n\nAdd more bullets if there are additional important items, but keep the list concise.\nIf there was no progress, say so and why.\nIf waiting for user input, state the exact question needed.\nDo not guess; if something is unknown, say so explicitly in English.\nUse short labels followed by \":\" for each bullet.\nAll headings and all content must be in English.\nDo not output JSON, code fences, or any extra text.\n\nPurpose writing rule:\n- The Purpose section must describe the intended outcome of the worktree/branch.\n- Treat PR/MR creation, PR template filling, merge/push, and status checks as means (how), not purpose (why).\n\nIgnore operational workflow chatter from this session except for content that changes direction:\n- PR/MR creation, branch operations, test/build/CI activity, and short status updates.\n- Keep summaries focused on user intent, decisions, constraints, outcomes, blockers, or pending actions.\n- Ignore one-line acknowledgements unless they contain a blocking issue or design decision.\nPrioritize substantive conversation over command history.";

const SESSION_SYSTEM_PROMPT_JA: &str = "You are a helpful assistant summarizing a coding agent session so the user can remember the original request and latest instruction.\nRespond in Japanese.\nReturn Markdown only with the following format and headings, in this exact order:\n\n## 目的\n<1文: Worktree/ブランチで達成する成果（Why） + 重要な制約 + 明示された除外>\n\n## 要約\n<1-2文: 現在ステータス（明確な状態語を使う）+ 最新のユーザー指示。ブロックされているなら明記>\n\n## ハイライト\n- <元の依頼: ...>\n- <最新指示: ...>\n- <決定事項/制約: ...>\n- <除外/やらないこと: ...>\n- <ステータス: ...>\n- <進捗: ...>\n- <直近の意味のある行動（1-3件）: ...>\n- <ユーザーに必要な入力（質問として）: ...>\n- <キーワード（3つ）: ...>\n\n重要な項目があれば箇条書きを追加してよいが、簡潔にすること。\n進捗がない場合は、その旨と理由を書くこと。\nユーザー入力待ちの場合は、必要な質問をそのまま書くこと。\n推測しない。不明な点は不明と明記すること。\n各箇条書きは短いラベル + \":\" で始め、ラベルも日本語にすること。\n見出しと本文はすべて日本語にすること。\nJSONやコードフェンス、余計なテキストを出力しないこと。\n\n目的の記述ルール:\n- 目的にはWorktree/ブランチの達成成果（Why）を書く。\n- PR/MR作成、PR本文テンプレート記入、merge/push、ステータス確認は手段（How）として扱い、目的にしない。\n\n以下の運用的なやり取りは、方向性が変わる内容を除き無視すること:\n- PR/MR作成、ブランチ操作、テスト/ビルド/CI、短いステータス更新\n- 要約はユーザー意図、決定事項、制約、結果、ブロッカー、未完了作業に集中する\n- ブロッキングや設計判断を含まない1行の相槌は無視する\n会話の中身を優先し、コマンド履歴に引っ張られないこと。";

const MAX_MESSAGE_CHARS: usize = 220;
const MAX_PROMPT_CHARS: usize = 8000;
const MAX_PURPOSE_TEXT_CHARS: usize = 180;

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

    pub fn set(
        &mut self,
        branch: String,
        tool_id: String,
        session_id: String,
        language: String,
        summary: SessionSummary,
        mtime: SystemTime,
    ) {
        self.cache.insert(branch.clone(), summary);
        self.last_modified.insert(branch.clone(), mtime);
        self.session_ids.insert(branch.clone(), session_id);
        self.tool_ids.insert(branch.clone(), tool_id);
        self.languages.insert(branch, language);
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

/// Summarizes a terminal scrollback as plain text, bypassing session parsers.
///
/// The scrollback text should already have ANSI sequences stripped.
/// Large texts are sampled (first 40% + last 60%) to fit within MAX_PROMPT_CHARS.
pub fn summarize_scrollback(
    client: &AIClient,
    scrollback_text: &str,
    branch_name: &str,
    language: &str,
) -> Result<SessionSummary, AIError> {
    let fallback_language = fallback_purpose_language_for_text(language, scrollback_text);
    let derived_purpose =
        derive_worktree_purpose_from_scrollback(scrollback_text, branch_name, fallback_language);
    let sampled = sample_scrollback_text(scrollback_text);
    let mut user_prompt = format!(
        "Branch: {branch_name}\nDerived worktree purpose ({}): {}\nPurpose guidance: \
In the Purpose section, describe the worktree objective (why). \
Treat PR/MR creation, template filling, merge/push, and status checks as means (how), not purpose.\n\nTerminal session output:\n{sampled}",
        derived_purpose.confidence_label(),
        derived_purpose.text
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

    Ok(SessionSummary {
        task_overview: task_overview.or(fields.task_overview),
        short_summary: fields.short_summary,
        bullet_points: fields.bullet_points,
        markdown: Some(markdown),
        metrics,
        last_updated: Some(SystemTime::now()),
    })
}

/// Samples scrollback text to fit within MAX_PROMPT_CHARS.
///
/// If the text fits, returns it as-is. Otherwise, takes the first 40%
/// and last 60% of the allowed characters, with a separator in between.
fn sample_scrollback_text(text: &str) -> String {
    let char_count = text.chars().count();
    if char_count <= MAX_PROMPT_CHARS {
        return text.to_string();
    }
    let head_chars = MAX_PROMPT_CHARS * 2 / 5; // 40%
    let separator = "\n...[truncated]...\n";
    let tail_chars = MAX_PROMPT_CHARS - head_chars - separator.len();

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
                    "違います。目的はfeature/agent-modeで作った成果をdevelopに取り込むことです。"
                        .to_string(),
                timestamp: None,
            },
        ];

        let derived = derive_worktree_purpose_from_messages(
            &messages,
            "feature/agent-mode",
            SummaryLanguage::Ja,
        );
        assert_eq!(derived.source, PurposeSource::Explicit);
        assert!(derived.text.contains("feature/agent-mode"));
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
            "feature/agent-mode",
            SummaryLanguage::En,
        );
        assert_eq!(derived.source, PurposeSource::Inferred);
        assert!(derived.text.contains("feature/agent-mode"));
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
            "feature/agent-mode",
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
            "feature/agent-mode の成果を develop に取り込むこと".to_string(),
        );
        let enforced = enforce_worktree_purpose_in_markdown(
            markdown,
            SummaryLanguage::Ja,
            "feature/agent-mode",
            &derived,
        );

        assert!(enforced.contains("feature/agent-mode の成果を develop に取り込むこと"));
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
        let markdown = "## 目的\nfeature/agent-mode ブランチから develop への PR を作成し、PR 本文テンプレートを埋めること\n\n## 要約\n完了\n\n## ハイライト\n- Original request: feature/agent-mode ブランチから develop への PR を作成\n- Latest instruction: PR 本文テンプレートを埋める";
        let derived = DerivedPurpose::explicit(
            "feature/agent-mode で達成した成果を develop に取り込むこと".to_string(),
        );

        let enforced = enforce_worktree_purpose_in_markdown(
            markdown,
            SummaryLanguage::Ja,
            "feature/agent-mode",
            &derived,
        );

        assert!(enforced.contains("feature/agent-mode で達成した成果を develop に取り込むこと"));
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

    // --- sample_scrollback_text tests ---

    #[test]
    fn test_sample_scrollback_within_limit() {
        let text = "a".repeat(100);
        let result = sample_scrollback_text(&text);
        assert_eq!(result, text);
    }

    #[test]
    fn test_sample_scrollback_large_text() {
        let text = "x".repeat(MAX_PROMPT_CHARS * 2);
        let result = sample_scrollback_text(&text);
        assert!(result.chars().count() <= MAX_PROMPT_CHARS);
        assert!(result.contains("...[truncated]..."));
        assert!(result.starts_with('x'));
        assert!(result.ends_with('x'));
    }

    #[test]
    fn test_sample_scrollback_exact_limit() {
        let text = "y".repeat(MAX_PROMPT_CHARS);
        let result = sample_scrollback_text(&text);
        assert_eq!(result, text);
    }
}
