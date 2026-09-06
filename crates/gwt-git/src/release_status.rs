//! Interrupted-release standing check (Issue #3516).
//!
//! `/release` lands `chore(release): vX.Y.Z` (version bump + CHANGELOG) on the
//! release branch and only then opens the `develop -> main` Release PR. When it
//! stops between those two steps the bump sits on the branch with nothing
//! driving it to a release, and the gap is invisible until a human notices it
//! (PR #3513 was such a manual recovery).
//!
//! This module turns that gap into an observable state the resident PM loop can
//! read every cycle, plus an idempotent reconcile that opens the missing
//! Release PR.
//!
//! The check is deliberately ordered cheapest-first: the release-branch
//! subjects and the tag lookup are local git reads, so a repository with no
//! pending bump — the common case — never spends GitHub budget.

use std::collections::HashSet;
use std::path::Path;

use gwt_core::{GwtError, Result};
use serde::{Deserialize, Serialize};

use crate::pr_status::{run_gh_command, GhCliOutput};

/// Branch `/release` bumps the version on.
pub const DEFAULT_RELEASE_BRANCH: &str = "develop";

/// Branch the Release PR targets.
pub const DEFAULT_BASE_BRANCH: &str = "main";

/// How many release-branch subjects the check scans for the bump commit.
///
/// The bump is the head commit of a healthy release, but ordinary work keeps
/// landing on `develop` while the release stalls, so the window has to cover a
/// few hours of activity.
pub const DEFAULT_SCAN_COMMITS: usize = 30;

/// Conventional-commit subject prefix `/release` writes for the version bump.
const BUMP_SUBJECT_PREFIX: &str = "chore(release):";

/// State of the release pipeline between the version bump and the Release PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseCheckState {
    /// No `chore(release): vX.Y.Z` commit in the scanned window: nothing is
    /// pending.
    NoBump,
    /// The bumped version is already tagged; the release completed.
    Released,
    /// A Release PR for the bumped version is already open.
    PrOpen,
    /// The bump landed, the version is untagged, and no Release PR exists:
    /// `/release` stopped early.
    Stalled,
}

impl ReleaseCheckState {
    /// Machine-readable name used in JSON output and in log lines.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoBump => "no_bump",
            Self::Released => "released",
            Self::PrOpen => "pr_open",
            Self::Stalled => "stalled",
        }
    }
}

/// Inputs the classification is derived from.
///
/// Keeping the observations separate from the reads makes every branch of the
/// decision table testable without a repository, a network, or a `gh` binary.
#[derive(Debug, Clone)]
pub struct ReleaseCheckInput {
    /// Branch the version bump lands on.
    pub release_branch: String,
    /// Branch the Release PR targets.
    pub base_branch: String,
    /// Release-branch commit subjects, newest first.
    pub recent_subjects: Vec<String>,
    /// Tag names that exist, normalized as `vX.Y.Z`.
    pub existing_tags: HashSet<String>,
    /// Number of an open PR with base `base_branch` and head `release_branch`.
    pub open_release_pr: Option<u64>,
}

/// Outcome of the standing check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseCheck {
    /// Classified state.
    pub state: ReleaseCheckState,
    /// Version carried by the newest bump commit, if any (`vX.Y.Z`).
    pub version: Option<String>,
    /// Open Release PR number, when one was found.
    pub release_pr: Option<u64>,
    /// Branch the bump was looked for on.
    pub release_branch: String,
    /// Branch the Release PR targets.
    pub base_branch: String,
}

impl ReleaseCheck {
    /// True when `/release` stopped between the bump and the Release PR.
    pub fn is_stalled(&self) -> bool {
        self.state == ReleaseCheckState::Stalled
    }
}

/// Options for the repository-backed check.
#[derive(Debug, Clone)]
pub struct ReleaseCheckOptions {
    /// Branch the version bump lands on.
    pub release_branch: String,
    /// Branch the Release PR targets.
    pub base_branch: String,
    /// How many release-branch subjects to scan.
    pub scan_commits: usize,
}

impl Default for ReleaseCheckOptions {
    fn default() -> Self {
        Self {
            release_branch: DEFAULT_RELEASE_BRANCH.to_string(),
            base_branch: DEFAULT_BASE_BRANCH.to_string(),
            scan_commits: DEFAULT_SCAN_COMMITS,
        }
    }
}

/// Result of the idempotent reconcile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleasePrEnsure {
    /// State after the reconcile.
    pub check: ReleaseCheck,
    /// Whether this call created the Release PR.
    pub created: bool,
    /// URL of the Release PR when this call created it.
    pub pr_url: Option<String>,
}

/// Extract the released version from a `chore(release): vX.Y.Z` subject.
///
/// The leading `v` is optional in the subject and always present in the
/// returned version, so callers can compare against tag names directly.
pub fn parse_release_bump_version(subject: &str) -> Option<String> {
    let rest = subject.trim().strip_prefix(BUMP_SUBJECT_PREFIX)?.trim();
    let rest = rest.strip_prefix('v').unwrap_or(rest);
    if rest.is_empty() {
        return None;
    }
    let core = rest
        .split_once(['-', '+'])
        .map_or(rest, |(core, _suffix)| core);
    let mut parts = core.split('.');
    for _ in 0..3 {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
    }
    if parts.next().is_some() {
        return None;
    }
    Some(format!("v{rest}"))
}

/// Newest bump version in a list of subjects ordered newest-first.
pub fn newest_bump_version(subjects: &[String]) -> Option<String> {
    subjects
        .iter()
        .find_map(|subject| parse_release_bump_version(subject))
}

/// Classify the release pipeline from already-collected observations.
///
/// Precedence mirrors the read order of [`fetch_release_check`]: an untagged,
/// PR-less bump is the only state that asks for action, and both "already
/// tagged" and "PR already open" mean the reconcile does nothing.
pub fn classify_release_check(input: &ReleaseCheckInput) -> ReleaseCheck {
    let version = newest_bump_version(&input.recent_subjects);
    let state = match version.as_deref() {
        None => ReleaseCheckState::NoBump,
        Some(version) if input.existing_tags.contains(version) => ReleaseCheckState::Released,
        Some(_) if input.open_release_pr.is_some() => ReleaseCheckState::PrOpen,
        Some(_) => ReleaseCheckState::Stalled,
    };
    let release_pr = match state {
        ReleaseCheckState::PrOpen => input.open_release_pr,
        _ => None,
    };
    ReleaseCheck {
        state,
        version,
        release_pr,
        release_branch: input.release_branch.clone(),
        base_branch: input.base_branch.clone(),
    }
}

/// Parse `gh pr list --json number` output into the open Release PR number.
///
/// The smallest number wins so repeated reads answer identically regardless of
/// the order `gh` happens to return.
pub fn parse_open_release_pr(json: &str) -> Result<Option<u64>> {
    let rows: Vec<serde_json::Value> = serde_json::from_str(json)
        .map_err(|error| GwtError::Other(format!("gh pr list JSON: {error}")))?;
    Ok(rows
        .iter()
        .filter_map(|row| row.get("number").and_then(serde_json::Value::as_u64))
        .min())
}

/// Extract the CHANGELOG section for `version` so the Release PR body carries
/// the release notes rather than a bare pointer.
///
/// Matches the `## [9.91.0] - 2026-09-06` heading style git-cliff writes, and
/// stops at the next `## ` heading.
pub fn changelog_section(changelog: &str, version: &str) -> Option<String> {
    let bare = version.strip_prefix('v').unwrap_or(version);
    let mut lines = changelog.lines();
    let heading = lines.by_ref().find(|line| is_version_heading(line, bare))?;
    let mut section = vec![heading.to_string()];
    for line in lines {
        if line.starts_with("## ") {
            break;
        }
        section.push(line.to_string());
    }
    while section.last().is_some_and(|line| line.trim().is_empty()) {
        section.pop();
    }
    Some(section.join("\n"))
}

/// True when `line` is the `## ...` heading that introduces `bare` version.
fn is_version_heading(line: &str, bare: &str) -> bool {
    let Some(rest) = line.strip_prefix("## ") else {
        return false;
    };
    let rest = rest.trim();
    let rest = rest.strip_prefix('[').unwrap_or(rest);
    let rest = rest.strip_prefix('v').unwrap_or(rest);
    let Some(after) = rest.strip_prefix(bare) else {
        return false;
    };
    after
        .chars()
        .next()
        .is_none_or(|next| !next.is_ascii_digit() && next != '.')
}

/// Run the standing check against a repository.
pub fn fetch_release_check(
    repo_path: &Path,
    options: &ReleaseCheckOptions,
) -> Result<ReleaseCheck> {
    fetch_release_check_with(
        repo_path,
        options,
        crate::commit::branch_recent_subjects,
        crate::refs::list_existing_refs,
        run_gh_command,
    )
}

/// [`fetch_release_check`] with the git and `gh` reads injected.
fn fetch_release_check_with<S, T, G>(
    repo_path: &Path,
    options: &ReleaseCheckOptions,
    mut subjects: S,
    mut tags: T,
    mut run_gh: G,
) -> Result<ReleaseCheck>
where
    S: FnMut(&Path, &str, usize) -> Result<Vec<String>>,
    T: FnMut(&Path, &[&str]) -> Result<HashSet<String>>,
    G: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let recent_subjects = subjects(repo_path, &options.release_branch, options.scan_commits)?;
    let mut input = ReleaseCheckInput {
        release_branch: options.release_branch.clone(),
        base_branch: options.base_branch.clone(),
        recent_subjects,
        existing_tags: HashSet::new(),
        open_release_pr: None,
    };

    let Some(version) = newest_bump_version(&input.recent_subjects) else {
        return Ok(classify_release_check(&input));
    };

    let tag_ref = format!("refs/tags/{version}");
    let existing = tags(repo_path, &[tag_ref.as_str()])?;
    if existing.contains(&tag_ref) || existing.contains(&version) {
        input.existing_tags.insert(version);
        return Ok(classify_release_check(&input));
    }

    input.open_release_pr = fetch_open_release_pr(
        repo_path,
        &options.base_branch,
        &options.release_branch,
        &mut run_gh,
    )?;
    Ok(classify_release_check(&input))
}

/// Read the open `base <- head` pull request number through `gh`.
fn fetch_open_release_pr<G>(
    repo_path: &Path,
    base: &str,
    head: &str,
    run_gh: &mut G,
) -> Result<Option<u64>>
where
    G: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let output = run_gh(
        repo_path,
        &[
            "pr", "list", "--base", base, "--head", head, "--state", "open", "--json", "number",
            "--limit", "10",
        ],
    )?;
    if !output.success {
        return Err(GwtError::Git(format!(
            "gh pr list release: {}",
            output.stderr.trim()
        )));
    }
    parse_open_release_pr(&output.stdout)
}

/// Detect a stalled release and open the missing Release PR.
///
/// Idempotent: the classification runs first, and only [`ReleaseCheckState::
/// Stalled`] reaches the `gh pr create` call, so a second run over a repository
/// that already has the PR (or the tag) mutates nothing.
pub fn ensure_release_pr(
    repo_path: &Path,
    options: &ReleaseCheckOptions,
) -> Result<ReleasePrEnsure> {
    ensure_release_pr_with(
        repo_path,
        options,
        crate::commit::branch_recent_subjects,
        crate::refs::list_existing_refs,
        run_gh_command,
        |root| std::fs::read_to_string(root.join("CHANGELOG.md")).ok(),
    )
}

/// [`ensure_release_pr`] with the git, `gh`, and CHANGELOG reads injected.
fn ensure_release_pr_with<S, T, G, C>(
    repo_path: &Path,
    options: &ReleaseCheckOptions,
    subjects: S,
    tags: T,
    mut run_gh: G,
    mut changelog: C,
) -> Result<ReleasePrEnsure>
where
    S: FnMut(&Path, &str, usize) -> Result<Vec<String>>,
    T: FnMut(&Path, &[&str]) -> Result<HashSet<String>>,
    G: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
    C: FnMut(&Path) -> Option<String>,
{
    let check = fetch_release_check_with(repo_path, options, subjects, tags, &mut run_gh)?;
    if !check.is_stalled() {
        return Ok(ReleasePrEnsure {
            check,
            created: false,
            pr_url: None,
        });
    }

    let version = check
        .version
        .clone()
        .ok_or_else(|| GwtError::Other("stalled release without a version".to_string()))?;
    let title = format!("{BUMP_SUBJECT_PREFIX} {version}");
    let body = release_pr_body(changelog(repo_path).as_deref(), &version);
    let output = run_gh(
        repo_path,
        &[
            "pr",
            "create",
            "--base",
            &check.base_branch,
            "--head",
            &check.release_branch,
            "--title",
            &title,
            "--body",
            &body,
        ],
    )?;
    if !output.success {
        return Err(GwtError::Git(format!(
            "gh pr create release: {}",
            output.stderr.trim()
        )));
    }

    let pr_url = output.stdout.trim().lines().last().map(str::to_string);
    let release_pr = pr_url.as_deref().and_then(parse_pr_number_from_url);
    Ok(ReleasePrEnsure {
        check: ReleaseCheck {
            state: ReleaseCheckState::PrOpen,
            release_pr,
            ..check
        },
        created: true,
        pr_url,
    })
}

/// Body for the recovered Release PR: the CHANGELOG section when it can be
/// read, otherwise a self-describing placeholder so the PR is never empty.
fn release_pr_body(changelog: Option<&str>, version: &str) -> String {
    let notes = changelog
        .and_then(|text| changelog_section(text, version))
        .unwrap_or_else(|| format!("## {version}"));
    format!("{notes}\n\nRecovered by the gwt interrupted-release check (Issue #3516).\n")
}

/// Pull the PR number out of the URL `gh pr create` prints.
fn parse_pr_number_from_url(url: &str) -> Option<u64> {
    url.trim().rsplit('/').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn subjects(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn input(recent: &[&str], tags: &[&str], pr: Option<u64>) -> ReleaseCheckInput {
        ReleaseCheckInput {
            release_branch: DEFAULT_RELEASE_BRANCH.to_string(),
            base_branch: DEFAULT_BASE_BRANCH.to_string(),
            recent_subjects: subjects(recent),
            existing_tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
            open_release_pr: pr,
        }
    }

    fn ok_gh(stdout: &str) -> Result<GhCliOutput> {
        Ok(GhCliOutput {
            success: true,
            stdout: stdout.to_string(),
            stderr: String::new(),
        })
    }

    #[test]
    fn parses_the_bump_subject_and_normalizes_the_version() {
        assert_eq!(
            parse_release_bump_version("chore(release): v9.91.0").as_deref(),
            Some("v9.91.0")
        );
        assert_eq!(
            parse_release_bump_version("chore(release): 9.91.0").as_deref(),
            Some("v9.91.0")
        );
        assert_eq!(
            parse_release_bump_version("chore(release): v10.0.0-rc.1").as_deref(),
            Some("v10.0.0-rc.1")
        );
    }

    #[test]
    fn rejects_subjects_that_are_not_a_version_bump() {
        for subject in [
            "chore: v9.91.0",
            "chore(release): prepare",
            "chore(release):",
            "chore(release): v9.91",
            "chore(release): v9.91.0.1",
            "feat(release): v9.91.0",
        ] {
            assert!(
                parse_release_bump_version(subject).is_none(),
                "expected no version from {subject}"
            );
        }
    }

    #[test]
    fn newest_bump_wins_when_several_are_in_the_window() {
        let found = newest_bump_version(&subjects(&[
            "fix(gui): tidy",
            "chore(release): v9.91.0",
            "chore(release): v9.90.0",
        ]));
        assert_eq!(found.as_deref(), Some("v9.91.0"));
    }

    // AC-1: bump landed, no tag, no Release PR.
    #[test]
    fn classifies_an_untagged_bump_without_a_pr_as_stalled() {
        let check = classify_release_check(&input(
            &["chore(release): v9.91.0", "fix(gui): tidy"],
            &["v9.90.0"],
            None,
        ));
        assert_eq!(check.state, ReleaseCheckState::Stalled);
        assert_eq!(check.version.as_deref(), Some("v9.91.0"));
        assert_eq!(check.release_pr, None);
        assert!(check.is_stalled());
    }

    // AC-3: an existing tag or an open PR is a no-op state.
    #[test]
    fn classifies_a_tagged_bump_as_released() {
        let check =
            classify_release_check(&input(&["chore(release): v9.91.0"], &["v9.91.0"], None));
        assert_eq!(check.state, ReleaseCheckState::Released);
        assert!(!check.is_stalled());
    }

    #[test]
    fn classifies_an_open_release_pr_as_pr_open() {
        let check = classify_release_check(&input(&["chore(release): v9.91.0"], &[], Some(3513)));
        assert_eq!(check.state, ReleaseCheckState::PrOpen);
        assert_eq!(check.release_pr, Some(3513));
        assert!(!check.is_stalled());
    }

    #[test]
    fn a_tag_wins_over_a_still_open_pull_request() {
        let check = classify_release_check(&input(
            &["chore(release): v9.91.0"],
            &["v9.91.0"],
            Some(3513),
        ));
        assert_eq!(check.state, ReleaseCheckState::Released);
    }

    #[test]
    fn classifies_a_window_without_a_bump_as_no_bump() {
        let check = classify_release_check(&input(&["fix(gui): tidy", "feat(pm): add"], &[], None));
        assert_eq!(check.state, ReleaseCheckState::NoBump);
        assert_eq!(check.version, None);
    }

    #[test]
    fn parses_the_smallest_open_release_pr_number() {
        assert_eq!(
            parse_open_release_pr(r#"[{"number":3520},{"number":3513}]"#).unwrap(),
            Some(3513)
        );
        assert_eq!(parse_open_release_pr("[]").unwrap(), None);
        assert!(parse_open_release_pr("not json").is_err());
    }

    #[test]
    fn extracts_the_changelog_section_for_the_bumped_version() {
        let changelog = "# Changelog\n\n## [9.91.0] - 2026-09-06\n\n### Features\n\n- something\n\n## [9.90.0] - 2026-09-05\n\n- older\n";
        let section = changelog_section(changelog, "v9.91.0").unwrap();
        assert!(section.starts_with("## [9.91.0]"));
        assert!(section.contains("- something"));
        assert!(!section.contains("older"));
        assert!(changelog_section(changelog, "v9.89.0").is_none());
    }

    #[test]
    fn changelog_lookup_does_not_match_a_longer_version() {
        let changelog = "## [9.9.0] - 2026-01-01\n\n- old\n";
        assert!(changelog_section(changelog, "v9.9").is_none());
    }

    // AC-1: the repository-backed read reaches the same verdict.
    #[test]
    fn fetch_reports_stalled_when_the_tag_and_the_pr_are_both_absent() {
        let repo = PathBuf::from("/repo");
        let check = fetch_release_check_with(
            &repo,
            &ReleaseCheckOptions::default(),
            |_, branch, _| {
                assert_eq!(branch, "develop");
                Ok(subjects(&["chore(release): v9.91.0"]))
            },
            |_, _| Ok(HashSet::new()),
            |_, _| ok_gh("[]"),
        )
        .unwrap();
        assert_eq!(check.state, ReleaseCheckState::Stalled);
        assert_eq!(check.version.as_deref(), Some("v9.91.0"));
    }

    // Budget guard: no bump means no tag read and no GitHub call.
    #[test]
    fn fetch_skips_the_tag_and_github_reads_when_no_bump_is_present() {
        let repo = PathBuf::from("/repo");
        let mut tag_reads = 0_u32;
        let mut gh_calls = 0_u32;
        let check = fetch_release_check_with(
            &repo,
            &ReleaseCheckOptions::default(),
            |_, _, _| Ok(subjects(&["fix(gui): tidy"])),
            |_, _| {
                tag_reads += 1;
                Ok(HashSet::new())
            },
            |_, _| {
                gh_calls += 1;
                ok_gh("[]")
            },
        )
        .unwrap();
        assert_eq!(check.state, ReleaseCheckState::NoBump);
        assert_eq!(tag_reads, 0);
        assert_eq!(gh_calls, 0);
    }

    #[test]
    fn fetch_skips_the_github_read_when_the_version_is_already_tagged() {
        let repo = PathBuf::from("/repo");
        let mut gh_calls = 0_u32;
        let check = fetch_release_check_with(
            &repo,
            &ReleaseCheckOptions::default(),
            |_, _, _| Ok(subjects(&["chore(release): v9.91.0"])),
            |_, candidates| {
                assert_eq!(candidates, ["refs/tags/v9.91.0"]);
                Ok(HashSet::from(["refs/tags/v9.91.0".to_string()]))
            },
            |_, _| {
                gh_calls += 1;
                ok_gh("[]")
            },
        )
        .unwrap();
        assert_eq!(check.state, ReleaseCheckState::Released);
        assert_eq!(gh_calls, 0);
    }

    // AC-2: detection flows straight into the Release PR creation.
    #[test]
    fn ensure_creates_the_release_pr_when_the_release_is_stalled() {
        let repo = PathBuf::from("/repo");
        let mut created_args: Vec<String> = Vec::new();
        let outcome = ensure_release_pr_with(
            &repo,
            &ReleaseCheckOptions::default(),
            |_, _, _| Ok(subjects(&["chore(release): v9.91.0"])),
            |_, _| Ok(HashSet::new()),
            |_, args| {
                if args.first() == Some(&"pr") && args.get(1) == Some(&"create") {
                    created_args = args.iter().map(|arg| (*arg).to_string()).collect();
                    return ok_gh("https://github.com/akiojin/gwt/pull/3513\n");
                }
                ok_gh("[]")
            },
            |_| Some("## [9.91.0] - 2026-09-06\n\n- released thing\n".to_string()),
        )
        .unwrap();

        assert!(outcome.created);
        assert_eq!(outcome.check.state, ReleaseCheckState::PrOpen);
        assert_eq!(outcome.check.release_pr, Some(3513));
        assert_eq!(
            outcome.pr_url.as_deref(),
            Some("https://github.com/akiojin/gwt/pull/3513")
        );
        assert!(created_args.contains(&"--base".to_string()));
        assert!(created_args.contains(&"main".to_string()));
        assert!(created_args.contains(&"develop".to_string()));
        assert!(created_args.contains(&"chore(release): v9.91.0".to_string()));
        let body = created_args.last().expect("body argument");
        assert!(body.contains("- released thing"), "body was: {body}");
    }

    // AC-3: re-running over an already-open Release PR mutates nothing.
    #[test]
    fn ensure_is_a_no_op_when_a_release_pr_is_already_open() {
        let repo = PathBuf::from("/repo");
        let mut create_calls = 0_u32;
        let outcome = ensure_release_pr_with(
            &repo,
            &ReleaseCheckOptions::default(),
            |_, _, _| Ok(subjects(&["chore(release): v9.91.0"])),
            |_, _| Ok(HashSet::new()),
            |_, args| {
                if args.get(1) == Some(&"create") {
                    create_calls += 1;
                }
                ok_gh(r#"[{"number":3513}]"#)
            },
            |_| None,
        )
        .unwrap();

        assert!(!outcome.created);
        assert_eq!(outcome.check.state, ReleaseCheckState::PrOpen);
        assert_eq!(outcome.check.release_pr, Some(3513));
        assert_eq!(create_calls, 0);
    }

    // AC-3: an already-tagged version is a no-op too.
    #[test]
    fn ensure_is_a_no_op_when_the_version_is_already_tagged() {
        let repo = PathBuf::from("/repo");
        let mut create_calls = 0_u32;
        let outcome = ensure_release_pr_with(
            &repo,
            &ReleaseCheckOptions::default(),
            |_, _, _| Ok(subjects(&["chore(release): v9.91.0"])),
            |_, _| Ok(HashSet::from(["refs/tags/v9.91.0".to_string()])),
            |_, args| {
                if args.get(1) == Some(&"create") {
                    create_calls += 1;
                }
                ok_gh("[]")
            },
            |_| None,
        )
        .unwrap();

        assert!(!outcome.created);
        assert_eq!(outcome.check.state, ReleaseCheckState::Released);
        assert_eq!(create_calls, 0);
    }

    #[test]
    fn ensure_reports_a_failed_pr_create_instead_of_claiming_success() {
        let repo = PathBuf::from("/repo");
        let error = ensure_release_pr_with(
            &repo,
            &ReleaseCheckOptions::default(),
            |_, _, _| Ok(subjects(&["chore(release): v9.91.0"])),
            |_, _| Ok(HashSet::new()),
            |_, args| {
                if args.get(1) == Some(&"create") {
                    return Ok(GhCliOutput {
                        success: false,
                        stdout: String::new(),
                        stderr: "no commits between main and develop".to_string(),
                    });
                }
                ok_gh("[]")
            },
            |_| None,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("no commits between"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn body_falls_back_to_a_heading_when_the_changelog_is_unreadable() {
        let body = release_pr_body(None, "v9.91.0");
        assert!(body.contains("## v9.91.0"));
        assert!(body.contains("Issue #3516"));
    }
}
