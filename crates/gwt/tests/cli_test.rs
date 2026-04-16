//! Integration tests for the `gwt issue spec` CLI dispatch (SPEC-12 Phase 6).

use gwt::cli::{
    dispatch, parse_actions_args, parse_issue_args, parse_pr_args, should_dispatch_cli, CliCommand,
    CliParseError, LinkedPrSummary, PrCheckItem, PrChecksSummary, PrCreateCall, PrEditCall,
    PrReview, PrReviewThread, PrReviewThreadComment, TestEnv,
};
use gwt_git::PrStatus;
use gwt_github::{
    client::{CommentId, CommentSnapshot, IssueNumber, IssueSnapshot, IssueState, UpdatedAt},
    Cache, SectionName,
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
    assert!(should_dispatch_cli(&argv(&[
        "gwt",
        "hook",
        "workflow-policy"
    ])));
    assert!(should_dispatch_cli(&argv(&[
        "gwt",
        "hook",
        "coordination-event",
        "SessionStart"
    ])));
    assert!(!should_dispatch_cli(&argv(&["gwt"])));
    assert!(!should_dispatch_cli(&argv(&["gwt", "/some/repo/path"])));
}

#[test]
fn red_90_parse_hook_runtime_state() {
    use gwt::cli::parse_hook_args;
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
    use gwt::cli::parse_hook_args;
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
fn red_92_parse_hook_workflow_policy_without_args() {
    use gwt::cli::parse_hook_args;
    let cmd = parse_hook_args(&[s("workflow-policy")]).unwrap();
    assert_eq!(
        cmd,
        CliCommand::Hook {
            name: "workflow-policy".to_string(),
            rest: vec![],
        }
    );
}

#[test]
fn red_92a_parse_hook_coordination_event() {
    use gwt::cli::parse_hook_args;
    let cmd = parse_hook_args(&[s("coordination-event"), s("SessionStart")]).unwrap();
    assert_eq!(
        cmd,
        CliCommand::Hook {
            name: "coordination-event".to_string(),
            rest: vec!["SessionStart".to_string()],
        }
    );
}

#[test]
fn red_93_parse_hook_empty_is_usage_error() {
    use gwt::cli::{parse_hook_args, CliParseError};
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
fn dispatch_hook_coordination_event_missing_event_exits_2() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let code = dispatch(&mut env, &argv(&["gwt", "hook", "coordination-event"]));
    assert_eq!(code, 2, "coordination-event without <event> should exit 2");
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
fn red_94_parse_issue_view() {
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
fn red_96a_parse_issue_comment_rejects_trailing_args() {
    let err = parse_issue_args(&[
        s("comment"),
        s("42"),
        s("-f"),
        s("/tmp/comment.md"),
        s("extra"),
    ])
    .unwrap_err();
    assert!(matches!(err, CliParseError::Usage));
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
fn red_104a_parse_pr_create() {
    let cmd = parse_pr_args(&[
        s("create"),
        s("--base"),
        s("develop"),
        s("--head"),
        s("feature/hooks"),
        s("--title"),
        s("feat(hooks): canonical gwt pr create"),
        s("-f"),
        s("/tmp/pr-body.md"),
        s("--label"),
        s("release"),
        s("--draft"),
    ])
    .unwrap();
    assert_eq!(
        cmd,
        CliCommand::PrCreate {
            base: "develop".into(),
            head: Some("feature/hooks".into()),
            title: "feat(hooks): canonical gwt pr create".into(),
            file: "/tmp/pr-body.md".into(),
            labels: vec!["release".into()],
            draft: true,
        }
    );
}

#[test]
fn red_104b_parse_pr_edit() {
    let cmd = parse_pr_args(&[
        s("edit"),
        s("42"),
        s("--title"),
        s("feat(hooks): updated title"),
        s("-f"),
        s("/tmp/pr-body.md"),
        s("--add-label"),
        s("release"),
    ])
    .unwrap();
    assert_eq!(
        cmd,
        CliCommand::PrEdit {
            number: 42,
            title: Some("feat(hooks): updated title".into()),
            file: Some("/tmp/pr-body.md".into()),
            add_labels: vec!["release".into()],
        }
    );
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
fn red_105e_parse_pr_fixed_arity_subcommands_reject_trailing_args() {
    let cases = [
        vec![s("current"), s("extra")],
        vec![s("view"), s("42"), s("extra")],
        vec![
            s("comment"),
            s("42"),
            s("-f"),
            s("/tmp/reply.md"),
            s("extra"),
        ],
        vec![s("reviews"), s("42"), s("extra")],
        vec![s("review-threads"), s("42"), s("extra")],
        vec![
            s("review-threads"),
            s("reply-and-resolve"),
            s("42"),
            s("-f"),
            s("/tmp/reply.md"),
            s("extra"),
        ],
        vec![s("checks"), s("42"), s("extra")],
    ];

    for args in cases {
        let err = parse_pr_args(&args).unwrap_err();
        assert!(
            matches!(err, CliParseError::Usage),
            "expected usage for {:?}, got {:?}",
            args,
            err
        );
    }
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

#[test]
fn red_107a_parse_actions_rejects_trailing_args() {
    let err = parse_actions_args(&[s("logs"), s("--run"), s("101"), s("extra")]).unwrap_err();
    assert!(matches!(err, CliParseError::Usage));

    let err = parse_actions_args(&[s("job-logs"), s("--job"), s("202"), s("extra")]).unwrap_err();
    assert!(matches!(err, CliParseError::Usage));
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
fn red_86_dispatch_spec_create_json_from_stdin() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.stdin = r#"{
  "background": [
    "The CLI should accept structured SPEC input.",
    "Formatting must stay consistent across create and edit flows."
  ],
  "user_stories": [
    {
      "title": "Create a spec from JSON",
      "priority": "p0",
      "statement": "As an agent, I want to create a SPEC from JSON, so that formatting is generated consistently.",
      "acceptance_scenarios": [
        "Given structured JSON on stdin, when the command runs, then a canonical spec section is created."
      ]
    }
  ],
  "edge_cases": [
    "Unknown fields should not break parsing."
  ],
  "functional_requirements": [
    "Accept structured JSON input on stdin.",
    "Render canonical Markdown for the spec section."
  ],
  "non_functional_requirements": [
    "Keep the formatting rules in one place."
  ],
  "success_criteria": [
    "The created spec follows the canonical heading and numbering rules."
  ]
}"#
        .to_string();

    let code = dispatch(
        &mut env,
        &argv(&[
            "gwt",
            "issue",
            "spec",
            "create",
            "--json",
            "--title",
            "SPEC: JSON create flow",
        ]),
    );
    assert_eq!(code, 0);

    let spec = Cache::new(tmp.path().to_path_buf())
        .read_section(IssueNumber(1), &SectionName("spec".into()))
        .unwrap()
        .expect("spec section");
    assert!(spec.contains("# JSON create flow"));
    assert!(spec.contains("## Background"));
    assert!(spec.contains("## User Stories"));
    assert!(spec.contains("### US-1: Create a spec from JSON (P0)"));
    assert!(spec.contains("- **FR-001**: Accept structured JSON input on stdin."));
    assert!(spec.contains("- **NFR-001**: Keep the formatting rules in one place."));
    assert!(spec.contains(
        "- **SC-001**: The created spec follows the canonical heading and numbering rules."
    ));
}

#[test]
fn red_87_dispatch_spec_rename_updates_issue_title_and_cache() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let snapshot = IssueSnapshot {
        number: IssueNumber(42),
        title: "SPEC-42: Old title".to_string(),
        body: mk_body_spec_and_tasks_in_body("# Old title", "- [ ] T-001"),
        labels: vec!["gwt-spec".to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("seeded"),
        comments: Vec::new(),
    };
    env.client.seed(snapshot.clone());
    Cache::new(tmp.path().to_path_buf())
        .write_snapshot(&snapshot)
        .unwrap();

    let code = dispatch(
        &mut env,
        &argv(&[
            "gwt",
            "issue",
            "spec",
            "42",
            "--rename",
            "SPEC-42: New title",
        ]),
    );
    assert_eq!(code, 0);

    let cached = Cache::new(tmp.path().to_path_buf())
        .load_entry(IssueNumber(42))
        .expect("cache entry");
    assert_eq!(cached.snapshot.title, "SPEC-42: New title");
    assert!(env
        .client
        .call_log()
        .iter()
        .any(|entry| entry == "patch_title:#42"));
}

#[test]
fn red_88_dispatch_spec_create_help_prints_json_schema() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());

    let code = dispatch(
        &mut env,
        &argv(&["gwt", "issue", "spec", "create", "--help"]),
    );
    assert_eq!(code, 0);

    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("gwt issue spec create --json"));
    assert!(out.contains("\"background\""));
    assert!(out.contains("\"user_stories\""));
    assert!(out.contains("\"functional_requirements\""));
    assert!(out.contains("\"success_criteria\""));
}

#[test]
fn red_89_dispatch_spec_edit_json_merges_named_sections() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let existing_spec = r#"# Existing title

## Background

Old background paragraph.

## User Stories

### US-1: Keep existing story (P1)

As a user, I want the existing story to remain, so that partial updates are safe.

**Acceptance Scenarios:**

1. Given the old spec, when a partial update runs, then untouched sections remain.

## Functional Requirements

- **FR-001**: Keep untouched requirements intact.

## Success Criteria

- **SC-001**: Old success criterion.
"#;
    seed(&env, 42, existing_spec, "- [ ] T-001");
    env.files.insert(
        "/virtual/spec-update.json".to_string(),
        r#"{
  "background": [
    "New background paragraph."
  ],
  "success_criteria": [
    "Updated success criterion."
  ]
}"#
        .to_string(),
    );

    let code = dispatch(
        &mut env,
        &argv(&[
            "gwt",
            "issue",
            "spec",
            "42",
            "--edit",
            "spec",
            "--json",
            "-f",
            "/virtual/spec-update.json",
        ]),
    );
    assert_eq!(code, 0);

    let spec = Cache::new(tmp.path().to_path_buf())
        .read_section(IssueNumber(42), &SectionName("spec".into()))
        .unwrap()
        .expect("spec section");
    assert!(spec.contains("New background paragraph."));
    assert!(spec.contains("### US-1: Keep existing story (P1)"));
    assert!(spec.contains("- **SC-001**: Updated success criterion."));
    assert!(!spec.contains("Old success criterion."));
}

#[test]
fn red_89a_dispatch_spec_edit_json_replace_rewrites_spec_section() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    let existing_spec = r#"# Existing title

## Background

Old background paragraph.

## User Stories

### US-1: Old story (P1)

As a user, I want the old story, so that the old content exists.

**Acceptance Scenarios:**

1. Given the old spec, when replace is not used, then this story would remain.

## Functional Requirements

- **FR-001**: Old requirement.
"#;
    seed(&env, 42, existing_spec, "- [ ] T-001");
    env.files.insert(
        "/virtual/spec-replace.json".to_string(),
        r#"{
  "background": [
    "Replacement background."
  ],
  "user_stories": [
    {
      "title": "Replacement story",
      "priority": "P0",
      "statement": "As a user, I want the replacement story, so that the section is regenerated.",
      "acceptance_scenarios": [
        "Given replace mode, when the command runs, then the old sections disappear."
      ]
    }
  ]
}"#
        .to_string(),
    );

    let code = dispatch(
        &mut env,
        &argv(&[
            "gwt",
            "issue",
            "spec",
            "42",
            "--edit",
            "spec",
            "--json",
            "--replace",
            "-f",
            "/virtual/spec-replace.json",
        ]),
    );
    assert_eq!(code, 0);

    let spec = Cache::new(tmp.path().to_path_buf())
        .read_section(IssueNumber(42), &SectionName("spec".into()))
        .unwrap()
        .expect("spec section");
    assert!(spec.contains("Replacement background."));
    assert!(spec.contains("### US-1: Replacement story (P0)"));
    assert!(!spec.contains("Old requirement."));
    assert!(!spec.contains("Old story"));
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
fn red_108a_dispatch_pr_create_uses_live_transport() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.files.insert(
        "/virtual/pr-body.md".to_string(),
        "## Summary\n\nBody".to_string(),
    );
    env.seed_created_pr(PrStatus {
        number: 88,
        title: "Created PR".to_string(),
        state: gwt_git::pr_status::PrState::Open,
        url: "https://example.com/pr/88".to_string(),
        ci_status: "PENDING".to_string(),
        mergeable: "UNKNOWN".to_string(),
        review_status: "REVIEW_REQUIRED".to_string(),
    });

    let code = dispatch(
        &mut env,
        &argv(&[
            "gwt",
            "pr",
            "create",
            "--base",
            "develop",
            "--head",
            "feature/hooks",
            "--title",
            "Created PR",
            "-f",
            "/virtual/pr-body.md",
            "--label",
            "release",
            "--draft",
        ]),
    );
    assert_eq!(code, 0);
    assert_eq!(
        env.pr_create_call_log,
        vec![PrCreateCall {
            base: "develop".to_string(),
            head: Some("feature/hooks".to_string()),
            title: "Created PR".to_string(),
            body: "## Summary\n\nBody".to_string(),
            labels: vec!["release".to_string()],
            draft: true,
        }]
    );

    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("created pull request"));
    assert!(out.contains("#88 [OPEN] Created PR"));
}

#[test]
fn red_108b_dispatch_pr_edit_uses_live_transport() {
    let tmp = TempDir::new().unwrap();
    let mut env = TestEnv::new(tmp.path().to_path_buf());
    env.files.insert(
        "/virtual/pr-body.md".to_string(),
        "## Summary\n\nUpdated".to_string(),
    );
    env.seed_pr(
        42,
        PrStatus {
            number: 42,
            title: "Updated PR".to_string(),
            state: gwt_git::pr_status::PrState::Open,
            url: "https://example.com/pr/42".to_string(),
            ci_status: "SUCCESS".to_string(),
            mergeable: "MERGEABLE".to_string(),
            review_status: "APPROVED".to_string(),
        },
    );

    let code = dispatch(
        &mut env,
        &argv(&[
            "gwt",
            "pr",
            "edit",
            "42",
            "--title",
            "Updated PR",
            "-f",
            "/virtual/pr-body.md",
            "--add-label",
            "release",
        ]),
    );
    assert_eq!(code, 0);
    assert_eq!(
        env.pr_edit_call_log,
        vec![PrEditCall {
            number: 42,
            title: Some("Updated PR".to_string()),
            body: Some("## Summary\n\nUpdated".to_string()),
            add_labels: vec!["release".to_string()],
        }]
    );

    let out = String::from_utf8(env.stdout.clone()).unwrap();
    assert!(out.contains("updated pull request"));
    assert!(out.contains("#42 [OPEN] Updated PR"));
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
