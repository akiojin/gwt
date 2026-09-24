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
//! Release observations are fetched from GitHub on every check. Local branch
//! refs and tags can lag behind remote releases and are never a fallback.

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
    /// Highest semantic version tag observed on the remote.
    pub version: Option<String>,
    /// Newest bump that has not been tagged yet.
    pub pending_version: Option<String>,
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
    let bump = newest_bump_version(&input.recent_subjects);
    let version = input
        .existing_tags
        .iter()
        .filter_map(|tag| {
            semver::Version::parse(tag.trim_start_matches('v'))
                .ok()
                .map(|v| (v, tag))
        })
        .max()
        .map(|(_, tag)| tag.clone());
    let pending_version = bump.clone().filter(|bump| {
        !input
            .existing_tags
            .iter()
            .any(|tag| tag.trim_start_matches('v') == bump.trim_start_matches('v'))
    });
    let state = match bump.as_deref() {
        None => ReleaseCheckState::NoBump,
        Some(_) if pending_version.is_none() => ReleaseCheckState::Released,
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
        pending_version,
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

/// Fetch every tag on every call, without a local-ref or cached fallback.
fn fetch_remote_tags(repo_path: &Path) -> Result<HashSet<String>> {
    fetch_remote_tags_with(repo_path, run_gh_command)
}

fn fetch_remote_tags_with<G>(repo_path: &Path, mut run_gh: G) -> Result<HashSet<String>>
where
    G: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    #[derive(Deserialize)]
    struct Tag {
        name: String,
    }
    let output = run_gh(
        repo_path,
        &[
            "api",
            "repos/{owner}/{repo}/tags?per_page=100",
            "--paginate",
            "--slurp",
        ],
    )?;
    if !output.success {
        return Err(GwtError::Git(format!(
            "release remote tags unavailable: {}",
            output.stderr.trim()
        )));
    }
    let pages: Vec<Vec<Tag>> = serde_json::from_str(&output.stdout)
        .map_err(|error| GwtError::Git(format!("release remote tags JSON: {error}")))?;
    Ok(pages.into_iter().flatten().map(|tag| tag.name).collect())
}

fn fetch_remote_subjects(repo_path: &Path, branch: &str, count: usize) -> Result<Vec<String>> {
    fetch_remote_subjects_with(repo_path, branch, count, run_gh_command)
}

/// Read the requested number of non-merge subjects from the remote branch.
/// Pagination preserves scan_commits without trusting a stale local branch.
fn fetch_remote_subjects_with<G>(
    repo_path: &Path,
    branch: &str,
    count: usize,
    mut run_gh: G,
) -> Result<Vec<String>>
where
    G: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    #[derive(Deserialize)]
    struct Commit {
        commit: Message,
        parents: Vec<serde_json::Value>,
    }
    #[derive(Deserialize)]
    struct Message {
        message: String,
    }
    let mut subjects = Vec::new();
    let mut page = 1;
    let per_page = count.min(100);
    while subjects.len() < count {
        let output = run_gh(
            repo_path,
            &[
                "api",
                "repos/{owner}/{repo}/commits",
                "--method",
                "GET",
                "-f",
                &format!("sha={branch}"),
                "-f",
                &format!("per_page={per_page}"),
                "-f",
                &format!("page={page}"),
            ],
        )?;
        if !output.success {
            return Err(GwtError::Git(format!(
                "release remote branch unavailable: {}",
                output.stderr.trim()
            )));
        }
        let commits: Vec<Commit> = serde_json::from_str(&output.stdout)
            .map_err(|error| GwtError::Git(format!("release remote commits JSON: {error}")))?;
        let last_page = commits.len() < per_page;
        subjects.extend(
            commits
                .into_iter()
                .filter(|commit| commit.parents.len() < 2)
                .filter_map(|commit| commit.commit.message.lines().next().map(str::to_string)),
        );
        if last_page {
            break;
        }
        page += 1;
    }
    subjects.truncate(count);
    Ok(subjects)
}

/// Run the standing check against fresh remote observations.
pub fn fetch_release_check(
    repo_path: &Path,
    options: &ReleaseCheckOptions,
) -> Result<ReleaseCheck> {
    fetch_release_check_with(
        repo_path,
        options,
        fetch_remote_subjects,
        fetch_remote_tags,
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
    T: FnMut(&Path) -> Result<HashSet<String>>,
    G: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let recent_subjects = subjects(repo_path, &options.release_branch, options.scan_commits)?;
    let mut input = ReleaseCheckInput {
        release_branch: options.release_branch.clone(),
        base_branch: options.base_branch.clone(),
        recent_subjects,
        existing_tags: tags(repo_path)?,
        open_release_pr: None,
    };

    let check = classify_release_check(&input);
    if check.pending_version.is_none() {
        return Ok(check);
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

/// Build stamp compiled into the running binary.
///
/// The values are produced by the binary crate's build script, so this crate
/// only ever receives them as data (a build script's `rustc-env` reaches its
/// own crate and nothing else).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeBuildStamp {
    /// Commit the running binary was built from, when git was available.
    pub commit: Option<String>,
    /// RFC 3339 timestamp of the build, when it could be resolved.
    pub time: Option<String>,
}

impl RuntimeBuildStamp {
    /// True when nothing about the build is known.
    pub fn is_unknown(&self) -> bool {
        self.commit.is_none()
    }
}

/// Observations the runtime-generation classification is derived from.
#[derive(Debug, Clone)]
pub struct RuntimeGenerationInput {
    /// Branch the running binary is compared against.
    pub branch: String,
    /// What the running binary was built from.
    pub build: RuntimeBuildStamp,
    /// Head commit of `branch`, when it could be read.
    pub default_branch_head: Option<String>,
    /// Commits on `branch` the build does not contain, when countable.
    pub behind_commits: Option<u64>,
}

/// How the running binary's generation compares to the branch it came from.
///
/// Every field is optional on purpose: this exists because a PM reported a
/// merged fix as running when it was not, so an unknown comparison must read
/// as unknown rather than as "up to date" (SPEC #4249 FR-002 / AC-3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeGeneration {
    /// Branch the running binary was compared against.
    pub branch: String,
    /// Commit the running binary was built from.
    pub build_commit: Option<String>,
    /// RFC 3339 timestamp of the build.
    pub build_time: Option<String>,
    /// Head commit of [`Self::branch`].
    pub default_branch_head: Option<String>,
    /// Commits on the branch the running binary does not contain.
    pub behind_commits: Option<u64>,
    /// `true` behind, `false` current, `None` when the comparison is unknown.
    pub stale_runtime: Option<bool>,
    /// The one step the owner takes, present only when the runtime is stale.
    pub owner_action: Option<String>,
}

impl RuntimeGeneration {
    /// A generation nothing is known about, for callers that cannot read one.
    pub fn unknown(branch: &str) -> Self {
        Self {
            branch: branch.to_string(),
            build_commit: None,
            build_time: None,
            default_branch_head: None,
            behind_commits: None,
            stale_runtime: None,
            owner_action: None,
        }
    }
}

/// Classify the runtime generation from already-collected observations.
///
/// The only way to reach `stale_runtime: true` is to know both ends of the
/// comparison *and* to have counted the gap. Two different commit ids alone do
/// not prove the binary is behind — a build off a side branch is not stale —
/// so an uncountable gap stays unknown.
pub fn classify_runtime_generation(input: RuntimeGenerationInput) -> RuntimeGeneration {
    let RuntimeGenerationInput {
        branch,
        build,
        default_branch_head,
        behind_commits,
    } = input;
    let current = match (build.commit.as_deref(), default_branch_head.as_deref()) {
        (Some(built), Some(head)) => Some(built == head),
        _ => None,
    };
    // Identical commits are zero apart even when no count was taken.
    let behind_commits = if current == Some(true) {
        Some(0)
    } else {
        behind_commits
    };
    let stale_runtime = match current {
        None => None,
        Some(true) => Some(false),
        Some(false) => behind_commits.map(|count| count > 0),
    };
    let owner_action = (stale_runtime == Some(true))
        .then(|| stale_runtime_owner_action(&branch, default_branch_head.as_deref()));
    RuntimeGeneration {
        branch,
        build_commit: build.commit,
        build_time: build.time,
        default_branch_head,
        behind_commits,
        stale_runtime,
        owner_action,
    }
}

/// The single line a stale runtime hands the owner.
fn stale_runtime_owner_action(branch: &str, head: Option<&str>) -> String {
    let head = head.unwrap_or(branch);
    format!(
        "restart GWT.app on a build that contains {head} — apply the pending update in the GUI, \
         or reinstall from a fresh {branch} build"
    )
}

/// Read how the running binary's generation compares to `branch`.
///
/// Never fails: the PM runs `release.status` every cycle, so a GitHub outage
/// or a commit missing from this clone degrades to unknown instead of turning
/// the whole operation into an error.
pub fn fetch_runtime_generation(
    repo_path: &Path,
    branch: &str,
    build: RuntimeBuildStamp,
) -> RuntimeGeneration {
    fetch_runtime_generation_with(
        repo_path,
        branch,
        build,
        run_gh_command,
        count_commits_behind,
    )
}

/// [`fetch_runtime_generation`] with the `gh` and git reads injected.
fn fetch_runtime_generation_with<G, C>(
    repo_path: &Path,
    branch: &str,
    build: RuntimeBuildStamp,
    mut run_gh: G,
    mut count_behind: C,
) -> RuntimeGeneration
where
    G: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
    C: FnMut(&Path, &str, &str) -> Result<u64>,
{
    // No build stamp means nothing to compare, so spend no budget at all.
    if build.is_unknown() {
        return RuntimeGeneration::unknown(branch);
    }
    let default_branch_head = fetch_branch_head(repo_path, branch, &mut run_gh);
    let behind_commits = match (build.commit.as_deref(), default_branch_head.as_deref()) {
        (Some(built), Some(head)) if built != head => count_behind(repo_path, built, head).ok(),
        _ => None,
    };
    classify_runtime_generation(RuntimeGenerationInput {
        branch: branch.to_string(),
        build,
        default_branch_head,
        behind_commits,
    })
}

/// Head commit of `branch` as GitHub has it, or `None` when it cannot be read.
///
/// `git/ref/heads/...` is the smallest response that carries a branch head, so
/// the every-cycle read stays cheap.
fn fetch_branch_head<G>(repo_path: &Path, branch: &str, run_gh: &mut G) -> Option<String>
where
    G: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let endpoint = format!("repos/{{owner}}/{{repo}}/git/ref/heads/{branch}");
    let output = run_gh(
        repo_path,
        &["api", endpoint.as_str(), "--jq", ".object.sha"],
    )
    .ok()?;
    if !output.success {
        return None;
    }
    let sha = output.stdout.trim();
    let is_sha = sha.len() >= 7 && sha.bytes().all(|byte| byte.is_ascii_hexdigit());
    is_sha.then(|| sha.to_string())
}

/// Commits reachable from `head` but not from `base`.
fn count_commits_behind(repo_path: &Path, base: &str, head: &str) -> Result<u64> {
    let range = format!("{base}..{head}");
    let output = gwt_core::process::run_git_logged(
        &["rev-list", "--count", range.as_str()],
        Some(repo_path),
    )
    .map_err(|error| GwtError::Git(format!("rev-list --count {range}: {error}")))?;
    if !output.status.success() {
        return Err(GwtError::Git(format!(
            "rev-list --count {range}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .map_err(|error| GwtError::Git(format!("rev-list --count {range}: {error}")))
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
        fetch_remote_subjects,
        fetch_remote_tags,
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
    T: FnMut(&Path) -> Result<HashSet<String>>,
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
        .pending_version
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
///
/// The body is reference-only (Issue #3545): `main` is the default branch,
/// so a closing keyword that survives here would close its Issue on merge
/// regardless of open acceptance criteria.
fn release_pr_body(changelog: Option<&str>, version: &str) -> String {
    let notes = changelog
        .and_then(|text| changelog_section(text, version))
        .unwrap_or_else(|| format!("## {version}"));
    neutralize_closing_keywords(&format!(
        "{notes}\n\nRecovered by the gwt interrupted-release check (Issue #3516).\n"
    ))
}

/// GitHub closing keywords. One of these followed by an optional colon,
/// whitespace, and an Issue reference links the PR as closing that Issue.
const CLOSING_KEYWORDS: [&str; 9] = [
    "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
];

/// Wrap every Issue reference that follows a closing keyword in a code span
/// so GitHub cannot read the pair as a closing link (Issue #3545). Plain
/// references stay untouched and the rewrite is idempotent.
fn neutralize_closing_keywords(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut rest = text;
    while let Some((ref_start, ref_end)) = next_closing_reference(rest) {
        out.push_str(&rest[..ref_start]);
        out.push('`');
        out.push_str(&rest[ref_start..ref_end]);
        out.push('`');
        rest = &rest[ref_end..];
    }
    out.push_str(rest);
    out
}

/// Byte range of the first Issue reference that follows a closing keyword.
fn next_closing_reference(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    for (index, _) in text.char_indices() {
        let word_start = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        if !word_start {
            continue;
        }
        let Some(keyword_end) = closing_keyword_end(text, index) else {
            continue;
        };
        let mut cursor = keyword_end;
        if bytes.get(cursor) == Some(&b':') {
            cursor += 1;
        }
        let after_sep = text[cursor..].trim_start();
        let sep_len = text.len() - cursor - after_sep.len();
        if sep_len == 0 {
            continue;
        }
        let ref_start = cursor + sep_len;
        let ref_end = ref_start + issue_reference_len(after_sep);
        if ref_end > ref_start {
            return Some((ref_start, ref_end));
        }
    }
    None
}

/// End of the closing keyword that starts at `start`, when one does.
fn closing_keyword_end(text: &str, start: usize) -> Option<usize> {
    let rest = &text[start..];
    CLOSING_KEYWORDS
        .iter()
        .filter(|keyword| {
            // Compare bytes: slicing `rest` at a byte offset would panic when a
            // multi-byte character follows the keyword (`fix対応`).
            rest.len() >= keyword.len()
                && rest.as_bytes()[..keyword.len()].eq_ignore_ascii_case(keyword.as_bytes())
        })
        .map(|keyword| start + keyword.len())
        .find(|&end| {
            !text
                .as_bytes()
                .get(end)
                .is_some_and(u8::is_ascii_alphanumeric)
        })
}

/// Length of the `#N`, `owner/repo#N`, or Issue-URL reference at the start of
/// `text`, or 0 when there is none.
fn issue_reference_len(text: &str) -> usize {
    let token_len = text
        .find(|c: char| {
            !(c.is_ascii_alphanumeric() || matches!(c, '#' | '/' | '.' | '-' | '_' | ':'))
        })
        .unwrap_or(text.len());
    let token = &text[..token_len];
    let is_reference = match token.split_once('#') {
        Some((prefix, digits))
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) =>
        {
            prefix.is_empty() || prefix.matches('/').count() == 1
        }
        _ => {
            token.starts_with("https://github.com/")
                && token.rsplit_once("/issues/").is_some_and(|(_, digits)| {
                    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
                })
        }
    };
    if is_reference {
        token_len
    } else {
        0
    }
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
        assert_eq!(check.pending_version.as_deref(), Some("v9.91.0"));
        assert_eq!(check.version.as_deref(), Some("v9.90.0"));
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
    fn version_follows_tags_even_when_the_bump_history_is_older() {
        let mut observation = input(
            &["chore(release): v9.79.0"],
            &["v9.79.0", "v9.99.0", "v9.101.1"],
            None,
        );
        let check = classify_release_check(&observation);
        assert_eq!(check.version.as_deref(), Some("v9.101.1"));
        assert_eq!(check.state, ReleaseCheckState::Released);
        observation.existing_tags.insert("v9.102.0".into());
        assert_eq!(
            classify_release_check(&observation).version.as_deref(),
            Some("v9.102.0")
        );
        observation.recent_subjects = subjects(&["chore(release): v9.103.0"]);
        assert_eq!(
            classify_release_check(&observation)
                .pending_version
                .as_deref(),
            Some("v9.103.0")
        );
        observation.existing_tags.insert("v9.103.0".into());
        let released = classify_release_check(&observation);
        assert_eq!(released.version.as_deref(), Some("v9.103.0"));
        assert_eq!(released.pending_version, None);
        assert_eq!(released.state, ReleaseCheckState::Released);
    }

    #[test]
    fn latest_tag_is_reported_without_a_bump_in_the_scan_window() {
        let check = classify_release_check(&input(&["fix: tidy"], &["v9.102.0"], None));
        assert_eq!(check.version.as_deref(), Some("v9.102.0"));
    }

    #[test]
    fn remote_tags_are_read_without_cache_and_fail_explicitly() {
        let tags = fetch_remote_tags_with(Path::new("/repo"), |_, args| {
            assert!(args.contains(&"--paginate"));
            assert!(!args.contains(&"--cache"));
            ok_gh("[[{\"name\":\"v9.79.0\"}],[{\"name\":\"v9.102.0\"}]]")
        })
        .unwrap();
        assert!(tags.contains("v9.102.0"));
        let error = fetch_remote_tags_with(Path::new("/repo"), |_, _| {
            Ok(GhCliOutput {
                success: false,
                stdout: String::new(),
                stderr: "offline".into(),
            })
        })
        .unwrap_err();
        assert!(error.to_string().contains("offline"));
    }

    #[test]
    fn remote_bump_history_uses_the_requested_branch() {
        let subjects = fetch_remote_subjects_with(Path::new("/repo"), "develop", 30, |_, args| {
            assert!(args.contains(&"sha=develop"));
            ok_gh(r#"[{"commit":{"message":"chore(release): v9.103.0\n\nnotes"},"parents":[{}]}]"#)
        })
        .unwrap();
        assert_eq!(newest_bump_version(&subjects).as_deref(), Some("v9.103.0"));
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
            |_| Ok(HashSet::new()),
            |_, _| ok_gh("[]"),
        )
        .unwrap();
        assert_eq!(check.state, ReleaseCheckState::Stalled);
        assert_eq!(check.pending_version.as_deref(), Some("v9.91.0"));
    }

    // Even without a bump, refresh tags; no open-PR lookup is needed.
    #[test]
    fn fetch_refreshes_tags_but_skips_pr_lookup_when_no_bump_is_present() {
        let repo = PathBuf::from("/repo");
        let mut tag_reads = 0_u32;
        let mut gh_calls = 0_u32;
        let check = fetch_release_check_with(
            &repo,
            &ReleaseCheckOptions::default(),
            |_, _, _| Ok(subjects(&["fix(gui): tidy"])),
            |_| {
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
        assert_eq!(tag_reads, 1);
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
            |_| Ok(HashSet::from(["v9.91.0".to_string()])),
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
            |_| Ok(HashSet::new()),
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
            |_| Ok(HashSet::new()),
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
            |_| Ok(HashSet::from(["v9.91.0".to_string()])),
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
            |_| Ok(HashSet::new()),
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

    #[test]
    fn recovered_body_neutralizes_closing_keywords_from_the_changelog() {
        // Issue #3545: a Release PR body must never carry a closing keyword
        // followed by an Issue reference, whatever a commit subject said.
        let changelog = "## [9.92.0] - 2026-09-07\n\n### Bug Fixes\n\n\
            - **launch:** Skip legacy resume, fixes #3527\n\
            - Closes: #3528 and RESOLVES akiojin/gwt#3529\n\
            - closed https://github.com/akiojin/gwt/issues/3530\n\
            - Plain reference #3531 and hotfix #3532 stay\n\
            - 修正 fixed #3533 対応、fix対応 #3534\n";
        let body = release_pr_body(Some(changelog), "v9.92.0");
        assert!(body.contains("fixes `#3527`"), "body was: {body}");
        assert!(body.contains("Closes: `#3528`"), "body was: {body}");
        assert!(
            body.contains("RESOLVES `akiojin/gwt#3529`"),
            "body was: {body}"
        );
        assert!(
            body.contains("closed `https://github.com/akiojin/gwt/issues/3530`"),
            "body was: {body}"
        );
        assert!(
            body.contains("reference #3531 and hotfix #3532 stay"),
            "body was: {body}"
        );
        assert!(
            body.contains("修正 fixed `#3533` 対応、fix対応 #3534"),
            "body was: {body}"
        );
        assert_eq!(
            neutralize_closing_keywords(&body),
            body,
            "rewrite is idempotent"
        );
    }

    // --- Runtime generation (SPEC #4249 FR-002 / AC-3) -------------------

    /// AC-3's measured case: the app binary was built on 2026-09-10 from a
    /// commit that predates the `develop` head, so the trust fix it is
    /// reported to carry (`5b10bca75`) is not actually running.
    const MEASURED_BUILD_COMMIT: &str = "5b10bca75c4f2a6d9e8b1c3f0a7d4e2b6c8f1a39";
    const MEASURED_DEVELOP_HEAD: &str = "85a216ee6b1d4c7f9a2e8b5c0d3f6a1e4b7c9d02";

    fn generation_input(
        build_commit: Option<&str>,
        head: Option<&str>,
        behind: Option<u64>,
    ) -> RuntimeGenerationInput {
        RuntimeGenerationInput {
            branch: DEFAULT_RELEASE_BRANCH.to_string(),
            build: RuntimeBuildStamp {
                commit: build_commit.map(str::to_string),
                time: Some("2026-09-10T01:29:00+00:00".to_string()),
            },
            default_branch_head: head.map(str::to_string),
            behind_commits: behind,
        }
    }

    #[test]
    fn stale_runtime_is_true_when_the_running_build_is_behind_the_branch_head() {
        let generation = classify_runtime_generation(generation_input(
            Some(MEASURED_BUILD_COMMIT),
            Some(MEASURED_DEVELOP_HEAD),
            Some(37),
        ));
        assert_eq!(generation.stale_runtime, Some(true));
        assert_eq!(generation.behind_commits, Some(37));
        assert_eq!(
            generation.build_commit.as_deref(),
            Some(MEASURED_BUILD_COMMIT)
        );
        assert_eq!(
            generation.build_time.as_deref(),
            Some("2026-09-10T01:29:00+00:00")
        );
        let action = generation
            .owner_action
            .expect("a stale runtime asks for an owner action");
        assert!(action.contains(MEASURED_DEVELOP_HEAD), "{action}");
        assert!(action.contains("GWT.app"), "{action}");
    }

    #[test]
    fn stale_runtime_is_false_when_the_running_build_is_the_branch_head() {
        let generation = classify_runtime_generation(generation_input(
            Some(MEASURED_DEVELOP_HEAD),
            Some(MEASURED_DEVELOP_HEAD),
            None,
        ));
        assert_eq!(generation.stale_runtime, Some(false));
        // Identical commits are zero apart even when the count was unavailable.
        assert_eq!(generation.behind_commits, Some(0));
        assert_eq!(generation.owner_action, None);
    }

    #[test]
    fn stale_runtime_is_null_when_either_end_of_the_comparison_is_unknown() {
        // No build stamp: a binary built outside a git checkout.
        let no_stamp =
            classify_runtime_generation(generation_input(None, Some(MEASURED_DEVELOP_HEAD), None));
        assert_eq!(no_stamp.stale_runtime, None);
        assert_eq!(no_stamp.owner_action, None);

        // No branch head: the GitHub read failed. Unknown is never false.
        let no_head =
            classify_runtime_generation(generation_input(Some(MEASURED_BUILD_COMMIT), None, None));
        assert_eq!(no_head.stale_runtime, None);
        assert_eq!(no_head.owner_action, None);

        // Both ends known and different, but the commit count is unavailable
        // (the build commit is not in this clone): still unknown, not stale.
        let uncounted = classify_runtime_generation(generation_input(
            Some(MEASURED_BUILD_COMMIT),
            Some(MEASURED_DEVELOP_HEAD),
            None,
        ));
        assert_eq!(uncounted.stale_runtime, None);
        assert_eq!(uncounted.behind_commits, None);
        assert_eq!(uncounted.owner_action, None);
    }

    #[test]
    fn a_build_that_already_contains_the_head_is_not_stale() {
        let generation = classify_runtime_generation(generation_input(
            Some(MEASURED_BUILD_COMMIT),
            Some(MEASURED_DEVELOP_HEAD),
            Some(0),
        ));
        assert_eq!(generation.stale_runtime, Some(false));
        assert_eq!(generation.owner_action, None);
    }

    #[test]
    fn the_runtime_generation_read_resolves_the_branch_head_through_gh() {
        let mut seen: Vec<String> = Vec::new();
        let generation = fetch_runtime_generation_with(
            &PathBuf::from("/repo"),
            DEFAULT_RELEASE_BRANCH,
            RuntimeBuildStamp {
                commit: Some(MEASURED_BUILD_COMMIT.to_string()),
                time: None,
            },
            |_path, args| {
                seen = args.iter().map(|arg| (*arg).to_string()).collect();
                ok_gh(&format!("{MEASURED_DEVELOP_HEAD}\n"))
            },
            |_path, base, head| {
                assert_eq!(base, MEASURED_BUILD_COMMIT);
                assert_eq!(head, MEASURED_DEVELOP_HEAD);
                Ok(37)
            },
        );
        assert!(
            seen.iter().any(|arg| arg.contains("git/ref/heads/develop")),
            "expected a git ref read, got {seen:?}"
        );
        assert_eq!(
            generation.default_branch_head.as_deref(),
            Some(MEASURED_DEVELOP_HEAD)
        );
        assert_eq!(generation.behind_commits, Some(37));
        assert_eq!(generation.stale_runtime, Some(true));
    }

    #[test]
    fn the_runtime_generation_read_degrades_to_nulls_instead_of_failing() {
        // `release.status` is the PM's every-cycle read: a GitHub outage must
        // not turn the whole operation into an error.
        let generation = fetch_runtime_generation_with(
            &PathBuf::from("/repo"),
            DEFAULT_RELEASE_BRANCH,
            RuntimeBuildStamp {
                commit: Some(MEASURED_BUILD_COMMIT.to_string()),
                time: None,
            },
            |_path, _args| {
                Ok(GhCliOutput {
                    success: false,
                    stdout: String::new(),
                    stderr: "gh: API rate limit exceeded".to_string(),
                })
            },
            |_path, _base, _head| unreachable!("no head means nothing to count"),
        );
        assert_eq!(generation.default_branch_head, None);
        assert_eq!(generation.behind_commits, None);
        assert_eq!(generation.stale_runtime, None);
    }

    #[test]
    fn an_uncountable_range_leaves_the_behind_count_unknown() {
        let generation = fetch_runtime_generation_with(
            &PathBuf::from("/repo"),
            DEFAULT_RELEASE_BRANCH,
            RuntimeBuildStamp {
                commit: Some(MEASURED_BUILD_COMMIT.to_string()),
                time: None,
            },
            |_path, _args| ok_gh(MEASURED_DEVELOP_HEAD),
            |_path, _base, _head| Err(GwtError::Git("bad revision".to_string())),
        );
        assert_eq!(
            generation.default_branch_head.as_deref(),
            Some(MEASURED_DEVELOP_HEAD)
        );
        assert_eq!(generation.behind_commits, None);
        assert_eq!(generation.stale_runtime, None);
    }

    #[test]
    fn an_absent_build_stamp_skips_every_read() {
        let generation = fetch_runtime_generation_with(
            &PathBuf::from("/repo"),
            DEFAULT_RELEASE_BRANCH,
            RuntimeBuildStamp::default(),
            |_path, _args| unreachable!("no build stamp means nothing to compare against"),
            |_path, _base, _head| unreachable!("no build stamp means nothing to count"),
        );
        assert_eq!(generation.build_commit, None);
        assert_eq!(generation.default_branch_head, None);
        assert_eq!(generation.stale_runtime, None);
    }
}
