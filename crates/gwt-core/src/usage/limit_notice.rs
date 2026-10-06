//! Recognize a provider's own "you are out of quota" notice on an agent's
//! final terminal screen (Issue #3616).
//!
//! Running out of quota is neither success nor failure of the work: the
//! account ran out, the branch did not. The providers state this outright and
//! also state when access returns, so this is the one stall whose cause and
//! duration can be read directly instead of inferred from elapsed time.
//!
//! Detection requires the CLI's refusal response or rate-limit-options menu.
//! A quotation anywhere in an agent transcript is not refusal evidence, even
//! when it persists at the bottom of the screen (Issue #5037).

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Offset, TimeZone, Utc};

use super::UsageProvider;

/// How many trailing non-empty lines may carry the notice. The Codex notice
/// soft-wraps over two lines and Claude's over one, so a small window is
/// enough while still rejecting a quoted mention scrolled above.
const TAIL_LINES: usize = 8;

/// A provider notice that the account's quota is exhausted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderLimitNotice {
    /// Provider named by the notice itself, when its wording identifies one.
    /// `None` when the text is provider-neutral — callers that know which agent
    /// the pane is running should prefer that, since the pane's own agent id is
    /// authoritative about whose account ran out.
    pub provider: Option<UsageProvider>,
    /// When the provider said access returns, `None` when it printed no usable
    /// instant. Callers must treat `None` as "unknown", never as "now".
    pub resets_at: Option<DateTime<Utc>>,
}

/// A native screen match and the structural evidence that authenticated it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderLimitScreenMatch {
    pub notice: ProviderLimitNotice,
    pub region: &'static str,
    pub pattern: &'static str,
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
    detect_provider_limit_screen_match(screen, now).map(|matched| matched.notice)
}

/// Preserve the whole rendered screen when calling this: leading indentation
/// and Markdown fences distinguish tool/code quotations from native responses.
pub fn detect_provider_limit_screen_match<Tz: TimeZone>(
    screen: &str,
    now: &DateTime<Tz>,
) -> Option<ProviderLimitScreenMatch> {
    let lines: Vec<&str> = screen.lines().collect();
    let start = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())?
        .saturating_sub(TAIL_LINES - 1);
    let mut fenced = false;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced || index < start || *line != line.trim_start() {
            continue;
        }
        let text = normalize_tail(&lines[index..].join("\n"));
        let haystack = text.to_ascii_lowercase();
        let (provider, region, pattern) = if trimmed == "What do you want to do?"
            && lines[index + 1..].iter().any(|line| {
                let option = line.trim();
                option.starts_with("❯ ") && option.contains("Wait for limit to reset")
            }) {
            (
                UsageProvider::ClaudeCode,
                "rate_limit_options",
                "claude_wait_for_reset",
            )
        } else {
            let heading = &text;
            let native = if heading.starts_with("■ You've hit your usage limit.") {
                Some((UsageProvider::Codex, "codex_usage_limit"))
            } else if heading.starts_with("Claude usage limit reached.") {
                Some((UsageProvider::ClaudeCode, "claude_usage_limit"))
            } else if heading.starts_with("You've hit your ")
                && lines[index + 1..]
                    .iter()
                    .any(|line| line.trim_start().starts_with("/usage-credits "))
            {
                Some((UsageProvider::ClaudeCode, "claude_usage_credits"))
            } else {
                None
            };
            let Some((provider, pattern)) = native else {
                continue;
            };
            if !states_a_limit_was_reached(&haystack)
                || !states_a_consequence(&haystack)
                || is_model_fallback(&haystack)
            {
                continue;
            }
            (provider, "provider_response", pattern)
        };
        return Some(ProviderLimitScreenMatch {
            notice: ProviderLimitNotice {
                provider: Some(provider),
                resets_at: parse_reset(&text, &haystack, now),
            },
            region,
            pattern,
        });
    }
    None
}

/// The account-limit half of a notice, across every wording both providers are
/// known to use: Codex's "hit your usage limit", Claude's "hit your weekly
/// limit", and the "`<window>` limit reached" family.
fn states_a_limit_was_reached(haystack: &str) -> bool {
    const HIT: [&str; 4] = [
        "hit your usage limit",
        "hit your weekly limit",
        "hit your 5-hour limit",
        "hit your session limit",
    ];
    const REACHED: [&str; 4] = [
        "usage limit reached",
        "weekly limit reached",
        "-hour limit reached",
        "rate limit reached",
    ];
    HIT.iter()
        .chain(REACHED.iter())
        .any(|phrase| haystack.contains(phrase))
}

/// A model fallback says the opposite of a block: the agent keeps working on a
/// smaller model. Treating "Opus limit reached, now using Sonnet" as a quota
/// hold would release a launch that is still making progress.
fn is_model_fallback(haystack: &str) -> bool {
    haystack.contains("now using")
        || haystack.contains("falling back to")
        || haystack.contains("switching to")
}

/// The second half a real block always carries: when access returns, or where
/// to buy more. Requiring it keeps a bare mention of the words from matching.
fn states_a_consequence(haystack: &str) -> bool {
    haystack.contains("reset")
        || haystack.contains("try again at")
        || haystack.contains("usage-credits")
        || haystack.contains("purchase more credits")
        || haystack.contains("upgrade to")
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
        if let Some(naive) = parse_reset_clause(rest, local_now) {
            return offset
                .from_local_datetime(&naive)
                .earliest()
                .map(|resolved| resolved.with_timezone(&Utc));
        }
    }
    None
}

/// The three shapes observed in the wild, most specific first:
/// `Aug 22nd, 2026 12:46 PM` (Codex), `Aug 20 at 6am` (Claude weekly), and a
/// bare `3pm` (Claude rolling window).
fn parse_reset_clause(rest: &str, local_now: NaiveDateTime) -> Option<NaiveDateTime> {
    parse_dated(rest, local_now).or_else(|| parse_time_of_day(rest, local_now))
}

/// A month/day reset, with the year either printed or inferred.
///
/// A year-less date resolves to its next occurrence: a December notice read in
/// January must not schedule a hold eleven months in the past, which
/// `retry_ready` would silently treat as "ready".
fn parse_dated(rest: &str, local_now: NaiveDateTime) -> Option<NaiveDateTime> {
    let cleaned = strip_ordinal_suffixes(rest);
    let mut tokens = cleaned.split_whitespace().peekable();
    let month = month_from_name(tokens.next()?)?;
    let day: u32 = tokens.next()?.trim_end_matches(',').parse().ok()?;
    let printed_year: Option<i32> = tokens
        .peek()
        .and_then(|token| token.trim_end_matches(',').parse().ok())
        .filter(|year: &i32| (1970..=9999).contains(year));
    if printed_year.is_some() {
        tokens.next();
    }
    // "Aug 20 at 6am" — the separator carries no information.
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
    for year in [local_now.year(), local_now.year() + 1] {
        let Some(candidate) = NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|date| date.and_hms_opt(hour, minute, 0))
        else {
            continue;
        };
        if candidate > local_now {
            return Some(candidate);
        }
    }
    None
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
/// `provider` is the label the caller resolved (from the pane's agent id, or
/// the notice's own hint). Passing `None` keeps the sentence honest rather than
/// naming a provider nobody established.
pub fn describe_provider_limit_notice(
    notice: &ProviderLimitNotice,
    provider: Option<&str>,
) -> String {
    let subject = match provider {
        Some(provider) => format!("{provider} usage limit reached"),
        None => "Provider usage limit reached".to_string(),
    };
    match notice.resets_at {
        Some(resets_at) => format!(
            "{subject} — resumes after {}",
            resets_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        ),
        None => format!("{subject} — reset time unknown"),
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

        assert_eq!(notice.provider, Some(UsageProvider::Codex));
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

        assert_eq!(notice.provider, Some(UsageProvider::ClaudeCode));
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

    /// The exact Claude wording observed on 2026-08-17 (Issue #3616 comment).
    ///
    /// Nothing in it matches the Codex phrasing this module was first written
    /// against: it says "hit your weekly limit", not "usage limit", and its
    /// reset is a month/day with no year and a bare hour.
    #[test]
    fn claude_weekly_limit_notice_is_detected_with_its_dated_reset() {
        let screen = "\
> read the file
You've hit your weekly limit · resets Aug 20 at 6am (Asia/Tokyo)
/usage-credits to finish what you're working on.";

        let notice = detect_provider_limit_notice(screen, &jst("2026-08-17T09:20:00+09:00"))
            .expect("claude weekly notice");

        assert_eq!(notice.provider, Some(UsageProvider::ClaudeCode));
        assert_eq!(
            notice
                .resets_at
                .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            Some("2026-08-19T21:00:00Z".to_string()),
            "6am on Aug 20 in the caller's +09:00 zone is 21:00Z the day before"
        );
    }

    /// A year-less date resolves to the next occurrence, so a December notice
    /// read in January does not schedule a hold eleven months in the past.
    #[test]
    fn a_year_less_date_rolls_into_the_next_year_when_it_has_already_passed() {
        let screen =
            "You've hit your weekly limit · resets Jan 2 at 6am\n/usage-credits to continue.";

        let notice = detect_provider_limit_notice(screen, &jst("2026-12-30T09:00:00+09:00"))
            .expect("claude weekly notice");

        assert_eq!(
            notice
                .resets_at
                .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            Some("2027-01-01T21:00:00Z".to_string()),
        );
    }

    /// A model fallback is not a block: the agent keeps working on a smaller
    /// model. Treating it as a quota hold would release a healthy launch.
    #[test]
    fn a_model_fallback_notice_is_not_a_block() {
        for screen in [
            "Claude Opus 4 limit reached, now using Sonnet 4",
            "Opus limit reached · now using Sonnet for the rest of this session",
        ] {
            assert_eq!(
                detect_provider_limit_notice(screen, &jst("2026-08-17T09:20:00+09:00")),
                None,
                "false positive for: {screen}"
            );
        }
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

    /// Issue #5037: terminal persistence and full telemetry do not turn quoted
    /// agent/tool output into the provider's own refusal.
    #[test]
    fn quoted_provider_refusals_are_not_notices() {
        for screen in [
            r#"{"body":"旧 Claude PM pane: You've hit your weekly limit · resets Oct 8, 6am"}"#,
            "• The Board quoted \"You've hit your weekly limit · resets Oct 8, 6am\".",
            "```text\n■ You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage\n  to purchase more credits or try again at Oct 8, 2026 6am.\n```",
            "⏺ Read(log.txt)\n  ⎿ You've hit your weekly limit · resets Oct 8, 6am\n    /usage-credits to finish what you're working on.\n❯",
        ] {
            assert_eq!(
                detect_provider_limit_notice(screen, &jst("2026-10-05T14:31:32+09:00")),
                None,
                "quoted output is not a native refusal: {screen}"
            );
        }
    }

    #[test]
    fn claude_native_rate_limit_options_identify_the_waiting_provider() {
        // Wording read from the installed Claude Code 2.1.291 menu.
        let screen =
            "What do you want to do?\n❯ 1. Wait for limit to reset\n  2. Upgrade to Max 20x";
        let notice = detect_provider_limit_notice(screen, &jst("2026-10-05T14:31:32+09:00"))
            .expect("the CLI's rate-limit-options menu is a structural refusal");
        assert_eq!(notice.provider, Some(UsageProvider::ClaudeCode));
        assert_eq!(notice.resets_at, None);
    }

    #[test]
    fn native_refusals_remain_recognizable_when_the_pane_soft_wraps() {
        for (screen, provider) in [
            ("■ You've hit your\nusage limit. Visit https://chatgpt.com/codex/settings/usage\nto purchase more credits or try again at Oct 8, 2026 6am.", UsageProvider::Codex),
            ("You've hit your weekly limit · resets Aug 20 at\n6am (Asia/Tokyo)\n/usage-credits to finish what you're working on.", UsageProvider::ClaudeCode),
        ] {
            let notice = detect_provider_limit_notice(screen, &jst("2026-10-05T14:31:32+09:00"))
                .expect("wrapping preserves the native refusal region");
            assert_eq!(notice.provider, Some(provider));
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
            describe_provider_limit_notice(
                &ProviderLimitNotice {
                    provider: Some(UsageProvider::Codex),
                    resets_at: Some(Utc.with_ymd_and_hms(2026, 8, 22, 3, 46, 0).unwrap()),
                },
                Some("Codex")
            ),
            "Codex usage limit reached — resumes after 2026-08-22T03:46:00Z"
        );
        assert_eq!(
            describe_provider_limit_notice(
                &ProviderLimitNotice {
                    provider: Some(UsageProvider::ClaudeCode),
                    resets_at: None,
                },
                Some("Claude Code")
            ),
            "Claude Code usage limit reached — reset time unknown"
        );
        assert_eq!(
            describe_provider_limit_notice(
                &ProviderLimitNotice {
                    provider: None,
                    resets_at: None,
                },
                None
            ),
            "Provider usage limit reached — reset time unknown",
            "an unattributed notice must not invent an account"
        );
    }
}
