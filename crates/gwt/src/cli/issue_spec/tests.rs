//! Test suite for `cli::issue_spec` (SPEC-1942 SC-027 split). Lives as a
//! sibling file to `mod.rs`, included via `#[cfg(test)] mod tests;`.

#![cfg(test)]

use gwt_github::client::{IssueSnapshot, IssueState, UpdatedAt};

use crate::cli::env::TestEnv;

use super::*;

fn sample_structured_input() -> StructuredSpecInput {
    StructuredSpecInput {
        background: Some(TextBlock::Paragraphs(vec![
            "First paragraph.".to_string(),
            "Second paragraph.".to_string(),
        ])),
        user_stories: Some(vec![StructuredUserStory {
            title: "US-9: Launch agent".to_string(),
            priority: Some("1".to_string()),
            status: Some("READY".to_string()),
            statement: None,
            as_a: Some("developer".to_string()),
            i_want: Some("a launch workflow".to_string()),
            so_that: Some("I can start quickly".to_string()),
            acceptance_scenarios: vec![
                "- Given a branch, when I open the wizard, then it lists agent options".to_string(),
                "2. Given Docker support, when selected, then runtime options appear".to_string(),
            ],
        }]),
        edge_cases: Some(vec!["- Missing branch".to_string()]),
        functional_requirements: Some(vec!["FR-999: Launch selected agent".to_string()]),
        non_functional_requirements: Some(vec!["Low latency".to_string()]),
        success_criteria: Some(vec!["1. Users can launch an agent".to_string()]),
    }
}

fn issue_body(spec: &str, tasks: &str) -> String {
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

fn seed_issue(env: &TestEnv, number: u64, title: &str, spec: &str, tasks: &str, labels: &[&str]) {
    let snapshot = IssueSnapshot {
        number: IssueNumber(number),
        title: title.to_string(),
        body: issue_body(spec, tasks),
        labels: labels.iter().map(|label| (*label).to_string()).collect(),
        state: IssueState::Open,
        updated_at: UpdatedAt::new(format!("seed-{number}")),
        comments: Vec::new(),
    };
    env.client.seed(snapshot.clone());
    Cache::new(env.cache_root())
        .write_snapshot(&snapshot)
        .unwrap();
}

#[test]
fn parse_and_render_structured_spec_include_all_sections() {
    let parsed = parse_structured_spec_json(
        r#"{
            "background": ["First paragraph.", "Second paragraph."],
            "user_stories": [{
                "title": "Launch agent",
                "priority": "P0",
                "statement": "As a developer, I want to launch an agent, so that I can work faster.",
                "acceptance_scenarios": ["Given a branch, when I launch, then the agent starts"]
            }],
            "edge_cases": ["Missing branch"],
            "functional_requirements": ["Launch selected agent"],
            "non_functional_requirements": ["Low latency"],
            "success_criteria": ["Agents launch from the selected branch"]
        }"#,
    )
    .expect("parse structured json");
    let rendered = render_structured_spec("Launch agents from GUI", &parsed);

    assert!(rendered.starts_with("# Launch agents from GUI\n"));
    assert!(rendered.contains("## Background"));
    assert!(rendered.contains("First paragraph.\n\nSecond paragraph."));
    assert!(rendered.contains("## User Stories"));
    assert!(rendered.contains("### US-1: Launch agent (P0)"));
    assert!(rendered.contains("**Acceptance Scenarios:**"));
    assert!(rendered.contains("## Edge Cases"));
    assert!(rendered.contains("- Missing branch"));
    assert!(rendered.contains("## Functional Requirements"));
    assert!(rendered.contains("- **FR-001**: Launch selected agent"));
    assert!(rendered.contains("## Non-Functional Requirements"));
    assert!(rendered.contains("- **NFR-001**: Low latency"));
    assert!(rendered.contains("## Success Criteria"));
    assert!(rendered.contains("- **SC-001**: Agents launch from the selected branch"));

    let err = parse_structured_spec_json("{not-json").unwrap_err();
    assert!(err.to_string().contains("invalid spec json"));
}

#[test]
fn merge_structured_spec_updates_known_sections_and_preserves_unknown_content() {
    let existing = r#"# SPEC: Launch agents

## Background

Old background.

## User Stories

### US-1: Old story

Old statement.

## Custom Notes

Keep this note.
"#;
    let patch = StructuredSpecInput {
        background: Some(TextBlock::Text("".to_string())),
        user_stories: sample_structured_input().user_stories,
        edge_cases: Some(vec!["New edge".to_string()]),
        functional_requirements: None,
        non_functional_requirements: None,
        success_criteria: None,
    };

    let merged = merge_structured_spec(existing, &patch);

    assert!(merged.starts_with("# SPEC: Launch agents\n"));
    assert!(!merged.contains("## Background"));
    assert!(merged.contains("### US-1: Launch agent (P1) -- READY"));
    assert!(merged.contains("## Edge Cases"));
    assert!(merged.contains("- New edge"));
    assert!(merged.contains("## Custom Notes"));
    assert!(merged.contains("Keep this note."));
}

#[test]
fn split_and_normalize_helpers_strip_labels_and_build_story_text() {
    let existing = r#"# SPEC-77: Launch agents

## Background

Background text.

## User Stories

### US-1: Existing story

Statement.

## Extra

Preserve me.
"#;

    let (title, known, unknown) = split_structured_spec(existing);
    assert_eq!(title, "SPEC-77: Launch agents");
    assert_eq!(
        extract_document_title(existing),
        Some("SPEC-77: Launch agents".to_string())
    );
    assert_eq!(
        normalize_spec_heading_from_title("SPEC-77: Launch agents"),
        "Launch agents"
    );
    assert!(known.contains_key("Background"));
    assert!(known.contains_key("User Stories"));
    assert_eq!(unknown, vec!["## Extra\n\nPreserve me.".to_string()]);

    assert_eq!(
        build_user_story_statement(
            &sample_structured_input()
                .user_stories
                .unwrap()
                .into_iter()
                .next()
                .unwrap()
        ),
        Some("As developer, I want a launch workflow, so that I can start quickly.".to_string())
    );
    assert_eq!(
        normalize_user_story_title("US-9: Launch agent"),
        "Launch agent"
    );
    assert_eq!(normalize_priority("1"), "P1");
    assert_eq!(strip_list_marker("2. Listed item"), "Listed item");
    assert_eq!(strip_list_marker("- Bullet item"), "Bullet item");
    assert_eq!(
        strip_requirement_label("- **FR-001**: Requirement text"),
        "Requirement text"
    );
    assert_eq!(
        render_background_section(&TextBlock::Text("  ".to_string())),
        None
    );
    assert_eq!(
        render_bullet_section("Edge Cases", &["".to_string(), "- case".to_string()]),
        Some("## Edge Cases\n\n- case".to_string())
    );
    assert_eq!(
        render_numbered_requirement_section(
            "Functional Requirements",
            "FR",
            &["FR-009: Requirement".to_string()],
        ),
        Some("## Functional Requirements\n\n- **FR-001**: Requirement".to_string())
    );
}

#[test]
fn read_cli_input_uses_stdin_and_named_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut env = TestEnv::new(temp.path().to_path_buf());
    env.stdin = "from-stdin".to_string();
    env.files
        .insert("spec.json".to_string(), "from-file".to_string());

    assert_eq!(read_cli_input(&mut env, None).unwrap(), "from-stdin");
    assert_eq!(
        read_cli_input(&mut env, Some("spec.json")).unwrap(),
        "from-file"
    );
}

#[test]
fn parse_supports_list_create_pull_repair_and_edit_modes() {
    let args = [
        "list".to_string(),
        "--phase".to_string(),
        "review".to_string(),
        "--state".to_string(),
        "closed".to_string(),
    ];
    let refs = args.iter().collect::<Vec<_>>();
    assert!(matches!(
        parse(&refs),
        Ok(IssueCommand::SpecList { phase, state })
            if phase.as_deref() == Some("review") && state.as_deref() == Some("closed")
    ));

    let args = [
        "create".to_string(),
        "--json".to_string(),
        "--title".to_string(),
        "SPEC: Launch".to_string(),
        "--label".to_string(),
        "phase/review".to_string(),
        "-f".to_string(),
        "spec.json".to_string(),
    ];
    let refs = args.iter().collect::<Vec<_>>();
    assert!(matches!(
        parse(&refs),
        Ok(IssueCommand::SpecCreateJson { title, file, labels })
            if title == "SPEC: Launch"
                && file.as_deref() == Some("spec.json")
                && labels == vec!["phase/review".to_string()]
    ));

    let args = ["create".to_string(), "--help".to_string()];
    let refs = args.iter().collect::<Vec<_>>();
    assert!(matches!(parse(&refs), Ok(IssueCommand::SpecCreateHelp)));

    let args = [
        "1942".to_string(),
        "--edit".to_string(),
        "spec".to_string(),
        "--json".to_string(),
        "--replace".to_string(),
        "-f".to_string(),
        "update.json".to_string(),
    ];
    let refs = args.iter().collect::<Vec<_>>();
    assert!(matches!(
        parse(&refs),
        Ok(IssueCommand::SpecEditSectionJson {
            number,
            section,
            file,
            replace
        })
            if number == 1942
                && section == "spec"
                && file.as_deref() == Some("update.json")
                && replace
    ));

    let args = ["pull".to_string(), "--all".to_string(), "77".to_string()];
    let refs = args.iter().collect::<Vec<_>>();
    assert!(matches!(
        parse(&refs),
        Ok(IssueCommand::SpecPull { all, numbers }) if all && numbers == vec![77]
    ));

    let args = ["repair".to_string(), "77".to_string()];
    let refs = args.iter().collect::<Vec<_>>();
    assert!(matches!(
        parse(&refs),
        Ok(IssueCommand::SpecRepair { number }) if number == 77
    ));

    let args = [
        "77".to_string(),
        "--rename".to_string(),
        "SPEC: Renamed".to_string(),
    ];
    let refs = args.iter().collect::<Vec<_>>();
    assert!(matches!(
        parse(&refs),
        Ok(IssueCommand::SpecRename { number, title })
            if number == 77 && title == "SPEC: Renamed"
    ));

    let args = ["create".to_string(), "--title".to_string()];
    let refs = args.iter().collect::<Vec<_>>();
    assert!(matches!(
        parse(&refs),
        Err(CliParseError::MissingFlag("--title"))
    ));

    let args = [
        "77".to_string(),
        "--rename".to_string(),
        "SPEC: Renamed".to_string(),
        "--json".to_string(),
    ];
    let refs = args.iter().collect::<Vec<_>>();
    assert!(matches!(parse(&refs), Err(CliParseError::Usage)));

    let args = ["list".to_string(), "--bogus".to_string()];
    let refs = args.iter().collect::<Vec<_>>();
    assert!(matches!(
        parse(&refs),
        Err(CliParseError::UnknownSubcommand(value)) if value == "--bogus"
    ));
}

#[test]
fn run_supports_read_create_pull_repair_and_rename_workflows() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut env = TestEnv::new(temp.path().to_path_buf());
    seed_issue(
        &env,
        42,
        "SPEC: Launch agents",
        "spec body",
        "tasks body",
        &["gwt-spec", "phase/review"],
    );

    let mut out = String::new();
    assert_eq!(
        run(&mut env, IssueCommand::SpecReadAll { number: 42 }, &mut out).unwrap(),
        0
    );
    assert!(out.contains("=== spec ===\nspec body"));
    assert!(out.contains("=== tasks ===\ntasks body"));

    out.clear();
    assert_eq!(
        run(
            &mut env,
            IssueCommand::SpecReadSection {
                number: 42,
                section: "tasks".to_string(),
            },
            &mut out,
        )
        .unwrap(),
        0
    );
    assert_eq!(out, "tasks body\n");

    out.clear();
    assert_eq!(
        run(
            &mut env,
            IssueCommand::SpecList {
                phase: Some("review".to_string()),
                state: Some("open".to_string()),
            },
            &mut out,
        )
        .unwrap(),
        0
    );
    assert!(out.contains("#42 [OPEN] [phase/review] SPEC: Launch agents"));

    env.files.insert(
        "legacy.md".to_string(),
        issue_body("created spec", "created tasks"),
    );
    out.clear();
    assert_eq!(
        run(
            &mut env,
            IssueCommand::SpecCreate {
                title: "SPEC: Created from markdown".to_string(),
                file: "legacy.md".to_string(),
                labels: vec!["gwt-spec".to_string()],
            },
            &mut out,
        )
        .unwrap(),
        0
    );
    assert!(out.contains("created issue #43"));

    env.stdin = serde_json::json!({
        "background": "Created from json",
        "success_criteria": ["Agents launch from CLI"]
    })
    .to_string();
    out.clear();
    assert_eq!(
        run(
            &mut env,
            IssueCommand::SpecCreateJson {
                title: "SPEC: Created from json".to_string(),
                file: None,
                labels: vec!["gwt-spec".to_string()],
            },
            &mut out,
        )
        .unwrap(),
        0
    );
    assert!(out.contains("created issue #44"));

    out.clear();
    assert_eq!(
        run(&mut env, IssueCommand::SpecCreateHelp, &mut out).unwrap(),
        0
    );
    assert!(out.contains("Input JSON schema:"));

    out.clear();
    assert_eq!(
        run(
            &mut env,
            IssueCommand::SpecPull {
                all: true,
                numbers: Vec::new(),
            },
            &mut out,
        )
        .unwrap(),
        0
    );
    assert_eq!(out, "pulled all gwt-spec issues\n");

    out.clear();
    assert_eq!(
        run(
            &mut env,
            IssueCommand::SpecPull {
                all: false,
                numbers: vec![42],
            },
            &mut out,
        )
        .unwrap(),
        0
    );
    assert_eq!(out, "pulled #42\n");

    let err = run(
        &mut env,
        IssueCommand::SpecPull {
            all: false,
            numbers: Vec::new(),
        },
        &mut out,
    )
    .unwrap_err();
    assert!(err.to_string().contains("pull requires --all or <n>"));

    out.clear();
    assert_eq!(
        run(&mut env, IssueCommand::SpecRepair { number: 42 }, &mut out).unwrap(),
        0
    );
    assert_eq!(out, "repaired cache for #42\n");

    out.clear();
    assert_eq!(
        run(
            &mut env,
            IssueCommand::SpecRename {
                number: 42,
                title: "SPEC: Renamed".to_string(),
            },
            &mut out,
        )
        .unwrap(),
        0
    );
    assert!(out.contains("renamed issue #42 to 'SPEC: Renamed'"));
    let cache = Cache::new(env.cache_root());
    assert_eq!(
        cache.load_entry(IssueNumber(42)).unwrap().snapshot.title,
        "SPEC: Renamed"
    );
}

#[test]
fn run_edit_commands_cover_plain_and_structured_json_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut env = TestEnv::new(temp.path().to_path_buf());
    seed_issue(
        &env,
        7,
        "SPEC: Launch agents",
        "# Launch agents\n\n## Background\n\nOld background.",
        "old tasks",
        &["gwt-spec", "phase/review"],
    );

    env.files
        .insert("tasks.md".to_string(), "updated tasks".to_string());
    let mut out = String::new();
    assert_eq!(
        run(
            &mut env,
            IssueCommand::SpecEditSection {
                number: 7,
                section: "tasks".to_string(),
                file: "tasks.md".to_string(),
            },
            &mut out,
        )
        .unwrap(),
        0
    );
    assert!(out.contains("wrote 13 bytes to section 'tasks'"));

    out.clear();
    env.stdin = serde_json::json!({
        "background": ["New background"],
        "edge_cases": ["Missing branch"]
    })
    .to_string();
    assert_eq!(
        run(
            &mut env,
            IssueCommand::SpecEditSectionJson {
                number: 7,
                section: "spec".to_string(),
                file: None,
                replace: false,
            },
            &mut out,
        )
        .unwrap(),
        0
    );
    let cache = Cache::new(env.cache_root());
    let merged_body = cache.load_entry(IssueNumber(7)).unwrap().snapshot.body;
    assert!(merged_body.contains("## Background\n\nNew background"));
    assert!(merged_body.contains("## Edge Cases"));

    env.files.insert(
        "replace.json".to_string(),
        serde_json::json!({
            "background": "Replacement background",
            "success_criteria": ["Replacement criteria"]
        })
        .to_string(),
    );
    out.clear();
    assert_eq!(
        run(
            &mut env,
            IssueCommand::SpecEditSectionJson {
                number: 7,
                section: "spec".to_string(),
                file: Some("replace.json".to_string()),
                replace: true,
            },
            &mut out,
        )
        .unwrap(),
        0
    );
    let replaced_body = Cache::new(env.cache_root())
        .load_entry(IssueNumber(7))
        .unwrap()
        .snapshot
        .body;
    assert!(replaced_body.contains("# Launch agents"));
    assert!(replaced_body.contains("Replacement background"));
    assert!(replaced_body.contains("## Success Criteria"));
    assert!(!replaced_body.contains("## Edge Cases"));

    let err = run(
        &mut env,
        IssueCommand::SpecEditSectionJson {
            number: 7,
            section: "tasks".to_string(),
            file: None,
            replace: false,
        },
        &mut out,
    )
    .unwrap_err();
    assert!(err
        .to_string()
        .contains("structured JSON edit only supports section 'spec'"));
}
