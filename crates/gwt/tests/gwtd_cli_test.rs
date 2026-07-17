use std::{collections::BTreeSet, fs, io::Write, path::Path, process::Stdio};

use gwt_agent::{AgentId, Session};
use gwt_core::process::hidden_command;
use gwt_core::{
    paths::project_scope_hash, workspace_projection::load_workspace_projection_from_path,
};
use tempfile::TempDir;

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
    let output = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
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
    let output = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
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
    let mut child = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
        .current_dir(project.path())
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
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
    let projection_path = home
        .path()
        .join(".gwt/projects")
        .join(project_scope_hash(project.path()).as_str())
        .join("project-state/current.json");
    let projection = load_workspace_projection_from_path(&projection_path)
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
        let output = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
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
    let output = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
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

    let output = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
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
    let output = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
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
    let mut child = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
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

#[test]
fn gwtd_gwt_self_improvement_stop_remains_argv_transport_exception() {
    let home = tempfile::tempdir().expect("home tempdir");
    let repo = tempfile::tempdir().expect("repo tempdir");
    assert!(hidden_command("git")
        .arg("init")
        .arg("-q")
        .arg(repo.path())
        .status()
        .expect("git init")
        .success());
    assert!(hidden_command("git")
        .arg("-C")
        .arg(repo.path())
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/example/project.git"
        ])
        .status()
        .expect("git remote add")
        .success());

    let output = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
        .current_dir(repo.path())
        .args(["hook", "gwt-self-improvement-stop"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .stdin(Stdio::null())
        .output()
        .expect("run gwtd gwt self-improvement hook");

    assert!(
        output.status.success(),
        "direct self-improvement hook should exit 0 outside akiojin/gwt, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "non-gwt repos must receive no hook output, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// Initialize a worktree whose `origin` is `akiojin/gwt` so generation emits the
/// repo-owned self-improvement Stop hook alongside the shared managed hooks.
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
        token.contains("gwtd") || token.contains("gwt_bin") || token.contains("GWT_HOOK_BIN")
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
    let mut child = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
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
/// emits must stay inside gwtd's argv transport allowlist. The self-improvement
/// Stop hook regressed because its generated `hook gwt-self-improvement-stop`
/// command had no matching `is_allowed_argv_exception` entry, so each Stop hit
/// the legacy-argv rejection. This test derives the subcommands from the actual
/// generated artifacts (not a hard-coded list) and runs each through the real
/// binary, so a new generation site that drifts ahead of the allowlist fails
/// here instead of silently at runtime.
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

    for expected in ["event", "provider-event", "gwt-self-improvement-stop"] {
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
            "gwt-self-improvement-stop" => (vec!["hook", "gwt-self-improvement-stop"], ""),
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

/// Walk up from the gwt crate dir to the repository root that owns the
/// committed managed-hook settings (`.claude/settings.json`).
#[cfg(unix)]
fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|dir| dir.join(".claude/settings.json").is_file())
        .map(Path::to_path_buf)
        .expect("locate repository root containing .claude/settings.json")
}

/// Collect every committed managed-hook `command` string that invokes the
/// repo-owned `gwt-self-improvement-stop` hook (Claude + Codex transports).
#[cfg(unix)]
fn committed_self_improvement_stop_commands() -> Vec<String> {
    let root = repo_root();
    let mut commands = Vec::new();
    for relative in [".claude/settings.json", ".codex/hooks.json"] {
        let path = root.join(relative);
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let value: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|err| panic!("parse {relative}: {err}"));
        let Some(events) = value.get("hooks").and_then(|hooks| hooks.as_object()) else {
            continue;
        };
        for matchers in events.values() {
            for matcher in matchers.as_array().into_iter().flatten() {
                for hook in matcher
                    .get("hooks")
                    .and_then(|hooks| hooks.as_array())
                    .into_iter()
                    .flatten()
                {
                    if let Some(command) = hook.get("command").and_then(|c| c.as_str()) {
                        if command.contains("gwt-self-improvement-stop") {
                            commands.push(command.to_string());
                        }
                    }
                }
            }
        }
    }
    commands
}

/// Regression guard for issue #3178's actual harm: the committed self-improvement
/// Stop hook command must NOT leak gwtd's legacy-argv rejection into the agent's
/// Stop loop when the installed gwtd predates the `gwt-self-improvement-stop`
/// transport exception (e.g. v9.61.0). The command is repo-committed, so it runs
/// against whatever gwtd a developer has installed; it must degrade silently on
/// older binaries the same way the OpenCode/OpenClaw JS bridges already do
/// (stderr/exit ignored). A `HookOutput::StopBlock` from a current binary exits 0
/// and writes its decision JSON to stdout, so a graceful wrapper that drops
/// stderr and forces exit 0 still surfaces a real block.
#[test]
#[cfg(unix)]
fn committed_self_improvement_stop_hook_degrades_on_unsupported_gwtd() {
    let commands = committed_self_improvement_stop_commands();
    assert!(
        !commands.is_empty(),
        "expected at least one committed gwt-self-improvement-stop hook command to guard"
    );

    // A fake gwtd that mimics a pre-v9.63.0 binary: it rejects the unknown argv
    // with the legacy-argv error on stderr and a non-zero exit.
    let fake_dir = tempfile::tempdir().expect("fake bin dir");
    let fake_gwtd = fake_dir.path().join("gwtd");
    fs::write(
        &fake_gwtd,
        "#!/bin/sh\n\
         echo 'gwtd hook: legacy argv invocation is disabled; use stdin JSON envelope.' >&2\n\
         exit 2\n",
    )
    .expect("write fake gwtd");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_gwtd, fs::Permissions::from_mode(0o755))
            .expect("chmod fake gwtd");
    }

    for command in &commands {
        let output = hidden_command("sh")
            .arg("-c")
            .arg(command)
            .env("GWT_BIN_PATH", &fake_gwtd)
            .stdin(Stdio::null())
            .output()
            .expect("run committed self-improvement stop command");

        assert_eq!(
            output.status.code(),
            Some(0),
            "committed self-improvement Stop command must exit 0 on an unsupported gwtd so it \
             does not block the agent's Stop loop (issue #3178); command: {command}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stderr.is_empty(),
            "committed self-improvement Stop command must not leak gwtd's legacy-argv rejection \
             into the agent's Stop feedback (issue #3178); command: {command}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
