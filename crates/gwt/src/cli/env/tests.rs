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

use gwt_agent::session::GWT_SESSION_ID_ENV;
use gwt_core::workspace_projection::load_or_default_workspace_projection;
use gwt_git::PrStatus;
use gwt_github::{
    client::{fake::FakeIssueClient, IssueClient},
    IssueNumber, SpecListFilter,
};

use super::*;

fn sample_pr_status() -> PrStatus {
    PrStatus {
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
    [api, endpoint] if api == "api" && endpoint == "repos/akiojin/gwt/pulls/12" => {
        println!("{{\"number\":12,\"node_id\":\"PR_node_12\",\"draft\":true}}");
        ExitCode::SUCCESS
    }
    [api, endpoint] if api == "api" && endpoint.starts_with("repos/akiojin/gwt/labels/") => {
        if endpoint.ends_with("/tested") {
            println!("{{\"name\":\"tested\",\"color\":\"aabbcc\"}}");
            ExitCode::SUCCESS
        } else {
            eprintln!("gh: Not Found (HTTP 404)");
            ExitCode::FAILURE
        }
    }
    [api, method_flag, method, endpoint, ..]
        if api == "api"
            && method_flag == "--method"
            && method == "PATCH"
            && endpoint.contains("/pulls/") =>
    {
        println!("{{\"number\":12}}");
        ExitCode::SUCCESS
    }
    [api, method_flag, method, endpoint, ..]
        if api == "api"
            && method_flag == "--method"
            && method == "POST"
            && endpoint.contains("/labels") =>
    {
        println!("[]");
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
        } else if joined.contains("markPullRequestReadyForReview") {
            println!("{{\"data\":{{\"markPullRequestReadyForReview\":{{\"pullRequest\":{{\"number\":12,\"isDraft\":false}}}}}}}}");
        } else if joined.contains("convertPullRequestToDraft") {
            println!("{{\"data\":{{\"convertPullRequestToDraft\":{{\"pullRequest\":{{\"number\":12,\"isDraft\":true}}}}}}}}");
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
    // Pin the child's working directory: parallel tests may set the
    // process-wide CWD to a tempdir that gets dropped, and rustc refuses to
    // start from a deleted CWD ("Current directory is invalid", #3006).
    let status = gwt_core::process::hidden_command("rustc")
        .current_dir(bin_dir)
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
    let _env_lock = crate::env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            will_close_target: true,
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
            PrReviewThread {
                id: "thread-3".to_string(),
                is_resolved: false,
                is_outdated: true,
                path: "src/old.rs".to_string(),
                line: None,
                comments: Vec::new(),
            },
        ],
    );
    assert_eq!(env.fetch_pr_review_threads(7).expect("threads").len(), 3);
    assert_eq!(
        env.reply_and_resolve_pr_review_threads(7, "done")
            .expect("reply"),
        2
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
fn dispatch_accepts_json_envelope_workspace_update_without_argv_flags() {
    let _env_lock = crate::env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("home tempdir");
    let _home = crate::cli::test_support::ScopedEnvVar::set("HOME", home.path());
    let _session = crate::cli::test_support::ScopedEnvVar::unset(GWT_SESSION_ID_ENV);
    let mut env = TestEnv::new(temp.path().to_path_buf());
    env.stdin = r#"{
        "schema_version": 1,
        "operation": "workspace.update",
        "params": {
            "agent_session": "session-json",
            "purpose": "JSON envelope contract",
            "current_focus": "writing RED tests"
        }
    }"#
    .to_string();

    let code = dispatch(&mut env, &["gwtd".to_string()]);

    assert_eq!(
        code,
        0,
        "JSON envelope dispatch should succeed, stderr: {}",
        String::from_utf8_lossy(&env.stderr)
    );
    let stdout: serde_json::Value =
        serde_json::from_slice(&env.stdout).expect("parse JSON envelope response");
    assert_eq!(
        stdout.get("ok").and_then(|value| value.as_bool()),
        Some(true),
        "JSON envelope output must be machine-readable success JSON, got: {}",
        String::from_utf8_lossy(&env.stdout)
    );
    let projection =
        load_or_default_workspace_projection(temp.path()).expect("load workspace projection");
    let agent = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == "session-json")
        .expect("agent upserted by workspace update");
    assert_eq!(
        agent.title_summary.as_deref(),
        Some("JSON envelope contract")
    );
    assert_eq!(agent.current_focus.as_deref(), Some("writing RED tests"));
}

#[test]
fn dispatch_json_envelope_hook_health_returns_managed_health_json() {
    let temp = tempfile::tempdir().expect("tempdir");
    gwt_skills::generate_settings_local(temp.path()).expect("claude hooks");
    gwt_skills::generate_codex_hooks(temp.path()).expect("codex hooks");
    let runtime_path = temp.path().join("runtime-state.json");
    crate::cli::hook::runtime_state::write_for_event(&runtime_path, "PreToolUse")
        .expect("runtime state");
    let profile_path = temp.path().join("profile.jsonl");
    fs::write(
        &profile_path,
        serde_json::to_string(&serde_json::json!({
            "event": "PreToolUse",
            "handler": "workflow-policy",
            "status": "ok",
            "duration_ms": 1400.0,
            "occurred_at": "2026-06-17T00:00:00.000Z"
        }))
        .unwrap(),
    )
    .expect("profile");
    let mut env = TestEnv::new(temp.path().to_path_buf());
    env.stdin = serde_json::json!({
        "schema_version": 1,
        "operation": "hook.health",
        "params": {
            "runtime_state_path": runtime_path,
            "profile_path": profile_path
        }
    })
    .to_string();

    let code = dispatch(&mut env, &["gwtd".to_string()]);

    assert_eq!(
        code,
        0,
        "hook.health JSON envelope should succeed, stderr: {}",
        String::from_utf8_lossy(&env.stderr)
    );
    let stdout: serde_json::Value =
        serde_json::from_slice(&env.stdout).expect("parse JSON envelope response");
    assert_eq!(stdout["ok"].as_bool(), Some(true));
    assert_eq!(stdout["operation"].as_str(), Some("hook.health"));
    let health: serde_json::Value =
        serde_json::from_str(stdout["output"].as_str().expect("output string"))
            .expect("parse hook health output");
    assert_eq!(health["status"].as_str(), Some("needs_attention"));
    assert_eq!(health["last_event"].as_str(), Some("PreToolUse"));
    assert_eq!(
        health["slow_handlers"][0]["handler"].as_str(),
        Some("workflow-policy")
    );
}

#[test]
fn dispatch_json_envelope_hook_doctor_can_repair_missing_managed_configs() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join(".codex")).expect("codex dir");
    let mut env = TestEnv::new(temp.path().to_path_buf());
    env.stdin = serde_json::json!({
        "schema_version": 1,
        "operation": "hook.doctor",
        "params": {
            "repair": true
        }
    })
    .to_string();

    let code = dispatch(&mut env, &["gwtd".to_string()]);

    assert_eq!(
        code,
        0,
        "hook.doctor JSON envelope should succeed, stderr: {}",
        String::from_utf8_lossy(&env.stderr)
    );
    assert!(temp.path().join(".codex/hooks.json").exists());
    let stdout: serde_json::Value =
        serde_json::from_slice(&env.stdout).expect("parse JSON envelope response");
    let doctor: serde_json::Value =
        serde_json::from_str(stdout["output"].as_str().expect("output string"))
            .expect("parse hook doctor output");
    assert_eq!(doctor["repair"]["repaired"].as_bool(), Some(true));
    assert_eq!(doctor["health"]["status"].as_str(), Some("inactive"));
}

#[test]
fn dispatch_json_envelope_board_post_rejects_purpose_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut env = TestEnv::new(temp.path().to_path_buf());
    env.stdin = r#"{
        "schema_version": 1,
        "operation": "board.post",
        "params": {
            "kind": "status",
            "body": "現在の状態: Board投稿です。",
            "purpose": "Should not update title"
        }
    }"#
    .to_string();

    let code = dispatch(&mut env, &["gwtd".to_string()]);

    assert_eq!(code, 2);
    let stderr = String::from_utf8(env.stderr.clone()).expect("stderr utf8");
    assert!(
        stderr.contains("board.post") && stderr.contains("purpose"),
        "JSON envelope must reject Board purpose mutation, got: {stderr}"
    );
}

#[test]
fn dispatch_json_envelope_actions_logs_uses_json_params() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut env = TestEnv::new(temp.path().to_path_buf());
    env.seed_run_log(273, "run log from JSON envelope");
    env.stdin = r#"{
        "schema_version": 1,
        "operation": "actions.logs",
        "params": {
            "run_id": 273
        }
    }"#
    .to_string();

    let code = dispatch(&mut env, &["gwtd".to_string()]);

    assert_eq!(
        code,
        0,
        "actions.logs JSON envelope should succeed, stderr: {}",
        String::from_utf8_lossy(&env.stderr)
    );
    assert_eq!(env.run_log_call_log, vec![273]);
    let stdout = String::from_utf8(env.stdout.clone()).expect("stdout utf8");
    assert!(stdout.contains(r#""operation":"actions.logs""#), "{stdout}");
    assert!(stdout.contains("run log from JSON envelope"), "{stdout}");
}

#[test]
fn dispatch_json_envelope_issue_create_uses_body_param() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut env = TestEnv::new(temp.path().to_path_buf());
    env.stdin = r#"{
        "schema_version": 1,
        "operation": "issue.create",
        "params": {
            "title": "JSON body issue",
            "body": "Body supplied directly through params.body",
            "labels": ["bug", "agent"]
        }
    }"#
    .to_string();

    let code = dispatch(&mut env, &["gwtd".to_string()]);

    assert_eq!(
        code,
        0,
        "issue.create JSON envelope should succeed, stderr: {}",
        String::from_utf8_lossy(&env.stderr)
    );
    let snapshot = match env
        .client
        .fetch(IssueNumber(1), None)
        .expect("created issue fetch")
    {
        gwt_github::client::FetchResult::Updated(snapshot) => snapshot,
        gwt_github::client::FetchResult::NotModified => panic!("fresh fetch should update"),
    };
    assert_eq!(snapshot.title, "JSON body issue");
    assert_eq!(snapshot.body, "Body supplied directly through params.body");
    assert_eq!(snapshot.labels, vec!["bug", "agent"]);
}

#[test]
fn dispatch_json_envelope_pr_create_uses_body_param() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut env = TestEnv::new(temp.path().to_path_buf());
    env.seed_created_pr(sample_pr_status());
    env.stdin = r#"{
        "schema_version": 1,
        "operation": "pr.create",
        "params": {
            "base": "develop",
            "head": "work/json-envelope",
            "title": "Wire JSON envelope",
            "body": "PR body supplied directly through params.body",
            "labels": ["agent"],
            "draft": true
        }
    }"#
    .to_string();

    let code = dispatch(&mut env, &["gwtd".to_string()]);

    assert_eq!(
        code,
        0,
        "pr.create JSON envelope should succeed, stderr: {}",
        String::from_utf8_lossy(&env.stderr)
    );
    assert_eq!(env.pr_create_call_log.len(), 1);
    let call = &env.pr_create_call_log[0];
    assert_eq!(call.base, "develop");
    assert_eq!(call.head.as_deref(), Some("work/json-envelope"));
    assert_eq!(call.title, "Wire JSON envelope");
    assert_eq!(call.body, "PR body supplied directly through params.body");
    assert_eq!(call.labels, vec!["agent"]);
    assert!(call.draft);
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

        let readied = env.mark_pr_ready(12).expect("mark pr ready");
        assert_eq!(readied.number, 12);

        let drafted = env.convert_pr_to_draft(12).expect("convert pr to draft");
        assert_eq!(drafted.number, 12);

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

// Issue #3080: the CLI error prefix must reflect the invoked binary name
// (`gwt` vs `gwtd`), not a hardcoded `"gwt"`. `gwt` and `gwtd` share this
// `dispatch()`, so a hardcoded prefix makes `gwtd` errors read as `gwt ...`,
// which misleads users into thinking the wrong binary was used.

fn dispatch_stderr(program: &str) -> String {
    let mut env = TestEnv::new(PathBuf::from("cache-root"));
    let args = vec![program.to_string(), "board".to_string(), "list".to_string()];
    // `board list` fails at parse time (board only has `show`/`post`), so the
    // error path runs without needing a repo or board backend.
    let code = dispatch(&mut env, &args);
    assert_eq!(code, 2, "parse error must exit with code 2");
    String::from_utf8(env.stderr.clone()).expect("stderr is utf8")
}

#[test]
fn dispatch_error_prefix_reflects_gwtd_binary() {
    let stderr = dispatch_stderr("gwtd");
    assert!(
        stderr.starts_with("gwtd board: unknown subcommand: list"),
        "gwtd invocation must use a `gwtd` prefix, got: {stderr:?}"
    );
}

#[test]
fn dispatch_error_prefix_uses_basename_of_full_path() {
    let stderr = dispatch_stderr("/Applications/GWT.app/Contents/MacOS/gwtd");
    assert!(
        stderr.starts_with("gwtd board:"),
        "full-path invocation must derive `gwtd` from the basename, got: {stderr:?}"
    );
}

#[test]
fn dispatch_error_prefix_reflects_gwt_binary() {
    let stderr = dispatch_stderr("gwt");
    assert!(
        stderr.starts_with("gwt board:"),
        "gwt invocation must keep the `gwt` prefix, got: {stderr:?}"
    );
}

// Regression guard for the OTHER changed line: the run-error arm
// (`dispatch()` exit code 1, reached when `run()` returns `Err`). The three
// tests above only drive the parse-error arm (exit code 2), so without this a
// future single-line revert of the run-error arm would silently reintroduce
// #3080 for execution failures. `board post -f <unseeded path>` parses
// successfully then fails reading the missing file before any session/board
// access, so it reaches the run-error arm deterministically and offline.
#[test]
fn dispatch_run_error_prefix_reflects_gwtd_binary() {
    let mut env = TestEnv::new(PathBuf::from("cache-root"));
    let args = vec![
        "gwtd".to_string(),
        "board".to_string(),
        "post".to_string(),
        "--kind".to_string(),
        "note".to_string(),
        "-f".to_string(),
        "/no/such/file".to_string(),
    ];
    let code = dispatch(&mut env, &args);
    assert_eq!(code, 1, "run error must exit with code 1");
    let stderr = String::from_utf8(env.stderr.clone()).expect("stderr is utf8");
    assert!(
        stderr.starts_with("gwtd board:"),
        "run-error arm must also use the invoked binary prefix, got: {stderr:?}"
    );
}

// Lock the `program_name` contract directly (it is module-private but reachable
// via `use super::*`): the `.exe` stripping and the fallback-to-"gwt" branches
// are not reachable through `dispatch()` with real binary names.
#[test]
fn program_name_strips_exe_extension() {
    assert_eq!(program_name(&["gwtd.exe".to_string()]), "gwtd");
    assert_eq!(program_name(&["gwt.exe".to_string()]), "gwt");
}

#[test]
fn program_name_falls_back_to_gwt_for_unusable_args() {
    assert_eq!(program_name(&[]), "gwt");
    assert_eq!(program_name(&[String::new()]), "gwt");
    assert_eq!(program_name(&["/".to_string()]), "gwt");
}
