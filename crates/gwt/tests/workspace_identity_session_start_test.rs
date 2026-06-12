//! SPEC-2359: SessionStart hook must register the running agent session
//! into `projection.agents[]` so that `gwtd workspace update --title-summary`
//! reaches the matching agent record instead of being silently dropped at
//! the `apply_update` matcher in `gwt_core::work_projection`.

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use gwt::cli::hook::event_dispatcher;
use gwt_agent::{session::GWT_SESSION_ID_ENV, AgentId, Session};
use gwt_core::{
    paths::gwt_sessions_dir,
    work_projection::{
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
    _guard: std::sync::MutexGuard<'static, ()>,
    previous_home: Option<std::ffi::OsString>,
    previous_session_id: Option<std::ffi::OsString>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous_home.take() {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
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
    let previous_home = std::env::var_os("HOME");
    let previous_session_id = std::env::var_os(GWT_SESSION_ID_ENV);
    std::env::set_var("HOME", home);
    std::env::set_var(GWT_SESSION_ID_ENV, session_id);
    EnvGuard {
        _guard: guard,
        previous_home,
        previous_session_id,
    }
}

fn init_repo(home: &TempDir) -> PathBuf {
    let repo_path = home.path().join("repo");
    std::fs::create_dir_all(&repo_path).expect("create repo dir");
    assert!(std::process::Command::new("git")
        .arg("init")
        .arg(&repo_path)
        .status()
        .expect("git init")
        .success());
    assert!(std::process::Command::new("git")
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
    let mut session = Session::new(repo_path, "work/session-start-test", AgentId::Codex);
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

    let output = event_dispatcher::handle_with_input("SessionStart", "{}", &repo_path, None)
        .expect("SessionStart dispatch should succeed");
    drop(output);

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
        event_dispatcher::handle_with_input("SessionStart", "{}", &repo_path, None)
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
}
