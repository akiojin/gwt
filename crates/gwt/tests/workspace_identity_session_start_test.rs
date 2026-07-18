//! SPEC-2359: SessionStart hook must register the running agent session
//! into `projection.agents[]` so that `workspace.update` JSON operations
//! reaches the matching agent record instead of being silently dropped at
//! the `apply_update` matcher in `gwt_core::workspace_projection`.

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use gwt::cli::hook::{event_dispatcher, HookOutput, IntentBoundaryEvent};
use gwt_agent::{
    session::{GWT_SESSION_ID_ENV, GWT_SESSION_RUNTIME_PATH_ENV},
    AgentId, Session,
};
use gwt_core::process::hidden_command;
use gwt_core::{
    paths::gwt_sessions_dir,
    test_support::{ScopedEnvVar, ScopedGwtHome},
    workspace_projection::{
        load_workspace_projection, update_workspace_projection_with_journal,
        WorkspaceProjectionUpdate,
    },
};
use tempfile::TempDir;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvGuard {
    previous_session_id: Option<std::ffi::OsString>,
    _home: ScopedGwtHome,
    _home_env: ScopedEnvVar,
    _userprofile_env: ScopedEnvVar,
    _runtime_path_env: ScopedEnvVar,
    _codex_thread_id_env: ScopedEnvVar,
    // Declared last so it drops last: the env-lock stays held while the
    // ScopedEnvVar guards above restore HOME / USERPROFILE, keeping the
    // process-global env mutation serialized against other tests.
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous_session_id.take() {
            Some(value) => std::env::set_var(GWT_SESSION_ID_ENV, value),
            None => std::env::remove_var(GWT_SESSION_ID_ENV),
        }
    }
}

fn with_temp_env(home: &Path, session_id: &str) -> EnvGuard {
    let guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home_guard = ScopedGwtHome::set(home);
    // Isolate gwt-config's HOME-based resolution as well. `Settings::load()`
    // (and therefore Board provider selection) reads `$HOME/.gwt/config.toml`,
    // which `ScopedGwtHome` (a gwt-core thread-local) does not cover. Without
    // this, a developer machine configured with `board.provider = slack|teams`
    // makes the SessionStart Board dispatch fail with "<provider> is not signed
    // in", so the test is non-hermetic. Pointing HOME/USERPROFILE at the temp
    // home leaves no config there, defaulting the provider to local.
    let home_env = ScopedEnvVar::set("HOME", home);
    let userprofile_env = ScopedEnvVar::set("USERPROFILE", home);
    let previous_session_id = std::env::var_os(GWT_SESSION_ID_ENV);
    std::env::set_var(GWT_SESSION_ID_ENV, session_id);
    let runtime_path = gwt_agent::runtime_state_path(&gwt_sessions_dir(), session_id);
    let runtime_path_env = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);
    let codex_thread_id_env = ScopedEnvVar::unset("CODEX_THREAD_ID");
    EnvGuard {
        previous_session_id,
        _home: home_guard,
        _home_env: home_env,
        _userprofile_env: userprofile_env,
        _runtime_path_env: runtime_path_env,
        _codex_thread_id_env: codex_thread_id_env,
        _guard: guard,
    }
}

fn init_repo(home: &TempDir) -> PathBuf {
    let repo_path = home.path().join("repo");
    std::fs::create_dir_all(&repo_path).expect("create repo dir");
    assert!(hidden_command("git")
        .arg("init")
        .arg(&repo_path)
        .status()
        .expect("git init")
        .success());
    assert!(hidden_command("git")
        .arg("-C")
        .arg(&repo_path)
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/example/gwt-session-start.git",
        ])
        .status()
        .expect("git remote add")
        .success());
    repo_path
}

fn save_session(repo_path: &Path) -> String {
    save_session_with_agent(repo_path, AgentId::Codex)
}

fn save_session_with_agent(repo_path: &Path, agent_id: AgentId) -> String {
    let mut session = Session::new(repo_path, "work/session-start-test", agent_id);
    session.id = "session-start-fixture".to_string();
    session.save(&gwt_sessions_dir()).expect("save session");
    session.id
}

#[test]
fn session_start_hook_registers_agent_so_workspace_update_persists_title_summary() {
    let home = tempfile::tempdir().expect("temp home");
    let _env = with_temp_env(home.path(), "session-start-fixture");
    let repo_path = init_repo(&home);
    let session_id = save_session(&repo_path);

    let output = event_dispatcher::handle_with_input(
        "SessionStart",
        r#"{"session_id":"codex-conversation-1"}"#,
        &repo_path,
        None,
    )
    .expect("SessionStart dispatch should succeed");
    drop(output);

    let loaded = Session::load(&gwt_sessions_dir().join(format!("{session_id}.toml")))
        .expect("load session");
    assert_eq!(
        loaded.agent_session_id.as_deref(),
        Some("codex-conversation-1")
    );
    assert_eq!(loaded.session_history.len(), 1);
    assert_eq!(
        loaded.session_history[0].agent_session_id,
        "codex-conversation-1"
    );

    let projection = load_workspace_projection(&repo_path)
        .expect("load projection")
        .expect("projection should exist after SessionStart");
    let agent = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == session_id)
        .expect("SessionStart must register the running session");
    assert!(agent.is_unassigned());
    assert_eq!(agent.agent_id, "codex");
    assert_eq!(agent.title_summary, None);

    let update = WorkspaceProjectionUpdate {
        title: None,
        status_category: None,
        status_text: None,
        owner: None,
        next_action: None,
        summary: None,
        progress_summary: None,
        agent_session_id: Some(session_id.clone()),
        agent_current_focus: None,
        agent_title_summary: Some("verify session start fixes".to_string()),
    };
    update_workspace_projection_with_journal(&repo_path, update).expect("workspace update");

    let projection = load_workspace_projection(&repo_path)
        .expect("reload projection")
        .expect("projection should exist after update");
    let agent = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == session_id)
        .expect("agent must remain after update");
    assert_eq!(
        agent.title_summary.as_deref(),
        Some("verify session start fixes"),
        "title_summary must reach the registered agent record"
    );
}

#[test]
fn session_start_hook_is_idempotent_across_repeated_invocations() {
    let home = tempfile::tempdir().expect("temp home");
    let _env = with_temp_env(home.path(), "session-start-fixture");
    let repo_path = init_repo(&home);
    let session_id = save_session(&repo_path);

    for _ in 0..3 {
        event_dispatcher::handle_with_input(
            "SessionStart",
            r#"{"session_id":"codex-conversation-1"}"#,
            &repo_path,
            None,
        )
        .expect("SessionStart dispatch should succeed");
    }

    let projection = load_workspace_projection(&repo_path)
        .expect("load projection")
        .expect("projection should exist after SessionStart");
    let matches = projection
        .agents
        .iter()
        .filter(|agent| agent.session_id == session_id)
        .count();
    assert_eq!(matches, 1, "SessionStart hook must not duplicate agents");

    let loaded = Session::load(&gwt_sessions_dir().join(format!("{session_id}.toml")))
        .expect("load session");
    assert_eq!(loaded.session_history.len(), 1);
    assert_eq!(
        loaded.session_history[0].agent_session_id,
        "codex-conversation-1"
    );
}

#[test]
fn session_start_without_provider_session_id_returns_diagnostic_and_does_not_register_agent() {
    let home = tempfile::tempdir().expect("temp home");
    let _env = with_temp_env(home.path(), "session-start-fixture");
    let repo_path = init_repo(&home);
    let session_id = save_session_with_agent(&repo_path, AgentId::ClaudeCode);

    let output = event_dispatcher::handle_with_input("SessionStart", "{}", &repo_path, None)
        .expect("SessionStart dispatch should fail open with diagnostics");

    let HookOutput::HookSpecificAdditionalContext { event, text } = output else {
        panic!("expected SessionStart diagnostic context");
    };
    assert_eq!(event, IntentBoundaryEvent::SessionStart);
    assert!(
        text.contains("SessionStart did not include a provider session id"),
        "{text}"
    );
    assert!(
        text.contains("gwt could not associate this agent session"),
        "{text}"
    );

    let loaded = Session::load(&gwt_sessions_dir().join(format!("{session_id}.toml")))
        .expect("load session");
    assert!(
        loaded.agent_session_id.is_none(),
        "missing provider id must not synthesize an exact resume id"
    );
    assert!(
        loaded.session_history.is_empty(),
        "missing provider id must not synthesize session history"
    );

    let projection = load_workspace_projection(&repo_path).expect("load projection");
    if let Some(projection) = projection {
        assert!(
            projection
                .agents
                .iter()
                .all(|agent| agent.session_id != session_id),
            "session-less SessionStart must not register a normal Workspace agent"
        );
    }
}
