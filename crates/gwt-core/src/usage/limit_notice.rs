//! Recognize a provider's own "you are out of quota" notice on an agent's
//! final terminal screen (Issue #3616).
//!
//! Running out of quota is neither success nor failure of the work: the
//! account ran out, the branch did not. The providers state this outright and
//! also state when access returns, so this is the one stall whose cause and
//! duration can be read directly instead of inferred from elapsed time.
//!
//! Detection is deliberately conservative. An agent transcript can legitimately
//! *contain* the sentence (an agent working on this very Issue prints it), so a
//! match requires the provider's distinctive phrase plus a corroborating clause,
//! and only within the tail of the screen — the notice is the last thing a
//! quota-exhausted CLI writes before exiting.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Offset, TimeZone, Utc};

use super::UsageProvider;

/// How many trailing non-empty lines may carry the notice. The Codex notice
/// soft-wraps over two lines and Claude's over one, so a small window is
/// enough while still rejecting a quoted mention scrolled above.
const TAIL_LINES: usize = 8;

/// A provider notice that the account's quota is exhausted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderLimitNotice {
    pub provider: UsageProvider,
    /// When the provider said access returns, `None` when it printed no usable
    /// instant. Callers must treat `None` as "unknown", never as "now".
    pub resets_at: Option<DateTime<Utc>>,
}

/// Classify the tail of `screen` as a provider usage-limit notice.
///
/// `now` supplies both the reference instant and the offset that bare
/// wall-clock times ("resets 3pm") are interpreted in — providers print local
/// times without a zone, so the caller's zone is the only available anchor.
pub fn detect_provider_limit_notice<Tz: TimeZone>(
    screen: &str,
    now: &DateTime<Tz>,
) -> Option<ProviderLimitNotice> {
    let tail = normalize_tail(screen);
    let haystack = tail.to_ascii_lowercase();

    if is_codex_notice(&haystack) {
        return Some(ProviderLimitNotice {
            provider: UsageProvider::Codex,
            resets_at: parse_reset(&tail, &haystack, now),
        });
    }
    if is_claude_notice(&haystack) {
        return Some(ProviderLimitNotice {
            provider: UsageProvider::ClaudeCode,
            resets_at: parse_reset(&tail, &haystack, now),
        });
    }
    None
}

/// Join the trailing non-empty lines into one whitespace-collapsed string so a
/// soft-wrapped notice reads the same at any terminal width.
fn normalize_tail(screen: &str) -> String {
    let lines: Vec<&str> = screen
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    let joined = lines[start..].join(" ");
    let mut out = String::with_capacity(joined.len());
    let mut pending_space = false;
    for character in joined.chars() {
        let character = match character {
            '\u{2019}' => '\'',
            other => other,
        };
        if character.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(character);
    }
    out
}

fn is_codex_notice(haystack: &str) -> bool {
    let phrase = haystack.contains("hit your usage limit");
    let corroborated = haystack.contains("chatgpt.com/codex/settings/usage")
        || haystack.contains("purchase more credits")
        || haystack.contains("try again at");
    phrase && corroborated
}

fn is_claude_notice(haystack: &str) -> bool {
    let phrase = haystack.contains("usage limit reached")
        || haystack.contains("-hour limit reached")
        || haystack.contains("weekly limit reached");
    phrase && (haystack.contains("reset") || haystack.contains("upgrade to"))
}

/// Pull the first reset instant the notice states, in the caller's zone.
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
    for anchor in ANCHORS {
        let Some(index) = haystack.find(anchor) else {
            continue;
        };
        let rest = &tail[index + anchor.len()..];
        if let Some(naive) = parse_absolute(rest).or_else(|| parse_time_of_day(rest, local_now)) {
            return offset
                .from_local_datetime(&naive)
                .earliest()
                .map(|resolved| resolved.with_timezone(&Utc));
        }
    }
    None
}

/// `Aug 22nd, 2026 12:46 PM` — the full instant Codex prints.
fn parse_absolute(rest: &str) -> Option<NaiveDateTime> {
    let cleaned = strip_ordinal_suffixes(rest);
    let mut tokens = cleaned.split_whitespace();
    let month = month_from_name(tokens.next()?)?;
    let day: u32 = tokens.next()?.trim_end_matches(',').parse().ok()?;
    let year: i32 = tokens.next()?.trim_end_matches(',').parse().ok()?;
    let clock = tokens.next()?;
    let meridiem = tokens.next().unwrap_or("");
    let (hour, minute) = parse_clock(clock, meridiem)?;
    NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, 0)
}

/// `3pm`, `3:30 pm`, `15:00` — Claude prints only a wall-clock time, so the
/// instant is its next occurrence at or after `local_now`.
fn parse_time_of_day(rest: &str, local_now: NaiveDateTime) -> Option<NaiveDateTime> {
    let cleaned = rest.trim_start();
    let mut tokens = cleaned.split_whitespace();
    let clock = tokens.next()?;
    let meridiem = tokens.next().unwrap_or("");
    let (hour, minute) = parse_clock(clock, meridiem)?;
    let today = local_now.date().and_hms_opt(hour, minute, 0)?;
    if today > local_now {
        return Some(today);
    }
    local_now.date().succ_opt()?.and_hms_opt(hour, minute, 0)
}

/// Split `3pm` / `3:30` / `15:00` / `12:46` (+ a detached `PM`) into 24h parts.
fn parse_clock(clock: &str, detached_meridiem: &str) -> Option<(u32, u32)> {
    let lowered = clock.trim_end_matches(['.', ',']).to_ascii_lowercase();
    let (digits, attached_meridiem) = match lowered.strip_suffix("am") {
        Some(head) => (head, "am"),
        None => match lowered.strip_suffix("pm") {
            Some(head) => (head, "pm"),
            None => (lowered.as_str(), ""),
        },
    };
    let meridiem = if attached_meridiem.is_empty() {
        let detached = detached_meridiem
            .trim_end_matches(['.', ','])
            .to_ascii_lowercase();
        match detached.as_str() {
            "am" | "pm" => detached,
            _ => String::new(),
        }
    } else {
        attached_meridiem.to_string()
    };
    let digits = digits.trim_end_matches(':');
    let (hour_text, minute_text) = match digits.split_once(':') {
        Some((hour, minute)) => (hour, minute),
        None => (digits, "0"),
    };
    let mut hour: u32 = hour_text.parse().ok()?;
    let minute: u32 = minute_text.parse().ok()?;
    if minute > 59 {
        return None;
    }
    match meridiem.as_str() {
        "am" if hour == 12 => hour = 0,
        "am" | "" => {}
        "pm" if hour < 12 => hour += 12,
        "pm" => {}
        _ => return None,
    }
    (hour <= 23).then_some((hour, minute))
}

fn strip_ordinal_suffixes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let characters: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < characters.len() {
        let current = characters[index];
        let suffix_starts_here = index + 1 < characters.len()
            && index > 0
            && characters[index - 1].is_ascii_digit()
            && matches!(
                [
                    current.to_ascii_lowercase(),
                    characters[index + 1].to_ascii_lowercase()
                ],
                ['s', 't'] | ['n', 'd'] | ['r', 'd'] | ['t', 'h']
            )
            && characters
                .get(index + 2)
                .is_none_or(|next| !next.is_ascii_alphanumeric());
        if suffix_starts_here {
            index += 2;
            continue;
        }
        out.push(current);
        index += 1;
    }
    out
}

fn month_from_name(token: &str) -> Option<u32> {
    let lowered = token.trim_end_matches(['.', ',']).to_ascii_lowercase();
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    MONTHS
        .iter()
        .position(|month| lowered.starts_with(month))
        .map(|index| index as u32 + 1)
}

/// The provider notice rendered for a window detail / Monitor diagnostic.
pub fn describe_provider_limit_notice(notice: &ProviderLimitNotice) -> String {
    let provider = match notice.provider {
        UsageProvider::Codex => "Codex",
        UsageProvider::ClaudeCode => "Claude Code",
    };
    match notice.resets_at {
        Some(resets_at) => format!(
            "{provider} usage limit reached — resumes after {}",
            resets_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        ),
        None => format!("{provider} usage limit reached — reset time unknown"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn jst(text: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(text).expect("fixture instant")
    }

    #[test]
    fn codex_notice_is_detected_with_its_absolute_reset() {
        let screen = "\
Running cargo test -p gwt
■ You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage
  to purchase more credits or try again at Aug 22nd, 2026 12:46 PM.
";
        let notice = detect_provider_limit_notice(screen, &jst("2026-08-16T11:26:00+09:00"))
            .expect("codex notice");

        assert_eq!(notice.provider, UsageProvider::Codex);
        assert_eq!(
            notice
                .resets_at
                .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            Some("2026-08-22T03:46:00Z".to_string()),
            "12:46 PM in the caller's +09:00 zone is 03:46Z"
        );
    }

    #[test]
    fn claude_notice_resolves_a_bare_time_to_its_next_occurrence() {
        let screen = "Claude usage limit reached. Your limit will reset at 3pm (Asia/Tokyo).";

        let notice = detect_provider_limit_notice(screen, &jst("2026-08-16T11:26:00+09:00"))
            .expect("claude notice");

        assert_eq!(notice.provider, UsageProvider::ClaudeCode);
        assert_eq!(
            notice
                .resets_at
                .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            Some("2026-08-16T06:00:00Z".to_string()),
        );
    }

    #[test]
    fn a_bare_time_already_past_today_rolls_to_tomorrow() {
        let screen = "Claude usage limit reached. Your limit will reset at 3pm.";

        let notice = detect_provider_limit_notice(screen, &jst("2026-08-16T16:00:00+09:00"))
            .expect("claude notice");

        assert_eq!(
            notice
                .resets_at
                .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            Some("2026-08-17T06:00:00Z".to_string()),
        );
    }

    #[test]
    fn the_notice_must_be_at_the_end_of_the_screen() {
        let quoted = format!(
            "{}\n{}",
            "■ You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage \
             to purchase more credits or try again at Aug 22nd, 2026 12:46 PM.",
            (0..12)
                .map(|index| format!("line {index} of ordinary work output"))
                .collect::<Vec<_>>()
                .join("\n")
        );

        assert_eq!(
            detect_provider_limit_notice(&quoted, &jst("2026-08-16T11:26:00+09:00")),
            None,
            "a mention scrolled above the tail is transcript content, not a live block"
        );
    }

    #[test]
    fn an_unrelated_mention_of_limits_is_not_a_notice() {
        for screen in [
            "The GitHub API rate limit was exceeded; retrying.",
            "We should document what happens when you hit your usage limit.",
            "Agent exited without completing the work",
        ] {
            assert_eq!(
                detect_provider_limit_notice(screen, &jst("2026-08-16T11:26:00+09:00")),
                None,
                "false positive for: {screen}"
            );
        }
    }

    #[test]
    fn an_unparseable_reset_still_reports_the_block() {
        let screen = "■ You've hit your usage limit. Visit \
                      https://chatgpt.com/codex/settings/usage to purchase more credits.";

        let notice = detect_provider_limit_notice(screen, &jst("2026-08-16T11:26:00+09:00"))
            .expect("codex notice without a reset instant");

        assert_eq!(notice.resets_at, None);
    }

    #[test]
    fn description_names_the_provider_and_the_reset() {
        assert_eq!(
            describe_provider_limit_notice(&ProviderLimitNotice {
                provider: UsageProvider::Codex,
                resets_at: Some(Utc.with_ymd_and_hms(2026, 8, 22, 3, 46, 0).unwrap()),
            }),
            "Codex usage limit reached — resumes after 2026-08-22T03:46:00Z"
        );
        assert_eq!(
            describe_provider_limit_notice(&ProviderLimitNotice {
                provider: UsageProvider::ClaudeCode,
                resets_at: None,
            }),
            "Claude Code usage limit reached — reset time unknown"
        );
    }
}
