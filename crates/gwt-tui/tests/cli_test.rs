//! Integration tests for the `gwt issue spec` CLI dispatch (SPEC-12 Phase 6).

use gwt_github::client::{IssueNumber, IssueSnapshot, IssueState, UpdatedAt};
use gwt_tui::cli::{
    dispatch, parse_issue_args, should_dispatch_cli, CliCommand, CliParseError, TestEnv,
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
fn red_70_should_dispatch_cli_when_first_arg_is_issue_or_hook() {
    assert!(should_dispatch_cli(&argv(&["gwt", "issue"])));
    assert!(should_dispatch_cli(&argv(&["gwt", "issue", "spec", "42"])));
    assert!(should_dispatch_cli(&argv(&[
        "gwt",
        "hook",
        "runtime-state",
        "PreToolUse"
    ])));
    assert!(should_dispatch_cli(&argv(&[
        "gwt",
        "hook",
        "block-git-branch-ops"
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
    let cmd = parse_hook_args(&[s("block-git-branch-ops")]).unwrap();
    assert_eq!(
        cmd,
        CliCommand::Hook {
            name: "block-git-branch-ops".to_string(),
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
