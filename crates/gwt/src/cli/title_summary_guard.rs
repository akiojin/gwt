//! Validation for Agent title-summary / `params.purpose` values.
//!
//! The Agent titlebar renders `projection.agents[<i>].title_summary`
//! (SPEC-2359 US-58 / FR-344), and SPEC #3075 defines the purpose as the
//! Work's stable identity, separate from mutable status. Every CLI write
//! path for that field (workspace.update / join / create / ensure JSON
//! envelopes and the legacy `--title-summary` flags) funnels through
//! [`validate_title_summary_work_name`], making this module the single
//! choke point that keeps status snapshots (SPEC-2359) and transient
//! activity phases (Issue #3184) out of the title.

use super::CliParseError;

const TITLE_SUMMARY_WORK_NAME_REASON: &str =
    "purpose must be a work name, not a status/result; keep completion, progress, or blocker state in params.status, params.current_focus, params.summary, or Board body";

const TITLE_SUMMARY_TRANSIENT_ACTIVITY_REASON: &str =
    "purpose must stay the stable work purpose, not a transient activity phase (browser check, verification, merging, server startup, ...); keep the existing purpose and put the activity in params.current_focus or params.status_text";

pub(crate) fn validate_title_summary_work_name(
    flag: &'static str,
    value: &str,
) -> Result<(), CliParseError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || title_summary_has_status_marker(trimmed) {
        return Err(CliParseError::InvalidValue {
            flag,
            reason: TITLE_SUMMARY_WORK_NAME_REASON,
        });
    }
    if title_summary_is_transient_activity(trimmed) {
        return Err(CliParseError::InvalidValue {
            flag,
            reason: TITLE_SUMMARY_TRANSIENT_ACTIVITY_REASON,
        });
    }
    Ok(())
}

/// Issue #3184: transient helper-workflow phases (browser-check, verification,
/// merging, server startup) must never become the Agent title.
///
/// Matching heuristic, tuned against the review findings on both bypass and
/// over-blocking:
/// - values led by a repair/build verb ("Fix browser check") are work names
///   and always pass;
/// - values led by an activity gerund ("merging develop") reject;
/// - an English label rejects as the whole value, as the phrase head suffix
///   ("Headless browser check"), or when followed only by a prepositional /
///   parenthetical modifier ("browser check for issue 3184",
///   "Browser check (fresh instance)") — while attributive compounds that
///   grow a new head ("browser check timeout bug") stay valid;
/// - Japanese labels reject as the whole value or as the head-final suffix
///   ("ヘッドレスブラウザチェック", "ブラウザで動作確認") on a separator-free
///   normal form ("ブラウザ・チェック", "ブラウザ チェック").
fn title_summary_is_transient_activity(value: &str) -> bool {
    let folded = fold_fullwidth_ascii(value);
    let normalized = trim_title_edge_symbols(&folded);
    let lower = normalized
        .to_ascii_lowercase()
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    const WORK_VERBS: [&str; 17] = [
        "add",
        "block",
        "debug",
        "design",
        "fix",
        "guard",
        "harden",
        "implement",
        "improve",
        "investigate",
        "patch",
        "prevent",
        "redesign",
        "refactor",
        "remove",
        "resolve",
        "stop",
    ];
    const ACTIVITY_GERUNDS: [&str; 7] = [
        "checking",
        "launching",
        "merging",
        "rebasing",
        "restarting",
        "starting",
        "verifying",
    ];
    const MODIFIER_INTRODUCERS: [&str; 17] = [
        "across", "after", "against", "at", "before", "during", "for", "from", "in", "of", "on",
        "over", "per", "to", "toward", "via", "with",
    ];

    if let Some(first) = lower.split(' ').next() {
        if WORK_VERBS.contains(&first) {
            return false;
        }
        if ACTIVITY_GERUNDS.contains(&first) {
            return true;
        }
    }

    let english_activity_labels = [
        "browser check",
        "browser checks",
        "browser checking",
        "browser verification",
        "visual check",
        "visual verification",
        "verification",
        "verify",
        "merge",
        "server startup",
        "server start",
        "server launch",
    ];
    if english_activity_labels.iter().any(|label| {
        if lower == *label || (label.contains(' ') && lower.ends_with(&format!(" {label}"))) {
            return true;
        }
        lower
            .strip_prefix(&format!("{label} "))
            .and_then(|rest| rest.split_whitespace().next())
            .is_some_and(|next| next.starts_with('(') || MODIFIER_INTRODUCERS.contains(&next))
    }) {
        return true;
    }

    let compact: String = normalized
        .chars()
        .filter(|ch| !ch.is_whitespace() && !matches!(ch, '・' | '･'))
        .collect();
    let japanese_activity_labels = [
        "ブラウザチェック",
        "ブラウザ確認",
        "ブラウザ検証",
        "視覚確認",
        "目視確認",
        "動作確認",
        "検証",
        "マージ",
        "サーバー起動",
        "サーバ起動",
        "ビルド確認",
    ];
    japanese_activity_labels
        .iter()
        .any(|label| compact == *label || compact.ends_with(label))
}

fn title_summary_has_status_marker(value: &str) -> bool {
    let normalized = trim_title_edge_symbols(value);
    let lower = normalized.to_ascii_lowercase();
    let english_status_suffixes = [
        "blocked",
        "complete",
        "completed",
        "done",
        "finished",
        "fixed",
        "implemented",
        "in progress",
        "merged",
        "verified",
        "wip",
    ];
    if english_status_suffixes
        .iter()
        .any(|suffix| lower == *suffix || lower.ends_with(&format!(" {suffix}")))
    {
        return true;
    }

    let japanese_status_suffixes = [
        "完了",
        "完了済み",
        "完了しました",
        "対応済み",
        "実装済み",
        "修正済み",
        "検証済み",
        "マージ済み",
        "作業中",
        "進行中",
        "対応中",
        "実装中",
        "修正中",
        "検証中",
        "レビュー中",
        "ブロック中",
        "チェック中",
        "確認中",
        "起動中",
        "マージ中",
        "ビルド中",
        "テスト中",
    ];
    japanese_status_suffixes
        .iter()
        .any(|suffix| normalized == *suffix || normalized.ends_with(suffix))
}

/// Strip decoration (quotes, brackets, emoji, punctuation) from both edges so
/// wrapped activity labels like `"browser check"` / `(browser check)` /
/// `browser check 🔍` still hit the deny lists.
fn trim_title_edge_symbols(value: &str) -> &str {
    value.trim_matches(|ch: char| !ch.is_alphanumeric())
}

/// Fold full-width ASCII (U+FF01..=U+FF5E) and the ideographic space to their
/// ASCII forms so IME-shaped labels like `ｂｒｏｗｓｅｒ ｃｈｅｃｋ` normalize
/// into the English deny list.
fn fold_fullwidth_ascii(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '\u{3000}' => ' ',
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch),
            _ => ch,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_activity_detection_normalizes_case_separators_and_punctuation() {
        for value in [
            "BROWSER CHECK",
            "browser_check",
            "Browser-Check",
            "Headless browser check.",
            "browser check！",
            "\"browser check\"",
            "(browser check)",
            "browser check 🔍",
            "ｂｒｏｗｓｅｒ ｃｈｅｃｋ",
        ] {
            assert!(title_summary_is_transient_activity(value), "{value}");
        }
    }

    #[test]
    fn transient_activity_detection_rejects_trailing_modifier_and_gerund_phrases() {
        for value in [
            "browser check for issue 3184",
            "Browser check (fresh instance)",
            "browser check of gwt UI",
            "browser checking",
            "verification of the fix",
            "merging develop",
            "verifying tests",
        ] {
            assert!(title_summary_is_transient_activity(value), "{value}");
        }
    }

    #[test]
    fn transient_activity_detection_rejects_japanese_activity_phrases() {
        for value in [
            "ヘッドレスブラウザチェック",
            "ブラウザで動作確認",
            "ブラウザ・チェック",
            "ブラウザ チェック",
        ] {
            assert!(title_summary_is_transient_activity(value), "{value}");
        }
    }

    #[test]
    fn transient_activity_detection_keeps_real_work_names_valid() {
        for value in [
            "browser-check purpose overwrite guard",
            "browser check timeout bug",
            "Fix browser check",
            "Fix server startup",
            "release verification pipeline",
            "Shell",
            "エージェントタイトル目的化",
            "ブラウザチェック改善",
            "Issue #3184 title guard",
        ] {
            assert!(!title_summary_is_transient_activity(value), "{value}");
        }
    }

    /// Issue #3184: progressive-form activity phases (「チェック中」等) are
    /// status snapshots and must reject through the status-marker path.
    #[test]
    fn validate_rejects_japanese_progressive_activity_forms() {
        for value in [
            "ブラウザチェック中",
            "ブラウザ確認中",
            "サーバー起動中",
            "マージ中",
            "ビルド中",
            "テスト中",
        ] {
            assert!(
                validate_title_summary_work_name("params.purpose", value).is_err(),
                "{value}"
            );
        }
    }
}
