//! Shared test-only infrastructure for `cli` family modules (SPEC-1942
//! SC-025 helper migration). Hosts the fake `gh` compile/run harness and
//! sample fixtures so each family's `tests` mod can drop them in without
//! duplicating the substantial fake-binary source.
//!
//! Division of labor (SPEC-3016 FR-003): generic, crate-agnostic helpers
//! (`env_lock`, `ScopedEnvVar`) live in `gwt_core::test_support` and are
//! re-exported here; gwt-only machinery (fake `gh`, CLI fixtures) stays in
//! this module.
//!
//! `#[cfg(test)]` only — never compiled into the production library.

#![cfg(test)]

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use gwt_github::{CommentId, CommentSnapshot, IssueNumber, IssueSnapshot, IssueState, UpdatedAt};
use tempfile::tempdir;

use super::PrReviewThread;

pub fn fake_gh_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

pub fn compile_fake_gh(bin_dir: &Path) {
    let source = r###"
use std::{env, fs, process::ExitCode};

fn pr_json(number: &str, title: &str) -> String {
    format!(
        "{{\"number\":{number},\"title\":\"{title}\",\"state\":\"OPEN\",\"url\":\"https://github.com/akiojin/gwt/pull/{number}\",\"createdAt\":\"2026-05-07T08:00:00Z\",\"mergeable\":\"MERGEABLE\",\"mergeStateStatus\":\"CLEAN\",\"statusCheckRollup\":[{{\"name\":\"ci\",\"status\":\"COMPLETED\",\"conclusion\":\"SUCCESS\"}}],\"reviewDecision\":\"APPROVED\"}}"
    )
}

fn behind_pr_json(number: &str, title: &str) -> String {
    format!(
        "{{\"number\":{number},\"title\":\"{title}\",\"state\":\"OPEN\",\"url\":\"https://github.com/akiojin/gwt/pull/{number}\",\"createdAt\":\"2026-05-07T08:00:00Z\",\"mergeable\":\"MERGEABLE\",\"mergeStateStatus\":\"BEHIND\",\"statusCheckRollup\":[{{\"name\":\"ci\",\"status\":\"COMPLETED\",\"conclusion\":\"SUCCESS\"}}],\"reviewDecision\":\"REVIEW_REQUIRED\"}}"
    )
}

fn review_threads_json(resolved_after_fail: bool) -> String {
    let resolved = if resolved_after_fail { "true" } else { "false" };
    r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{"nodes":[
{"id":"thread-1","isResolved":__RESOLVED__,"isOutdated":false,"path":"src/lib.rs","line":10,"comments":{"nodes":[{"id":"comment-1","body":"done","createdAt":"2026-04-20T00:00:00Z","updatedAt":"2026-04-20T00:00:00Z","author":{"login":"reviewer"}}]}},
{"id":"thread-2","isResolved":false,"isOutdated":false,"path":"src/main.rs","line":12,"comments":{"nodes":[{"id":"comment-2","body":"needs changes","createdAt":"2026-04-20T01:00:00Z","updatedAt":"2026-04-20T01:00:00Z","author":{"login":"reviewer"}}]}}
]}}}}}"#
        .replace("__RESOLVED__", resolved)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let mode = env::var("GWT_FAKE_GH_MODE").unwrap_or_else(|_| "success".to_string());
    let state_file = env::var("GWT_FAKE_GH_STATE_FILE").ok();

    match args.as_slice() {
        [pr, list, ..] if pr == "pr" && list == "list" => {
            if mode == "multi-pr-current" {
                println!("{}", r#"[
{"number":2537,"title":"Older PR","state":"CLOSED","url":"https://github.com/akiojin/gwt/pull/2537","createdAt":"2026-05-07T08:05:00Z","mergeable":"UNKNOWN","mergeStateStatus":"UNKNOWN","statusCheckRollup":[],"reviewDecision":"UNKNOWN","headRefName":"work/20260507-0808","headRepositoryOwner":{"login":"akiojin"},"headRepository":{"name":"gwt"}},
{"number":2538,"title":"Newer PR","state":"OPEN","url":"https://github.com/akiojin/gwt/pull/2538","createdAt":"2026-05-07T08:20:00Z","mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","statusCheckRollup":[],"reviewDecision":"APPROVED","headRefName":"work/20260507-0808","headRepositoryOwner":{"login":"akiojin"},"headRepository":{"name":"gwt"}}
]"#);
            } else if mode == "multi-pr-cross-fork-current" {
                println!("{}", r#"[
{"number":2538,"title":"Current repo PR","state":"OPEN","url":"https://github.com/akiojin/gwt/pull/2538","createdAt":"2026-05-07T08:20:00Z","mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","statusCheckRollup":[],"reviewDecision":"APPROVED","headRefName":"work/20260507-0808","headRepositoryOwner":{"login":"akiojin"},"headRepository":{"name":"gwt"}},
{"number":2539,"title":"Fork PR","state":"OPEN","url":"https://github.com/akiojin/gwt/pull/2539","createdAt":"2026-05-07T08:25:00Z","mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","statusCheckRollup":[],"reviewDecision":"APPROVED","headRefName":"work/20260507-0808","headRepositoryOwner":{"login":"other-user"},"headRepository":{"name":"gwt"}}
]"#);
            } else {
                println!("[{}]", pr_json("12", "Current PR"));
            }
            return ExitCode::SUCCESS;
        }
        [pr, view, json_flag, ..] if pr == "pr" && view == "view" && json_flag == "--json" => {
            if mode == "no-current-pr" {
                eprintln!("no pull requests found for branch");
                return ExitCode::from(1);
            }
            if mode == "behind" {
                println!("{}", behind_pr_json("12", "Current PR"));
            } else {
                println!("{}", pr_json("12", "Current PR"));
            }
            return ExitCode::SUCCESS;
        }
        [pr, view, number, repo_flag, _, json_flag, ..]
            if pr == "pr" && view == "view" && repo_flag == "--repo" && json_flag == "--json" =>
        {
            if mode == "behind" {
                println!("{}", behind_pr_json(number, "Fetched PR"));
            } else {
                println!("{}", pr_json(number, "Fetched PR"));
            }
            return ExitCode::SUCCESS;
        }
        [pr, create, ..] if pr == "pr" && create == "create" => {
            println!("https://github.com/akiojin/gwt/pull/12");
            return ExitCode::SUCCESS;
        }
        [pr, edit, ..] if pr == "pr" && edit == "edit" => {
            return ExitCode::SUCCESS;
        }
        [pr, comment, ..] if pr == "pr" && comment == "comment" => {
            return ExitCode::SUCCESS;
        }
        [pr, checks, _, json_flag, fields] if pr == "pr" && checks == "checks" && json_flag == "--json" => {
            if mode == "checks-fallback" && !fields.contains("bucket") {
                eprintln!("unknown JSON field\nAvailable fields:\n  name\n  state\n  bucket\n  link\n  startedAt\n  completedAt\n  workflow");
                return ExitCode::from(1);
            }
            if fields.contains("bucket") {
                println!("[{{\"name\":\"CI\",\"state\":\"COMPLETED\",\"bucket\":\"pass\",\"link\":\"https://example.test/checks/12\",\"startedAt\":\"2026-04-20T00:00:00Z\",\"completedAt\":\"2026-04-20T00:01:00Z\",\"workflow\":\"coverage\"}}]");
            } else {
                println!("[{{\"name\":\"CI\",\"state\":\"COMPLETED\",\"conclusion\":\"SUCCESS\",\"detailsUrl\":\"https://example.test/checks/12\",\"startedAt\":\"2026-04-20T00:00:00Z\",\"completedAt\":\"2026-04-20T00:01:00Z\"}}]");
            }
            return ExitCode::SUCCESS;
        }
        [run, view, run_id, log_flag] if run == "run" && view == "view" && log_flag == "--log" => {
            println!("run log {run_id}");
            return ExitCode::SUCCESS;
        }
        [api, endpoint] if api == "api" && endpoint == "repos/akiojin/gwt/pulls/12/reviews" => {
            println!("[{{\"id\":42,\"state\":\"APPROVED\",\"body\":\"Looks good\",\"submitted_at\":\"2026-04-20T02:00:00Z\",\"user\":{{\"login\":\"reviewer\"}}}}]");
            return ExitCode::SUCCESS;
        }
        [api, endpoint] if api == "api" && endpoint == "/repos/akiojin/gwt/actions/jobs/91/logs" => {
            if mode == "job-log-zip" {
                print!("PKZIP");
            } else {
                print!("job log 91");
            }
            return ExitCode::SUCCESS;
        }
        [api, graphql, ..] if api == "api" && graphql == "graphql" => {
            let joined = args.join("\n");
            if joined.contains("timelineItems") {
                println!(
                    "{}",
                    r#"{"data":{"repository":{"issue":{"timelineItems":{"nodes":[
{"__typename":"CrossReferencedEvent","source":{"__typename":"PullRequest","number":12,"title":"Coverage Gate","state":"OPEN","url":"https://github.com/akiojin/gwt/pull/12"}},
{"__typename":"ConnectedEvent","subject":{"__typename":"PullRequest","number":13,"title":"Follow-up","state":"MERGED","url":"https://github.com/akiojin/gwt/pull/13"}},
{"__typename":"ConnectedEvent","subject":{"__typename":"PullRequest","number":12,"title":"Duplicate","state":"OPEN","url":"https://github.com/akiojin/gwt/pull/12"}}
]}}}}}"#
                );
                return ExitCode::SUCCESS;
            }
            if joined.contains("reviewThreads") {
                let resolved_after_fail = state_file
                    .as_deref()
                    .map(fs::metadata)
                    .transpose()
                    .ok()
                    .flatten()
                    .is_some();
                println!("{}", review_threads_json(resolved_after_fail));
                return ExitCode::SUCCESS;
            }
            if joined.contains("addPullRequestReviewThreadReply") {
                println!("{{\"data\":{{\"addPullRequestReviewThreadReply\":{{\"comment\":{{\"id\":\"reply-1\"}}}}}}}}");
                return ExitCode::SUCCESS;
            }
            if joined.contains("resolveReviewThread") {
                if mode == "resolve-fails-but-resolved" {
                    let already_failed = state_file
                        .as_deref()
                        .map(fs::metadata)
                        .transpose()
                        .ok()
                        .flatten()
                        .is_some();
                    if !already_failed {
                        if let Some(state_file) = state_file.as_deref() {
                            let _ = fs::write(state_file, "resolved");
                        }
                        eprintln!("thread already resolved");
                        return ExitCode::from(1);
                    }
                }
                println!("{{\"data\":{{\"resolveReviewThread\":{{\"thread\":{{\"id\":\"thread-1\",\"isResolved\":true}}}}}}}}");
                return ExitCode::SUCCESS;
            }
        }
        _ => {}
    }

    eprintln!("unexpected fake gh args: {args:?}");
    ExitCode::from(1)
}
"###;

    let source_path = bin_dir.join("gh.rs");
    fs::write(&source_path, source).expect("write fake gh source");
    let output_path = bin_dir.join(format!("gh{}", env::consts::EXE_SUFFIX));
    // Pin the child's working directory: parallel tests may set the
    // process-wide CWD to a tempdir that gets dropped, and rustc refuses to
    // start from a deleted CWD ("Current directory is invalid", #3006).
    let status = std::process::Command::new("rustc")
        .arg(&source_path)
        .arg("-o")
        .arg(&output_path)
        .current_dir(bin_dir)
        .status()
        .expect("compile fake gh");
    assert!(status.success(), "fake gh compilation failed");
}

pub fn with_fake_gh<T>(mode: &str, test: impl FnOnce(&Path) -> T) -> T {
    let _env_lock = crate::env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    compile_fake_gh(temp.path());

    let repo_path = temp.path().join("repo");
    fs::create_dir_all(&repo_path).expect("create repo path");

    let old_path = env::var_os("PATH");
    let old_mode = env::var_os("GWT_FAKE_GH_MODE");
    let old_state = env::var_os("GWT_FAKE_GH_STATE_FILE");
    let state_file = temp.path().join("gh-state");
    let joined_path = env::join_paths(
        std::iter::once(PathBuf::from(temp.path()))
            .chain(old_path.iter().flat_map(env::split_paths)),
    )
    .expect("join PATH");
    env::set_var("PATH", joined_path);
    env::set_var("GWT_FAKE_GH_MODE", mode);
    env::set_var("GWT_FAKE_GH_STATE_FILE", &state_file);

    let result = test(&repo_path);

    match old_path {
        Some(value) => env::set_var("PATH", value),
        None => env::remove_var("PATH"),
    }
    match old_mode {
        Some(value) => env::set_var("GWT_FAKE_GH_MODE", value),
        None => env::remove_var("GWT_FAKE_GH_MODE"),
    }
    match old_state {
        Some(value) => env::set_var("GWT_FAKE_GH_STATE_FILE", value),
        None => env::remove_var("GWT_FAKE_GH_STATE_FILE"),
    }

    result
}

pub fn sample_thread() -> PrReviewThread {
    PrReviewThread {
        id: "thread-1".to_string(),
        is_resolved: false,
        is_outdated: false,
        path: "src/lib.rs".to_string(),
        line: Some(12),
        comments: vec![],
    }
}

pub fn sample_issue_snapshot() -> IssueSnapshot {
    IssueSnapshot {
        number: IssueNumber(42),
        title: "Coverage gate".to_string(),
        body: "Raise the project coverage gate.\n".to_string(),
        labels: vec!["gwt-spec".to_string(), "coverage".to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("2026-04-20T00:00:00Z"),
        comments: vec![CommentSnapshot {
            id: CommentId(7),
            body: "Need more tests.".to_string(),
            updated_at: UpdatedAt::new("2026-04-20T01:00:00Z"),
        }],
    }
}

pub fn sample_pr_status() -> gwt_git::PrStatus {
    gwt_git::PrStatus {
        number: 128,
        title: "Enforce coverage".to_string(),
        state: gwt_git::pr_status::PrState::Open,
        url: "https://github.com/akiojin/gwt/pull/128".to_string(),
        created_at: None,
        ci_status: "SUCCESS".to_string(),
        mergeable: "MERGEABLE".to_string(),
        merge_state_status: "CLEAN".to_string(),
        review_status: "APPROVED".to_string(),
    }
}

// SPEC-3016 FR-003: the env-var guard is shared infrastructure; its
// canonical home is gwt-core (`test-support` feature). Re-exported so cli
// tests keep one import path.
pub use gwt_core::test_support::ScopedEnvVar;

pub fn commands_for_event<'a>(value: &'a serde_json::Value, event: &str) -> Vec<&'a str> {
    value["hooks"][event]
        .as_array()
        .unwrap_or_else(|| panic!("hooks missing for event {event}"))
        .iter()
        .flat_map(|entry| entry["hooks"].as_array().into_iter().flatten())
        .filter_map(|hook| hook["command"].as_str())
        .collect()
}
