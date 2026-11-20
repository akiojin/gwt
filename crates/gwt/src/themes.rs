//! Themes: the screen of a project that groups Issues (Issue #7643, SPEC
//! #7007 T-025).
//!
//! A theme is a GitHub Issue that has sub-issues. How far a theme has come is
//! GitHub's own sub-issue summary, read each time the screen opens; nothing
//! about it is stored here. "Completed today" is the one figure built locally:
//! it counts the closes the Issue cache took in today.

use std::collections::BTreeSet;

use gwt_core::timestamp::UtcTimestamp;
use gwt_github::{
    client::{ThemeIssue, ThemeListing},
    ApiError, Cache, CloseIntakeRecord, IssueClient, IssueCloseReason, IssueNumber, IssueState,
};
use serde::Serialize;

/// The longest theme description the list and the detail show.
const SUMMARY_MAX_CHARS: usize = 280;

/// What the Themes screen shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThemesView {
    pub themes: Vec<ThemeView>,
    /// Issues whose close the cache took in today, in local time.
    pub completed_today: u64,
    /// The repository has more open Issues than one listing reads.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThemeView {
    pub number: u64,
    pub title: String,
    /// The first paragraph of the Issue body, as plain text.
    pub summary: String,
    pub url: String,
    pub issues_total: u64,
    pub issues_completed: u64,
    pub issues: Vec<ThemeIssueView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThemeIssueView {
    pub number: u64,
    pub title: String,
    /// `open` or `closed`.
    pub state: &'static str,
    pub url: String,
    /// Whether the Issue belongs to this project's repository, so it can be
    /// opened in the Issues screen.
    pub in_repository: bool,
}

/// Why a theme request was refused, as the screen shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeRefusal(pub String);

impl From<ApiError> for ThemeRefusal {
    fn from(error: ApiError) -> Self {
        Self(match error {
            ApiError::Unauthorized => {
                "GitHub sign-in is required. Sign in to GitHub, then try again.".to_string()
            }
            ApiError::PermissionDenied { message } => {
                format!("GitHub refused the request: {message}")
            }
            other => format!("GitHub could not be reached: {other}"),
        })
    }
}

/// Read the themes of the repository and what was completed today.
pub fn load_themes_view(
    client: &dyn IssueClient,
    cache: &Cache,
    now: UtcTimestamp,
) -> Result<ThemesView, ThemeRefusal> {
    Ok(themes_view(client.list_themes()?, cache, now))
}

/// Delete one theme and answer with the themes that remain. The number must
/// name a current theme: this request never touches any other Issue.
///
/// A theme is deleted by closing its GitHub Issue as not planned. Nothing is
/// erased: the Issue and the issues under it stay on GitHub, and reopening the
/// Issue there brings the theme back.
pub fn delete_theme(
    client: &dyn IssueClient,
    cache: &Cache,
    number: u64,
    now: UtcTimestamp,
) -> Result<ThemesView, ThemeRefusal> {
    let mut listing = client.list_themes()?;
    if !listing.themes.iter().any(|theme| theme.number.0 == number) {
        return Err(ThemeRefusal(format!(
            "Issue #{number} is not a theme of this project, so it was not closed."
        )));
    }
    client
        .set_state(
            IssueNumber(number),
            IssueState::Closed,
            Some(IssueCloseReason::NotPlanned),
        )
        .map_err(|error| {
            ThemeRefusal(match error {
                ApiError::PermissionDenied { .. } => format!(
                    "GitHub did not close Issue #{number}. Closing an Issue needs write access to the repository."
                ),
                other => ThemeRefusal::from(other).0,
            })
        })?;
    // Drop the cached copy instead of rewriting it as closed: a theme given up
    // is not work completed today, and the next refresh reads it back closed.
    let _ = cache.remove_issue(IssueNumber(number));
    listing.themes.retain(|theme| theme.number.0 != number);
    Ok(themes_view(listing, cache, now))
}

fn themes_view(listing: ThemeListing, cache: &Cache, now: UtcTimestamp) -> ThemesView {
    let completed_today = completed_today(&cache.close_intake_records(), now, |number| {
        cache
            .load_entry(IssueNumber(number))
            .is_none_or(|entry| entry.snapshot.state == IssueState::Closed)
    });
    ThemesView {
        themes: listing.themes.into_iter().map(theme_view).collect(),
        completed_today,
        truncated: listing.truncated,
    }
}

fn theme_view(theme: ThemeIssue) -> ThemeView {
    ThemeView {
        number: theme.number.0,
        summary: theme_summary(&theme.body),
        title: theme.title,
        url: theme.url,
        issues_total: theme.sub_issues_total,
        issues_completed: theme.sub_issues_completed,
        issues: theme
            .sub_issues
            .into_iter()
            .map(|issue| ThemeIssueView {
                number: issue.number.0,
                title: issue.title,
                state: match issue.state {
                    IssueState::Open => "open",
                    IssueState::Closed => "closed",
                },
                url: issue.url,
                in_repository: issue.in_repository,
            })
            .collect(),
    }
}

/// Count the Issues whose close was taken in on the local day of `now`. An
/// Issue closed twice today counts once, and one reopened since does not
/// count: `still_closed` answers for the Issue as the cache has it now.
pub fn completed_today(
    records: &[CloseIntakeRecord],
    now: UtcTimestamp,
    still_closed: impl Fn(u64) -> bool,
) -> u64 {
    let today = now.local_datetime().date();
    records
        .iter()
        .filter(|record| {
            UtcTimestamp::parse_rfc3339(&record.ingested_at)
                .is_some_and(|at| at.local_datetime().date() == today)
        })
        .map(|record| record.number)
        .collect::<BTreeSet<u64>>()
        .into_iter()
        .filter(|number| still_closed(*number))
        .count() as u64
}

/// The first paragraph of an Issue body that says something: comments,
/// headings and blank lines are skipped, and a long paragraph is cut short.
pub fn theme_summary(body: &str) -> String {
    let mut text = String::new();
    let mut in_comment = false;
    for line in body.lines() {
        let mut line = line.trim();
        if in_comment {
            match line.split_once("-->") {
                Some((_, rest)) => {
                    in_comment = false;
                    line = rest.trim();
                }
                None => continue,
            }
        }
        if let Some((before, rest)) = line.split_once("<!--") {
            in_comment = !rest.contains("-->");
            line = before.trim();
        }
        if line.is_empty() || line.starts_with('#') {
            if text.is_empty() {
                continue;
            }
            break;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(line);
    }
    if text.chars().count() > SUMMARY_MAX_CHARS {
        text = text.chars().take(SUMMARY_MAX_CHARS).collect::<String>() + "…";
    }
    text
}

#[cfg(test)]
mod tests {
    use gwt_github::{client::ThemeSubIssue, FakeIssueClient, IssueSnapshot, UpdatedAt};
    use time::Duration;

    use super::*;

    fn record(number: u64, at: UtcTimestamp) -> CloseIntakeRecord {
        CloseIntakeRecord {
            number,
            ingested_at: at.to_rfc3339(),
        }
    }

    fn snapshot(number: u64, state: IssueState) -> IssueSnapshot {
        IssueSnapshot {
            number: IssueNumber(number),
            author_login: None,
            title: format!("Issue {number}"),
            body: String::new(),
            labels: Vec::new(),
            state,
            updated_at: UpdatedAt::new("t"),
            comments: Vec::new(),
        }
    }

    fn theme(number: u64, sub_issues: &[(u64, IssueState)]) -> ThemeIssue {
        ThemeIssue {
            number: IssueNumber(number),
            title: format!("Theme {number}"),
            body: "<!-- marker -->\n## Goal\n\nRebuild every screen.\nIn one pass.\n\nLater text."
                .to_string(),
            url: format!("https://github.com/o/r/issues/{number}"),
            sub_issues_total: sub_issues.len() as u64,
            sub_issues_completed: sub_issues
                .iter()
                .filter(|(_, state)| *state == IssueState::Closed)
                .count() as u64,
            sub_issues: sub_issues
                .iter()
                .map(|(number, state)| ThemeSubIssue {
                    number: IssueNumber(*number),
                    title: format!("Issue {number}"),
                    state: *state,
                    url: format!("https://github.com/o/r/issues/{number}"),
                    in_repository: true,
                })
                .collect(),
        }
    }

    #[test]
    fn completed_today_counts_each_issue_closed_today_once() {
        let now = UtcTimestamp::now();
        let long_ago = now.checked_add(Duration::days(-3)).unwrap();
        let records = [
            record(1, now),
            record(1, now),
            record(2, now),
            record(3, long_ago),
            CloseIntakeRecord {
                number: 4,
                ingested_at: "not a time".to_string(),
            },
        ];

        assert_eq!(completed_today(&records, now, |_| true), 2);
        assert_eq!(
            completed_today(&records, now, |number| number != 2),
            1,
            "an Issue reopened since is not completed"
        );
        assert_eq!(completed_today(&[], now, |_| true), 0);
    }

    #[test]
    fn summary_is_the_first_paragraph_without_markup_lines() {
        assert_eq!(
            theme_summary("<!-- a\nb -->\n# Title\n\nFirst line.\nSecond line.\n\nNext."),
            "First line. Second line."
        );
        assert_eq!(theme_summary("Lead <!-- note --> text\n\nNext."), "Lead");
        assert_eq!(theme_summary("\n\n"), "");
        let long = "x".repeat(400);
        let summary = theme_summary(&long);
        assert_eq!(summary.chars().count(), SUMMARY_MAX_CHARS + 1);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn view_carries_github_totals_and_todays_closes() {
        let temp = tempfile::tempdir().unwrap();
        let cache = Cache::new(temp.path().to_path_buf());
        cache
            .write_snapshot(&snapshot(9, IssueState::Open))
            .unwrap();
        cache
            .write_snapshot(&snapshot(9, IssueState::Closed))
            .unwrap();
        // Closed and reopened today: not completed.
        cache
            .write_snapshot(&snapshot(10, IssueState::Open))
            .unwrap();
        cache
            .write_snapshot(&snapshot(10, IssueState::Closed))
            .unwrap();
        cache
            .write_snapshot(&snapshot(10, IssueState::Open))
            .unwrap();
        let client = FakeIssueClient::new();
        client.seed_themes(vec![theme(
            7,
            &[(8, IssueState::Open), (9, IssueState::Closed)],
        )]);

        let view = load_themes_view(&client, &cache, UtcTimestamp::now()).unwrap();

        assert_eq!(view.completed_today, 1);
        assert!(!view.truncated);
        let theme = &view.themes[0];
        assert_eq!(theme.number, 7);
        assert_eq!(theme.summary, "Rebuild every screen. In one pass.");
        assert_eq!((theme.issues_total, theme.issues_completed), (2, 1));
        assert_eq!(theme.issues[0].state, "open");
        assert_eq!(theme.issues[1].state, "closed");
        assert!(theme.issues[1].in_repository);
    }

    #[test]
    fn deleting_a_theme_closes_its_issue_as_not_planned_and_drops_it_from_the_view() {
        let temp = tempfile::tempdir().unwrap();
        let cache = Cache::new(temp.path().to_path_buf());
        cache
            .write_snapshot(&snapshot(7, IssueState::Open))
            .unwrap();
        let client = FakeIssueClient::new();
        client.seed(snapshot(7, IssueState::Open));
        client.seed_themes(vec![
            theme(7, &[(8, IssueState::Open)]),
            theme(20, &[(21, IssueState::Open)]),
        ]);

        let view = delete_theme(&client, &cache, 7, UtcTimestamp::now()).unwrap();

        assert_eq!(
            view.themes
                .iter()
                .map(|theme| theme.number)
                .collect::<Vec<_>>(),
            vec![20]
        );
        assert!(client
            .call_log()
            .contains(&"set_state:#7:not_planned".to_string()));
        // A theme given up is not an Issue completed today.
        assert!(cache.load_entry(IssueNumber(7)).is_none());
        assert_eq!(view.completed_today, 0);
    }

    #[test]
    fn only_a_theme_can_be_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let cache = Cache::new(temp.path().to_path_buf());
        let client = FakeIssueClient::new();
        client.seed(snapshot(8, IssueState::Open));
        client.seed_themes(vec![theme(7, &[(8, IssueState::Open)])]);

        let refusal = delete_theme(&client, &cache, 8, UtcTimestamp::now()).unwrap_err();

        assert!(refusal.0.contains("#8"));
        assert!(!client
            .call_log()
            .iter()
            .any(|call| call.starts_with("set_state")));
    }

    #[test]
    fn a_failed_close_is_refused_with_the_reason() {
        let temp = tempfile::tempdir().unwrap();
        let cache = Cache::new(temp.path().to_path_buf());
        let client = FakeIssueClient::new();
        // The Issue is unknown to the client, so the close itself fails.
        client.seed_themes(vec![theme(7, &[(8, IssueState::Open)])]);

        let refusal = delete_theme(&client, &cache, 7, UtcTimestamp::now()).unwrap_err();

        assert!(refusal.0.contains("could not be reached"), "{}", refusal.0);
    }

    #[test]
    fn refusals_say_what_to_do() {
        assert!(ThemeRefusal::from(ApiError::Unauthorized)
            .0
            .contains("Sign in"));
        assert!(ThemeRefusal::from(ApiError::PermissionDenied {
            message: "no".into()
        })
        .0
        .contains("no"));
        assert!(ThemeRefusal::from(ApiError::Network("down".into()))
            .0
            .contains("down"));
        // No themes is an answer, not a refusal.
        let temp = tempfile::tempdir().unwrap();
        let cache = Cache::new(temp.path().to_path_buf());
        let client = FakeIssueClient::new();
        assert!(load_themes_view(&client, &cache, UtcTimestamp::now())
            .unwrap()
            .themes
            .is_empty());
    }
}
