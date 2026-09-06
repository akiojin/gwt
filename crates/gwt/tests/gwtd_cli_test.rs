use std::{
    collections::BTreeSet,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
};

use chrono::Utc;
use gwt_agent::{AgentId, Session};
use gwt_core::process::hidden_command;
use gwt_core::{
    paths::project_scope_hash,
    repo_hash::{compute_path_hash, compute_repo_hash},
    workspace_projection::{
        append_workspace_work_event_to_path, load_workspace_projection_from_path,
        save_workspace_projection_to_path, save_workspace_work_items_projection_to_path, WorkEvent,
        WorkEventKind, WorkItemsProjection, WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary,
        WorkspaceExecutionContainerRef, WorkspaceProjection, WorkspaceStatusCategory,
    },
};
use tempfile::TempDir;

fn isolated_gwtd_command() -> std::process::Command {
    let mut command = hidden_command(env!("CARGO_BIN_EXE_gwtd"));
    for key in [
        "GWT_BIN_PATH",
        "GWT_BROWSER_URL_FILE",
        "GWT_HOOK_BIN",
        "GWT_HOOK_FORWARD_TOKEN",
        "GWT_HOOK_FORWARD_URL",
        "GWT_PROJECT_ROOT",
        "GWT_REPO_HASH",
        "GWT_SESSION_ID",
        "GWT_SESSION_KIND",
        "GWT_SESSION_RUNTIME_PATH",
        "GWT_WORKTREE_HASH",
    ] {
        command.env_remove(key);
    }
    command
}

fn prepared_hook_session() -> (TempDir, TempDir, String) {
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let session = Session::new(worktree.path(), "work/hook-transport", AgentId::Codex);
    let session_id = session.id.clone();
    session
        .save(&home.path().join(".gwt").join("sessions"))
        .expect("save hook session");
    (home, worktree, session_id)
}

#[test]
fn gwtd_dispatches_internal_hook_cli_without_gui_output() {
    let output = isolated_gwtd_command()
        .args(["__internal", "daemon-hook", "forward"])
        .stdin(Stdio::null())
        .output()
        .expect("run gwtd");

    assert!(
        output.status.success(),
        "gwtd internal hook should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "headless internal hook should not print GUI guidance, got stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn gwtd_help_describes_the_headless_cli_surface() {
    let output = isolated_gwtd_command()
        .arg("--help")
        .output()
        .expect("run gwtd --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("gwtd"));
    assert!(stdout.contains("issue"));
    assert!(stdout.contains("pr"));
    assert!(stdout.contains("hook"));
    assert!(stdout.contains("memory"));
    assert!(
        !stdout.contains("Launch `gwt` instead"),
        "gwtd help must not redirect agent-facing CLI users to the GUI front door"
    );
}

#[test]
fn gwtd_no_args_dispatches_stdin_json_envelope() {
    let home = tempfile::tempdir().expect("home tempdir");
    let project = tempfile::tempdir().expect("project tempdir");
    let project_root = project
        .path()
        .canonicalize()
        .expect("canonical project root");
    let branch = "work/bin-json";
    let session_id = "session-bin-json";
    let work_id = "work-bin-json";
    let run_git = |args: &[&str]| {
        let output = hidden_command("git")
            .args(args)
            .current_dir(&project_root)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run_git(&["init", "-q", "-b", branch]);
    run_git(&[
        "remote",
        "add",
        "origin",
        "https://example.invalid/acme/gwtd-bin-json.git",
    ]);

    let mut session = Session::new(&project_root, branch, AgentId::Codex);
    session.id = session_id.to_string();
    session.project_state_root = Some(project_root.clone());
    session.linked_issue_number = Some(3412);
    assert!(
        session.repo_hash.is_some(),
        "fixture origin must set repo hash"
    );
    session
        .save(&home.path().join(".gwt/sessions"))
        .expect("save Session ledger fixture");

    let state_dir = home
        .path()
        .join(".gwt/projects")
        .join(project_scope_hash(&project_root).as_str())
        .join("project-state");
    let current_path = state_dir.join("current.json");
    let works_path = state_dir.join("works.json");
    let tracked_events_path = project_root.join(".gwt/work/events.jsonl");
    let now = Utc::now();
    let mut projection = WorkspaceProjection::default_for_project(&project_root);
    projection.agents.push(WorkspaceAgentSummary {
        session_id: session_id.to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: Some("fixture focus".to_string()),
        title_summary: Some("Fixture Work".to_string()),
        worktree_path: Some(project_root.clone()),
        branch: Some(branch.to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: Some(work_id.to_string()),
        updated_at: now,
    });
    save_workspace_projection_to_path(&current_path, &projection)
        .expect("save canonical Session assignment");

    let mut event = WorkEvent::new(WorkEventKind::Start, work_id, now);
    event.title = Some("Fixture Work".to_string());
    event.status_category = Some(WorkspaceStatusCategory::Active);
    event.owner = Some("Issue #3412".to_string());
    event.agent_session_id = Some(session_id.to_string());
    event.agent_id = Some("codex".to_string());
    event.display_name = Some("Codex".to_string());
    event.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some(branch.to_string()),
        worktree_path: Some(project_root.clone()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    let mut work_items = WorkItemsProjection::empty(now);
    let _ = work_items.apply_event(event.clone());
    save_workspace_work_items_projection_to_path(&works_path, &work_items)
        .expect("save assigned WorkItems fixture");
    append_workspace_work_event_to_path(&tracked_events_path, &event)
        .expect("save tracked Work event fixture");

    let mut child = isolated_gwtd_command()
        .current_dir(&project_root)
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("GWT_SESSION_ID", session_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd");

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(
            br#"{
                "schema_version": 1,
                "operation": "workspace.update",
                "params": {
                    "agent_session": "session-bin-json",
                    "purpose": "Binary JSON envelope",
                    "current_focus": "integration test"
                }
            }"#,
        )
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait gwtd");

    assert!(
        output.status.success(),
        "gwtd JSON envelope should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse gwtd JSON response");
    assert_eq!(
        response.get("ok").and_then(|value| value.as_bool()),
        Some(true),
        "stdout should be success JSON with ok=true, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let projection = load_workspace_projection_from_path(&current_path)
        .expect("load workspace projection")
        .expect("workspace projection should be written under isolated home");
    let agent = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == "session-bin-json")
        .expect("agent upserted by gwtd JSON envelope");
    assert_eq!(agent.title_summary.as_deref(), Some("Binary JSON envelope"));
}

#[test]
fn gwtd_rejects_legacy_family_argv_invocations() {
    for args in [
        ["board", "show"].as_slice(),
        ["issue", "view", "1"].as_slice(),
        ["hook", "register-codex-managed-hook-trust"].as_slice(),
        ["index", "--help"].as_slice(),
        ["workspace", "update", "--title-summary", "legacy"].as_slice(),
    ] {
        let output = isolated_gwtd_command()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("run gwtd legacy argv");

        assert_eq!(
            output.status.code(),
            Some(2),
            "legacy argv must exit 2 for args {args:?}; stdout={}, stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("stdin JSON envelope"),
            "stderr must point agents to JSON envelope for args {args:?}, got: {stderr}"
        );
    }
}

#[test]
fn gwtd_index_help_lists_every_rebuild_scope() {
    let output = isolated_gwtd_command()
        .args(["--help", "index"])
        .output()
        .expect("run gwtd --help index");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("all|issues|specs|memory|discussions|board|files|files-docs"),
        "index help must list every accepted rebuild scope, got: {stdout}"
    );
}

#[test]
fn gwtd_hook_register_codex_managed_hook_trust_writes_requested_config() {
    let project = tempfile::tempdir().expect("project tempdir");
    let codex_home = tempfile::tempdir().expect("codex tempdir");
    let config_path = codex_home.path().join("config.toml");
    let previous_hook_bin = std::env::var_os("GWT_HOOK_BIN");
    std::env::set_var("GWT_HOOK_BIN", env!("CARGO_BIN_EXE_gwtd"));
    gwt_skills::generate_codex_hooks(project.path()).expect("generate hooks");
    match previous_hook_bin {
        Some(value) => std::env::set_var("GWT_HOOK_BIN", value),
        None => std::env::remove_var("GWT_HOOK_BIN"),
    }

    let output = isolated_gwtd_command()
        .env("GWT_HOOK_BIN", env!("CARGO_BIN_EXE_gwtd"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd hook register");
    let mut child = output;
    write!(
        child.stdin.take().expect("stdin"),
        "{}",
        serde_json::json!({
            "schema_version": 1,
            "operation": "hook.register_codex_managed_hook_trust",
            "params": {
                "project_root": project.path().to_str().expect("project path utf8"),
                "codex_config": config_path.to_str().expect("config path utf8"),
            }
        })
    )
    .expect("write JSON envelope");
    let output = child.wait_with_output().expect("wait gwtd hook register");

    assert!(
        output.status.success(),
        "registration should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("hook registration stdout must be JSON");
    assert_eq!(response["ok"].as_bool(), Some(true));
    assert!(
        response["output"]
            .as_str()
            .is_some_and(|output| output.contains("trusted 5")),
        "JSON output field should report trusted hook count, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let config = fs::read_to_string(&config_path).expect("read config");
    assert!(
        config.contains("trusted_hash"),
        "Codex config must receive trusted hashes, got: {config}"
    );
    assert_eq!(
        config.matches("enabled = true").count(),
        5,
        "Codex config must enable every trusted managed hook, got: {config}"
    );
}

#[test]
fn gwtd_managed_hook_event_remains_argv_transport_exception() {
    let (home, worktree, session_id) = prepared_hook_session();
    let output = isolated_gwtd_command()
        .current_dir(worktree.path())
        .args(["hook", "event", "SessionStart"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("GWT_SESSION_ID", &session_id)
        .env_remove("GWT_SESSION_RUNTIME_PATH")
        .stdin(Stdio::null())
        .output()
        .expect("run gwtd hook event");

    assert!(
        output.status.success(),
        "managed hook transport should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("hookSpecificOutput"),
        "SessionStart should keep the managed hook stdout contract, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn gwtd_provider_hook_event_remains_argv_transport_exception() {
    let (home, worktree, session_id) = prepared_hook_session();
    let mut child = isolated_gwtd_command()
        .current_dir(worktree.path())
        .args(["hook", "provider-event", "opencode", "session.created"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("GWT_SESSION_ID", &session_id)
        .env_remove("GWT_SESSION_RUNTIME_PATH")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd provider hook event");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(br#"{"sessionId":"provider-session"}"#)
        .expect("write provider event payload");
    let output = child.wait_with_output().expect("wait provider hook event");

    assert!(
        output.status.success(),
        "provider hook transport should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("hookSpecificOutput"),
        "provider SessionStart should keep the hook stdout contract, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// Initialize a worktree whose `origin` is `akiojin/gwt`, the repository whose
/// generated hook assets this suite inspects.
fn init_gwt_origin_repo(worktree: &Path) {
    assert!(hidden_command("git")
        .arg("init")
        .arg("-q")
        .arg(worktree)
        .status()
        .expect("git init")
        .success());
    assert!(hidden_command("git")
        .arg("-C")
        .arg(worktree)
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/akiojin/gwt.git"
        ])
        .status()
        .expect("git remote add")
        .success());
}

/// Concatenate every generated text artifact under `dir` (skipping `.git`) so
/// the gwtd `hook <subcommand>` invocations from Claude/Codex settings and the
/// OpenCode/OpenClaw/Hermes provider bridges land in one corpus.
fn collect_generated_text(dir: &Path, out: &mut String) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some(".git") {
            continue;
        }
        if path.is_dir() {
            collect_generated_text(&path, out);
        } else if let Ok(content) = fs::read_to_string(&path) {
            out.push_str(&content);
            out.push('\n');
        }
    }
}

/// Extract the gwtd `hook <subcommand>` keywords that generated artifacts
/// invoke. Shell (`"$gwt_bin" hook event Stop`) and JS-array
/// (`["hook", "provider-event", ...]`) call forms both reduce to the same token
/// stream once quoting/bracket punctuation is flattened to whitespace, so a
/// `hook` token whose predecessor references the gwt hook binary yields the
/// routing subcommand. Anchoring on the binary predecessor keeps prose like the
/// OpenClaw manifest's "plugin hook events" out of the result set.
fn generated_hook_subcommands(corpus: &str) -> BTreeSet<String> {
    let flattened: String = corpus
        .chars()
        .map(|c| match c {
            '"' | '\'' | ',' | '[' | ']' | '(' | ')' | ';' | '{' | '}' | '|' | '=' => ' ',
            other => other,
        })
        .collect();
    let tokens: Vec<&str> = flattened.split_whitespace().collect();
    let references_hook_bin = |token: &str| {
        let normalized = token.to_ascii_lowercase();
        normalized.contains("gwtd")
            || normalized.contains("gwt_bin")
            || normalized.contains("gwtbin")
            || normalized.contains("gwt_hook_bin")
    };
    let is_subcommand = |token: &str| {
        let mut chars = token.chars();
        matches!(chars.next(), Some(first) if first.is_ascii_lowercase())
            && token.chars().all(|c| c.is_ascii_lowercase() || c == '-')
    };

    let mut subcommands = BTreeSet::new();
    for index in 1..tokens.len().saturating_sub(1) {
        if tokens[index] == "hook"
            && references_hook_bin(tokens[index - 1])
            && is_subcommand(tokens[index + 1])
        {
            subcommands.insert(tokens[index + 1].to_string());
        }
    }
    subcommands
}

/// Run `gwtd hook <args>` with an isolated home and report whether the binary
/// rejected the argv with the legacy-transport error that broke issue #3178.
fn gwtd_hook_argv_rejected(args: &[&str], stdin: &str) -> (bool, String) {
    let home = tempfile::tempdir().expect("home tempdir");
    let cwd = tempfile::tempdir().expect("cwd tempdir");
    let mut child = isolated_gwtd_command()
        .current_dir(cwd.path())
        .args(args)
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env_remove("GWT_SESSION_RUNTIME_PATH")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd hook argv");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write hook stdin");
    let output = child.wait_with_output().expect("wait gwtd hook argv");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let rejected = stderr.contains("legacy argv invocation is disabled");
    (rejected, stderr)
}

/// Regression guard for issue #3178: every managed-hook command that generation
/// emits must stay inside gwtd's argv transport allowlist. A generated command
/// with no matching `is_allowed_argv_exception` entry hits the legacy-argv
/// rejection on every hook invocation. This test derives the subcommands from
/// the actual generated artifacts (not a hard-coded list) and runs each through
/// the real binary, so a new generation site that drifts ahead of the allowlist
/// fails here instead of silently at runtime.
#[test]
fn generated_managed_hook_commands_stay_within_gwtd_argv_allowlist() {
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    init_gwt_origin_repo(worktree.path());

    gwt_skills::generate_settings_local(worktree.path()).expect("generate claude settings");
    gwt_skills::generate_codex_hooks(worktree.path()).expect("generate codex hooks");
    gwt_skills::generate_opencode_hooks(worktree.path()).expect("generate opencode hooks");
    gwt_skills::generate_openclaw_hooks(worktree.path()).expect("generate openclaw hooks");
    gwt_skills::generate_hermes_hooks(worktree.path()).expect("generate hermes hooks");

    let mut corpus = String::new();
    collect_generated_text(worktree.path(), &mut corpus);
    let subcommands = generated_hook_subcommands(&corpus);

    for expected in ["event", "provider-event"] {
        assert!(
            subcommands.contains(expected),
            "generation must still emit the `hook {expected}` managed-hook command; \
             discovered subcommands: {subcommands:?}"
        );
    }

    for subcommand in &subcommands {
        let (args, stdin): (Vec<&str>, &str) = match subcommand.as_str() {
            "event" => (vec!["hook", "event", "SessionStart"], ""),
            "provider-event" => (
                vec!["hook", "provider-event", "opencode", "session.created"],
                "{\"sessionId\":\"guard\"}",
            ),
            other => panic!(
                "generation emits an unmapped gwtd hook subcommand `{other}`. Add a representative \
                 argv here and confirm gwtd's is_allowed_argv_exception accepts it, or the same \
                 generation↔binary drift that broke issue #3178 will ship undetected."
            ),
        };
        let (rejected, stderr) = gwtd_hook_argv_rejected(&args, stdin);
        assert!(
            !rejected,
            "generated `hook {subcommand}` argv was rejected by gwtd's legacy-argv guard \
             (issue #3178 regression); stderr: {stderr}"
        );
    }
}

/// SPEC #3164 AC-R2: the retired `improvement.*` operations are gone from the
/// envelope dispatcher, so they must land on the pre-existing unknown-operation
/// rejection rather than on a bespoke refusal code.
#[test]
fn retired_improvement_operations_hit_the_unknown_operation_rejection() {
    let home = tempfile::tempdir().expect("home tempdir");
    let repo = tempfile::tempdir().expect("repo tempdir");

    for operation in ["improvement.list", "improvement.capture"] {
        let mut child = isolated_gwtd_command()
            .current_dir(repo.path())
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn gwtd");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(
                format!("{{\"schema_version\":1,\"operation\":\"{operation}\",\"params\":{{}}}}")
                    .as_bytes(),
            )
            .expect("write envelope");
        let output = child.wait_with_output().expect("wait gwtd");

        assert!(
            !output.status.success(),
            "`{operation}` must be refused after the improvement subsystem retirement"
        );
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined.contains("unknown"),
            "`{operation}` must be refused through the existing unknown-operation path, got: {combined}"
        );
    }
}

/// SPEC #3164 AC-R3: `gwtd hook gwt-self-improvement-stop` lost its argv
/// transport exception along with the hook, so the binary must refuse it
/// instead of running a retired gate.
#[test]
fn retired_self_improvement_stop_hook_is_no_longer_an_argv_exception() {
    let home = tempfile::tempdir().expect("home tempdir");
    let repo = tempfile::tempdir().expect("repo tempdir");

    let output = isolated_gwtd_command()
        .current_dir(repo.path())
        .args(["hook", "gwt-self-improvement-stop"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .stdin(Stdio::null())
        .output()
        .expect("run retired self-improvement hook");

    assert!(
        !output.status.success(),
        "the retired self-improvement Stop hook must not be dispatched"
    );
    assert!(
        output.stdout.is_empty(),
        "the retired hook must emit no hook decision, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("gwt-self-improvement-stop"),
        "the retired hook must not be advertised as a transport exception, got: {stderr}"
    );
}

/// Issue #3606: a JSON operation that acts on the project store must name the
/// store it landed in.
///
/// The PM ran `issue.monitor.priority.move` against a `project_root` whose
/// identity did not resolve, got `ok: true` plus a successful readback, and
/// nearly reported the wrong queue state to the user — the write had gone to an
/// isolated path-fallback store no running gwt reads. `ok: true` proves the
/// operation ran; it never proved *where*. These tests pin the proof onto the
/// envelope so the landing is checkable instead of inferable from file mtimes.
const STORE_LANDING_ORIGIN: &str = "https://example.invalid/acme/store-landing.git";

struct StoreLandingFixture {
    home: TempDir,
    _temp: TempDir,
    layout_root: PathBuf,
    worktree: PathBuf,
}

fn store_landing_git(cwd: &Path, args: &[&str]) {
    let output = hidden_command("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git store-landing fixture command");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A Nested Bare + Worktree layout: `<root>/gwt.git` plus `<root>/work/develop`.
/// This is the shape `E:/gwt` has in the report, where the layout root itself is
/// not a repository but still scopes exactly one project.
fn store_landing_fixture() -> StoreLandingFixture {
    let home = tempfile::tempdir().expect("home tempdir");
    let temp = tempfile::tempdir().expect("layout tempdir");
    let layout_root = temp.path().join("workbench");
    let bare = layout_root.join("gwt.git");
    let bootstrap = layout_root.join(".bootstrap");
    let worktree = layout_root.join("work").join("develop");
    fs::create_dir_all(worktree.parent().expect("work dir")).expect("create work dir");

    store_landing_git(
        temp.path(),
        &["init", "--bare", bare.to_str().expect("bare path utf8")],
    );
    store_landing_git(&bare, &["remote", "add", "origin", STORE_LANDING_ORIGIN]);
    store_landing_git(
        &layout_root,
        &[
            "clone",
            bare.to_str().expect("bare path utf8"),
            ".bootstrap",
        ],
    );
    store_landing_git(&bootstrap, &["checkout", "-b", "develop"]);
    store_landing_git(
        &bootstrap,
        &[
            "-c",
            "user.name=gwt-test",
            "-c",
            "user.email=gwt-test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    store_landing_git(&bootstrap, &["push", "origin", "develop"]);
    fs::remove_dir_all(&bootstrap).expect("remove bootstrap clone");
    store_landing_git(
        &bare,
        &[
            "worktree",
            "add",
            worktree.to_str().expect("worktree path utf8"),
            "develop",
        ],
    );

    StoreLandingFixture {
        home,
        _temp: temp,
        layout_root,
        worktree,
    }
}

fn run_store_landing_envelope(
    home: &Path,
    cwd: &Path,
    envelope: &serde_json::Value,
) -> serde_json::Value {
    let mut child = isolated_gwtd_command()
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn gwtd");
    write!(child.stdin.take().expect("stdin"), "{envelope}").expect("write JSON envelope");
    let output = child.wait_with_output().expect("wait gwtd");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "gwtd stdout must be a JSON envelope ({error}); stdout={}, stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn issue_monitor_status_names_the_project_store_it_resolved() {
    let fixture = store_landing_fixture();
    let expected = compute_repo_hash(STORE_LANDING_ORIGIN);

    let response = run_store_landing_envelope(
        fixture.home.path(),
        &fixture.layout_root,
        &serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.status",
            "params": { "project_root": fixture.layout_root.to_str().expect("layout path utf8") },
        }),
    );

    assert_eq!(response["ok"].as_bool(), Some(true), "response: {response}");
    let store = &response["project_store"];
    assert_eq!(
        store["hash"].as_str(),
        Some(expected.as_str()),
        "the envelope must name the store the operation resolved; response: {response}"
    );
    assert_eq!(
        store["identity_resolved"].as_bool(),
        Some(true),
        "a Nested Bare + Worktree layout root resolves a repository identity; response: {response}"
    );
    assert_eq!(
        store["source"].as_str(),
        Some("nested_bare_repository"),
        "response: {response}"
    );
    let store_path = store["store_path"].as_str().expect("store_path");
    assert!(
        Path::new(store_path).ends_with(expected.as_str()),
        "store_path must point at the store directory the caller can inspect, got {store_path}"
    );
}

#[test]
fn issue_monitor_status_reports_the_same_store_from_a_linked_worktree() {
    let fixture = store_landing_fixture();
    let expected = compute_repo_hash(STORE_LANDING_ORIGIN);

    // Issue #3606 AC-5': a linked worktree — the shape of a gwt `work/` checkout
    // and of the PM's `~/.gwt/projects/<hash>/pm/worktree` — must converge on the
    // same store as the layout root it was materialized from.
    let response = run_store_landing_envelope(
        fixture.home.path(),
        &fixture.worktree,
        &serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.status",
            "params": { "project_root": fixture.worktree.to_str().expect("worktree path utf8") },
        }),
    );

    assert_eq!(response["ok"].as_bool(), Some(true), "response: {response}");
    assert_eq!(
        response["project_store"]["hash"].as_str(),
        Some(expected.as_str()),
        "a linked worktree must resolve the repository identity store; response: {response}"
    );
    assert_eq!(
        response["project_store"]["source"].as_str(),
        Some("origin"),
        "response: {response}"
    );
}

#[test]
fn issue_monitor_write_reports_an_isolated_path_fallback_landing() {
    let home = tempfile::tempdir().expect("home tempdir");
    let unresolvable = tempfile::tempdir().expect("project tempdir");
    let expected = compute_path_hash(unresolvable.path());

    let response = run_store_landing_envelope(
        home.path(),
        unresolvable.path(),
        &serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.config.set",
            "params": {
                "project_root": unresolvable.path().to_str().expect("project path utf8"),
                "max_active": 7,
            },
        }),
    );

    // The write still succeeds — the store is simply isolated. What must change
    // is that the caller can see that from the response alone.
    assert_eq!(response["ok"].as_bool(), Some(true), "response: {response}");
    let store = &response["project_store"];
    assert_eq!(
        store["identity_resolved"].as_bool(),
        Some(false),
        "a path-fallback landing must be visible without reading store mtimes; response: {response}"
    );
    assert_eq!(
        store["source"].as_str(),
        Some("path_fallback"),
        "response: {response}"
    );
    assert_eq!(
        store["hash"].as_str(),
        Some(expected.as_str()),
        "response: {response}"
    );
}

#[test]
fn issue_monitor_status_lists_the_candidates_of_an_ambiguous_layout_root() {
    let fixture = store_landing_fixture();
    let second = fixture.layout_root.join("aa-other.git");
    store_landing_git(
        &fixture.layout_root,
        &["init", "--bare", second.to_str().expect("second path utf8")],
    );
    store_landing_git(
        &second,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/acme/other.git",
        ],
    );

    let response = run_store_landing_envelope(
        fixture.home.path(),
        &fixture.layout_root,
        &serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.status",
            "params": { "project_root": fixture.layout_root.to_str().expect("layout path utf8") },
        }),
    );

    let store = &response["project_store"];
    assert_eq!(
        store["source"].as_str(),
        Some("ambiguous_nested_bare_repositories"),
        "response: {response}"
    );
    assert_eq!(
        store["identity_resolved"].as_bool(),
        Some(false),
        "response: {response}"
    );
    let origins: BTreeSet<&str> = store["candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .filter_map(|candidate| candidate["normalized_origin"].as_str())
        .collect();
    assert_eq!(
        origins,
        BTreeSet::from([
            "example.invalid/acme/other",
            "example.invalid/acme/store-landing"
        ]),
        "an ambiguous layout root must say which origins it refused to choose between; \
         response: {response}"
    );
}
