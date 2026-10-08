//! Recognize the provider API error a CLI prints when it gives up on a turn
//! (Issue #4584).
//!
//! Quota exhaustion already surfaces through [`super::limit_notice`], and a
//! crashed process surfaces through its exit. A transient provider error is
//! neither: the CLI writes one line, abandons the turn, and returns to its
//! prompt with the process and the conversation both intact. Nothing else in
//! the runtime changes, so without reading this line the pane is
//! indistinguishable from one still working.
//!
//! Detection is anchored to a line that *starts* with the renderer's own
//! prefix, within the tail of the screen. A transcript can legitimately
//! discuss the sentence — the panes that produced the report for this Issue
//! were later told about it in prose, and that prose must not read as a
//! second outage.

/// A provider API error that ended a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderApiError {
    /// The HTTP status the provider reported, when the line carried one. Many
    /// real turn-enders are transport failures with no status at all, so
    /// `None` is ordinary and never means "healthy".
    pub http_status: Option<u16>,
    /// The error text, collapsed to one line and trimmed to a readable length.
    pub summary: String,
    /// Whether the cause is one a retry can clear on its own. `false` marks the
    /// errors that stay broken until a person acts — expired credentials above
    /// all — so a reader can tell "nudge it" from "go fix the account".
    pub transient: bool,
}

/// How many trailing non-empty lines may carry the error.
///
/// Measured against real panes rather than guessed: a healthy Claude screen
/// ends with about five lines of prompt furniture (rule, input box, rule,
/// status bar) and the CLI writes its "done" line under the error too, so the
/// error itself sits six or seven lines up before it has even wrapped. Eight
/// would clear that by nothing at all.
const TAIL_LINES: usize = 16;

/// The longest summary worth carrying into a status row.
const SUMMARY_MAX_CHARS: usize = 160;

/// The renderer prefixes every turn-ending provider failure with this.
const PREFIX: &str = "api error";

/// Classify the tail of `screen` as a turn-ending provider API error.
///
/// Returns `None` for a screen that merely mentions one, for a quota notice
/// (that is [`super::detect_provider_limit_notice`]'s call to make, and it
/// carries a reset instant this does not), and for an error that has already
/// scrolled out of the tail.
pub fn detect_provider_api_error(screen: &str) -> Option<ProviderApiError> {
    let lines: Vec<&str> = screen
        .lines()
        .map(|line| strip_gutter(line.trim()))
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    // Last wins: a pane that failed twice is reporting the newer failure.
    let (line, rest) = lines[start..]
        .iter()
        .rev()
        .find_map(|line| split_prefix(line).map(|rest| (*line, rest)))?;

    if is_quoted_mention(line, rest) {
        return None;
    }
    let summary = summarize(rest);
    if summary.is_empty() {
        return None;
    }
    if describes_a_quota_block(&summary.to_ascii_lowercase()) {
        return None;
    }
    let http_status = parse_http_status(&summary);
    Some(ProviderApiError {
        transient: is_transient(http_status, &summary.to_ascii_lowercase()),
        http_status,
        summary,
    })
}

/// Render one line for a reader who will not see the pane, naming the account
/// when the caller knows it.
pub fn describe_provider_api_error(error: &ProviderApiError, provider: Option<&str>) -> String {
    let mut out = String::from("Provider API error");
    if let Some(provider) = provider.filter(|provider| !provider.is_empty()) {
        out.push_str(&format!(" ({provider})"));
    }
    if let Some(status) = error.http_status {
        out.push_str(&format!(" HTTP {status}"));
    }
    out.push_str(": ");
    out.push_str(&error.summary);
    out.push_str(if error.transient {
        " — the turn ended and the pane is waiting for input; it resumes when told to continue."
    } else {
        " — the turn ended and the pane cannot resume until a person clears the cause."
    });
    out
}

/// Remove the glyphs a CLI draws in the left gutter so the prefix can be
/// anchored to the start of the message itself.
///
/// Only the renderer's own furniture is stripped. Markdown bullets (`-`,
/// `*`) and the input prompt (`>`) are deliberately left in place: prose
/// listing an error and a message typed or pasted into the prompt both put
/// the sentence at the start of a line, and stripping those markers would
/// turn a description of an outage into a report of one.
fn strip_gutter(line: &str) -> &str {
    line.trim_start_matches(|ch: char| {
        matches!(ch, '⎿' | '│' | '╰' | '╭' | '•' | '·' | '✻' | '⏺') || ch.is_whitespace()
    })
}

/// Split a line on the renderer's prefix, returning what follows it. Both
/// `API Error:` and `API Error (…)` are produced in the wild.
fn split_prefix(line: &str) -> Option<&str> {
    let lowered = line.to_ascii_lowercase();
    if !lowered.starts_with(PREFIX) {
        return None;
    }
    let rest = line[PREFIX.len()..].trim_start();
    rest.strip_prefix(':')
        .map(str::trim_start)
        .or_else(|| rest.starts_with('(').then_some(rest))
}

/// Whether this line is prose *about* an error rather than the error itself.
///
/// The give-away is what sits immediately before the prefix: the renderer
/// writes it at the start of its own line, so anything quoting it has to open
/// a quote or a code span first.
fn is_quoted_mention(line: &str, rest: &str) -> bool {
    let opened = line
        .len()
        .checked_sub(rest.len())
        .and_then(|cut| line[..cut].chars().next_back());
    matches!(opened, Some('`' | '"' | '\'' | '「' | '“' | '‘'))
}

/// Collapse the message to one readable line.
fn summarize(rest: &str) -> String {
    let mut out = String::with_capacity(rest.len().min(SUMMARY_MAX_CHARS));
    let mut pending_space = false;
    for ch in rest.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(ch);
    }
    while out.chars().count() > SUMMARY_MAX_CHARS {
        out.pop();
    }
    out.trim_end_matches(['.', ',', '—', '-', ' '])
        .trim()
        .to_string()
}

/// A three-digit HTTP status, taken only from the head of the message where
/// the renderer puts it. Scanning the whole line would pick digits out of the
/// prose that follows.
fn parse_http_status(summary: &str) -> Option<u16> {
    let token = summary.split_whitespace().next()?;
    let digits: String = token.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() != 3 {
        return None;
    }
    digits
        .parse()
        .ok()
        .filter(|status| (400..=599).contains(status))
}

/// Quota exhaustion is a different diagnosis with a different remedy and its
/// own detector, so it is declined here even when it arrives wearing a status.
fn describes_a_quota_block(summary: &str) -> bool {
    summary.contains("usage limit")
        || summary.contains("weekly limit")
        || summary.contains("limit reached")
        || summary.contains("resets at")
        || summary.contains("purchase more credits")
}

/// Whether waiting is enough. Authentication and authorization failures are
/// the ones that never clear on their own.
fn is_transient(http_status: Option<u16>, summary: &str) -> bool {
    if matches!(http_status, Some(401 | 403)) {
        return false;
    }
    !(summary.contains("re-authenticate")
        || summary.contains("reauthenticate")
        || summary.contains("access token has expired")
        || summary.contains("invalid api key")
        || summary.contains("authentication"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The renderer draws the error, then its own prompt furniture underneath.
    fn pane(error: &str) -> String {
        format!(
            "⏺ Running the verification matrix now.\n\n{error}\n\n\
             ╭──────────────────────────────────────╮\n\
             │ >                                    │\n\
             ╰──────────────────────────────────────╯\n\
             ? for shortcuts\n"
        )
    }

    /// Issue #4584: every wording observed ending a real turn on this machine.
    /// The HTTP family the Issue names is only part of it — the transport
    /// failures end the turn identically and carry no status at all.
    #[test]
    fn detects_every_turn_ending_error_in_the_observed_corpus() {
        let cases: [(&str, Option<u16>, bool); 8] = [
            (
                "API Error: 529 Overloaded. This is a server-side issue, usually temporary — try again in a moment.",
                Some(529),
                true,
            ),
            (
                "API Error: 429 Too Many Requests",
                Some(429),
                true,
            ),
            (
                "API Error: 500 Internal server error",
                Some(500),
                true,
            ),
            (
                "API Error: 503 Service Unavailable",
                Some(503),
                true,
            ),
            (
                "API Error: 401 OAuth access token has expired. Re-authenticate to continue.",
                Some(401),
                false,
            ),
            (
                "API Error: Can't reach the API server — check your internet or DNS (ENOTFOUND)",
                None,
                true,
            ),
            (
                "API Error: Connection dropped (ECONNRESET)",
                None,
                true,
            ),
            (
                "API Error: Your computer went to sleep mid-response. The response above may be incomplete.",
                None,
                true,
            ),
        ];

        for (line, status, transient) in cases {
            let detected = detect_provider_api_error(&pane(line))
                .unwrap_or_else(|| panic!("no detection for {line:?}"));
            assert_eq!(detected.http_status, status, "status for {line:?}");
            assert_eq!(detected.transient, transient, "transient for {line:?}");
            assert!(
                !detected.summary.is_empty(),
                "summary must carry the body for {line:?}"
            );
        }
    }

    /// AC-2 wants the status and the body, not just the fact of an error.
    #[test]
    fn summary_carries_the_body_the_status_row_must_show() {
        let detected = detect_provider_api_error(&pane(
            "API Error: 529 Overloaded. This is a server-side issue, usually temporary",
        ))
        .expect("detected");
        assert_eq!(
            detected.summary,
            "529 Overloaded. This is a server-side issue, usually temporary"
        );
        assert!(describe_provider_api_error(&detected, Some("claude")).contains("HTTP 529"));
    }

    /// AC-3: the description has to separate "nudge it" from "go fix it".
    #[test]
    fn description_distinguishes_a_nudge_from_an_intervention() {
        let transient = detect_provider_api_error(&pane("API Error: 529 Overloaded.")).unwrap();
        let permanent = detect_provider_api_error(&pane(
            "API Error: 401 OAuth access token has expired. Re-authenticate to continue.",
        ))
        .unwrap();
        assert!(describe_provider_api_error(&transient, None).contains("resumes when told"));
        assert!(describe_provider_api_error(&permanent, None).contains("until a person"));
    }

    /// The panes in this Issue's own evidence were later sent prose quoting the
    /// error. Reading that back as a fresh outage would re-report a pane that
    /// had already recovered.
    #[test]
    fn rejects_prose_that_merely_quotes_the_error() {
        let quoted = "⏺ 直前のターンは `API Error: 529 Overloaded` で終了していました。作業を再開します。\n\n> ";
        assert_eq!(detect_provider_api_error(quoted), None);
    }

    /// A PM writing *about* an outage reaches for a bullet list, and a nudge
    /// pasted into the prompt sits at the bottom of the screen where a real
    /// error would. Neither is the renderer reporting a failure.
    #[test]
    fn rejects_bulleted_prose_and_text_at_the_input_prompt() {
        for line in [
            "- API Error: 529 Overloaded stopped two windows this morning",
            "* API Error: 401 needs a re-auth, see the runbook",
            "> API Error: 529 Overloaded で止まっていたので再開してください",
        ] {
            assert_eq!(
                detect_provider_api_error(&format!("⏺ earlier work\n{line}\n")),
                None,
                "{line:?} describes an error, it is not one"
            );
        }
    }

    /// The error has to be the last thing written. Once work resumes it
    /// scrolls away, and that is what releases the pane's projected state.
    #[test]
    fn rejects_an_error_that_scrolled_out_of_the_tail() {
        let mut screen = String::from("API Error: 529 Overloaded.\n");
        for step in 0..TAIL_LINES {
            screen.push_str(&format!("⏺ resumed work, step {step}\n"));
        }
        assert_eq!(detect_provider_api_error(&screen), None);

        // One line fewer and it is still the pane's current state: the
        // boundary is what releases the hold, so pin both sides of it.
        let mut still_visible = String::from("API Error: 529 Overloaded.\n");
        for step in 0..TAIL_LINES - 1 {
            still_visible.push_str(&format!("⏺ resumed work, step {step}\n"));
        }
        assert!(detect_provider_api_error(&still_visible).is_some());
    }

    /// Quota exhaustion has its own detector, its own reset instant, and its
    /// own remedy. Two holds for one cause would fight over the same pane.
    #[test]
    fn declines_a_quota_block_to_the_limit_notice_detector() {
        let screen = pane("API Error: 429 usage limit reached. resets at 3pm");
        assert_eq!(detect_provider_api_error(&screen), None);
    }

    /// A healthy pane must never produce a hold.
    #[test]
    fn ignores_a_screen_with_no_error() {
        assert_eq!(
            detect_provider_api_error("⏺ All 44 tests passed.\n\n> "),
            None
        );
    }

    /// A pane that failed twice is reporting the newer failure.
    #[test]
    fn reports_the_most_recent_error_on_screen() {
        let screen = "API Error: 500 Internal server error\n\
                      ⏺ retrying\n\
                      API Error: 529 Overloaded.\n> ";
        let detected = detect_provider_api_error(screen).expect("detected");
        assert_eq!(detected.http_status, Some(529));
    }
}
