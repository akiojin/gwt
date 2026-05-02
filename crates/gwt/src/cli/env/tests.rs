//! Test suite for `cli::env` (SPEC-1942 SC-027 split). Lives as a sibling
//! file, included via `#[cfg(test)] mod tests;`.

#![cfg(test)]

use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use gwt_git::PrStatus;
use gwt_github::{client::fake::FakeIssueClient, IssueNumber, SpecListFilter};

use super::*;

fn sample_pr_status() -> PrStatus {
    PrStatus {
        number: 128,
        title: "Enforce coverage".to_string(),
        state: gwt_git::pr_status::PrState::Open,
        url: "https://github.com/akiojin/gwt/pull/128".to_string(),
        ci_status: "SUCCESS".to_string(),
        mergeable: "MERGEABLE".to_string(),
        merge_state_status: "CLEAN".to_string(),
        review_status: "APPROVED".to_string(),
    }
}

fn sample_issue_snapshot(number: u64, labels: &[&str]) -> gwt_github::client::IssueSnapshot {
    gwt_github::client::IssueSnapshot {
        number: IssueNumber(number),
        title: format!("Issue {number}"),
        body: "Body".to_string(),
        labels: labels.iter().map(|label| (*label).to_string()).collect(),
        state: gwt_github::IssueState::Open,
        updated_at: gwt_github::client::UpdatedAt::new("2026-04-20T00:00:00Z"),
        comments: vec![gwt_github::client::CommentSnapshot {
            id: gwt_github::client::CommentId(9),
            body: "comment".to_string(),
            updated_at: gwt_github::client::UpdatedAt::new("2026-04-20T00:01:00Z"),
        }],
    }
}

fn compile_fake_gh(bin_dir: &Path) {
    let source = r###"
use std::{env, process::ExitCode};

fn pr_json(number: &str, title: &str) -> String {
format!(
    "{{\"number\":{number},\"title\":\"{title}\",\"state\":\"OPEN\",\"url\":\"https://github.com/akiojin/gwt/pull/{number}\",\"mergeable\":\"MERGEABLE\",\"mergeStateStatus\":\"CLEAN\",\"statusCheckRollup\":[{{\"name\":\"ci\",\"status\":\"COMPLETED\",\"conclusion\":\"SUCCESS\"}}],\"reviewDecision\":\"APPROVED\"}}"
)
}

fn main() -> ExitCode {
let args: Vec<String> = env::args().skip(1).collect();
match args.as_slice() {
    [pr, view, json_flag, ..] if pr == "pr" && view == "view" && json_flag == "--json" => {
        println!("{}", pr_json("12", "Current PR"));
        ExitCode::SUCCESS
    }
    [pr, view, number, repo_flag, _, json_flag, ..]
        if pr == "pr" && view == "view" && repo_flag == "--repo" && json_flag == "--json" =>
    {
        println!("{}", pr_json(number, "Fetched PR"));
        ExitCode::SUCCESS
    }
    [pr, create, ..] if pr == "pr" && create == "create" => {
        println!("https://github.com/akiojin/gwt/pull/12");
        ExitCode::SUCCESS
    }
    [pr, edit, ..] if pr == "pr" && edit == "edit" => ExitCode::SUCCESS,
    [pr, comment, ..] if pr == "pr" && comment == "comment" => ExitCode::SUCCESS,
    [pr, checks, _, json_flag, _] if pr == "pr" && checks == "checks" && json_flag == "--json" => {
        println!("[{{\"name\":\"CI\",\"state\":\"COMPLETED\",\"conclusion\":\"SUCCESS\",\"detailsUrl\":\"https://example.test/checks/12\",\"startedAt\":\"2026-04-20T00:00:00Z\",\"completedAt\":\"2026-04-20T00:01:00Z\"}}]");
        ExitCode::SUCCESS
    }
    [run, view, run_id, log_flag] if run == "run" && view == "view" && log_flag == "--log" => {
        println!("run log {run_id}");
        ExitCode::SUCCESS
    }
    [api, endpoint] if api == "api" && endpoint == "repos/akiojin/gwt/pulls/12/reviews" => {
        println!("[{{\"id\":42,\"state\":\"APPROVED\",\"body\":\"Looks good\",\"submitted_at\":\"2026-04-20T02:00:00Z\",\"user\":{{\"login\":\"reviewer\"}}}}]");
        ExitCode::SUCCESS
    }
    [api, endpoint] if api == "api" && endpoint == "/repos/akiojin/gwt/actions/jobs/91/logs" => {
        print!("job log 91");
        ExitCode::SUCCESS
    }
    [api, graphql, ..] if api == "api" && graphql == "graphql" => {
        let joined = args.join("\n");
        if joined.contains("timelineItems") {
            println!(
                "{}",
                r#"{"data":{"repository":{"issue":{"timelineItems":{"nodes":[
{"__typename":"CrossReferencedEvent","source":{"__typename":"PullRequest","number":12,"title":"Coverage Gate","state":"OPEN","url":"https://github.com/akiojin/gwt/pull/12"}}
]}}}}}"#
            );
        } else if joined.contains("reviewThreads") {
            println!(
                "{}",
                r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{"nodes":[
{"id":"thread-1","isResolved":false,"isOutdated":false,"path":"src/lib.rs","line":10,"comments":{"nodes":[{"id":"comment-1","body":"done","createdAt":"2026-04-20T00:00:00Z","updatedAt":"2026-04-20T00:00:00Z","author":{"login":"reviewer"}}]}}
]}}}}}"#
            );
        } else if joined.contains("addPullRequestReviewThreadReply") {
            println!("{{\"data\":{{\"addPullRequestReviewThreadReply\":{{\"comment\":{{\"id\":\"reply-1\"}}}}}}}}");
        } else if joined.contains("resolveReviewThread") {
            println!("{{\"data\":{{\"resolveReviewThread\":{{\"thread\":{{\"id\":\"thread-1\",\"isResolved\":true}}}}}}}}");
        } else {
            eprintln!("unexpected graphql args: {args:?}");
            return ExitCode::from(1);
        }
        ExitCode::SUCCESS
    }
    _ => {
        eprintln!("unexpected fake gh args: {args:?}");
        ExitCode::from(1)
    }
}
}
"###;

    let source_path = bin_dir.join("gh.rs");
    fs::write(&source_path, source).expect("write fake gh source");
    let output_path = bin_dir.join(format!("gh{}", env::consts::EXE_SUFFIX));
    let status = std::process::Command::new("rustc")
        .arg(&source_path)
        .arg("-o")
        .arg(&output_path)
        .status()
        .expect("compile fake gh");
    assert!(status.success(), "fake gh compilation failed");
}

fn with_fake_gh<T>(test: impl FnOnce(&Path) -> T) -> T {
    let _lock = crate::cli::test_support::fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempfile::tempdir().expect("tempdir");
    compile_fake_gh(temp.path());

    let old_path = env::var_os("PATH");
    let joined_path = env::join_paths(
        std::iter::once(PathBuf::from(temp.path()))
            .chain(old_path.iter().flat_map(env::split_paths)),
    )
    .expect("join PATH");
    env::set_var("PATH", joined_path);

    let repo_path = temp.path().join("repo");
    fs::create_dir_all(&repo_path).expect("create repo");
    let result = test(&repo_path);

    match old_path {
        Some(value) => env::set_var("PATH", value),
        None => env::remove_var("PATH"),
    }

    result
}

fn failing_factory(counter: Arc<AtomicUsize>) -> Arc<IssueClientFactory> {
    Arc::new(move |_, _| {
        counter.fetch_add(1, Ordering::SeqCst);
        Err(gwt_github::client::ApiError::Unauthorized)
    })
}

#[test]
fn lazy_issue_client_defers_resolution_until_first_issue_call() {
    let calls = Arc::new(AtomicUsize::new(0));
    let client =
        LazyIssueClient::new_with_factory("akiojin", "gwt", failing_factory(calls.clone()));

    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let result = client.list_spec_issues(&SpecListFilter::default());
    assert!(matches!(
        result,
        Err(gwt_github::client::ApiError::Unauthorized)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn default_cli_env_construction_does_not_touch_issue_client_factory() {
    let calls = Arc::new(AtomicUsize::new(0));
    let env = DefaultCliEnv::new_with_client_factory(
        "akiojin",
        "gwt",
        PathBuf::from("."),
        failing_factory(calls.clone()),
    );

    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let result = env.client().list_spec_issues(&SpecListFilter::default());
    assert!(matches!(
        result,
        Err(gwt_github::client::ApiError::Unauthorized)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn new_for_hooks_keeps_detached_cache_root() {
    let env = DefaultCliEnv::new_for_hooks();

    assert_eq!(
        env.cache_root(),
        crate::issue_cache::detached_issue_cache_root()
    );
}

#[test]
fn test_env_records_io_and_pr_side_effects() {
    let mut env = TestEnv::new(PathBuf::from("cache-root"));
    env.stdin = "from stdin".to_string();
    env.files.insert("input.md".to_string(), "body".to_string());

    assert_eq!(env.read_stdin().expect("stdin"), "from stdin");
    assert_eq!(env.read_stdin().expect("stdin second read"), "");
    assert_eq!(env.read_file("input.md").expect("file"), "body");
    assert_eq!(
        env.read_file("missing.md").expect_err("missing").kind(),
        io::ErrorKind::NotFound
    );

    env.seed_linked_prs(
        42,
        vec![LinkedPrSummary {
            number: 128,
            title: "Coverage".to_string(),
            state: "OPEN".to_string(),
            url: "https://example.test/pr/128".to_string(),
        }],
    );
    assert_eq!(
        env.fetch_linked_prs(IssueNumber(42)).expect("linked").len(),
        1
    );
    assert_eq!(env.linked_pr_calls(), vec![42]);
    env.clear_linked_pr_calls();
    assert!(env.linked_pr_calls().is_empty());

    env.seed_current_pr(Some(sample_pr_status()));
    assert!(env.fetch_current_pr().expect("current").is_some());
    assert_eq!(env.pr_current_call_count, 1);

    env.seed_created_pr(sample_pr_status());
    let created = env
        .create_pr(
            "develop",
            Some("feature/coverage"),
            "Raise coverage",
            "Body",
            &["coverage".to_string()],
            true,
        )
        .expect("create pr");
    assert_eq!(created.number, 128);
    assert_eq!(env.pr_create_call_log.len(), 1);
    assert_eq!(
        env.pr_create_call_log[0].head.as_deref(),
        Some("feature/coverage")
    );
    assert!(env.pr_create_call_log[0].draft);

    env.seed_pr(7, sample_pr_status());
    let edited = env
        .edit_pr(7, Some("Edited"), Some("New body"), &["tested".to_string()])
        .expect("edit pr");
    assert_eq!(edited.number, 128);
    assert_eq!(env.pr_edit_call_log[0].number, 7);
    assert_eq!(env.pr_edit_call_log[0].title.as_deref(), Some("Edited"));
    assert_eq!(env.fetch_pr(7).expect("fetch pr").number, 128);
    assert_eq!(env.pr_view_call_log, vec![7]);

    env.comment_on_pr(7, "looks good").expect("comment");
    assert_eq!(env.pr_comments, vec![(7, "looks good".to_string())]);

    env.seed_pr_reviews(
        7,
        vec![PrReview {
            id: "review-1".to_string(),
            state: "APPROVED".to_string(),
            body: "Ship it".to_string(),
            submitted_at: "2026-04-20T10:00:00Z".to_string(),
            author: "codex".to_string(),
        }],
    );
    assert_eq!(env.fetch_pr_reviews(7).expect("reviews").len(), 1);
    assert_eq!(env.pr_reviews_call_log, vec![7]);

    env.seed_pr_review_threads(
        7,
        vec![
            PrReviewThread {
                id: "thread-1".to_string(),
                is_resolved: false,
                is_outdated: false,
                path: "src/main.rs".to_string(),
                line: Some(10),
                comments: vec![crate::cli::PrReviewThreadComment {
                    id: "comment-1".to_string(),
                    body: "Please add tests".to_string(),
                    created_at: "2026-04-20T10:00:00Z".to_string(),
                    updated_at: "2026-04-20T10:00:00Z".to_string(),
                    author: "reviewer".to_string(),
                }],
            },
            PrReviewThread {
                id: "thread-2".to_string(),
                is_resolved: true,
                is_outdated: false,
                path: "src/lib.rs".to_string(),
                line: Some(20),
                comments: Vec::new(),
            },
        ],
    );
    assert_eq!(env.fetch_pr_review_threads(7).expect("threads").len(), 2);
    assert_eq!(
        env.reply_and_resolve_pr_review_threads(7, "done")
            .expect("reply"),
        1
    );
    assert_eq!(
        env.pr_reply_and_resolve_call_log,
        vec![(7, "done".to_string())]
    );

    env.seed_pr_checks(
        7,
        PrChecksSummary {
            summary: "All green".to_string(),
            ci_status: "SUCCESS".to_string(),
            merge_status: "CLEAN".to_string(),
            review_status: "APPROVED".to_string(),
            checks: Vec::new(),
        },
    );
    assert_eq!(env.fetch_pr_checks(7).expect("checks").summary, "All green");
    assert_eq!(env.pr_checks_call_log, vec![7]);

    env.seed_run_log(90, "run log");
    env.seed_job_log(91, "job log");
    assert_eq!(env.fetch_actions_run_log(90).expect("run log"), "run log");
    assert_eq!(env.fetch_actions_job_log(91).expect("job log"), "job log");
    assert_eq!(env.run_log_call_log, vec![90]);
    assert_eq!(env.job_log_call_log, vec![91]);

    let output = env
        .run_internal_command(&["gwt".to_string(), "issue".to_string()], "")
        .expect("internal command");
    assert_eq!(output.status, 2);
    assert_eq!(env.internal_command_call_log.len(), 1);
    assert_eq!(
        env.internal_command_call_log[0],
        InternalCommandCall {
            args: vec!["gwt".to_string(), "issue".to_string()],
            stdin: String::new(),
        }
    );
}

#[test]
fn default_cli_env_routes_gh_backed_methods_and_internal_dispatch() {
    with_fake_gh(|repo_path| {
        let cache_root = repo_path.join(".cache");
        let mut env = DefaultCliEnv::new_with_client_factory_and_cache_root(
            "akiojin",
            "gwt",
            repo_path.to_path_buf(),
            cache_root,
            failing_factory(Arc::new(AtomicUsize::new(0))),
        );

        let linked = env.fetch_linked_prs(IssueNumber(42)).expect("linked prs");
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0].number, 12);

        let current = env
            .fetch_current_pr()
            .expect("current pr")
            .expect("current pr exists");
        assert_eq!(current.number, 12);

        let created = env
            .create_pr(
                "develop",
                Some("feature/coverage"),
                "Raise coverage",
                "Body",
                &["coverage".to_string()],
                true,
            )
            .expect("create pr");
        assert_eq!(created.number, 12);

        let edited = env
            .edit_pr(12, Some("Edited"), Some("Updated"), &["tested".to_string()])
            .expect("edit pr");
        assert_eq!(edited.number, 12);

        let fetched = env.fetch_pr(12).expect("fetch pr");
        assert_eq!(fetched.number, 12);

        env.comment_on_pr(12, "done").expect("comment");

        let reviews = env.fetch_pr_reviews(12).expect("reviews");
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].author, "reviewer");

        let threads = env.fetch_pr_review_threads(12).expect("threads");
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].path, "src/lib.rs");

        assert_eq!(
            env.reply_and_resolve_pr_review_threads(12, "done")
                .expect("reply and resolve"),
            1
        );

        let checks = env.fetch_pr_checks(12).expect("checks");
        assert_eq!(checks.checks.len(), 1);
        assert_eq!(checks.checks[0].conclusion, "SUCCESS");

        assert_eq!(
            env.fetch_actions_run_log(90).expect("run log").trim(),
            "run log 90"
        );
        assert_eq!(
            env.fetch_actions_job_log(91).expect("job log"),
            "job log 91"
        );

        let note_path = repo_path.join("note.md");
        fs::write(&note_path, "hello").expect("write note");
        assert_eq!(
            env.read_file(note_path.to_str().expect("utf8 path"))
                .expect("read file"),
            "hello"
        );

        // `DefaultCliEnv` spawns `current_exe()`, which is the Rust test harness in unit tests.
        // Use `--help` so the spawned binary exits successfully regardless of whether it is the
        // real CLI binary or the harness wrapper.
        let output = env
            .run_internal_command(&["gwt".to_string(), "--help".to_string()], "")
            .expect("internal dispatch");
        assert_eq!(output.status, 0);
    });
}

#[test]
fn client_ref_forwards_issue_client_methods_to_the_underlying_fake_client() {
    let client = FakeIssueClient::new();
    client.seed(sample_issue_snapshot(7, &["gwt-spec", "phase/in-progress"]));
    let client_ref = ClientRef { inner: &client };

    assert!(matches!(
        client_ref.fetch(IssueNumber(7), None).expect("fetch"),
        gwt_github::client::FetchResult::Updated(_)
    ));
    client_ref
        .patch_body(IssueNumber(7), "Updated body")
        .expect("patch body");
    client_ref
        .patch_title(IssueNumber(7), "Updated title")
        .expect("patch title");
    client_ref
        .patch_comment(gwt_github::client::CommentId(9), "Updated comment")
        .expect("patch comment");
    client_ref
        .create_comment(IssueNumber(7), "Another comment")
        .expect("create comment");

    let created = client_ref
        .create_issue("New issue", "Body", &["bug".to_string()])
        .expect("create issue");
    client_ref
        .set_labels(created.number, &["chore".to_string()])
        .expect("set labels");
    client_ref
        .set_state(created.number, gwt_github::IssueState::Closed)
        .expect("set state");

    let specs = client_ref
        .list_spec_issues(&SpecListFilter::default())
        .expect("list spec issues");
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].number, IssueNumber(7));

    let call_log = client.call_log();
    for expected in [
        "fetch:#7",
        "patch_body:#7",
        "patch_title:#7",
        "patch_comment:comment:9",
        "create_comment:#7",
        "create_issue:#8",
        "set_labels:#8",
        "set_state:#8",
        "list_spec_issues:*",
    ] {
        assert!(
            call_log.iter().any(|entry| entry == expected),
            "missing forwarded call {expected:?} in {call_log:?}"
        );
    }
}
