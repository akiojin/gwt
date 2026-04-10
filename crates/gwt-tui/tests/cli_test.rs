//! Integration tests for the `gwt issue spec` CLI dispatch (SPEC-12 Phase 6).

use gwt_git::PrStatus;
use gwt_github::client::{
    CommentId, CommentSnapshot, IssueNumber, IssueSnapshot, IssueState, UpdatedAt,
};
use gwt_github::Cache;
use gwt_tui::cli::{
    dispatch, parse_actions_args, parse_issue_args, parse_pr_args, should_dispatch_cli, CliCommand,
    CliParseError, LinkedPrSummary, PrCheckItem, PrChecksSummary, PrReview, PrReviewThread,
    PrReviewThreadComment, TestEnv,
};
use tempfile::TempDir;

fn s(v: &str) -> String {
    v.to_string()
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

// -----------------------------------------------------------------
// Argv parsing
// -----------------------------------------------------------------

#[test]
fn red_70_should_dispatch_cli_when_first_arg_is_cli_verb() {
    assert!(should_dispatch_cli(&argv(&["gwt", "issue"])));
    assert!(should_dispatch_cli(&argv(&["gwt", "issue", "spec", "42"])));
    assert!(should_dispatch_cli(&argv(&["gwt", "pr", "current"])));
    assert!(should_dispatch_cli(&argv(&[
        "gwt", "actions", "logs", "--run", "101"
    ])));
    assert!(should_dispatch_cli(&argv(&[
        "gwt",
        "hook",
        "runtime-state",
        "PreToolUse"
    ])));
    assert!(should_dispatch_cli(&argv(&[
        "gwt",
        "hook",
        "block-bash-policy"
    ])));
    assert!(!should_dispatch_cli(&argv(&["gwt"])));
    assert!(!should_dispatch_cli(&argv(&["gwt", "/some/repo/path"])));
}

#[test]
fn red_90_parse_hook_runtime_state() {
    use gwt_tui::cli::parse_hook_args;
    let cmd = parse_hook_args(&[s("runtime-state"), s("PreToolUse")]).unwrap();
    assert_eq!(
        cmd,
        CliCommand::Hook {
            name: "runtime-state".to_string(),
            rest: vec!["PreToolUse".to_string()],
        }
    );
}

#[test]
fn red_91_parse_hook_block_without_args() {
    use gwt_tui::cli::parse_hook_args;
    let cmd = parse_hook_args(&[s("block-bash-policy")]).unwrap();
    assert_eq!(
        cmd,
        CliCommand::Hook {
            name: "block-bash-policy".to_string(),
            rest: vec![],
        }
    );
}

#[test]
fn red_92_parse_hook_empty_is_usage_error() {
    use gwt_tui::cli::{parse_hook_args, CliParseError};
    let err = parse_hook_args(&[]).unwrap_err();
    assert!(matches!(err, CliParseError::Usage));
}

#[test]
fn dispatch_hook_runtime_state_without_env_is_silent_ok() {
    // T-025 (SPEC #1942): the old stub printed "not yet implemented"
    // and returned 0. The real handler now delegates to
    // `runtime_state::handle`, which returns Ok(()) as a silent no-op
    // when `GWT_SESSION_RUNTIME_PATH` is unset. Same exit code, quieter
    // stderr.
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let prev = std::env::var_os("GWT_SESSION_RUNTIME_PATH");
    std::env::remove_var("GWT_SESSION_RUNTIME_PATH");

    let code = dispatch(
        &mut env,
        &argv(&["gwt", "hook", "runtime-state", "PreToolUse"]),
    );

    if let Some(v) = prev {
        std::env::set_var("GWT_SESSION_RUNTIME_PATH", v);
    }

    assert_eq!(code, 0, "runtime-state with no env var should exit 0");
    let err_text = String::from_utf8(env.stderr.clone()).unwrap();
    assert!(
        err_text.is_empty(),
        "runtime-state no-op must not print to stderr, got {err_text:?}"
    );
}

#[test]
fn dispatch_hook_unknown_name_exits_2() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let code = dispatch(&mut env, &argv(&["gwt", "hook", "no-such-hook"]));
    assert_eq!(code, 2, "unknown hook should exit 2");
    let err_text = String::from_utf8(env.stderr.clone()).unwrap();
    assert!(
        err_text.contains("unknown hook 'no-such-hook'"),
        "stderr should name the unknown hook, got {err_text:?}"
    );
}

#[test]
fn dispatch_hook_runtime_state_missing_event_exits_2() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let code = dispatch(&mut env, &argv(&["gwt", "hook", "runtime-state"]));
    assert_eq!(code, 2, "runtime-state without <event> should exit 2");
    let err_text = String::from_utf8(env.stderr.clone()).unwrap();
    assert!(
        err_text.contains("missing <event> argument"),
        "stderr should explain the missing argument, got {err_text:?}"
    );
}

#[test]
fn red_71_parse_spec_read_all() {
    let cmd = parse_issue_args(&[s("spec"), s("42")]).unwrap();
    assert_eq!(cmd, CliCommand::SpecReadAll { number: 42 });
}

#[test]
fn red_72_parse_spec_read_section() {
    let cmd = parse_issue_args(&[s("spec"), s("42"), s("--section"), s("tasks")]).unwrap();
    assert_eq!(
        cmd,
        CliCommand::SpecReadSection {
            number: 42,
            section: "tasks".into()
        }
    );
}

#[test]
fn red_73_parse_spec_edit_section() {
    let cmd = parse_issue_args(&[
        s("spec"),
        s("42"),
        s("--edit"),
        s("tasks"),
        s("-f"),
        s("/tmp/new.md"),
    ])
    .unwrap();
    assert_eq!(
        cmd,
        CliCommand::SpecEditSection {
            number: 42,
            section: "tasks".into(),
            file: "/tmp/new.md".into()
        }
    );
}

#[test]
fn red_74_parse_spec_list_with_phase() {
    let cmd = parse_issue_args(&[s("spec"), s("list"), s("--phase"), s("implementation")]).unwrap();
    assert_eq!(
        cmd,
        CliCommand::SpecList {
            phase: Some("implementation".into()),
            state: None
        }
    );
}

#[test]
fn red_75_parse_spec_list_with_state() {
    let cmd = parse_issue_args(&[s("spec"), s("list"), s("--state"), s("closed")]).unwrap();
    assert_eq!(
        cmd,
        CliCommand::SpecList {
            phase: None,
            state: Some("closed".into())
        }
    );
}

#[test]
fn red_76_parse_missing_edit_file() {
    let err = parse_issue_args(&[s("spec"), s("42"), s("--edit"), s("tasks")]).unwrap_err();
    assert!(matches!(err, CliParseError::MissingFlag("-f")));
}

#[test]
fn red_77_parse_invalid_number() {
    let err = parse_issue_args(&[s("spec"), s("nope")]).unwrap_err();
    assert!(matches!(err, CliParseError::InvalidNumber(ref v) if v == "nope"));
}

#[test]
fn red_78_parse_unknown_subcommand() {
    let err = parse_issue_args(&[s("pull"), s("42")]).unwrap_err();
    assert!(matches!(err, CliParseError::UnknownSubcommand(_)));
}

#[test]
fn red_93_parse_issue_view() {
    let cmd = parse_issue_args(&[s("view"), s("42")]).unwrap();
    assert_eq!(
        cmd,
        CliCommand::IssueView {
            number: 42,
            refresh: false,
        }
    );
}

#[test]
fn red_94_parse_issue_comments_with_refresh() {
    let cmd = parse_issue_args(&[s("comments"), s("42"), s("--refresh")]).unwrap();
    assert_eq!(
        cmd,
        CliCommand::IssueComments {
            number: 42,
            refresh: true,
        }
    );
}

#[test]
fn red_95_parse_issue_create() {
    let cmd = parse_issue_args(&[
        s("create"),
        s("--title"),
        s("Plain issue"),
        s("-f"),
        s("/tmp/body.md"),
        s("--label"),
        s("bug"),
    ])
    .unwrap();
    assert_eq!(
        cmd,
        CliCommand::IssueCreate {
            title: "Plain issue".into(),
            file: "/tmp/body.md".into(),
            labels: vec!["bug".into()],
        }
    );
}

#[test]
fn red_96_parse_issue_comment() {
    let cmd = parse_issue_args(&[s("comment"), s("42"), s("-f"), s("/tmp/comment.md")]).unwrap();
    assert_eq!(
        cmd,
        CliCommand::IssueComment {
            number: 42,
            file: "/tmp/comment.md".into(),
        }
    );
}

#[test]
fn red_103_parse_pr_current() {
    let cmd = parse_pr_args(&[s("current")]).unwrap();
    assert_eq!(cmd, CliCommand::PrCurrent);
}

#[test]
fn red_104_parse_pr_view() {
    let cmd = parse_pr_args(&[s("view"), s("42")]).unwrap();
    assert_eq!(cmd, CliCommand::PrView { number: 42 });
}

#[test]
fn red_105_parse_pr_checks() {
    let cmd = parse_pr_args(&[s("checks"), s("42")]).unwrap();
    assert_eq!(cmd, CliCommand::PrChecks { number: 42 });
}

#[test]
fn red_105a_parse_pr_comment() {
    let cmd = parse_pr_args(&[s("comment"), s("42"), s("-f"), s("/tmp/reply.md")]).unwrap();
    assert_eq!(
        cmd,
        CliCommand::PrComment {
            number: 42,
            file: "/tmp/reply.md".into(),
        }
    );
}

#[test]
fn red_105b_parse_pr_reviews() {
    let cmd = parse_pr_args(&[s("reviews"), s("42")]).unwrap();
    assert_eq!(cmd, CliCommand::PrReviews { number: 42 });
}

#[test]
fn red_105c_parse_pr_review_threads() {
    let cmd = parse_pr_args(&[s("review-threads"), s("42")]).unwrap();
    assert_eq!(cmd, CliCommand::PrReviewThreads { number: 42 });
}

#[test]
fn red_105d_parse_pr_reply_and_resolve() {
    let cmd = parse_pr_args(&[
        s("review-threads"),
        s("reply-and-resolve"),
        s("42"),
        s("-f"),
        s("/tmp/reply.md"),
    ])
    .unwrap();
    assert_eq!(
        cmd,
        CliCommand::PrReviewThreadsReplyAndResolve {
            number: 42,
            file: "/tmp/reply.md".into(),
        }
    );
}

#[test]
fn red_106_parse_actions_logs() {
    let cmd = parse_actions_args(&[s("logs"), s("--run"), s("101")]).unwrap();
    assert_eq!(cmd, CliCommand::ActionsLogs { run_id: 101 });
}

#[test]
fn red_107_parse_actions_job_logs() {
    let cmd = parse_actions_args(&[s("job-logs"), s("--job"), s("202")]).unwrap();
    assert_eq!(cmd, CliCommand::ActionsJobLogs { job_id: 202 });
}

// -----------------------------------------------------------------
// End-to-end dispatch (with FakeIssueClient via TestEnv)
// -----------------------------------------------------------------

fn mk_body_spec_and_tasks_in_body(spec: &str, tasks: &str) -> String {
    format!(
        "<!-- gwt-spec id=42 version=1 -->\n\
<!-- sections:\n\
spec=body\n\
tasks=body\n\
-->\n\
\n\
<!-- artifact:spec BEGIN -->\n\
{spec}\n\
<!-- artifact:spec END -->\n\
\n\
<!-- artifact:tasks BEGIN -->\n\
{tasks}\n\
<!-- artifact:tasks END -->\n"
    )
}

fn seed(env: &TestEnv, number: u64, spec: &str, tasks: &str) {
    let snapshot = IssueSnapshot {
        number: IssueNumber(number),
        title: format!("Spec {number}"),
        body: mk_body_spec_and_tasks_in_body(spec, tasks),
        labels: vec!["gwt-spec".to_string(), "phase/review".to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("seeded"),
        comments: Vec::new(),
    };
    env.client.seed(snapshot);
}

#[test]
fn red_80_dispatch_spec_read_section() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    seed(&env, 42, "spec body", "tasks body");

    let code = dispatch(
        &mut env,
        &argv(&["gwt", "issue", "spec", "42", "--section", "tasks"]),
    );
    assert_eq!(code, 0);
    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("tasks body"));
}

#[test]
fn red_81_dispatch_spec_read_all_skips_missing() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    seed(&env, 42, "SPEC content", "- [ ] T-001");

    let code = dispatch(&mut env, &argv(&["gwt", "issue", "spec", "42"]));
    assert_eq!(code, 0);
    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("=== spec ==="));
    assert!(out.contains("SPEC content"));
    assert!(out.contains("=== tasks ==="));
    assert!(out.contains("- [ ] T-001"));
    // No plan section exists on this issue, so it must not show up.
    assert!(!out.contains("=== plan ==="));
}

#[test]
fn red_82_dispatch_spec_edit_section_from_file() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    seed(&env, 42, "spec body", "old tasks");
    env.files.insert(
        "/virtual/new.md".to_string(),
        "- [x] T-001 done".to_string(),
    );

    let code = dispatch(
        &mut env,
        &argv(&[
            "gwt",
            "issue",
            "spec",
            "42",
            "--edit",
            "tasks",
            "-f",
            "/virtual/new.md",
        ]),
    );
    assert_eq!(code, 0);

    // Verify the underlying client received a patch_body call that contains
    // the new tasks content.
    let log = env.client.call_log();
    assert!(log.iter().any(|l| l.starts_with("patch_body:#42")));
}

#[test]
fn red_83_dispatch_spec_list_filters_phase() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    // Seed three SPECs in different phases.
    env.client.seed(IssueSnapshot {
        number: IssueNumber(1),
        title: "draft one".to_string(),
        body: String::new(),
        labels: vec!["gwt-spec".to_string(), "phase/draft".to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("t1"),
        comments: Vec::new(),
    });
    env.client.seed(IssueSnapshot {
        number: IssueNumber(2),
        title: "impl two".to_string(),
        body: String::new(),
        labels: vec!["gwt-spec".to_string(), "phase/implementation".to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("t2"),
        comments: Vec::new(),
    });
    env.client.seed(IssueSnapshot {
        number: IssueNumber(3),
        title: "done three".to_string(),
        body: String::new(),
        labels: vec!["gwt-spec".to_string(), "phase/done".to_string()],
        state: IssueState::Closed,
        updated_at: UpdatedAt::new("t3"),
        comments: Vec::new(),
    });

    let code = dispatch(
        &mut env,
        &argv(&["gwt", "issue", "spec", "list", "--phase", "implementation"]),
    );
    assert_eq!(code, 0);
    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("#2"));
    assert!(out.contains("impl two"));
    assert!(!out.contains("draft one"));
    assert!(!out.contains("done three"));
}

#[test]
fn red_84_dispatch_invalid_usage_returns_nonzero() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let code = dispatch(&mut env, &argv(&["gwt", "issue"]));
    assert_ne!(code, 0);
    let err = String::from_utf8(env.stderr.clone()).unwrap();
    assert!(err.contains("usage"));
}

#[test]
fn red_85_dispatch_section_not_found_errors() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    seed(&env, 42, "s", "t");
    let code = dispatch(
        &mut env,
        &argv(&["gwt", "issue", "spec", "42", "--section", "plan"]),
    );
    assert_ne!(code, 0);
    let err = String::from_utf8(env.stderr.clone()).unwrap();
    assert!(err.contains("plan"));
}

#[test]
fn red_97_dispatch_issue_view_prefers_warm_cache() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let snapshot = IssueSnapshot {
        number: IssueNumber(42),
        title: "Cached title".to_string(),
        body: "Cached body".to_string(),
        labels: vec!["bug".to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("cached"),
        comments: Vec::new(),
    };
    Cache::new(tmp.path().to_path_buf())
        .write_snapshot(&snapshot)
        .unwrap();
    env.client.seed(IssueSnapshot {
        title: "Fetched title".to_string(),
        updated_at: UpdatedAt::new("fetched"),
        ..snapshot.clone()
    });

    let code = dispatch(&mut env, &argv(&["gwt", "issue", "view", "42"]));
    assert_eq!(code, 0);

    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("Cached title"));
    assert!(out.contains("Cached body"));
    assert!(
        env.client.call_log().is_empty(),
        "warm cache path must not fetch, got {:?}",
        env.client.call_log()
    );
}

#[test]
fn red_98_dispatch_issue_view_refresh_fetches_and_rewrites_cache() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let old_snapshot = IssueSnapshot {
        number: IssueNumber(42),
        title: "Old title".to_string(),
        body: "Old body".to_string(),
        labels: vec!["bug".to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("old"),
        comments: Vec::new(),
    };
    let new_snapshot = IssueSnapshot {
        title: "New title".to_string(),
        body: "New body".to_string(),
        updated_at: UpdatedAt::new("new"),
        ..old_snapshot.clone()
    };
    Cache::new(tmp.path().to_path_buf())
        .write_snapshot(&old_snapshot)
        .unwrap();
    env.client.seed(new_snapshot.clone());

    let code = dispatch(
        &mut env,
        &argv(&["gwt", "issue", "view", "42", "--refresh"]),
    );
    assert_eq!(code, 0);

    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("New title"));
    assert!(env
        .client
        .call_log()
        .iter()
        .any(|entry| entry == "fetch:#42"));

    let cached = Cache::new(tmp.path().to_path_buf())
        .load_entry(IssueNumber(42))
        .expect("cache entry");
    assert_eq!(cached.snapshot.title, "New title");
}

#[test]
fn red_99_dispatch_issue_comments_prefers_cache() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let snapshot = IssueSnapshot {
        number: IssueNumber(42),
        title: "Issue".to_string(),
        body: "Body".to_string(),
        labels: vec![],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("cached"),
        comments: vec![CommentSnapshot {
            id: CommentId(7),
            body: "Cached comment".to_string(),
            updated_at: UpdatedAt::new("cached"),
        }],
    };
    Cache::new(tmp.path().to_path_buf())
        .write_snapshot(&snapshot)
        .unwrap();

    let code = dispatch(&mut env, &argv(&["gwt", "issue", "comments", "42"]));
    assert_eq!(code, 0);
    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("Cached comment"));
    assert!(env.client.call_log().is_empty());
}

#[test]
fn red_100_dispatch_issue_create_updates_cache() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.files.insert(
        "/virtual/body.md".to_string(),
        "Plain issue body".to_string(),
    );

    let code = dispatch(
        &mut env,
        &argv(&[
            "gwt",
            "issue",
            "create",
            "--title",
            "Plain issue",
            "-f",
            "/virtual/body.md",
            "--label",
            "bug",
        ]),
    );
    assert_eq!(code, 0);
    assert!(
        env.client
            .call_log()
            .iter()
            .any(|entry| entry.starts_with("create_issue:#")),
        "create_issue must be called, got {:?}",
        env.client.call_log()
    );

    let cached = Cache::new(tmp.path().to_path_buf())
        .load_entry(IssueNumber(1))
        .expect("created issue cached");
    assert_eq!(cached.snapshot.title, "Plain issue");
    assert_eq!(cached.snapshot.body, "Plain issue body");
}

#[test]
fn red_101_dispatch_issue_comment_refreshes_cache_after_write() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.files.insert(
        "/virtual/comment.md".to_string(),
        "New comment body".to_string(),
    );
    env.client.seed(IssueSnapshot {
        number: IssueNumber(42),
        title: "Issue".to_string(),
        body: "Body".to_string(),
        labels: vec![],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("seed"),
        comments: Vec::new(),
    });

    let code = dispatch(
        &mut env,
        &argv(&["gwt", "issue", "comment", "42", "-f", "/virtual/comment.md"]),
    );
    assert_eq!(code, 0);

    let log = env.client.call_log();
    assert!(log.iter().any(|entry| entry == "create_comment:#42"));
    assert!(log.iter().any(|entry| entry == "fetch:#42"));

    let cached = Cache::new(tmp.path().to_path_buf())
        .load_entry(IssueNumber(42))
        .expect("updated cache");
    assert!(cached
        .snapshot
        .comments
        .iter()
        .any(|comment| comment.body == "New comment body"));
}

#[test]
fn red_102_dispatch_issue_linked_prs_is_cache_first() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.seed_linked_prs(
        42,
        vec![LinkedPrSummary {
            number: 10,
            title: "Cached linked PR".to_string(),
            state: "OPEN".to_string(),
            url: "https://example.com/pr/10".to_string(),
        }],
    );

    let code = dispatch(
        &mut env,
        &argv(&["gwt", "issue", "linked-prs", "42", "--refresh"]),
    );
    assert_eq!(code, 0);
    assert_eq!(env.linked_pr_calls(), vec![42]);

    env.stdout.clear();
    env.clear_linked_pr_calls();
    env.seed_linked_prs(
        42,
        vec![LinkedPrSummary {
            number: 11,
            title: "Fresh linked PR".to_string(),
            state: "OPEN".to_string(),
            url: "https://example.com/pr/11".to_string(),
        }],
    );

    let code = dispatch(&mut env, &argv(&["gwt", "issue", "linked-prs", "42"]));
    assert_eq!(code, 0);
    assert!(
        env.linked_pr_calls().is_empty(),
        "warm cache path must not re-fetch linked PRs"
    );
    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("Cached linked PR"));
    assert!(!out.contains("Fresh linked PR"));
}

#[test]
fn red_108_dispatch_pr_current_is_live_first() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.seed_current_pr(Some(PrStatus {
        number: 77,
        title: "Current PR".to_string(),
        state: gwt_git::pr_status::PrState::Open,
        url: "https://example.com/pr/77".to_string(),
        ci_status: "SUCCESS".to_string(),
        mergeable: "MERGEABLE".to_string(),
        review_status: "APPROVED".to_string(),
    }));

    let code = dispatch(&mut env, &argv(&["gwt", "pr", "current"]));
    assert_eq!(code, 0);
    assert_eq!(env.pr_current_call_count, 1);

    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("#77 [OPEN] Current PR"));
    assert!(out.contains("https://example.com/pr/77"));
}

#[test]
fn red_109_dispatch_pr_view_reads_live_data() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.seed_pr(
        42,
        PrStatus {
            number: 42,
            title: "Viewed PR".to_string(),
            state: gwt_git::pr_status::PrState::Merged,
            url: "https://example.com/pr/42".to_string(),
            ci_status: "SUCCESS".to_string(),
            mergeable: "UNKNOWN".to_string(),
            review_status: "APPROVED".to_string(),
        },
    );

    let code = dispatch(&mut env, &argv(&["gwt", "pr", "view", "42"]));
    assert_eq!(code, 0);
    assert_eq!(env.pr_view_call_log, vec![42]);

    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("#42 [MERGED] Viewed PR"));
    assert!(out.contains("mergeable: UNKNOWN"));
}

#[test]
fn red_109a_dispatch_pr_comment_posts_live_comment() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.files.insert(
        "/virtual/pr-comment.md".to_string(),
        "Looks good now.".to_string(),
    );

    let code = dispatch(
        &mut env,
        &argv(&["gwt", "pr", "comment", "42", "-f", "/virtual/pr-comment.md"]),
    );
    assert_eq!(code, 0);
    assert_eq!(env.pr_comments, vec![(42, "Looks good now.".to_string())]);

    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("created comment on PR #42"));
}

#[test]
fn red_109b_dispatch_pr_reviews_reads_live_data() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.seed_pr_reviews(
        42,
        vec![PrReview {
            id: "1".to_string(),
            state: "CHANGES_REQUESTED".to_string(),
            body: "Please add coverage.".to_string(),
            submitted_at: "2026-04-10T00:00:00Z".to_string(),
            author: "reviewer".to_string(),
        }],
    );

    let code = dispatch(&mut env, &argv(&["gwt", "pr", "reviews", "42"]));
    assert_eq!(code, 0);
    assert_eq!(env.pr_reviews_call_log, vec![42]);

    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("=== review:1 [CHANGES_REQUESTED] by reviewer"));
    assert!(out.contains("Please add coverage."));
}

#[test]
fn red_109c_dispatch_pr_review_threads_reads_live_data() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.seed_pr_review_threads(
        42,
        vec![PrReviewThread {
            id: "thread-1".to_string(),
            is_resolved: false,
            is_outdated: false,
            path: "src/lib.rs".to_string(),
            line: Some(12),
            comments: vec![PrReviewThreadComment {
                id: "comment-1".to_string(),
                body: "Please rename this variable.".to_string(),
                created_at: "2026-04-10T00:00:00Z".to_string(),
                updated_at: "2026-04-10T00:00:00Z".to_string(),
                author: "reviewer".to_string(),
            }],
        }],
    );

    let code = dispatch(&mut env, &argv(&["gwt", "pr", "review-threads", "42"]));
    assert_eq!(code, 0);
    assert_eq!(env.pr_review_threads_call_log, vec![42]);

    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("=== thread:thread-1 resolved=false"));
    assert!(out.contains("Please rename this variable."));
}

#[test]
fn red_109d_dispatch_pr_reply_and_resolve_targets_unresolved_threads() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.files.insert(
        "/virtual/reply.md".to_string(),
        "Fixed in latest commit.".to_string(),
    );
    env.seed_pr_review_threads(
        42,
        vec![
            PrReviewThread {
                id: "thread-1".to_string(),
                is_resolved: false,
                is_outdated: false,
                path: "src/lib.rs".to_string(),
                line: Some(12),
                comments: vec![],
            },
            PrReviewThread {
                id: "thread-2".to_string(),
                is_resolved: true,
                is_outdated: false,
                path: "src/main.rs".to_string(),
                line: Some(99),
                comments: vec![],
            },
        ],
    );

    let code = dispatch(
        &mut env,
        &argv(&[
            "gwt",
            "pr",
            "review-threads",
            "reply-and-resolve",
            "42",
            "-f",
            "/virtual/reply.md",
        ]),
    );
    assert_eq!(code, 0);
    assert_eq!(
        env.pr_reply_and_resolve_call_log,
        vec![(42, "Fixed in latest commit.".to_string())]
    );

    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("replied to and resolved 1 review threads on PR #42"));
}

#[test]
fn red_110_dispatch_pr_checks_renders_summary_and_checks() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.seed_pr_checks(
        42,
        PrChecksSummary {
            summary: "PR #42 | CI: FAILURE | Merge: CONFLICTING | Review: CHANGES_REQUESTED"
                .to_string(),
            ci_status: "FAILURE".to_string(),
            merge_status: "CONFLICTING".to_string(),
            review_status: "CHANGES_REQUESTED".to_string(),
            checks: vec![PrCheckItem {
                name: "test".to_string(),
                state: "COMPLETED".to_string(),
                conclusion: "FAILURE".to_string(),
                url: "https://example.com/runs/1".to_string(),
                started_at: "2026-04-10T00:00:00Z".to_string(),
                completed_at: "2026-04-10T00:01:00Z".to_string(),
                workflow: "CI".to_string(),
            }],
        },
    );

    let code = dispatch(&mut env, &argv(&["gwt", "pr", "checks", "42"]));
    assert_eq!(code, 0);
    assert_eq!(env.pr_checks_call_log, vec![42]);

    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("summary: PR #42 | CI: FAILURE"));
    assert!(out.contains("- test [COMPLETED / FAILURE]"));
    assert!(out.contains("workflow: CI"));
}

#[test]
fn red_111_dispatch_actions_logs_is_live_first() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.seed_run_log(101, "first run log");

    let code = dispatch(&mut env, &argv(&["gwt", "actions", "logs", "--run", "101"]));
    assert_eq!(code, 0);
    assert_eq!(env.run_log_call_log, vec![101]);
    let first = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(first.contains("first run log"));

    env.stdout.clear();
    env.seed_run_log(101, "fresh run log");
    let code = dispatch(&mut env, &argv(&["gwt", "actions", "logs", "--run", "101"]));
    assert_eq!(code, 0);
    assert_eq!(env.run_log_call_log, vec![101, 101]);
    let second = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(second.contains("fresh run log"));
}

#[test]
fn red_112_dispatch_actions_job_logs_is_live_first() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.seed_job_log(202, "first job log");

    let code = dispatch(
        &mut env,
        &argv(&["gwt", "actions", "job-logs", "--job", "202"]),
    );
    assert_eq!(code, 0);
    assert_eq!(env.job_log_call_log, vec![202]);
    let first = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(first.contains("first job log"));

    env.stdout.clear();
    env.seed_job_log(202, "fresh job log");
    let code = dispatch(
        &mut env,
        &argv(&["gwt", "actions", "job-logs", "--job", "202"]),
    );
    assert_eq!(code, 0);
    assert_eq!(env.job_log_call_log, vec![202, 202]);
    let second = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(second.contains("fresh job log"));
}
