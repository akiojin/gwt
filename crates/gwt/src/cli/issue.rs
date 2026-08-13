use std::{
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use gwt_github::{
    cache::{write_atomic, CacheGeneration, ValidatedCacheEntry},
    client::ApiError,
    Cache, IssueClient, IssueNumber, IssueSnapshot, IssueState, SpecOpsError,
};

use crate::cli::{CliEnv, CliParseError, IssueCommand, LinkedPrSummary};

fn io_as_api_error(err: io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

pub(super) fn parse(args: &[String]) -> Result<IssueCommand, CliParseError> {
    let mut it = args.iter().peekable();
    match it.next().map(String::as_str) {
        Some("spec") => super::issue_spec::parse(it.collect::<Vec<_>>().as_slice()),
        Some("view") => parse_issue_read_args(it.collect::<Vec<_>>().as_slice(), "view"),
        Some("comments") => parse_issue_read_args(it.collect::<Vec<_>>().as_slice(), "comments"),
        Some("linked-prs") => {
            parse_issue_read_args(it.collect::<Vec<_>>().as_slice(), "linked-prs")
        }
        Some("create") => parse_issue_create_args(it.collect::<Vec<_>>().as_slice()),
        Some("comment") => parse_issue_comment_args(it.collect::<Vec<_>>().as_slice()),
        Some(other) => Err(CliParseError::UnknownSubcommand(other.to_string())),
        None => Err(CliParseError::Usage),
    }
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    cmd: IssueCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if matches!(
        cmd,
        IssueCommand::SpecReadAll { .. }
            | IssueCommand::SpecReadSection { .. }
            | IssueCommand::SpecEditSection { .. }
            | IssueCommand::SpecEditSectionBody { .. }
            | IssueCommand::SpecEditSectionJson { .. }
            | IssueCommand::SpecEditSectionJsonBody { .. }
            | IssueCommand::SpecList { .. }
            | IssueCommand::SpecCreate { .. }
            | IssueCommand::SpecCreateBody { .. }
            | IssueCommand::SpecCreateJson { .. }
            | IssueCommand::SpecCreateJsonBody { .. }
            | IssueCommand::SpecCreateHelp
            | IssueCommand::SpecPull { .. }
            | IssueCommand::SpecRepair { .. }
            | IssueCommand::SpecRename { .. }
    ) {
        return super::issue_spec::run(env, cmd, out);
    }

    let code = match cmd {
        IssueCommand::View { number, refresh } => {
            let entry = load_or_refresh_issue(env, IssueNumber(number), refresh)?;
            render_issue(out, &entry.snapshot);
            0
        }
        IssueCommand::Comments { number, refresh } => {
            let entry = load_or_refresh_issue(env, IssueNumber(number), refresh)?;
            render_issue_comments(out, &entry.snapshot);
            0
        }
        IssueCommand::LinkedPrs { number, refresh } => {
            let linked_prs = load_or_refresh_linked_prs(env, IssueNumber(number), refresh)?;
            render_linked_prs(out, &linked_prs);
            0
        }
        IssueCommand::Create {
            title,
            file,
            labels,
        } => {
            let body = env.read_file(&file).map_err(super::io_as_api_error)?;
            let snapshot = env.client().create_issue(&title, &body, &labels)?;
            super::intake_outcome::auto_record_issue_operation(
                env.repo_path(),
                "issue.create",
                super::intake_outcome::IntakeOutcomeKind::IssueCreated,
                snapshot.number.0,
            );
            Cache::new(env.cache_root()).write_snapshot(&snapshot)?;
            out.push_str(&format!(
                "created issue #{} with labels {:?}\n",
                snapshot.number.0, snapshot.labels
            ));
            0
        }
        IssueCommand::CreateBody {
            title,
            body,
            labels,
        } => {
            let snapshot = env.client().create_issue(&title, &body, &labels)?;
            super::intake_outcome::auto_record_issue_operation(
                env.repo_path(),
                "issue.create",
                super::intake_outcome::IntakeOutcomeKind::IssueCreated,
                snapshot.number.0,
            );
            Cache::new(env.cache_root()).write_snapshot(&snapshot)?;
            out.push_str(&format!(
                "created issue #{} with labels {:?}\n",
                snapshot.number.0, snapshot.labels
            ));
            0
        }
        IssueCommand::Comment { number, file } => {
            let body = env.read_file(&file).map_err(super::io_as_api_error)?;
            let comment = env.client().create_comment(IssueNumber(number), &body)?;
            super::intake_outcome::auto_record_issue_operation(
                env.repo_path(),
                "issue.comment",
                super::intake_outcome::IntakeOutcomeKind::IssueUpdated,
                number,
            );
            let _ = refresh_issue_cache(env, IssueNumber(number))?;
            out.push_str(&format!(
                "created comment {} on #{}\n",
                comment.id.0, number
            ));
            0
        }
        IssueCommand::CommentBody { number, body } => {
            let comment = env.client().create_comment(IssueNumber(number), &body)?;
            super::intake_outcome::auto_record_issue_operation(
                env.repo_path(),
                "issue.comment",
                super::intake_outcome::IntakeOutcomeKind::IssueUpdated,
                number,
            );
            let _ = refresh_issue_cache(env, IssueNumber(number))?;
            out.push_str(&format!(
                "created comment {} on #{}\n",
                comment.id.0, number
            ));
            0
        }
        IssueCommand::MonitorReviewVerdict {
            issue_number,
            reviewed_sha,
            verdict_raw,
        } => run_monitor_review_verdict(env, issue_number, &reviewed_sha, &verdict_raw, out),
        _ => unreachable!("issue::run called with non-issue command"),
    };
    Ok(code)
}

/// SPEC #3200 Option A: publish an independent-review verdict to the Issue
/// Monitor daemon's control channel. The daemon re-judges the raw verdict
/// (SHA-bound) — this only transports it.
#[cfg(unix)]
fn run_monitor_review_verdict<E: CliEnv>(
    env: &mut E,
    issue_number: u64,
    reviewed_sha: &str,
    verdict_raw: &str,
    out: &mut String,
) -> i32 {
    let payload = crate::runtime_daemon_events::issue_monitor_payload(
        "control",
        serde_json::json!({
            "review_verdict": {
                "issue_number": issue_number,
                "reviewed_sha": reviewed_sha,
                "verdict_raw": verdict_raw,
            }
        }),
        std::process::id(),
    );
    match crate::daemon_publisher::publish_event(
        env.repo_path(),
        crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL,
        payload,
    ) {
        Ok(()) => {
            out.push_str(&format!(
                "review verdict published for #{issue_number} at {reviewed_sha}\n"
            ));
            0
        }
        Err(error) => {
            out.push_str(&format!(
                "review verdict publish failed for #{issue_number}: {error}\n"
            ));
            1
        }
    }
}

#[cfg(not(unix))]
fn run_monitor_review_verdict<E: CliEnv>(
    _env: &mut E,
    issue_number: u64,
    _reviewed_sha: &str,
    _verdict_raw: &str,
    out: &mut String,
) -> i32 {
    out.push_str(&format!(
        "review verdict publish unavailable on this platform (#{issue_number})\n"
    ));
    1
}

fn parse_issue_read_args(args: &[&String], mode: &str) -> Result<IssueCommand, CliParseError> {
    let Some(number_arg) = args.first() else {
        return Err(CliParseError::Usage);
    };
    let number = number_arg
        .parse()
        .map_err(|_| CliParseError::InvalidNumber((*number_arg).clone()))?;
    let mut refresh = false;
    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "--refresh" => refresh = true,
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
    }
    Ok(match mode {
        "view" => IssueCommand::View { number, refresh },
        "comments" => IssueCommand::Comments { number, refresh },
        "linked-prs" => IssueCommand::LinkedPrs { number, refresh },
        _ => return Err(CliParseError::Usage),
    })
}

fn parse_issue_create_args(args: &[&String]) -> Result<IssueCommand, CliParseError> {
    let mut title: Option<String> = None;
    let mut file: Option<String> = None;
    let mut labels: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--title" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--title"));
                }
                title = Some(args[i].clone());
            }
            "-f" | "--file" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("-f"));
                }
                file = Some(args[i].clone());
            }
            "--label" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--label"));
                }
                labels.push(args[i].clone());
            }
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 1;
    }
    Ok(IssueCommand::Create {
        title: title.ok_or(CliParseError::MissingFlag("--title"))?,
        file: file.ok_or(CliParseError::MissingFlag("-f"))?,
        labels,
    })
}

fn parse_issue_comment_args(args: &[&String]) -> Result<IssueCommand, CliParseError> {
    if args.len() != 3 {
        return Err(CliParseError::Usage);
    }
    let number = args[0]
        .parse()
        .map_err(|_| CliParseError::InvalidNumber(args[0].clone()))?;
    match args[1].as_str() {
        "-f" | "--file" => Ok(IssueCommand::Comment {
            number,
            file: args[2].clone(),
        }),
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

pub(super) fn issue_state_label(state: IssueState) -> &'static str {
    match state {
        IssueState::Open => "OPEN",
        IssueState::Closed => "CLOSED",
    }
}

pub(super) fn render_issue(out: &mut String, snapshot: &IssueSnapshot) {
    out.push_str(&format!(
        "#{} [{}] {}\n",
        snapshot.number.0,
        issue_state_label(snapshot.state),
        snapshot.title
    ));
    if !snapshot.labels.is_empty() {
        out.push_str(&format!("labels: {}\n", snapshot.labels.join(", ")));
    }
    out.push_str(&format!("updated_at: {}\n\n", snapshot.updated_at.0));
    if !snapshot.body.is_empty() {
        out.push_str(snapshot.body.trim_end_matches('\n'));
        out.push('\n');
    }
}

pub(super) fn render_issue_comments(out: &mut String, snapshot: &IssueSnapshot) {
    if snapshot.comments.is_empty() {
        out.push_str("no comments\n");
        return;
    }
    for comment in &snapshot.comments {
        out.push_str(&format!(
            "=== comment:{} ({}) ===\n{}\n",
            comment.id.0, comment.updated_at.0, comment.body
        ));
    }
}

pub(super) fn render_linked_prs(out: &mut String, linked_prs: &[LinkedPrSummary]) {
    if linked_prs.is_empty() {
        out.push_str("no linked pull requests\n");
        return;
    }
    for pr in linked_prs {
        out.push_str(&format!(
            "#{} [{}] {}\n{}\n",
            pr.number, pr.state, pr.title, pr.url
        ));
    }
}

pub(super) fn load_or_refresh_issue<E: CliEnv>(
    env: &mut E,
    number: IssueNumber,
    refresh: bool,
) -> Result<gwt_github::CacheEntry, SpecOpsError> {
    load_or_refresh_issue_with_index_rebuild(env, number, refresh, |repo_path| {
        if crate::index_worker::detect_repo_hash(repo_path).is_none() {
            return Ok(());
        }
        crate::index_worker::default_rebuild_runner(
            repo_path,
            crate::index_worker::IndexRebuildScope::Issues,
            None,
        )
    })
}

fn cache_resource_is_fresh(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age < crate::issue_cache::ISSUE_CACHE_TTL)
}

pub(super) fn refresh_issue_cache<E: CliEnv>(
    env: &mut E,
    number: IssueNumber,
) -> Result<gwt_github::CacheEntry, SpecOpsError> {
    refresh_issue_cache_with_index_rebuild(env, number, |repo_path| {
        if crate::index_worker::detect_repo_hash(repo_path).is_none() {
            return Ok(());
        }
        crate::index_worker::default_rebuild_runner(
            repo_path,
            crate::index_worker::IndexRebuildScope::Issues,
            None,
        )
    })
}

pub(super) fn refresh_issue_cache_with_index_rebuild<E, F>(
    env: &mut E,
    number: IssueNumber,
    rebuild_issue_index: F,
) -> Result<gwt_github::CacheEntry, SpecOpsError>
where
    E: CliEnv,
    F: FnMut(&std::path::Path) -> Result<(), String>,
{
    let generation = Cache::new(env.cache_root()).current_generation(number)?;
    refresh_issue_cache_with_index_rebuild_since(
        env,
        number,
        None,
        generation.as_ref(),
        None,
        false,
        rebuild_issue_index,
    )
}

fn load_or_refresh_issue_with_index_rebuild<E, F>(
    env: &mut E,
    number: IssueNumber,
    refresh: bool,
    rebuild_issue_index: F,
) -> Result<gwt_github::CacheEntry, SpecOpsError>
where
    E: CliEnv,
    F: FnMut(&std::path::Path) -> Result<(), String>,
{
    if refresh {
        let generation = Cache::new(env.cache_root()).current_generation(number)?;
        return refresh_issue_cache_with_index_rebuild_since(
            env,
            number,
            None,
            generation.as_ref(),
            None,
            false,
            rebuild_issue_index,
        );
    }

    match Cache::new(env.cache_root())
        .load_validated_entry(number, crate::issue_cache::ISSUE_CACHE_TTL)?
    {
        ValidatedCacheEntry::Fresh(entry) => Ok(entry.entry),
        ValidatedCacheEntry::Stale(entry) => refresh_issue_cache_with_index_rebuild_since(
            env,
            number,
            Some(&entry.entry.snapshot.updated_at),
            entry.generation.as_ref(),
            Some(&entry.entry.snapshot),
            false,
            rebuild_issue_index,
        ),
        ValidatedCacheEntry::Unvalidated(entry) => refresh_issue_cache_with_index_rebuild_since(
            env,
            number,
            None,
            entry.generation.as_ref(),
            None,
            true,
            rebuild_issue_index,
        ),
        ValidatedCacheEntry::Missing { generation } => {
            refresh_issue_cache_with_index_rebuild_since(
                env,
                number,
                None,
                generation.as_ref(),
                None,
                true,
                rebuild_issue_index,
            )
        }
    }
}

fn refresh_issue_cache_with_index_rebuild_since<E, F>(
    env: &mut E,
    number: IssueNumber,
    since: Option<&gwt_github::UpdatedAt>,
    expected_generation: Option<&CacheGeneration>,
    not_modified_snapshot: Option<&IssueSnapshot>,
    force_rebuild: bool,
    mut rebuild_issue_index: F,
) -> Result<gwt_github::CacheEntry, SpecOpsError>
where
    E: CliEnv,
    F: FnMut(&std::path::Path) -> Result<(), String>,
{
    let cache_root = env.cache_root();
    let before = crate::issue_cache::issue_cache_source_fingerprint(&cache_root)
        .map_err(|err| SpecOpsError::from(ApiError::Network(err)))?;
    let snapshot = match env.client().fetch(number, since)? {
        gwt_github::FetchResult::Updated(snapshot) => snapshot,
        gwt_github::FetchResult::NotModified => {
            let cache = Cache::new(cache_root);
            let expected = not_modified_snapshot.ok_or_else(|| {
                SpecOpsError::from(ApiError::Network(format!(
                    "issue #{} returned NotModified without a validated cache snapshot",
                    number.0
                )))
            })?;
            if !cache.renew_validation_receipt_if_generation(expected, expected_generation)? {
                return Err(SpecOpsError::from(ApiError::Network(format!(
                    "issue #{} cache changed during validation",
                    number.0
                ))));
            }
            return load_fresh_validated_entry(&cache, number);
        }
    };
    let cache = Cache::new(cache_root.clone());
    let Some(committed_generation) =
        cache.write_snapshot_if_generation(&snapshot, expected_generation)?
    else {
        return Err(SpecOpsError::from(ApiError::Network(format!(
            "issue #{} cache changed while fetching remote snapshot",
            number.0
        ))));
    };
    let after = crate::issue_cache::issue_cache_source_fingerprint(&cache_root)
        .map_err(|err| SpecOpsError::from(ApiError::Network(err)))?;
    if force_rebuild || crate::issue_cache::issue_cache_source_changed(&before, &after) {
        rebuild_issue_index(env.repo_path()).map_err(|err| {
            SpecOpsError::from(ApiError::Network(format!("rebuild issue index: {err}")))
        })?;
    }
    if !cache.renew_validation_receipt_if_generation(&snapshot, Some(&committed_generation))? {
        return Err(SpecOpsError::from(ApiError::Network(format!(
            "issue #{} cache changed before validation receipt publication",
            number.0
        ))));
    }
    load_fresh_validated_entry(&cache, number)
}

fn load_fresh_validated_entry(
    cache: &Cache,
    number: IssueNumber,
) -> Result<gwt_github::CacheEntry, SpecOpsError> {
    match cache.load_validated_entry(number, crate::issue_cache::ISSUE_CACHE_TTL)? {
        ValidatedCacheEntry::Fresh(entry) => Ok(entry.entry),
        _ => Err(SpecOpsError::from(ApiError::Network(format!(
            "issue #{} cache validation receipt is unstable",
            number.0
        )))),
    }
}

pub(super) fn load_or_refresh_linked_prs<E: CliEnv>(
    env: &mut E,
    number: IssueNumber,
    refresh: bool,
) -> Result<Vec<LinkedPrSummary>, SpecOpsError> {
    let cache_root = env.cache_root();
    if !refresh {
        if let Ok(Some(cached)) = read_linked_prs_cache(&cache_root, number) {
            if cache_resource_is_fresh(&linked_prs_cache_path(&cache_root, number)) {
                return Ok(cached);
            }
        }
    }
    let linked_prs = env.fetch_linked_prs(number).map_err(io_as_api_error)?;
    write_linked_prs_cache(&cache_root, number, &linked_prs)?;
    Ok(linked_prs)
}

pub(super) fn linked_prs_cache_path(cache_root: &std::path::Path, number: IssueNumber) -> PathBuf {
    cache_root
        .join(number.0.to_string())
        .join("linked_prs.json")
}

pub(super) fn read_linked_prs_cache(
    cache_root: &std::path::Path,
    number: IssueNumber,
) -> Result<Option<Vec<LinkedPrSummary>>, SpecOpsError> {
    let path = linked_prs_cache_path(cache_root, number);
    match fs::read_to_string(&path) {
        Ok(text) => {
            let parsed = serde_json::from_str(&text)
                .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
            Ok(Some(parsed))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(io_as_api_error(err)),
    }
}

pub(super) fn write_linked_prs_cache(
    cache_root: &std::path::Path,
    number: IssueNumber,
    linked_prs: &[LinkedPrSummary],
) -> Result<(), SpecOpsError> {
    let bytes = serde_json::to_vec_pretty(linked_prs)
        .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
    write_atomic(&linked_prs_cache_path(cache_root, number), &bytes).map_err(io_as_api_error)
}

pub(crate) fn fetch_linked_prs_via_gh(
    owner: &str,
    repo: &str,
    number: IssueNumber,
) -> io::Result<Vec<LinkedPrSummary>> {
    let query = r#"
query($owner: String!, $repo: String!, $number: Int!) {
  repository(owner: $owner, name: $repo) {
    issue(number: $number) {
      timelineItems(first: 100, itemTypes: [CROSS_REFERENCED_EVENT, CONNECTED_EVENT]) {
        nodes {
          __typename
          ... on CrossReferencedEvent {
            willCloseTarget
            source {
              __typename
              ... on PullRequest {
                number
                title
                state
                url
                body
              }
            }
          }
          ... on ConnectedEvent {
            subject {
              __typename
              ... on PullRequest {
                number
                title
                state
                url
                body
              }
            }
          }
        }
      }
    }
  }
}
"#;

    let hub = gwt_core::process_console::global();
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        &[
            "api",
            "graphql",
            "-f",
            &format!("query={query}"),
            "-f",
            &format!("owner={owner}"),
            "-f",
            &format!("repo={repo}"),
            "-F",
            &format!("number={}", number.0),
        ],
        gwt_core::process_console::SpawnOptions::new("gh api graphql issue timeline"),
    )?;

    if !output.success() {
        return Err(io::Error::other(format!(
            "gh api graphql failed: {}",
            output.stderr.trim()
        )));
    }

    let value: serde_json::Value = serde_json::from_str(&output.stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    Ok(parse_linked_pr_nodes(&value, number.0))
}

/// Parse the issue-timeline GraphQL response into linked-PR summaries.
/// `will_close_target` comes from `CrossReferencedEvent.willCloseTarget`;
/// `ConnectedEvent` (a manually linked PR) closes the issue on merge, so it
/// counts as closing. Duplicate PR numbers OR-merge the closing flag so a PR
/// seen as both a plain reference and a closing link keeps `true`.
pub(crate) fn parse_linked_pr_nodes(
    value: &serde_json::Value,
    issue_number: u64,
) -> Vec<LinkedPrSummary> {
    let nodes = value
        .get("data")
        .and_then(|v| v.get("repository"))
        .and_then(|v| v.get("issue"))
        .and_then(|v| v.get("timelineItems"))
        .and_then(|v| v.get("nodes"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut index: std::collections::BTreeMap<u64, usize> = std::collections::BTreeMap::new();
    let mut out: Vec<LinkedPrSummary> = Vec::new();
    for node in nodes {
        let typename = node
            .get("__typename")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let (pr, mut will_close_target) = match typename {
            "CrossReferencedEvent" => (
                node.get("source"),
                node.get("willCloseTarget")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            ),
            "ConnectedEvent" => (node.get("subject"), true),
            _ => (None, false),
        };
        let Some(pr) = pr else { continue };
        // gwt merges fixes into develop (not the default branch), so GitHub
        // reports willCloseTarget=false for every real fix PR (measured on
        // #3222/#3213). The closing INTENT therefore also comes from closing
        // keywords in the PR body targeting THIS issue (`Closes #N` — the gwt
        // PR-body contract).
        if !will_close_target {
            if let Some(body) = pr.get("body").and_then(|v| v.as_str()) {
                will_close_target = body_closes_issue(body, issue_number);
            }
        }
        if pr.get("__typename").and_then(|v| v.as_str()) != Some("PullRequest") {
            continue;
        }
        let Some(pr_number) = pr.get("number").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        if let Some(existing) = index.get(&pr_number) {
            out[*existing].will_close_target |= will_close_target;
            continue;
        }
        index.insert(pr_number, out.len());
        out.push(LinkedPrSummary {
            number: pr_number,
            will_close_target,
            title: pr
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            state: pr
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            url: pr
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        });
    }
    out
}

/// GitHub closing keywords (close/closes/closed, fix/fixes/fixed,
/// resolve/resolves/resolved) followed by `#<issue_number>`, matched
/// case-insensitively anywhere in the PR body.
pub(crate) fn body_closes_issue(body: &str, issue_number: u64) -> bool {
    let needle = format!("#{issue_number}");
    let lower = body.to_lowercase();
    let bytes = lower.as_bytes();
    for keyword in [
        "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
    ] {
        let mut start = 0;
        while let Some(pos) = lower[start..].find(keyword) {
            let begin = start + pos;
            let after = begin + keyword.len();
            start = after;
            // #3228 review: the keyword must stand alone. Without a leading
            // word boundary, `prefix #42` / `hotfix #42` (fix) and
            // `disclosed #42` / `enclosed #42` (closed) would count as
            // closing intent. A trailing boundary is required too so `close`
            // does not fire inside `closedown #42`-style words (the exact
            // keywords `closes`/`closed` match via their own entries).
            let leading_ok = begin == 0 || !bytes[begin - 1].is_ascii_alphanumeric();
            let trailing_ok = !bytes
                .get(after)
                .copied()
                .is_some_and(|next| next.is_ascii_alphanumeric());
            if !leading_ok || !trailing_ok {
                continue;
            }
            let rest = lower[after..].trim_start_matches([':', ' ', '\t']);
            if rest.starts_with(&needle) {
                let tail = &rest[needle.len()..];
                let digit_follows = tail
                    .chars()
                    .next()
                    .is_some_and(|next| next.is_ascii_digit());
                if !digit_follows {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use std::{
        fs::File,
        path::Path,
        time::{Duration, SystemTime},
    };

    use gwt_github::client::{CommentId, CommentSnapshot, IssueSnapshot, IssueState, UpdatedAt};
    use tempfile::TempDir;

    use super::*;

    fn s(value: &str) -> String {
        value.to_string()
    }

    fn set_modified(path: &Path, modified: SystemTime) {
        File::options()
            .write(true)
            .open(path)
            .expect("open cache receipt")
            .set_modified(modified)
            .expect("set cache receipt mtime");
    }

    fn stale_time() -> SystemTime {
        SystemTime::now() - crate::issue_cache::ISSUE_CACHE_TTL - Duration::from_secs(1)
    }

    fn write_issue_validation_receipt(
        cache_root: &Path,
        snapshot: &IssueSnapshot,
        validated_at: &str,
    ) -> String {
        let cache = Cache::new(cache_root.to_path_buf());
        assert!(cache
            .renew_validation_receipt_if_current(snapshot)
            .expect("publish validation receipt"));
        let path = cache.validation_receipt_path(snapshot.number);
        let mut receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read validation receipt"))
                .expect("parse validation receipt");
        let generation = receipt["generation"]
            .as_str()
            .expect("validation generation")
            .to_string();
        receipt["validated_at"] = serde_json::Value::String(validated_at.to_string());
        fs::write(
            path,
            serde_json::to_vec_pretty(&receipt).expect("serialize validation receipt"),
        )
        .expect("write validation receipt");
        generation
    }

    #[test]
    fn parse_linked_pr_nodes_tracks_will_close_target() {
        // codex #3226/#3227 review: the completion probe must distinguish PRs
        // that CLOSE the issue from mere cross-references. Measured reality:
        // in gwt's develop-based flow GitHub reports willCloseTarget=false for
        // every PR (auto-close only applies to the default branch), so the
        // closing INTENT must also be derived from closing keywords in the PR
        // body (`Closes #N` — the gwt PR-body contract).
        let value = serde_json::json!({"data":{"repository":{"issue":{"timelineItems":{"nodes":[
            {"__typename":"CrossReferencedEvent","willCloseTarget":true,
             "source":{"__typename":"PullRequest","number":10,"title":"closes it","state":"MERGED","url":"u10","body":""}},
            {"__typename":"CrossReferencedEvent","willCloseTarget":false,
             "source":{"__typename":"PullRequest","number":11,"title":"refs only","state":"MERGED","url":"u11","body":"Related to #42 (no closing keyword)"}},
            {"__typename":"ConnectedEvent",
             "subject":{"__typename":"PullRequest","number":12,"title":"manually linked","state":"OPEN","url":"u12","body":""}},
            {"__typename":"CrossReferencedEvent","willCloseTarget":false,
             "source":{"__typename":"PullRequest","number":13,"title":"develop-based fix","state":"MERGED","url":"u13",
                       "body":"## Closing Issues\n\nCloses #42"}},
            {"__typename":"CrossReferencedEvent","willCloseTarget":false,
             "source":{"__typename":"PullRequest","number":14,"title":"closes another","state":"MERGED","url":"u14",
                       "body":"Fixes #43"}}
        ]}}}}});
        let prs = parse_linked_pr_nodes(&value, 42);
        let get = |n: u64| prs.iter().find(|pr| pr.number == n).expect("pr");
        assert!(get(10).will_close_target, "GraphQL willCloseTarget");
        assert!(!get(11).will_close_target, "plain reference must NOT close");
        assert!(
            get(12).will_close_target,
            "manually connected PR closes on merge"
        );
        assert!(
            get(13).will_close_target,
            "body closing keyword for THIS issue counts (develop-based flow)"
        );
        assert!(
            !get(14).will_close_target,
            "closing keyword for a DIFFERENT issue does not count"
        );
    }

    #[test]
    fn body_closes_issue_requires_word_boundaries() {
        // codex/coderabbit #3228 review: `find(keyword)` matched substrings
        // inside longer words, so `prefix #42` (fix), `disclosed #42` /
        // `enclosed #42` (closed), `hotfix #42` (fix) all counted as closing.
        for negative in [
            "prefix #42",
            "hotfix #42",
            "disclosed #42",
            "enclosed #42",
            "unfixed #42",
        ] {
            assert!(
                !body_closes_issue(negative, 42),
                "substring keyword must not close: {negative}"
            );
        }
        for positive in [
            "Closes #42",
            "fixes #42",
            "Fixed: #42",
            "resolve #42",
            "- Fix #42 in the parser",
        ] {
            assert!(
                body_closes_issue(positive, 42),
                "standalone keyword closes: {positive}"
            );
        }
        // Trailing-digit boundary is preserved.
        assert!(!body_closes_issue("Closes #421", 42));
    }

    #[test]
    fn parse_linked_pr_nodes_or_merges_duplicate_pr_flags() {
        // The same PR seen first as a plain reference and later as a closing
        // link must keep will_close_target=true.
        let value = serde_json::json!({"data":{"repository":{"issue":{"timelineItems":{"nodes":[
            {"__typename":"CrossReferencedEvent","willCloseTarget":false,
             "source":{"__typename":"PullRequest","number":10,"title":"t","state":"MERGED","url":"u"}},
            {"__typename":"CrossReferencedEvent","willCloseTarget":true,
             "source":{"__typename":"PullRequest","number":10,"title":"t","state":"MERGED","url":"u"}}
        ]}}}}});
        let prs = parse_linked_pr_nodes(&value, 42);
        assert_eq!(prs.len(), 1);
        assert!(
            prs[0].will_close_target,
            "closing flag OR-merges across events"
        );
    }

    #[test]
    fn issue_family_parse_directly_handles_view() {
        let cmd = parse(&[s("view"), s("42")]).expect("parse issue family command");
        assert_eq!(
            cmd,
            IssueCommand::View {
                number: 42,
                refresh: false,
            }
        );
    }

    #[test]
    fn issue_spec_submodule_parse_directly_handles_list() {
        let args = [s("list"), s("--phase"), s("phase/implementation")];
        let refs = args.iter().collect::<Vec<_>>();
        let cmd = crate::cli::issue_spec::parse(&refs).expect("parse spec family command");
        assert_eq!(
            cmd,
            IssueCommand::SpecList {
                phase: Some("phase/implementation".to_string()),
                state: None,
            }
        );
    }

    #[test]
    fn issue_family_run_directly_renders_cached_issue() {
        let tmp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        let snapshot = IssueSnapshot {
            number: IssueNumber(42),
            title: "Issue family direct run".to_string(),
            body: "body".to_string(),
            labels: vec!["bug".to_string()],
            state: IssueState::Open,
            updated_at: UpdatedAt::new("2026-04-12T00:00:00Z"),
            comments: vec![],
        };
        gwt_github::Cache::new(tmp.path().to_path_buf())
            .write_snapshot(&snapshot)
            .expect("write cache");
        write_issue_validation_receipt(tmp.path(), &snapshot, &chrono::Utc::now().to_rfc3339());

        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::View {
                number: 42,
                refresh: false,
            },
            &mut out,
        )
        .expect("run issue family");

        assert_eq!(code, 0);
        assert!(out.contains("#42 [OPEN] Issue family direct run"));
    }

    // -------------------------------------------------------------------
    // SPEC-1942 SC-025 follow-up: issue-family helper tests relocated
    // from cli.rs.
    // -------------------------------------------------------------------

    use crate::cli::test_support::sample_issue_snapshot;
    use crate::cli::LinkedPrSummary;

    #[test]
    fn cache_backed_issue_and_linked_pr_helpers_reuse_cached_data() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let snapshot = sample_issue_snapshot();
        env.client.seed(snapshot.clone());

        let loaded = load_or_refresh_issue(&mut env, snapshot.number, false).expect("load issue");
        assert_eq!(loaded.snapshot.number, snapshot.number);
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);

        let cached = load_or_refresh_issue(&mut env, snapshot.number, false).expect("cached issue");
        assert_eq!(cached.snapshot.title, snapshot.title);
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);

        env.seed_linked_prs(
            42,
            vec![LinkedPrSummary {
                number: 128,
                title: "Enforce coverage".to_string(),
                state: "OPEN".to_string(),
                url: "https://github.com/akiojin/gwt/pull/128".to_string(),
                will_close_target: true,
            }],
        );
        let linked =
            load_or_refresh_linked_prs(&mut env, snapshot.number, false).expect("linked prs");
        assert_eq!(linked.len(), 1);
        assert_eq!(env.linked_pr_calls(), vec![42]);

        env.clear_linked_pr_calls();
        let cached_linked = load_or_refresh_linked_prs(&mut env, snapshot.number, false)
            .expect("cached linked prs");
        assert_eq!(cached_linked.len(), 1);
        assert!(env.linked_pr_calls().is_empty());

        let cache_path = linked_prs_cache_path(temp.path(), snapshot.number);
        std::fs::create_dir_all(cache_path.parent().expect("cache dir")).expect("create cache dir");
        std::fs::write(&cache_path, "{not-json").expect("write invalid json");
        assert!(read_linked_prs_cache(temp.path(), snapshot.number).is_err());
    }

    #[test]
    fn stale_issue_cache_revalidates_and_surfaces_remote_state() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let mut cached = sample_issue_snapshot();
        cached.state = IssueState::Open;
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write cached issue");
        write_issue_validation_receipt(temp.path(), &cached, "2020-01-01T00:00:00Z");

        let mut remote = cached.clone();
        remote.state = IssueState::Closed;
        remote.updated_at = UpdatedAt::new("2026-08-13T01:00:00Z");
        env.client.seed(remote);

        let loaded = load_or_refresh_issue(&mut env, cached.number, false)
            .expect("stale issue should revalidate");

        assert_eq!(loaded.snapshot.state, IssueState::Closed);
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
    }

    #[test]
    fn stale_unchanged_issue_renews_receipt_without_a_second_fetch() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let mut snapshot = sample_issue_snapshot();
        snapshot.body = "cached body must survive NotModified".to_string();
        Cache::new(env.cache_root())
            .write_snapshot(&snapshot)
            .expect("write cached issue");
        write_issue_validation_receipt(temp.path(), &snapshot, "2020-01-01T00:00:00Z");
        let mut remote = snapshot.clone();
        remote.body = "remote body must not transfer on NotModified".to_string();
        env.client.seed(remote);

        let first = load_or_refresh_issue(&mut env, snapshot.number, false)
            .expect("stale issue should revalidate");
        let second = load_or_refresh_issue(&mut env, snapshot.number, false)
            .expect("renewed receipt should be fresh");

        assert_eq!(first.snapshot.body, snapshot.body);
        assert_eq!(second.snapshot.body, snapshot.body);
        assert_eq!(
            Cache::new(env.cache_root())
                .load_entry(snapshot.number)
                .expect("cached issue after NotModified")
                .snapshot
                .body,
            snapshot.body
        );
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
    }

    #[test]
    fn stale_validation_sidecar_conditionally_revalidates_and_renews_generation() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let mut cached = sample_issue_snapshot();
        cached.body = "cached complete body".to_string();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write cached issue");
        let stale_generation =
            write_issue_validation_receipt(temp.path(), &cached, "2020-01-01T00:00:00Z");

        let mut remote = cached.clone();
        remote.body = "remote body must not replace NotModified cache".to_string();
        env.client.seed(remote);

        let loaded = load_or_refresh_issue(&mut env, cached.number, false)
            .expect("stale validation should conditionally revalidate");

        assert_eq!(loaded.snapshot.body, cached.body);
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
        let receipt: serde_json::Value = serde_json::from_slice(
            &fs::read(
                temp.path()
                    .join(cached.number.0.to_string())
                    .join("issue-validation.json"),
            )
            .expect("renewed validation receipt"),
        )
        .expect("parse renewed receipt");
        assert_ne!(receipt["generation"], stale_generation);
    }

    #[test]
    fn stale_issue_comments_refresh_remote_changes() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let cached = sample_issue_snapshot();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write cached issue");
        write_issue_validation_receipt(temp.path(), &cached, "2020-01-01T00:00:00Z");

        let mut remote = cached.clone();
        remote.updated_at = UpdatedAt::new("2026-08-13T02:00:00Z");
        remote.comments = vec![CommentSnapshot {
            id: CommentId(9001),
            body: "fresh remote comment".to_string(),
            updated_at: remote.updated_at.clone(),
        }];
        env.client.seed(remote);

        let mut out = String::new();
        run(
            &mut env,
            IssueCommand::Comments {
                number: cached.number.0,
                refresh: false,
            },
            &mut out,
        )
        .expect("stale comments should revalidate");

        assert!(out.contains("fresh remote comment"));
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
    }

    #[test]
    fn cache_without_validation_sidecar_full_fetches_partial_comments() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let mut partial = sample_issue_snapshot();
        partial.comments.clear();
        Cache::new(env.cache_root())
            .write_snapshot(&partial)
            .expect("write bulk-like partial cache");

        let mut remote = partial.clone();
        remote.comments = vec![CommentSnapshot {
            id: CommentId(9002),
            body: "comment omitted by bulk list snapshot".to_string(),
            updated_at: remote.updated_at.clone(),
        }];
        env.client.seed(remote);

        let loaded = load_or_refresh_issue(&mut env, partial.number, false)
            .expect("unvalidated partial cache should full fetch");

        assert_eq!(loaded.snapshot.comments.len(), 1);
        assert_eq!(
            loaded.snapshot.comments[0].body,
            "comment omitted by bulk list snapshot"
        );
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
    }

    #[test]
    fn stale_linked_pr_cache_refreshes_independently() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let number = IssueNumber(42);
        write_linked_prs_cache(
            temp.path(),
            number,
            &[LinkedPrSummary {
                number: 100,
                title: "cached PR".to_string(),
                state: "OPEN".to_string(),
                url: "https://example.test/100".to_string(),
                will_close_target: false,
            }],
        )
        .expect("write linked PR cache");
        set_modified(&linked_prs_cache_path(temp.path(), number), stale_time());
        env.seed_linked_prs(
            number.0,
            vec![LinkedPrSummary {
                number: 101,
                title: "fresh PR".to_string(),
                state: "MERGED".to_string(),
                url: "https://example.test/101".to_string(),
                will_close_target: true,
            }],
        );

        let linked = load_or_refresh_linked_prs(&mut env, number, false)
            .expect("stale linked PRs should refresh");

        assert_eq!(linked[0].number, 101);
        assert_eq!(env.linked_pr_calls(), vec![42]);
    }

    #[test]
    fn stale_linked_pr_revalidation_error_does_not_return_cached_data() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let number = IssueNumber(42);
        write_linked_prs_cache(
            temp.path(),
            number,
            &[LinkedPrSummary {
                number: 100,
                title: "stale cached PR".to_string(),
                state: "OPEN".to_string(),
                url: "https://example.test/100".to_string(),
                will_close_target: false,
            }],
        )
        .expect("write linked PR cache");
        let receipt = linked_prs_cache_path(temp.path(), number);
        let stale = stale_time();
        set_modified(&receipt, stale);
        env.seed_linked_pr_error(number.0, "linked PR refresh failed");

        let error = load_or_refresh_linked_prs(&mut env, number, false)
            .expect_err("failed linked PR refresh must fail closed");

        assert!(error.to_string().contains("linked PR refresh failed"));
        assert_eq!(env.linked_pr_calls(), vec![42]);
        assert!(!cache_resource_is_fresh(&receipt));
    }

    #[test]
    fn corrupt_linked_pr_cache_is_replaced_from_remote() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let number = IssueNumber(42);
        let receipt = linked_prs_cache_path(temp.path(), number);
        fs::create_dir_all(receipt.parent().expect("cache directory"))
            .expect("create cache directory");
        fs::write(&receipt, "{not-json").expect("write corrupt linked PR cache");
        env.seed_linked_prs(
            number.0,
            vec![LinkedPrSummary {
                number: 101,
                title: "recovered PR".to_string(),
                state: "OPEN".to_string(),
                url: "https://example.test/101".to_string(),
                will_close_target: true,
            }],
        );

        let linked = load_or_refresh_linked_prs(&mut env, number, false)
            .expect("corrupt linked PR cache should refresh");

        assert_eq!(linked[0].number, 101);
        assert_eq!(env.linked_pr_calls(), vec![42]);
        assert_eq!(
            read_linked_prs_cache(temp.path(), number)
                .expect("repaired linked PR cache")
                .expect("linked PR cache should exist")[0]
                .number,
            101
        );
    }

    #[test]
    fn future_dated_issue_receipt_is_revalidated() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let cached = sample_issue_snapshot();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write cached issue");
        write_issue_validation_receipt(temp.path(), &cached, "2999-01-01T00:00:00Z");

        let mut remote = cached.clone();
        remote.title = "future receipt was revalidated".to_string();
        remote.updated_at = UpdatedAt::new("2026-08-13T03:00:00Z");
        env.client.seed(remote);

        let loaded = load_or_refresh_issue(&mut env, cached.number, false)
            .expect("future receipt should revalidate");

        assert_eq!(loaded.snapshot.title, "future receipt was revalidated");
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
    }

    #[test]
    fn stale_issue_revalidation_error_does_not_return_cached_data() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let cached = sample_issue_snapshot();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write cached issue");
        write_issue_validation_receipt(temp.path(), &cached, "2020-01-01T00:00:00Z");

        let error = load_or_refresh_issue(&mut env, cached.number, false)
            .expect_err("failed revalidation must fail closed");

        assert!(error.to_string().contains("not found"));
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
    }

    #[test]
    fn explicit_issue_refresh_bypasses_a_fresh_receipt() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let cached = sample_issue_snapshot();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write cached issue");

        let mut remote = cached.clone();
        remote.title = "explicitly refreshed".to_string();
        remote.updated_at = UpdatedAt::new("2026-08-13T04:00:00Z");
        env.client.seed(remote);

        let loaded = load_or_refresh_issue(&mut env, cached.number, true)
            .expect("explicit refresh should fetch");

        assert_eq!(loaded.snapshot.title, "explicitly refreshed");
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
    }

    #[test]
    fn explicit_issue_refresh_rebuilds_issue_index_when_cache_source_changes() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let mut old = sample_issue_snapshot();
        old.state = IssueState::Open;
        gwt_github::Cache::new(env.cache_root())
            .write_snapshot(&old)
            .expect("write old cache");

        let mut updated = old.clone();
        updated.state = IssueState::Closed;
        env.client.seed(updated.clone());

        let mut rebuild_calls = Vec::new();
        let entry = refresh_issue_cache_with_index_rebuild(&mut env, updated.number, |repo_path| {
            rebuild_calls.push(repo_path.to_path_buf());
            Ok(())
        })
        .expect("refresh with rebuild");

        assert_eq!(entry.snapshot.state, IssueState::Closed);
        assert_eq!(rebuild_calls, vec![env.repo_path().to_path_buf()]);
    }

    #[test]
    fn index_rebuild_failure_keeps_receipt_absent_and_next_read_retries() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let mut cached = sample_issue_snapshot();
        cached.title = "old cache".to_string();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write old cache");

        let mut remote = cached.clone();
        remote.title = "remote snapshot".to_string();
        remote.updated_at = UpdatedAt::new("2026-08-13T05:00:00Z");
        env.client.seed(remote.clone());

        let mut rebuild_calls = 0;
        let first =
            load_or_refresh_issue_with_index_rebuild(&mut env, remote.number, false, |_| {
                rebuild_calls += 1;
                Err("injected rebuild failure".to_string())
            })
            .expect_err("first index rebuild should fail");
        assert!(first.to_string().contains("injected rebuild failure"));
        assert!(!Cache::new(env.cache_root())
            .validation_receipt_path(remote.number)
            .exists());

        let second =
            load_or_refresh_issue_with_index_rebuild(&mut env, remote.number, false, |_| {
                rebuild_calls += 1;
                Ok(())
            })
            .expect("unvalidated cache must retry index rebuild");
        assert_eq!(second.snapshot.title, remote.title);

        let third =
            load_or_refresh_issue_with_index_rebuild(&mut env, remote.number, false, |_| {
                rebuild_calls += 1;
                Ok(())
            })
            .expect("validated cache should be a warm hit");
        assert_eq!(third.snapshot.title, remote.title);
        assert_eq!(rebuild_calls, 2);
        assert_eq!(
            env.client.call_log(),
            vec!["fetch:#42".to_string(), "fetch:#42".to_string()]
        );
    }

    #[test]
    fn generation_change_during_rebuild_prevents_receipt_publication() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let cached = sample_issue_snapshot();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write old cache");
        let mut remote = cached.clone();
        remote.title = "remote snapshot".to_string();
        remote.updated_at = UpdatedAt::new("2026-08-13T06:00:00Z");
        env.client.seed(remote.clone());
        let cache_root = env.cache_root();
        let mut concurrent = remote.clone();
        concurrent.title = "concurrent writer wins".to_string();

        let error =
            load_or_refresh_issue_with_index_rebuild(&mut env, remote.number, false, move |_| {
                Cache::new(cache_root.clone())
                    .write_snapshot(&concurrent)
                    .map_err(|error| error.to_string())
            })
            .expect_err("changed generation must reject receipt publication");

        assert!(error
            .to_string()
            .contains("changed before validation receipt"));
        let cache = Cache::new(env.cache_root());
        assert_eq!(
            cache
                .load_entry(remote.number)
                .expect("concurrent cache")
                .snapshot
                .title,
            "concurrent writer wins"
        );
        assert!(!cache.validation_receipt_path(remote.number).exists());
    }

    #[test]
    fn identical_snapshot_aba_during_rebuild_rejects_original_generation() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let cached = sample_issue_snapshot();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write old cache");
        let mut remote = cached.clone();
        remote.title = "same bytes after concurrent commit".to_string();
        remote.updated_at = UpdatedAt::new("2026-08-13T07:00:00Z");
        env.client.seed(remote.clone());
        let cache_root = env.cache_root();
        let concurrent = remote.clone();

        let error =
            load_or_refresh_issue_with_index_rebuild(&mut env, remote.number, false, move |_| {
                Cache::new(cache_root.clone())
                    .write_snapshot(&concurrent)
                    .map_err(|error| error.to_string())
            })
            .expect_err("identical bytes with a different UUID must fail generation CAS");

        assert!(error
            .to_string()
            .contains("changed before validation receipt"));
        let cache = Cache::new(env.cache_root());
        let persisted = cache.load_entry(remote.number).unwrap().snapshot;
        assert_eq!(persisted.title, remote.title);
        assert_eq!(persisted.body, remote.body);
        assert_eq!(persisted.updated_at, remote.updated_at);
        assert_eq!(persisted.comments[0].body, remote.comments[0].body);
        assert!(!cache.validation_receipt_path(remote.number).exists());
    }
}
