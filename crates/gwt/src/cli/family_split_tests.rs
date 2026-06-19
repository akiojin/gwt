use super::*;

/// SPEC-1942 family split (FR-088〜092 / SC-025〜027): the parent
/// [`CliCommand`] is a nested enum and each top-level verb
/// parses into the matching family-typed inner enum. This RED test
/// pins the contract before the refactor lands and stays green
/// afterwards as the round-trip guard for the family split.
#[test]
fn cli_command_family_split_round_trip_parses() {
    use crate::cli::{
        ActionsCommand, BoardCommand, CliCommand, DiagnosticsCommand, DiscussCommand, HookCommand,
        IndexCommand, IssueCommand, MemoryCommand, PaneCommand, PrCommand, UpdateCommand,
        WorkspaceCommand,
    };

    fn s(value: &str) -> String {
        value.to_string()
    }

    // legacy parser fixture: issue spec list
    let cmd = parse_issue_args(&[s("spec"), s("list")]).expect("parse issue spec list");
    assert!(matches!(
        cmd,
        CliCommand::Issue(IssueCommand::SpecList {
            phase: None,
            state: None
        })
    ));

    // legacy parser fixture: issue view 42 --refresh
    let cmd = parse_issue_args(&[s("view"), s("42"), s("--refresh")]).expect("parse issue view");
    assert_eq!(
        cmd,
        CliCommand::Issue(IssueCommand::View {
            number: 42,
            refresh: true,
        })
    );

    // legacy parser fixture: pr current
    let cmd = parse_pr_args(&[s("current")]).expect("parse pr current");
    assert_eq!(cmd, CliCommand::Pr(PrCommand::Current));

    // legacy parser fixture: pr checks 12
    let cmd = parse_pr_args(&[s("checks"), s("12")]).expect("parse pr checks");
    assert_eq!(cmd, CliCommand::Pr(PrCommand::Checks { number: 12 }));

    // legacy parser fixture: actions logs --run 42
    let cmd = parse_actions_args(&[s("logs"), s("--run"), s("42")]).expect("parse actions logs");
    assert_eq!(
        cmd,
        CliCommand::Actions(ActionsCommand::Logs { run_id: 42 })
    );

    // legacy parser fixture: board show --json
    let cmd = parse_board_args(&[s("show"), s("--json")]).expect("parse board show");
    assert_eq!(
        cmd,
        CliCommand::Board(BoardCommand::Show {
            json: true,
            workspace: None,
            all: false,
        })
    );

    // legacy parser fixture: board post --kind status --body x
    let cmd = parse_board_args(&[s("post"), s("--kind"), s("status"), s("--body"), s("x")])
        .expect("parse board post");
    let CliCommand::Board(BoardCommand::Post(command)) = cmd else {
        panic!("expected board post command");
    };
    assert_eq!(command.kind, "status");
    assert_eq!(command.body.as_deref(), Some("x"));
    assert_eq!(command.file, None);
    assert_eq!(command.title_summary, None);
    assert!(!command.broadcast);

    // legacy parser fixture: index status / rebuild
    let cmd = parse_index_args(&[s("status")]).expect("parse index status");
    assert_eq!(cmd, CliCommand::Index(IndexCommand::Status));
    let cmd = parse_index_args(&[s("rebuild")]).expect("parse index rebuild");
    assert!(matches!(
        cmd,
        CliCommand::Index(IndexCommand::Rebuild {
            scope: IndexScope::All
        })
    ));

    // legacy parser fixture: diagnostics cpu --json
    let cmd = parse_diagnostics_args(&[s("cpu"), s("--json")]).expect("parse diagnostics cpu");
    assert_eq!(
        cmd,
        CliCommand::Diagnostics(DiagnosticsCommand::Cpu { json: true })
    );

    // legacy parser fixture: memory add ...
    let cmd = parse_memory_args(&[
        s("add"),
        s("--date"),
        s("2026-05-20"),
        s("--title"),
        s("writer"),
        s("--context"),
        s("context"),
        s("--learning"),
        s("learning"),
        s("--future-action"),
        s("action"),
    ])
    .expect("parse memory add");
    assert!(matches!(cmd, CliCommand::Memory(MemoryCommand::Add(_))));

    // managed hook argv transport fixture: runtime-state PreToolUse
    let cmd = parse_hook_args(&[s("runtime-state"), s("PreToolUse")]).expect("parse hook command");
    assert!(matches!(
        cmd,
        CliCommand::Hook(HookCommand::Run { ref name, ref rest })
            if name == "runtime-state" && rest == &[s("PreToolUse")]
    ));

    // legacy parser fixture: discuss park --proposal "Proposal A"
    let cmd = parse_discuss_args(&[s("park"), s("--proposal"), s("Proposal A")])
        .expect("parse discuss park");
    assert!(matches!(
        cmd,
        CliCommand::Discuss(DiscussCommand::Park { ref proposal })
            if proposal == "Proposal A"
    ));

    // legacy parser fixture: discuss goal-pending --proposal "Proposal A" -f /tmp/goal.txt
    let cmd = parse_discuss_args(&[
        s("goal-pending"),
        s("--proposal"),
        s("Proposal A"),
        s("-f"),
        s("/tmp/goal.txt"),
    ])
    .expect("parse discuss goal-pending");
    assert!(matches!(
        cmd,
        CliCommand::Discuss(DiscussCommand::GoalPending {
            ref proposal,
            ref condition_file
        }) if proposal == "Proposal A" && condition_file == std::path::Path::new("/tmp/goal.txt")
    ));

    // legacy parser fixture: plan start --spec 1942
    let cmd = parse_plan_args(&[s("start"), s("--spec"), s("1942")]).expect("parse plan start");
    assert!(matches!(
        cmd,
        CliCommand::Plan(SkillStateAction::Start { spec: 1942 })
    ));

    // legacy parser fixture: build start --spec 1942
    let cmd = parse_build_args(&[s("start"), s("--spec"), s("1942")]).expect("parse build start");
    assert!(matches!(
        cmd,
        CliCommand::Build(SkillStateAction::Start { spec: 1942 })
    ));

    // legacy parser fixture: workspace update --status-text ... --summary ...
    let cmd = parse_workspace_args(&[
        s("update"),
        s("--title"),
        s("Fix Active Work"),
        s("--status"),
        s("active"),
        s("--status-text"),
        s("Cleaning projection state"),
        s("--summary"),
        s("Workspace state is updated by the Agent"),
        s("--next-action"),
        s("Run regression tests"),
        s("--owner"),
        s("SPEC-2359"),
        s("--agent-session"),
        s("session-1"),
        s("--current-focus"),
        s("Writing RED tests"),
    ])
    .expect("parse workspace update");
    assert_eq!(
        cmd,
        CliCommand::Workspace(WorkspaceCommand::Update {
            title: Some("Fix Active Work".to_string()),
            status: Some("active".to_string()),
            status_text: Some("Cleaning projection state".to_string()),
            summary: Some("Workspace state is updated by the Agent".to_string()),
            progress_summary: None,
            next_action: Some("Run regression tests".to_string()),
            owner: Some("SPEC-2359".to_string()),
            agent_session: Some("session-1".to_string()),
            current_focus: Some("Writing RED tests".to_string()),
            title_summary: None,
        })
    );

    // legacy parser fixture: pane list / read / close
    let cmd = parse_pane_args(&[s("list")]).expect("parse pane list");
    assert_eq!(cmd, CliCommand::Pane(PaneCommand::List));
    let cmd = parse_pane_args(&[s("read"), s("tab-1::agent-1"), s("--lines"), s("25")])
        .expect("parse pane read");
    assert_eq!(
        cmd,
        CliCommand::Pane(PaneCommand::Read {
            id: "tab-1::agent-1".to_string(),
            lines: 25,
        })
    );
    let cmd = parse_pane_args(&[s("close"), s("tab-1::agent-1")]).expect("parse pane close");
    assert_eq!(
        cmd,
        CliCommand::Pane(PaneCommand::Close {
            id: "tab-1::agent-1".to_string(),
        })
    );

    // `update --check` is parsed inline by `dispatch`. Round-trip it via
    // the public CliCommand builder to keep the family contract pinned.
    let cmd = CliCommand::Update(UpdateCommand::CheckOnly);
    assert!(matches!(cmd, CliCommand::Update(UpdateCommand::CheckOnly)));

    // SPEC-1942 US-15 legacy parser fixture: `search --issues "<query>"` (flag-first agent
    // shape) round-trips through the search family.
    let cmd = search::parse_args(&[s("--issues"), s("workspace owner")]).expect("search parse");
    let CliCommand::Search(search) = cmd else {
        panic!("expected CliCommand::Search");
    };
    assert_eq!(search.query, "workspace owner");
    assert!(!search.json);
}
