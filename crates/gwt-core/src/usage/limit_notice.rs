//! Conservative recognition of a provider's own live usage-limit notice.
//!
//! This parser is intentionally screen-local and pure. Account usage is not
//! accepted here: callers may use it to corroborate a matching live-screen
//! candidate, but it cannot identify which window is blocked.

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Offset, TimeZone, Utc};

use super::UsageProvider;

const TAIL_LINES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderLimitNotice {
    pub provider: Option<UsageProvider>,
    pub resets_at: Option<DateTime<Utc>>,
}

pub fn detect_provider_limit_notice<Tz: TimeZone>(
    screen: &str,
    now: &DateTime<Tz>,
) -> Option<ProviderLimitNotice> {
    let lines = screen
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let tail = &lines[lines.len().saturating_sub(TAIL_LINES)..];
    let normalized = collapse_whitespace(&tail.join(" "));
    let haystack = normalized.to_ascii_lowercase();

    if !states_a_limit_was_reached(&haystack)
        || is_model_fallback(&haystack)
        || !states_a_blocking_consequence(&haystack)
        || limit_statement_is_only_quoted(tail)
    {
        return None;
    }
    Some(ProviderLimitNotice {
        provider: provider_hint(&haystack),
        resets_at: parse_reset(&normalized, &haystack, now),
    })
}

fn states_a_limit_was_reached(text: &str) -> bool {
    [
        "hit your usage limit",
        "hit your weekly limit",
        "hit your 5-hour limit",
        "hit your session limit",
        "usage limit reached",
        "weekly limit reached",
        "-hour limit reached",
        "rate limit reached",
    ]
    .iter()
    .any(|phrase| text.contains(phrase))
}

fn is_model_fallback(text: &str) -> bool {
    ["now using", "falling back to", "switching to"]
        .iter()
        .any(|phrase| text.contains(phrase))
}

fn states_a_blocking_consequence(text: &str) -> bool {
    [
        "reset",
        "try again at",
        "usage-credits",
        "purchase more credits",
        "upgrade to",
    ]
    .iter()
    .any(|phrase| text.contains(phrase))
}

fn limit_statement_is_only_quoted(lines: &[&str]) -> bool {
    let mut found = false;
    for line in lines {
        if !states_a_limit_was_reached(&line.to_ascii_lowercase()) {
            continue;
        }
        found = true;
        if !line.trim_start().starts_with('>') {
            return false;
        }
    }
    found
}

fn provider_hint(text: &str) -> Option<UsageProvider> {
    if text.contains("chatgpt.com") || text.contains("codex") {
        Some(UsageProvider::Codex)
    } else if text.contains("claude") || text.contains("usage-credits") {
        Some(UsageProvider::ClaudeCode)
    } else {
        None
    }
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_reset<Tz: TimeZone>(
    tail: &str,
    haystack: &str,
    now: &DateTime<Tz>,
) -> Option<DateTime<Utc>> {
    const ANCHORS: [&str; 5] = [
        "try again at ",
        "will reset at ",
        "resets at ",
        "reset at ",
        "resets ",
    ];
    let offset = now.offset().fix();
    let local_now = now.naive_local();
    ANCHORS.iter().find_map(|anchor| {
        let index = haystack.find(anchor)?;
        let naive = parse_reset_clause(&tail[index + anchor.len()..], local_now)?;
        offset
            .from_local_datetime(&naive)
            .earliest()
            .map(|resolved| resolved.with_timezone(&Utc))
    })
}

fn parse_reset_clause(rest: &str, local_now: NaiveDateTime) -> Option<NaiveDateTime> {
    parse_dated(rest, local_now).or_else(|| parse_time_of_day(rest, local_now))
}

fn parse_dated(rest: &str, local_now: NaiveDateTime) -> Option<NaiveDateTime> {
    let cleaned = strip_ordinal_suffixes(rest);
    let mut tokens = cleaned.split_whitespace().peekable();
    let month = month_from_name(tokens.next()?)?;
    let day: u32 = tokens.next()?.trim_end_matches(',').parse().ok()?;
    let printed_year = tokens
        .peek()
        .and_then(|token| token.trim_end_matches(',').parse::<i32>().ok())
        .filter(|year| (1970..=9999).contains(year));
    if printed_year.is_some() {
        tokens.next();
    }
    if tokens
        .peek()
        .is_some_and(|token| token.eq_ignore_ascii_case("at"))
    {
        tokens.next();
    }
    let clock = tokens.next()?;
    let meridiem = tokens.next().unwrap_or("");
    let (hour, minute) = parse_clock(clock, meridiem)?;
    if let Some(year) = printed_year {
        return NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, 0);
    }
    [local_now.year(), local_now.year() + 1]
        .into_iter()
        .filter_map(|year| {
            NaiveDate::from_ymd_opt(year, month, day)
                .and_then(|date| date.and_hms_opt(hour, minute, 0))
        })
        .find(|candidate| *candidate > local_now)
}

fn parse_time_of_day(rest: &str, local_now: NaiveDateTime) -> Option<NaiveDateTime> {
    let mut tokens = rest.trim_start().split_whitespace();
    let (hour, minute) = parse_clock(tokens.next()?, tokens.next().unwrap_or(""))?;
    let today = local_now.date().and_hms_opt(hour, minute, 0)?;
    if today > local_now {
        Some(today)
    } else {
        local_now.date().succ_opt()?.and_hms_opt(hour, minute, 0)
    }
}

fn parse_clock(clock: &str, detached_meridiem: &str) -> Option<(u32, u32)> {
    let lowered = clock.trim_end_matches(['.', ',']).to_ascii_lowercase();
    let (digits, attached_meridiem) = lowered
        .strip_suffix("am")
        .map(|head| (head, "am"))
        .or_else(|| lowered.strip_suffix("pm").map(|head| (head, "pm")))
        .unwrap_or((lowered.as_str(), ""));
    let meridiem = if attached_meridiem.is_empty() {
        match detached_meridiem
            .trim_end_matches(['.', ','])
            .to_ascii_lowercase()
            .as_str()
        {
            "am" => "am",
            "pm" => "pm",
            _ => "",
        }
    } else {
        attached_meridiem
    };
    let (hour_text, minute_text) = digits
        .trim_end_matches(':')
        .split_once(':')
        .unwrap_or((digits, "0"));
    let mut hour: u32 = hour_text.parse().ok()?;
    let minute: u32 = minute_text.parse().ok()?;
    if minute > 59 {
        return None;
    }
    match meridiem {
        "am" if hour == 12 => hour = 0,
        "am" | "" => {}
        "pm" if hour < 12 => hour += 12,
        "pm" => {}
        _ => return None,
    }
    (hour <= 23).then_some((hour, minute))
}

fn strip_ordinal_suffixes(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        let suffix = index + 1 < chars.len()
            && index > 0
            && chars[index - 1].is_ascii_digit()
            && matches!(
                [
                    chars[index].to_ascii_lowercase(),
                    chars[index + 1].to_ascii_lowercase()
                ],
                ['s', 't'] | ['n', 'd'] | ['r', 'd'] | ['t', 'h']
            )
            && chars
                .get(index + 2)
                .is_none_or(|next| !next.is_ascii_alphanumeric());
        if suffix {
            index += 2;
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    out
}

fn month_from_name(token: &str) -> Option<u32> {
    let token = token.trim_end_matches(['.', ',']).to_ascii_lowercase();
    [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|month| token.starts_with(month))
    .map(|index| index as u32 + 1)
}

#[cfg(test)]
mod tests {
    use super::detect_provider_limit_notice;
    use crate::usage::UsageProvider;
    use chrono::{DateTime, SecondsFormat};

    fn observed_at() -> DateTime<chrono::FixedOffset> {
        DateTime::parse_from_rfc3339("2026-08-29T09:00:00+09:00").expect("fixture instant")
    }

    #[test]
    fn t496_provider_limit_detector_accepts_exact_tail_not_ordinary_mentions() {
        let codex = "running tests\n■ You've hit your usage limit. Visit \
            https://chatgpt.com/codex/settings/usage\n\
            to purchase more credits or try again at Aug 30th, 2026 12:46 PM.";
        let claude = "working\nYou've hit your weekly limit · resets Aug 30 at 6am\n\
            /usage-credits to finish what you're working on.";

        assert_eq!(
            detect_provider_limit_notice(codex, &observed_at()).map(|notice| notice.provider),
            Some(Some(UsageProvider::Codex))
        );
        assert_eq!(
            detect_provider_limit_notice(claude, &observed_at()).map(|notice| notice.provider),
            Some(Some(UsageProvider::ClaudeCode))
        );
        assert_eq!(
            detect_provider_limit_notice(codex, &observed_at())
                .and_then(|notice| notice.resets_at)
                .map(|at| at.to_rfc3339_opts(SecondsFormat::Secs, true)),
            Some("2026-08-30T03:46:00Z".to_string())
        );
        for ordinary in [
            "We should document what happens when you hit your usage limit.",
            "The GitHub API rate limit was exceeded; retrying.",
            "Agent exited without completing the work",
        ] {
            assert_eq!(
                detect_provider_limit_notice(ordinary, &observed_at()),
                None,
                "{ordinary}"
            );
        }
    }

    #[test]
    fn t496_provider_limit_detector_rejects_model_fallback_and_scrolled_history() {
        for fallback in [
            "Claude Opus 4 limit reached, now using Sonnet 4",
            "Opus limit reached · falling back to Sonnet for this session",
        ] {
            assert_eq!(
                detect_provider_limit_notice(fallback, &observed_at()),
                None,
                "{fallback}"
            );
        }

        let history = format!(
            "■ You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage \
             to purchase more credits.\n{}",
            (0..9)
                .map(|index| format!("ordinary output line {index}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert_eq!(detect_provider_limit_notice(&history, &observed_at()), None);

        let quoted = "> ■ You've hit your usage limit. Visit \
            https://chatgpt.com/codex/settings/usage\n\
            > to purchase more credits or try again later.";
        assert_eq!(
            detect_provider_limit_notice(quoted, &observed_at()),
            None,
            "quoted provider text is transcript content, not provider UI"
        );
    }
}
