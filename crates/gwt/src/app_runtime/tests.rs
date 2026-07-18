use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex, RwLock},
    thread,
    time::{Duration, Instant},
};

use tempfile::tempdir;

use base64::Engine;
use chrono::{TimeZone, Utc};
use gwt::{
    empty_workspace_state, load_restored_workspace_state, load_session_state, ArrangeMode,
    BackendEvent, BranchCleanupInfo, BranchListEntry, BranchScope, ContentLimits,
    FocusCycleDirection, FrontendEvent, LaunchWizardAction, LaunchWizardContext, LaunchWizardState,
    LinkedIssueKind, ProfileEnvEntryView, ProjectKind, UiTracePayload, WindowCanvasState,
    WindowGeometry, WindowPlacement, WindowPreset, WindowProcessStatus,
};
use gwt_config::{Profile, Settings};
use gwt_core::{
    coordination::{
        coordination_events_path, load_snapshot, post_entry, AuthorKind, BoardAudienceScope,
        BoardEntry, BoardEntryKind, BoardMention, BoardMentionTargetKind, CoordinationEvent,
    },
    logging::{current_log_file, LogLevel},
    paths::gwt_cache_dir,
    repo_hash::detect_repo_hash,
};
use gwt_github::{
    ApiError, Cache, CommentId, CommentSnapshot, FakeIssueClient, FetchResult, IssueClient,
    IssueNumber, IssueSnapshot, IssueState, SpecListFilter, SpecSummary, UpdatedAt,
};
use gwt_terminal::Pane;
use tracing::{field::Visit, Event, Level, Subscriber};
use tracing_subscriber::{layer::Context, prelude::*, Layer};

use super::{
    active_work_projection_from_saved, dispatch_agent_launch_success,
    save_start_work_workspace_projection, save_workspace_launch_projection, ActiveAgentSession,
    AgentKanbanLaunchTarget, AgentLaunchCompletion, AppEventProxy, AppRuntime,
    AttachmentProgressPhase, BlockingTaskSpawner, DispatchTarget, KnowledgeLoadRequest,
    KnowledgeRefreshTask, KnowledgeSearchRequest, LaunchFeedbackContext, LaunchWizardMemoryCache,
    LaunchWizardSession, OutboundEvent, ProcessLaunch, ProjectTabRuntime, UserEvent, WindowRuntime,
    WorkspaceLaunchProjectionKind, WorkspaceResumeContext,
};
use crate::{
    combined_window_id, geometry_to_pty_size, same_worktree_path, AttachmentUploadStore,
    PtyWriterRegistry, UploadedAttachment,
};

#[test]
fn improvement_action_error_message_explains_missing_github_auth() {
    let message = super::improvement_action_error_message("network error: authentication required");

    assert!(
        message.contains("GitHub authentication is required"),
        "message should explain the missing auth cause: {message}"
    );
    assert!(
        message.contains("gh auth login"),
        "message should give a concrete recovery command: {message}"
    );
    assert!(
        message.contains("GH_TOKEN"),
        "message should mention browser-check token bridging: {message}"
    );
}

#[derive(Debug, Clone)]
struct CapturedTracingEvent {
    level: Level,
    target: String,
    fields: HashMap<String, String>,
}

#[derive(Clone)]
struct CaptureTracingLayer {
    events: Arc<Mutex<Vec<CapturedTracingEvent>>>,
}

impl<S> Layer<S> for CaptureTracingLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = CaptureTracingVisitor::default();
        event.record(&mut visitor);
        self.events
            .lock()
            .expect("captured tracing events")
            .push(CapturedTracingEvent {
                level: *event.metadata().level(),
                target: event.metadata().target().to_string(),
                fields: visitor.fields,
            });
    }
}

#[derive(Default)]
struct CaptureTracingVisitor {
    fields: HashMap<String, String>,
}

#[cfg(unix)]
struct KillOnDrop(std::process::Child);

#[cfg(unix)]
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl CaptureTracingVisitor {
    fn insert(&mut self, field: &tracing::field::Field, value: impl ToString) {
        self.fields
            .insert(field.name().to_string(), value.to_string());
    }
}

impl Visit for CaptureTracingVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let raw = format!("{value:?}");
        self.insert(field, raw.trim_matches('"'));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.insert(field, value);
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.insert(field, value);
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.insert(field, value);
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.insert(field, value);
    }
}

fn capture_tracing_events(run: impl FnOnce()) -> Vec<CapturedTracingEvent> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CaptureTracingLayer {
        events: Arc::clone(&events),
    });
    tracing::subscriber::with_default(subscriber, run);
    let captured_events = events.lock().expect("captured tracing events").clone();
    captured_events
}

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<OsString>,
}

impl ScopedEnvVar {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn env_test_lock() -> &'static Mutex<()> {
    crate::env_test_lock()
}

fn fake_gh_test_lock() -> &'static Mutex<()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn write_profile_config(path: &Path, settings: &Settings) {
    settings.save(path).expect("write profile config");
}

fn sample_issue_snapshot(
    number: u64,
    title: &str,
    labels: &[&str],
    body: &str,
    updated_at: &str,
) -> IssueSnapshot {
    IssueSnapshot {
        number: IssueNumber(number),
        title: title.to_string(),
        body: body.to_string(),
        labels: labels.iter().map(|label| (*label).to_string()).collect(),
        state: IssueState::Open,
        updated_at: UpdatedAt::new(updated_at),
        comments: vec![CommentSnapshot {
            id: CommentId(number * 10),
            body: format!("Comment for #{number}"),
            updated_at: UpdatedAt::new(updated_at),
        }],
    }
}

fn init_repo(repo_path: &Path) {
    let remote = format!(
        "https://github.com/example/repo-{:x}.git",
        remote_suffix(repo_path)
    );
    for args in [
        ["init", "-q"].as_slice(),
        ["remote", "add", "origin", remote.as_str()].as_slice(),
    ] {
        let output = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(repo_path)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn init_repo_with_initial_commit(repo_path: &Path) {
    init_repo(repo_path);
    run_git(repo_path, &["config", "user.name", "Test User"]);
    run_git(repo_path, &["config", "user.email", "test@example.com"]);
    run_git(repo_path, &["commit", "--allow-empty", "-m", "init"]);
}

fn init_repo_without_origin(repo_path: &Path) {
    let output = gwt_core::process::hidden_command("git")
        .args(["init", "-q"])
        .current_dir(repo_path)
        .output()
        .expect("run git init");
    assert!(
        output.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
fn init_workspace_home_with_child_bare(workspace_home: &Path) -> PathBuf {
    fs::create_dir_all(workspace_home).expect("create workspace home");
    let bare_repo = workspace_home.join("repo.git");
    let remote = format!(
        "https://github.com/example/repo-{:x}.git",
        remote_suffix(workspace_home)
    );
    let init = gwt_core::process::hidden_command("git")
        .args(["init", "--bare", bare_repo.to_str().unwrap()])
        .output()
        .expect("git init bare");
    assert!(
        init.status.success(),
        "git init bare failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    let remote_add = gwt_core::process::hidden_command("git")
        .args([
            "-C",
            bare_repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            remote.as_str(),
        ])
        .output()
        .expect("git remote add");
    assert!(
        remote_add.status.success(),
        "git remote add failed: {}",
        String::from_utf8_lossy(&remote_add.stderr)
    );
    bare_repo
}

fn remote_suffix(repo_path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    repo_path.display().to_string().hash(&mut hasher);
    hasher.finish()
}

fn issue_cache_root(repo_path: &Path) -> PathBuf {
    let repo_hash = detect_repo_hash(repo_path).expect("repo hash");
    gwt_cache_dir().join("issues").join(repo_hash.as_str())
}

fn write_issue_link_store(repo_path: &Path, branches: HashMap<String, u64>) {
    let repo_hash = detect_repo_hash(repo_path).expect("repo hash");
    let path = gwt_cache_dir()
        .join("issue-links")
        .join(format!("{}.json", repo_hash.as_str()));
    fs::create_dir_all(path.parent().expect("parent")).expect("create link dir");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({ "branches": branches }))
            .expect("serialize store"),
    )
    .expect("write link store");
}

#[cfg(unix)]
fn write_fake_project_index_runtime(home: &Path) {
    let legacy_python = home
        .join(".gwt")
        .join("runtime")
        .join("chroma-venv")
        .join("bin")
        .join("python3");
    let script = r#"#!/bin/sh
for arg in "$@"; do
  if [ "$arg" = "-c" ]; then
    exit 0
  fi
done
case "$*" in
  *"-m pip"*)
    exit 0
    ;;
  *"--action probe"*)
    exit 0
    ;;
  *"--action search-issues"*)
    printf '%s\n' '{"ok":true,"issueResults":[{"number":42,"distance":0.25}]}'
    exit 0
    ;;
  *"--action search-specs"*)
    printf '%s\n' '{"ok":true,"specResults":[{"spec_id":1930,"distance":0.4}]}'
    exit 0
    ;;
esac
printf '%s\n' '{"ok":false,"error":"unexpected fake python invocation"}'
exit 1
"#;
    for python in [
        legacy_python,
        gwt_core::runtime::project_index_python_path(),
    ] {
        fs::create_dir_all(python.parent().expect("fake python parent"))
            .expect("create fake python dir");
        fs::write(&python, script).expect("write fake python");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).expect("chmod fake python");
    }
}

fn write_fake_gh_issue_list(temp_root: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let fake_gh = temp_root.join("gh.cmd");
        fs::write(
                &fake_gh,
                "@echo off\r\n\
if not \"%GWT_FAKE_GH_MARKER%\"==\"\" echo called>>\"%GWT_FAKE_GH_MARKER%\"\r\n\
if /I \"%GWT_FAKE_GH_MODE%\"==\"fail\" (\r\n\
  >&2 echo gh refresh failed\r\n\
  exit /b 1\r\n\
)\r\n\
echo [{\"number\":43,\"title\":\"Refreshed issue\",\"body\":\"Fresh body\",\"labels\":[{\"name\":\"bug\"}],\"state\":\"OPEN\",\"url\":\"https://example.test/issues/43\",\"updatedAt\":\"2026-04-20T00:00:00Z\"}]\r\n\
exit /b 0\r\n",
            )
            .expect("write fake gh");
        fake_gh
    }
    #[cfg(not(windows))]
    {
        let fake_gh = temp_root.join("gh");
        fs::write(
                &fake_gh,
                r#"#!/bin/sh
	if [ -n "$GWT_FAKE_GH_MARKER" ]; then
	  touch "$GWT_FAKE_GH_MARKER"
	fi
	if [ -n "$GWT_FAKE_GH_EXPECT_CWD" ] && [ "$(pwd)" != "$GWT_FAKE_GH_EXPECT_CWD" ]; then
	  printf '%s\n' "wrong cwd: $(pwd)" >&2
	  exit 1
	fi
	if [ "$GWT_FAKE_GH_MODE" = "fail" ]; then
	  printf '%s\n' 'gh refresh failed' >&2
	  exit 1
fi
printf '%s\n' '[{"number":43,"title":"Refreshed issue","body":"Fresh body","labels":[{"name":"bug"}],"state":"OPEN","url":"https://example.test/issues/43","updatedAt":"2026-04-20T00:00:00Z"}]'
exit 0
"#,
            )
            .expect("write fake gh");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_gh, fs::Permissions::from_mode(0o755)).expect("chmod fake gh");
        fake_gh
    }
}

fn write_fake_git_recorder(temp_root: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let fake_git = temp_root.join("git.cmd");
        fs::write(
            &fake_git,
            "@echo off\r\n\
if not \"%GWT_FAKE_GIT_LOG%\"==\"\" echo %*>>\"%GWT_FAKE_GIT_LOG%\"\r\n\
exit /b 1\r\n",
        )
        .expect("write fake git");
        fake_git
    }
    #[cfg(not(windows))]
    {
        let fake_git = temp_root.join("git");
        fs::write(
            &fake_git,
            r#"#!/bin/sh
if [ -n "$GWT_FAKE_GIT_LOG" ]; then
  printf '%s\n' "$*" >> "$GWT_FAKE_GIT_LOG"
fi
exit 1
"#,
        )
        .expect("write fake git");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o755)).expect("chmod fake git");
        fake_git
    }
}

fn prepend_tool_parent_to_path(tool: &Path) -> ScopedEnvVar {
    let parent = tool.parent().expect("tool parent");
    let mut paths = vec![parent.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    let joined = std::env::join_paths(paths).expect("join PATH");
    ScopedEnvVar::set("PATH", joined)
}

fn prepend_fake_gh_to_path(fake_gh: &Path) -> ScopedEnvVar {
    prepend_tool_parent_to_path(fake_gh)
}

fn canvas_bounds() -> WindowGeometry {
    WindowGeometry {
        x: 0.0,
        y: 0.0,
        width: 1400.0,
        height: 900.0,
    }
}

fn sample_window(
    raw_id: &str,
    preset: WindowPreset,
    status: WindowProcessStatus,
) -> gwt::PersistedWindowState {
    gwt::PersistedWindowState {
        id: raw_id.to_string(),
        title: "Sample".to_string(),
        preset,
        geometry: WindowGeometry {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 420.0,
        },
        geometry_revision: 0,
        z_index: 1,
        status,
        placement: WindowPlacement::Canvas,
        persist: true,
        purpose_title: None,
        dynamic_title: None,
        dynamic_title_detail: None,
        agent_id: None,
        agent_color: None,
        lane_kind: gwt::WindowLaneKind::Unknown,
        tab_group_id: None,
        tab_group_active: false,
        session_id: None,
    }
}

fn sample_project_tab_with_window(
    tab_id: &str,
    raw_window_id: &str,
    preset: WindowPreset,
    status: WindowProcessStatus,
) -> ProjectTabRuntime {
    sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        PathBuf::from("E:/gwt/test-repo"),
        preset,
        status,
    )
}

fn sample_project_tab_with_window_at(
    tab_id: &str,
    raw_window_id: &str,
    project_root: PathBuf,
    preset: WindowPreset,
    status: WindowProcessStatus,
) -> ProjectTabRuntime {
    let mut persisted = empty_workspace_state();
    persisted
        .windows
        .push(sample_window(raw_window_id, preset, status));
    persisted.next_z_index = 2;
    ProjectTabRuntime {
        id: tab_id.to_string(),
        title: "Repo".to_string(),
        project_root,
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    }
}

fn sample_project_tab(
    tab_id: &str,
    title: &str,
    project_root: PathBuf,
    kind: ProjectKind,
    presets: &[WindowPreset],
) -> ProjectTabRuntime {
    let mut workspace = WindowCanvasState::from_persisted(empty_workspace_state());
    for preset in presets {
        let _ = workspace.add_window(*preset, canvas_bounds());
    }
    ProjectTabRuntime {
        id: tab_id.to_string(),
        title: title.to_string(),
        project_root,
        kind,
        workspace,
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    }
}

#[test]
fn app_runtime_rejects_removed_legacy_memo_window_creation() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.create_window_events(WindowPreset::Memo, canvas_bounds());

    assert!(events.is_empty());
    assert!(runtime.window_lookup.is_empty());
    assert!(runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .is_empty());
}

// SPEC-2359 Workspace → Work → Session: a Work row (keyed by the gwt session
// id / launch) is enriched with its Session history (agent-tool conversation
// UUIDs) read from the persisted Session, with the latest marked active.
#[test]
fn workspace_work_agent_view_attaches_session_history() {
    let mut session = gwt_agent::Session::new("/tmp/wt", "feature/x", gwt_agent::AgentId::Codex);
    session.id = "work-1".to_string();
    session.agent_session_id = Some("conv-2".to_string());
    session.session_history = vec![
        gwt_agent::AgentSessionHistoryEntry {
            agent_session_id: "conv-1".to_string(),
            started_at: chrono::Utc::now(),
        },
        gwt_agent::AgentSessionHistoryEntry {
            agent_session_id: "conv-2".to_string(),
            started_at: chrono::Utc::now(),
        },
    ];
    let sessions = vec![session];
    let index = super::work_session_index(&sessions);

    let agent_ref = gwt_core::workspace_projection::WorkAgentRef {
        session_id: "work-1".to_string(),
        agent_id: Some("codex".to_string()),
        display_name: Some("Codex".to_string()),
        updated_at: chrono::Utc::now(),
        attached_by: None,
    };
    let view = super::workspace_work_agent_view_from_ref(&agent_ref, &index, Path::new("/"));

    let ids: Vec<&str> = view
        .sessions
        .iter()
        .map(|session| session.agent_session_id.as_str())
        .collect();
    assert_eq!(ids, vec!["conv-1", "conv-2"]);
    assert!(!view.sessions[0].is_active);
    assert!(view.sessions[1].is_active, "latest conversation is active");

    // A Work with no persisted Session yields an empty Session list, not a panic.
    let empty_index = super::work_session_index(&[]);
    let empty_view =
        super::workspace_work_agent_view_from_ref(&agent_ref, &empty_index, Path::new("/"));
    assert!(empty_view.sessions.is_empty());
}

fn sample_active_agent_session(tab_id: &str, window_id: &str) -> ActiveAgentSession {
    ActiveAgentSession {
        window_id: window_id.to_string(),
        session_id: "session-1".to_string(),
        agent_id: "codex".to_string(),
        branch_name: "feature/test".to_string(),
        display_name: "Codex".to_string(),
        worktree_path: PathBuf::from("E:/gwt/test-repo"),
        agent_project_root: "E:/gwt/test-repo".to_string(),
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        tab_id: tab_id.to_string(),
    }
}

fn save_assigned_workspace_projection_for_test(
    repo: &Path,
    session: &ActiveAgentSession,
) -> Result<(), String> {
    let context = WorkspaceResumeContext {
        title: Some("Start Work".to_string()),
        owner: Some("SPEC-2359".to_string()),
        summary: Some("Assigned Workspace".to_string()),
        next_action: Some("Check Board for latest updates".to_string()),
    };
    let live: std::collections::HashSet<String> =
        std::iter::once(session.session_id.clone()).collect();
    save_workspace_launch_projection(
        repo,
        session,
        Some("develop"),
        None,
        Some(&context),
        WorkspaceLaunchProjectionKind::StartWork,
        &live,
    )
}

fn workspace_agent_summary_for_test(
    session_id: &str,
    workspace_id: Option<&str>,
) -> gwt_core::workspace_projection::WorkspaceAgentSummary {
    gwt_core::workspace_projection::WorkspaceAgentSummary {
        session_id: session_id.to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
        current_focus: Some("Board audience follow-up".to_string()),
        title_summary: Some("Board audience follow-up".to_string()),
        worktree_path: None,
        branch: Some("work/test".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status:
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: workspace_id.map(str::to_string),
        updated_at: chrono::Utc::now(),
    }
}

// #3065: the Workspace Resume context must come from the resumed branch's
// own Work item — never from the repo-shared current projection, whose
// identity may belong to a different Work.
#[test]
fn workspace_resume_context_prefers_work_item_over_shared_projection() {
    let _env_guard = env_test_lock().lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");

    // Shared current projection carries a foreign work's identity.
    let mut shared =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    shared.title = "gwt-manage-pr".to_string();
    shared.owner = Some("SPEC-2359".to_string());
    shared.next_action = Some("foreign next action".to_string());
    gwt_core::workspace_projection::save_workspace_projection(&repo, &shared)
        .expect("save shared projection");

    // The resumed branch has its own Work item with its own identity.
    let now = chrono::Utc::now();
    let work_id = gwt_core::workspace_projection::canonical_work_id(&repo, Some("work/foo"), None)
        .expect("canonical id");
    let mut event = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id,
        now,
    );
    event.title = Some("fix foo".to_string());
    event.owner = Some("Issue #42".to_string());
    event.next_action = Some("own next action".to_string());
    event.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/foo".to_string()),
            worktree_path: Some(repo.join("wt-foo")),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, event)
        .expect("record work event");

    let context = super::workspace_resume_context_for_work_item(
        &repo,
        Some("work/foo"),
        &repo.join("wt-foo"),
    );
    assert_eq!(context.title.as_deref(), Some("fix foo"));
    assert_eq!(context.owner.as_deref(), Some("Issue #42"));
    assert_eq!(context.next_action.as_deref(), Some("own next action"));

    // Unknown container: neutral context — never the shared identity.
    let fallback = super::workspace_resume_context_for_work_item(
        &repo,
        Some("work/unknown"),
        &repo.join("wt-unknown"),
    );
    assert_eq!(fallback.owner, None, "shared owner must not leak");
    assert_eq!(fallback.title, None, "shared title must not leak");
    assert_eq!(fallback.next_action, None);
}

// #3065: launch saves retain only live agent sessions in the shared
// projection so dead entries stop accumulating ("765 active agents").
#[test]
fn save_workspace_launch_projection_retains_only_live_agents() {
    let _env_guard = env_test_lock().lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");
    let session = sample_active_agent_session("tab-1", "win-1");

    let work_id = gwt_core::workspace_projection::canonical_work_id(
        &repo,
        Some(&session.branch_name),
        Some(&session.worktree_path),
    )
    .expect("canonical id");
    let mut shared =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    shared.id = work_id.clone();
    shared
        .agents
        .push(workspace_agent_summary_for_test("dead-1", Some(&work_id)));
    gwt_core::workspace_projection::save_workspace_projection(&repo, &shared)
        .expect("save shared projection");

    let live: std::collections::HashSet<String> =
        std::iter::once(session.session_id.clone()).collect();
    let context = WorkspaceResumeContext {
        title: None,
        owner: None,
        summary: None,
        next_action: None,
    };
    save_workspace_launch_projection(
        &repo,
        &session,
        Some("develop"),
        None,
        Some(&context),
        WorkspaceLaunchProjectionKind::StartWork,
        &live,
    )
    .expect("save launch projection");

    let stored = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load")
        .expect("projection exists");
    assert!(
        stored
            .agents
            .iter()
            .all(|agent| agent.session_id != "dead-1"),
        "dead agent session is dropped"
    );
    assert!(
        stored
            .agents
            .iter()
            .any(|agent| agent.session_id == session.session_id),
        "launching session is kept"
    );
    assert_eq!(stored.status_text, "Codex is running");
}

// SPEC-2359 US-80 (FR-427/FR-429): a Start-Work Shell registers as a
// first-class Work and is not pruned when an agent later launches on another
// branch.
#[test]
fn save_shell_work_projection_registers_shell_and_survives_agent_launch() {
    let _env_guard = env_test_lock().lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");

    // シナリオ1: registering a Start-Work Shell makes it a Work in the projection.
    let empty: std::collections::HashSet<String> = std::collections::HashSet::new();
    super::save_shell_work_projection(
        &repo,
        "tab-1:shell-3",
        Some(repo.join("wt-shell")),
        Some("work/shell-x".to_string()),
        &empty,
    )
    .expect("register shell work");

    let stored = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load")
        .expect("projection exists");
    let shell = stored
        .agents
        .iter()
        .find(|agent| agent.is_shell_work())
        .expect("shell work present");
    assert_eq!(shell.session_id, "tab-1:shell-3");
    assert_eq!(shell.display_name, "Shell");
    assert_eq!(shell.branch.as_deref(), Some("work/shell-x"));

    // シナリオ3 / FR-429: launching an agent on a different branch keeps the
    // running Shell Work.
    let session = sample_active_agent_session("tab-2", "win-2");
    save_assigned_workspace_projection_for_test(&repo, &session).expect("agent launch save");

    let after = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load")
        .expect("projection exists");
    assert!(
        after
            .agents
            .iter()
            .any(|agent| agent.is_shell_work() && agent.session_id == "tab-1:shell-3"),
        "agent launch on another branch must keep the Shell Work"
    );
    assert!(
        after
            .agents
            .iter()
            .any(|agent| !agent.is_shell_work() && agent.session_id == session.session_id),
        "the launched agent is present"
    );
}

// SPEC-2359 US-80 (FR-429): the Active Work broadcast rebuild prunes dead
// agents but must keep Shell Works (no agent session), otherwise a
// just-registered shell would vanish from the broadcast.
#[test]
fn retain_live_workspace_agents_keeps_shell_work_with_no_live_sessions() {
    let repo = std::path::Path::new("/repo");
    let now = chrono::Utc::now();
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(repo);
    projection.agents.push(
        gwt_core::workspace_projection::WorkspaceAgentSummary::shell_work(
            "tab-1:shell-3",
            Some(repo.join("wt-x")),
            Some("work/x".to_string()),
            gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            now,
        ),
    );
    projection
        .agents
        .push(workspace_agent_summary_for_test("dead-agent", None));

    super::retain_live_workspace_agents(&mut projection, &[], now);

    assert!(
        projection.agents.iter().any(|agent| agent.is_shell_work()),
        "shell work survives the broadcast retain with no live sessions"
    );
    assert!(
        !projection
            .agents
            .iter()
            .any(|agent| agent.session_id == "dead-agent"),
        "dead agent is pruned"
    );
    assert!(
        projection.has_current_agents(),
        "a lone shell keeps the projection out of the idle reset"
    );
}

// SPEC-2359 US-80 (FR-430, シナリオ1/2): a Shell Work summary surfaces as a
// Work row in the Active Work projection view, and an agent on the same branch
// groups into the same Work.
#[test]
fn active_work_view_surfaces_shell_work_and_groups_with_same_branch_agent() {
    let repo = std::path::Path::new("/repo");
    let now = chrono::Utc::now();
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(repo);
    projection.agents.push(
        gwt_core::workspace_projection::WorkspaceAgentSummary::shell_work(
            "tab-1:shell-3",
            Some(repo.join("wt-x")),
            Some("work/x".to_string()),
            gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            now,
        ),
    );

    let view = super::active_work_projection_from_saved(projection.clone());
    assert_eq!(
        view.active_agents, 1,
        "the shell work surfaces as one active work row"
    );

    // Same-branch agent groups into one Work (FR-430 / シナリオ2).
    let mut agent = workspace_agent_summary_for_test("agent-1", None);
    agent.branch = Some("work/x".to_string());
    agent.worktree_path = Some(repo.join("wt-x"));
    projection.agents.push(agent);
    let grouped = super::active_work_projection_from_saved(projection);
    assert_eq!(
        grouped.active_agents, 2,
        "shell + agent on the same branch both appear as active rows"
    );
}

#[test]
fn image_paste_prepare_uses_drop_files_relative_path_reference() {
    let temp = tempdir().expect("tempdir");
    let payload = base64::engine::general_purpose::STANDARD.encode(b"image-bytes");
    let agent_root = temp.path().display().to_string();

    let prepared = super::prepare_image_paste_file(
        temp.path(),
        &agent_root,
        &payload,
        "image/png",
        Some("../Screen Shot.png"),
        "20260507-160000",
    )
    .expect("prepare image paste");
    let expected_path = temp
        .path()
        .join(".gwt")
        .join("drop-files")
        .join("20260507-160000-screen-shot.png");

    assert_eq!(prepared.bytes.as_deref(), Some(&b"image-bytes"[..]));
    assert_eq!(prepared.storage_path, expected_path);
    assert_eq!(
        prepared.agent_path,
        ".gwt/drop-files/20260507-160000-screen-shot.png"
    );
    assert_eq!(
        prepared.prompt_text,
        "Image file: .gwt/drop-files/20260507-160000-screen-shot.png"
    );
}

#[test]
fn image_paste_prepare_uses_docker_project_root_reference() {
    let temp = tempdir().expect("tempdir");
    let payload = base64::engine::general_purpose::STANDARD.encode(b"jpeg-bytes");

    let prepared = super::prepare_image_paste_file(
        temp.path(),
        "/workspace/project",
        &payload,
        "image/jpeg",
        Some("Clipboard Image"),
        "20260507-160001",
    )
    .expect("prepare docker image paste");

    assert_eq!(
        prepared.storage_path,
        temp.path()
            .join(".gwt")
            .join("drop-files")
            .join("20260507-160001-clipboard-image.jpg")
    );
    assert_eq!(
        prepared.agent_path,
        ".gwt/drop-files/20260507-160001-clipboard-image.jpg"
    );
    assert_eq!(
        prepared.prompt_text,
        "Image file: .gwt/drop-files/20260507-160001-clipboard-image.jpg"
    );
}

#[test]
fn image_paste_prepare_rejects_unsupported_mime_and_empty_payload() {
    let temp = tempdir().expect("tempdir");
    let payload = base64::engine::general_purpose::STANDARD.encode(b"gif-bytes");

    let unsupported = super::prepare_image_paste_file(
        temp.path(),
        "/workspace/project",
        &payload,
        "image/gif",
        Some("unsupported.gif"),
        "20260507-160002",
    );
    assert!(matches!(
        unsupported,
        Err(super::ImagePasteError::UnsupportedMimeType(mime)) if mime == "image/gif"
    ));

    let empty = super::prepare_image_paste_file(
        temp.path(),
        "/workspace/project",
        "",
        "image/png",
        None,
        "20260507-160003",
    );
    assert!(matches!(empty, Err(super::ImagePasteError::EmptyPayload)));
}

#[test]
fn image_paste_event_saves_file_under_worktree() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/image-paste".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );
    let payload = base64::engine::general_purpose::STANDARD.encode(b"webp-bytes");
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "paste_image",
        "id": window_id,
        "data_base64": payload,
        "mime_type": "image/webp",
        "filename": "capture.webp"
    }))
    .expect("deserialize paste image event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(events.is_empty());
    let drop_dir = worktree.join(".gwt").join("drop-files");
    let files = fs::read_dir(&drop_dir)
        .expect("read drop dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect paste files");
    assert_eq!(files.len(), 1, "expected one saved image");
    assert!(
        !worktree.join(".gwt").join("paste-images").exists(),
        "new image paste files must not be written to legacy paste-images"
    );
    let saved_path = files[0].path();
    assert_eq!(
        saved_path.extension().and_then(|ext| ext.to_str()),
        Some("webp")
    );
    assert_eq!(
        fs::read(saved_path).expect("read saved image"),
        b"webp-bytes"
    );
}

#[test]
fn uploaded_image_paste_event_saves_file_under_drop_files() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let uploaded_path = temp.path().join("image-upload.tmp");
    fs::write(&uploaded_path, b"png-upload").expect("write uploaded image");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    runtime
        .attachment_uploads
        .insert(
            "image-upload-1".to_string(),
            UploadedAttachment {
                path: uploaded_path.clone(),
                filename: "Screenshot.png".to_string(),
                mime_type: Some("image/png".to_string()),
                size: 10,
            },
        )
        .expect("register image upload");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/image-paste".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "paste_image_uploaded",
        "id": window_id,
        "upload_id": "image-upload-1",
        "mime_type": "image/png",
        "filename": "Screenshot.png",
        "size": 10
    }))
    .expect("deserialize uploaded paste image event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(events.is_empty());
    let drop_dir = worktree.join(".gwt").join("drop-files");
    let files = fs::read_dir(&drop_dir)
        .expect("read drop dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect drop files");
    assert_eq!(files.len(), 1, "expected one saved uploaded image");
    assert_eq!(
        fs::read(files[0].path()).expect("read saved image"),
        b"png-upload"
    );
    assert!(
        !uploaded_path.exists(),
        "uploaded temp image should be removed"
    );
    assert!(
        !worktree.join(".gwt").join("paste-images").exists(),
        "uploaded image paste must not create legacy paste-images"
    );
}

#[test]
fn image_paste_event_ignores_non_agent_terminal_window() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab_id = "tab-1";
    let raw_window_id = "shell-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Shell,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    let payload = base64::engine::general_purpose::STANDARD.encode(b"png-bytes");
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "paste_image",
        "id": window_id,
        "data_base64": payload,
        "mime_type": "image/png",
        "filename": "capture.png"
    }))
    .expect("deserialize paste image event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(events.is_empty());
    assert!(
        !worktree.join(".gwt").join("drop-files").exists(),
        "non-agent terminal paste must not create image files"
    );
}

#[test]
fn file_attachment_prepare_copies_host_native_path_under_drop_files() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let source = temp.path().join("report.pdf");
    fs::write(&source, b"host-pdf").expect("write source file");

    let prepared = super::prepare_file_attachment(
        &worktree,
        &worktree.display().to_string(),
        gwt_agent::LaunchRuntimeTarget::Host,
        &gwt::FileAttachment::NativePath {
            path: source.display().to_string(),
        },
        "20260524-attach",
        ContentLimits::default(),
        &AttachmentUploadStore::in_system_temp(),
    )
    .expect("prepare host native path attachment");

    let expected_path = worktree
        .join(".gwt")
        .join("drop-files")
        .join("20260524-attach-report.pdf");
    assert_eq!(prepared.bytes, None);
    assert_eq!(prepared.source_path.as_deref(), Some(source.as_path()));
    assert_eq!(
        prepared.storage_path.as_deref(),
        Some(expected_path.as_path())
    );
    assert_eq!(
        prepared.agent_path,
        ".gwt/drop-files/20260524-attach-report.pdf"
    );
    assert_eq!(
        super::format_file_attachment_prompt(&[prepared.agent_path]),
        "File: \".gwt/drop-files/20260524-attach-report.pdf\""
    );
}

#[test]
fn file_attachment_prepare_copies_inline_file_under_worktree() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let payload = base64::engine::general_purpose::STANDARD.encode(b"notes-bytes");

    let prepared = super::prepare_file_attachment(
        &worktree,
        &worktree.display().to_string(),
        gwt_agent::LaunchRuntimeTarget::Host,
        &gwt::FileAttachment::Inline {
            filename: "../Notes 2026.txt".to_string(),
            mime_type: Some("application/octet-stream".to_string()),
            size: 11,
            data_base64: payload,
        },
        "20260524-inline",
        ContentLimits::default(),
        &AttachmentUploadStore::in_system_temp(),
    )
    .expect("prepare inline file attachment");

    let expected_path = worktree
        .join(".gwt")
        .join("drop-files")
        .join("20260524-inline-notes-2026.txt");
    assert_eq!(prepared.bytes.as_deref(), Some(&b"notes-bytes"[..]));
    assert_eq!(
        prepared.storage_path.as_deref(),
        Some(expected_path.as_path())
    );
    assert_eq!(
        prepared.agent_path,
        ".gwt/drop-files/20260524-inline-notes-2026.txt"
    );
}

#[test]
fn file_attachment_prepare_preserves_japanese_unicode_basename() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let payload = base64::engine::general_purpose::STANDARD.encode(b"unicode-notes");

    let prepared = super::prepare_file_attachment(
        &worktree,
        &worktree.display().to_string(),
        gwt_agent::LaunchRuntimeTarget::Host,
        &gwt::FileAttachment::Inline {
            filename: "../資料 日本語.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            size: 13,
            data_base64: payload,
        },
        "20260604-inline",
        ContentLimits::default(),
        &AttachmentUploadStore::in_system_temp(),
    )
    .expect("prepare unicode filename attachment");

    assert_eq!(
        prepared.storage_path.as_deref(),
        Some(
            worktree
                .join(".gwt")
                .join("drop-files")
                .join("20260604-inline-資料-日本語.txt")
                .as_path()
        )
    );
    assert_eq!(
        prepared.agent_path,
        ".gwt/drop-files/20260604-inline-資料-日本語.txt"
    );
    assert_eq!(
        super::format_file_attachment_prompt(&[prepared.agent_path]),
        "File: \".gwt/drop-files/20260604-inline-資料-日本語.txt\""
    );
}

#[test]
fn file_attachment_prepare_copies_native_file_for_docker_agent_path() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let source = temp.path().join("Data Set.bin");
    fs::write(&source, b"docker-bytes").expect("write source file");

    let prepared = super::prepare_file_attachment(
        &worktree,
        "/workspace/project",
        gwt_agent::LaunchRuntimeTarget::Docker,
        &gwt::FileAttachment::NativePath {
            path: source.display().to_string(),
        },
        "20260524-docker",
        ContentLimits::default(),
        &AttachmentUploadStore::in_system_temp(),
    )
    .expect("prepare docker native file attachment");

    assert_eq!(prepared.bytes, None);
    assert_eq!(prepared.source_path.as_deref(), Some(source.as_path()));
    assert_eq!(
        prepared.storage_path.as_deref(),
        Some(
            worktree
                .join(".gwt")
                .join("drop-files")
                .join("20260524-docker-data-set.bin")
                .as_path()
        )
    );
    assert_eq!(
        prepared.agent_path,
        ".gwt/drop-files/20260524-docker-data-set.bin"
    );
}

#[test]
fn file_attachment_prepare_rejects_invalid_items() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let payload = base64::engine::general_purpose::STANDARD.encode(b"too-large");
    let limits = ContentLimits {
        text_max_bytes: 16,
        binary_chunk_max_bytes: 3,
    };

    let too_large = super::prepare_file_attachment(
        &worktree,
        "/workspace/project",
        gwt_agent::LaunchRuntimeTarget::Host,
        &gwt::FileAttachment::Inline {
            filename: "large.dat".to_string(),
            mime_type: None,
            size: 9,
            data_base64: payload,
        },
        "20260524-large",
        limits,
        &AttachmentUploadStore::in_system_temp(),
    );
    assert!(matches!(
        too_large,
        Err(super::FileAttachmentError::TooLarge { size: 9, limit: 3 })
    ));

    let directory = super::prepare_file_attachment(
        &worktree,
        "/workspace/project",
        gwt_agent::LaunchRuntimeTarget::Host,
        &gwt::FileAttachment::NativePath {
            path: temp.path().display().to_string(),
        },
        "20260524-dir",
        ContentLimits::default(),
        &AttachmentUploadStore::in_system_temp(),
    );
    assert!(matches!(
        directory,
        Err(super::FileAttachmentError::NotAFile(path)) if path == temp.path().display().to_string()
    ));
}

#[test]
fn file_attachment_prompt_formats_single_and_multiple_paths_without_newlines() {
    let single = super::format_file_attachment_prompt(&["/tmp/a\"b\nc.txt".to_string()]);
    assert_eq!(single, "File: \"/tmp/a\\\"b\\nc.txt\"");
    assert!(
        !single.contains('\n'),
        "single-file prompt must never inject a newline"
    );

    let multiple = super::format_file_attachment_prompt(&[
        "/tmp/a.txt".to_string(),
        "C:\\tmp\\b\r.txt".to_string(),
    ]);
    assert_eq!(
        multiple,
        "Files: [\"/tmp/a.txt\", \"C:\\\\tmp\\\\b\\r.txt\"]"
    );
    assert!(
        !multiple.contains('\r') && !multiple.contains('\n'),
        "multi-file prompt must stay on one terminal input line"
    );
}

#[test]
fn file_attachment_event_saves_inline_file_under_worktree() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/file-drop".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );
    let payload = base64::engine::general_purpose::STANDARD.encode(b"text-bytes");
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "attach_files",
        "id": window_id,
        "files": [
            {
                "source": "inline",
                "filename": "notes.txt",
                "mime_type": "text/plain",
                "size": 10,
                "data_base64": payload
            }
        ]
    }))
    .expect("deserialize attach files event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(events.is_empty());
    let drop_dir = worktree.join(".gwt").join("drop-files");
    let files = fs::read_dir(&drop_dir)
        .expect("read drop dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect drop files");
    assert_eq!(files.len(), 1, "expected one saved dropped file");
    assert_eq!(
        fs::read(files[0].path()).expect("read saved file"),
        b"text-bytes"
    );
}

#[test]
fn file_attachment_event_saves_native_path_under_drop_files() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let source = temp.path().join("large-host-file.bin");
    fs::write(&source, b"native-bytes").expect("write native source");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/file-drop".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "attach_files",
        "id": window_id,
        "files": [
            {
                "source": "native_path",
                "path": source.display().to_string()
            }
        ]
    }))
    .expect("deserialize attach files event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(events.is_empty());
    let drop_dir = worktree.join(".gwt").join("drop-files");
    let files = fs::read_dir(&drop_dir)
        .expect("read drop dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect drop files");
    assert_eq!(files.len(), 1, "expected one saved native file");
    assert_eq!(
        fs::read(files[0].path()).expect("read saved file"),
        b"native-bytes"
    );
    assert!(
        files[0]
            .file_name()
            .to_string_lossy()
            .ends_with("large-host-file.bin"),
        "saved native file should keep sanitized source basename"
    );
}

#[test]
fn file_attachment_event_saves_uploaded_file_under_drop_files() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let uploaded_path = temp.path().join("upload.tmp");
    fs::write(&uploaded_path, b"uploaded-bytes").expect("write uploaded temp");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    runtime
        .attachment_uploads
        .insert(
            "upload-1".to_string(),
            UploadedAttachment {
                path: uploaded_path.clone(),
                filename: "Browser Large.bin".to_string(),
                mime_type: Some("application/octet-stream".to_string()),
                size: 14,
            },
        )
        .expect("register upload");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/file-drop".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "attach_files",
        "id": window_id,
        "files": [
            {
                "source": "uploaded",
                "upload_id": "upload-1",
                "filename": "Browser Large.bin",
                "mime_type": "application/octet-stream",
                "size": 14
            }
        ]
    }))
    .expect("deserialize attach files event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(events.is_empty());
    let drop_dir = worktree.join(".gwt").join("drop-files");
    let files = fs::read_dir(&drop_dir)
        .expect("read drop dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect drop files");
    assert_eq!(files.len(), 1, "expected one saved uploaded file");
    assert_eq!(
        fs::read(files[0].path()).expect("read saved file"),
        b"uploaded-bytes"
    );
    assert!(
        !uploaded_path.exists(),
        "uploaded temp file should be removed after staging"
    );
}

#[test]
fn file_attachment_operation_dispatches_failed_progress_without_prompt_on_stage_failure() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/file-drop".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "attach_files",
        "id": window_id,
        "operation_id": "attachment-op-fail",
        "files": [
            {
                "source": "native_path",
                "path": temp.path().display().to_string()
            }
        ]
    }))
    .expect("deserialize operation attach files event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(
            events.iter().any(|event| matches!(
                &event.event,
                BackendEvent::AttachmentProgress {
                    id,
                    operation_id,
                    phase: AttachmentProgressPhase::Queued,
                    ..
                } if id == &window_id && operation_id == "attachment-op-fail"
            )),
            "operation-aware attachment handling should acknowledge queued progress immediately: {events:?}"
        );
    wait_for_recorded_event("failed attachment progress", &recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                event,
                UserEvent::Dispatch(dispatched)
                    if dispatched.iter().any(|outbound| matches!(
                        &outbound.event,
                        BackendEvent::AttachmentProgress {
                            id,
                            operation_id,
                            phase: AttachmentProgressPhase::Failed,
                            message: Some(message),
                            ..
                        } if id == &window_id
                            && operation_id == "attachment-op-fail"
                            && message.contains("not a file")
                    ))
            )
        })
    });
    {
        let events = recorded_events.lock().expect("event log");
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, UserEvent::AttachmentPromptReady { .. })),
            "failed staging must not enqueue terminal prompt injection"
        );
    }
}

#[test]
fn file_attachment_copy_reports_byte_progress() {
    let temp = tempdir().expect("tempdir");
    let source = temp.path().join("日本語-source.bin");
    let storage = temp
        .path()
        .join("repo")
        .join(".gwt")
        .join("drop-files")
        .join("20260604-日本語-source.bin");
    let payload = vec![b'x'; 192 * 1024 + 7];
    fs::write(&source, &payload).expect("write source file");
    let prepared = super::PreparedFileAttachment {
        bytes: None,
        source_path: Some(source.clone()),
        remove_source_after_save: false,
        storage_path: Some(storage.clone()),
        agent_path: ".gwt/drop-files/20260604-日本語-source.bin".to_string(),
    };
    let mut progress = Vec::new();

    super::save_file_attachment_with_progress(&prepared, |bytes_done, bytes_total| {
        progress.push((bytes_done, bytes_total));
    })
    .expect("copy attachment with progress");

    assert_eq!(fs::read(&storage).expect("read copied file"), payload);
    assert!(
            progress.len() >= 3,
            "copy should report an initial sample, at least one chunk, and final completion: {progress:?}"
        );
    assert_eq!(progress.first(), Some(&(0, Some(192 * 1024 + 7))));
    assert_eq!(
        progress.last(),
        Some(&(192 * 1024 + 7, Some(192 * 1024 + 7)))
    );
    assert!(
        progress
            .windows(2)
            .all(|pair| pair[0].0 <= pair[1].0 && pair[0].1 == pair[1].1),
        "copy progress must be monotonic with stable total: {progress:?}"
    );
}

#[test]
fn file_attachment_event_saves_prepared_files_incrementally() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/file-drop".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );
    let valid_payload = base64::engine::general_purpose::STANDARD.encode(b"saved-first");
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "attach_files",
        "id": window_id,
        "files": [
            {
                "source": "inline",
                "filename": "first.txt",
                "mime_type": "text/plain",
                "size": 11,
                "data_base64": valid_payload
            },
            {
                "source": "inline",
                "filename": "invalid.txt",
                "mime_type": "text/plain",
                "size": 4,
                "data_base64": "not base64"
            }
        ]
    }))
    .expect("deserialize attach files event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(
        events.is_empty(),
        "invalid later attachment must not inject a partial prompt",
    );
    let drop_dir = worktree.join(".gwt").join("drop-files");
    let files = fs::read_dir(&drop_dir)
        .expect("read drop dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect drop files");
    assert_eq!(
        files.len(),
        1,
        "first attachment should be saved before a later file fails",
    );
    assert_eq!(
        fs::read(files[0].path()).expect("read saved file"),
        b"saved-first"
    );
}

#[test]
fn file_attachment_event_ignores_non_agent_terminal_window() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab_id = "tab-1";
    let raw_window_id = "shell-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Shell,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    let payload = base64::engine::general_purpose::STANDARD.encode(b"ignored");
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "attach_files",
        "id": window_id,
        "files": [
            {
                "source": "inline",
                "filename": "ignored.txt",
                "mime_type": "text/plain",
                "size": 7,
                "data_base64": payload
            }
        ]
    }))
    .expect("deserialize attach files event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(events.is_empty());
    assert!(
        !worktree.join(".gwt").join("drop-files").exists(),
        "non-agent terminal file drop must not create files"
    );
}

fn runtime_hook_state(status: &str, session_id: &str) -> gwt::RuntimeHookEvent {
    runtime_hook_state_for_event(status, "Stop", session_id)
}

fn runtime_hook_state_for_event(
    status: &str,
    source_event: &str,
    session_id: &str,
) -> gwt::RuntimeHookEvent {
    gwt::RuntimeHookEvent {
        kind: gwt::RuntimeHookEventKind::RuntimeState,
        source_event: Some(source_event.to_string()),
        gwt_session_id: Some(session_id.to_string()),
        agent_session_id: Some("agent-session-1".to_string()),
        project_root: Some("E:/gwt/test-repo".to_string()),
        branch: Some("feature/test".to_string()),
        status: Some(status.to_string()),
        tool_name: None,
        message: None,
        occurred_at: "2026-04-25T00:00:00Z".to_string(),
    }
}

fn runtime_hook_coordination_event(session_id: &str) -> gwt::RuntimeHookEvent {
    gwt::RuntimeHookEvent {
        kind: gwt::RuntimeHookEventKind::CoordinationEvent,
        source_event: Some("PostToolUse".to_string()),
        gwt_session_id: Some(session_id.to_string()),
        agent_session_id: Some("agent-session-1".to_string()),
        project_root: Some("E:/gwt/test-repo".to_string()),
        branch: Some("feature/test".to_string()),
        status: None,
        tool_name: Some("TodoWrite".to_string()),
        message: Some("coordination:PostToolUse".to_string()),
        occurred_at: "2026-04-25T00:00:00Z".to_string(),
    }
}

fn sample_runtime(
    temp_root: &Path,
    tabs: Vec<ProjectTabRuntime>,
    active_tab_id: Option<&str>,
) -> AppRuntime {
    sample_runtime_with_events(temp_root, tabs, active_tab_id).0
}

/// portable-pty falls back to `$HOME` as the child's cwd when no cwd is given
/// and `chdir`s to it unchecked. Tests mutate HOME concurrently (and glibc
/// env access is not thread-safe), so test pane spawns pin an always-existing
/// cwd that needs no environment lookup — otherwise a pane test racing an
/// env-mutating test dies with `PtyCreationFailed(ENOENT)` (Issue #3220).
fn test_pane_cwd() -> Option<PathBuf> {
    if cfg!(windows) {
        Some(std::env::temp_dir())
    } else {
        Some(PathBuf::from("/"))
    }
}

fn long_running_test_pane(id: &str) -> Pane {
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "ping -n 30 127.0.0.1 > nul".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "sleep 30".to_string()],
        )
    };
    Pane::new(
        id.to_string(),
        command,
        args,
        80,
        24,
        HashMap::new(),
        test_pane_cwd(),
    )
    .expect("test pane")
}

fn insert_test_pane_runtime(runtime: &mut AppRuntime, window_id: &str) {
    runtime.runtimes.insert(
        window_id.to_string(),
        WindowRuntime {
            pane: Arc::new(Mutex::new(long_running_test_pane(window_id))),
            output_thread: None,
            status_thread: None,
        },
    );
}

fn sample_runtime_with_events(
    temp_root: &Path,
    tabs: Vec<ProjectTabRuntime>,
    active_tab_id: Option<&str>,
) -> (AppRuntime, Arc<Mutex<Vec<UserEvent>>>) {
    let (proxy, _events) = AppEventProxy::stub();
    let sessions_dir = temp_root.join("sessions");
    let log_dir = temp_root.join("logs");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    fs::create_dir_all(&log_dir).expect("create log dir");
    let launch_wizard_cache =
        LaunchWizardMemoryCache::load_with_agent_options(&sessions_dir, sample_agent_options());
    let pty_writers: PtyWriterRegistry = Arc::new(RwLock::new(HashMap::new()));
    let blocking_tasks = BlockingTaskSpawner::thread();
    let persist_dispatcher = super::persist_dispatcher::PersistDispatcher::new(&blocking_tasks);
    let mut runtime = AppRuntime {
        tabs,
        active_tab_id: active_tab_id.map(str::to_owned),
        recent_projects: Vec::new(),
        profile_selections: HashMap::new(),
        profile_config_path: Some(temp_root.join("profile-config.toml")),
        runtimes: HashMap::new(),
        window_details: HashMap::new(),
        launch_error_terminal_details: HashMap::new(),
        window_lookup: HashMap::new(),
        board_all_view_windows: std::collections::HashSet::new(),
        session_state_path: temp_root.join("session-state.json"),
        log_dir,
        proxy,
        blocking_tasks,
        sessions_dir,
        launch_wizard_cache,
        launch_wizard: None,
        pending_workspace_resume_contexts: HashMap::new(),
        inflight_launches: HashMap::new(),
        pending_launch_feedback_contexts: HashMap::new(),
        pending_auto_resume_sources: HashMap::new(),
        pending_startup_auto_resume_sessions: Vec::new(),
        active_agent_sessions: HashMap::<String, ActiveAgentSession>::new(),
        work_merged_branches: HashMap::new(),
        work_cleanup_ready_branches: HashMap::new(),
        work_tip_subjects: HashMap::new(),
        work_pr_titles: HashMap::new(),
        work_ai_summaries: HashMap::new(),
        session_ledger_cache: std::cell::RefCell::new(
            crate::session_ledger_cache::SessionLedgerCache::new(),
        ),
        work_items_cache: std::cell::RefCell::new(
            gwt_core::workspace_projection::WorkItemsCache::new(),
        ),
        last_work_events_ingest: std::cell::RefCell::new(HashMap::new()),
        local_worktree_branches: std::cell::RefCell::new(HashMap::new()),
        window_pty_statuses: HashMap::new(),
        window_hook_states: HashMap::new(),
        recoverable_agent_error_windows: HashSet::new(),
        hook_forward_target: None,
        issue_link_cache_dir: gwt_cache_dir(),
        issue_client_factory: super::default_issue_client_factory(),
        pending_update: None,
        pty_writers,
        attachment_uploads: AttachmentUploadStore::new(temp_root.join("attachment-uploads")),
        persist_dispatcher,
        file_tree_worktree_roots: HashMap::new(),
        server_url: None,
        usage_refresh: None,
        image_paste_sequence: std::sync::atomic::AtomicU64::new(0),
        agent_launch_stage_counter: std::sync::atomic::AtomicU64::new(1),
    };
    runtime.rebuild_window_lookup();
    runtime.seed_window_pty_statuses();
    (runtime, _events)
}

fn wait_for_recorded_event(
    label: &str,
    events: &Arc<Mutex<Vec<UserEvent>>>,
    predicate: impl Fn(&[UserEvent]) -> bool,
) {
    for _ in 0..800 {
        {
            let events = events.lock().expect("event log");
            if predicate(&events) {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let snapshot = events.lock().expect("event log").clone();
    panic!("timed out waiting for {label}: {snapshot:?}");
}

/// Issue #3297: `load_knowledge_bridge_events` replies off the GUI event
/// loop through the stub proxy. Wait for the dispatched knowledge view for
/// `window_id` and return that dispatch's outbound events so tests can keep
/// asserting on the entries/detail pair.
fn wait_for_knowledge_view_dispatch(
    events: &Arc<Mutex<Vec<UserEvent>>>,
    window_id: &str,
) -> Vec<OutboundEvent> {
    for _ in 0..800 {
        {
            let recorded = events.lock().expect("event log");
            for event in recorded.iter() {
                if let UserEvent::Dispatch(dispatched) = event {
                    if dispatched.iter().any(|outbound| {
                        matches!(
                            &outbound.event,
                            BackendEvent::KnowledgeEntries { id, .. } if id == window_id
                        )
                    }) {
                        return dispatched.clone();
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let snapshot = events.lock().expect("event log").clone();
    panic!("timed out waiting for knowledge view dispatch for {window_id}: {snapshot:?}");
}

fn dispatch_launch_materialization_request(
    runtime: &mut AppRuntime,
    recorded_events: &Arc<Mutex<Vec<UserEvent>>>,
    label: &str,
) -> Vec<OutboundEvent> {
    wait_for_recorded_event(label, recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                event,
                UserEvent::LaunchWizardLaunchMaterializationRequested { .. }
            )
        })
    });
    let request = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    UserEvent::LaunchWizardLaunchMaterializationRequested { .. }
                )
            })
            .map(|index| events.remove(index))
            .expect("launch materialization event")
    };
    let UserEvent::LaunchWizardLaunchMaterializationRequested {
        wizard_id,
        client_id,
        config,
        bounds,
    } = request
    else {
        unreachable!("matched above")
    };
    runtime.handle_launch_wizard_launch_materialization_requested(
        wizard_id, client_id, *config, bounds,
    )
}

fn resolve_launch_wizard_runtime_confirmation(
    runtime: &mut AppRuntime,
    recorded_events: &Arc<Mutex<Vec<UserEvent>>>,
    label: &str,
) {
    let events = runtime.handle_launch_wizard_action(LaunchWizardAction::Submit, None);
    assert_eq!(events.len(), 1);
    let pending_view = runtime
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .view();
    assert!(pending_view.runtime_resolution_pending);
    assert!(!pending_view.runtime_context_resolved);

    wait_for_recorded_event(label, recorded_events, |events| {
        events
            .iter()
            .any(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
    });
    let resolved_event = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
            .map(|index| events.remove(index))
            .expect("runtime resolved event")
    };
    let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
        unreachable!("matched above")
    };
    let resolved_events = runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
    assert_eq!(resolved_events.len(), 1);
}

fn wait_for_path(label: &str, path: &Path) {
    for _ in 0..800 {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for {label}: {}", path.display());
}

#[test]
fn project_index_bootstrap_runs_in_background_without_blocking_launch() {
    let temp = tempdir().expect("tempdir");
    let (proxy, events) = AppEventProxy::stub();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let spawn_started = Instant::now();
    let service = crate::project_index_bootstrap::ProjectIndexBootstrapService::new_for_test();

    let spawned = service.spawn_with(
        proxy,
        temp.path().to_path_buf(),
        move |_project_root| {
            started_tx.send(()).expect("signal bootstrap start");
            release_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("release bootstrap");
            Ok(())
        },
        |_project_root| {
            gwt::ProjectIndexStatusView::new(
                gwt::ProjectIndexStatusState::Ready,
                "test bootstrap complete",
            )
        },
    );
    let spawn_elapsed = spawn_started.elapsed();

    assert_eq!(
        spawned,
        crate::project_index_bootstrap::ProjectIndexBootstrapRequest::Spawned
    );
    assert!(
        spawn_elapsed < Duration::from_millis(250),
        "spawning bootstrap must return before the slow bootstrap body completes"
    );
    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("background bootstrap should start promptly");
    assert!(
        events.lock().expect("event log").is_empty(),
        "no status should be emitted before the slow bootstrap completes"
    );

    release_tx.send(()).expect("release bootstrap");
    wait_for_recorded_event("project index status", &events, |events| {
        events.iter().any(|event| {
            matches!(
                event,
                UserEvent::ProjectIndexStatus {
                    project_root,
                    status,
                } if project_root == &dunce::canonicalize(temp.path())
                        .unwrap_or_else(|_| temp.path().to_path_buf())
                        .display()
                        .to_string()
                    && status.state == gwt::ProjectIndexStatusState::Ready
                    && status.detail == "test bootstrap complete"
            )
        })
    });
}

#[test]
fn agent_launch_success_dispatches_launch_complete_before_project_index_status() {
    let temp = tempdir().expect("tempdir");
    let (proxy, events) = AppEventProxy::stub();
    let completion: AgentLaunchCompletion = (
        ProcessLaunch {
            command: "agent".to_string(),
            args: Vec::new(),
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: Some(temp.path().to_path_buf()),
        },
        "session-1".to_string(),
        "feature/test".to_string(),
        "Codex".to_string(),
        temp.path().to_path_buf(),
        gwt_agent::AgentId::Codex,
        None,
        None,
        gwt_agent::LaunchRuntimeTarget::Host,
        temp.path().display().to_string(),
    );

    dispatch_agent_launch_success(
        proxy,
        "tab-1::agent-1".to_string(),
        completion,
        |proxy, project_root| {
            proxy.send(UserEvent::ProjectIndexStatus {
                project_root: project_root.display().to_string(),
                status: gwt::ProjectIndexStatusView::new(
                    gwt::ProjectIndexStatusState::Ready,
                    "ready",
                ),
            });
        },
    );

    let recorded = events.lock().expect("events");
    assert!(
        matches!(recorded.first(), Some(UserEvent::LaunchComplete { .. })),
        "LaunchComplete must be emitted first"
    );
    assert!(
        matches!(
            recorded.get(1),
            Some(UserEvent::ProjectIndexStatus {
                project_root,
                status,
            }) if project_root == &temp.path().display().to_string()
                && status.state == gwt::ProjectIndexStatusState::Ready
        ),
        "ProjectIndexStatus must follow LaunchComplete and carry project root"
    );
}

fn sample_launch_wizard_session(tab_id: &str, project_root: &Path) -> LaunchWizardSession {
    LaunchWizardSession {
        tab_id: tab_id.to_string(),
        wizard_id: "wizard-1".to_string(),
        wizard: LaunchWizardState::open_loading(
            LaunchWizardContext {
                selected_branch: BranchListEntry {
                    name: "feature/demo".to_string(),
                    scope: BranchScope::Local,
                    is_head: false,
                    upstream: None,
                    ahead: 0,
                    behind: 0,
                    last_commit_date: None,
                    cleanup_ready: true,
                    cleanup: BranchCleanupInfo::default(),
                    resume: gwt::BranchResumeInfo::unavailable(),
                    start_work_eligibility: None,
                },
                normalized_branch_name: "feature/demo".to_string(),
                worktree_path: None,
                quick_start_root: project_root.to_path_buf(),
                live_sessions: Vec::new(),
                docker_context: None,
                docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
                linked_issue_number: Some(42),
                linked_issue_kind: None,
                ultracode_supported: false,
                claude_workflows_enabled: false,
                ephemeral_base_ref: None,
            },
            Vec::new(),
        ),
        workspace_resume_context: None,
        agent_kanban_target: None,
        auto_submit_after_runtime_resolution: None,
        issue_monitor_profile_save: None,
        issue_monitor_launch_issue_number: None,
    }
}

fn sample_agent_options() -> Vec<gwt::AgentOption> {
    vec![gwt::AgentOption {
        id: "codex".to_string(),
        name: "Codex".to_string(),
        available: true,
        installed_version: Some("latest".to_string()),
        versions: vec!["latest".to_string()],
        custom_agent: None,
    }]
}

fn sample_issue_monitor_launch_profile() -> gwt::IssueMonitorLaunchProfile {
    gwt::IssueMonitorLaunchProfile {
        agent_id: "claude".to_string(),
        model: Some("gpt-5.5".to_string()),
        reasoning: Some("high".to_string()),
        version: Some("latest".to_string()),
        session_mode: gwt_agent::SessionMode::Normal,
        skip_permissions: true,
        codex_fast_mode: true,
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        docker_service: None,
        docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
        windows_shell: None,
    }
}

fn run_git(repo: &Path, args: &[&str]) {
    let status = gwt_core::process::hidden_command("git")
        .args(args)
        .current_dir(repo)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

fn run_git_with_paths(args: &[&str], paths: &[&Path]) {
    let mut command = gwt_core::process::hidden_command("git");
    command.args(args);
    for path in paths {
        command.arg(path);
    }
    let status = command.status().expect("run git with paths");
    assert!(
        status.success(),
        "git {args:?} {paths:?} failed with {status}"
    );
}

fn init_git_clone_with_origin(repo: &Path) -> PathBuf {
    let root = repo.parent().expect("repo parent");
    let seed = root.join("seed");
    let origin = root.join("origin.git");
    fs::create_dir_all(&seed).expect("create seed");
    run_git(&seed, &["init", "-q", "-b", "develop"]);
    run_git(&seed, &["config", "user.name", "Codex"]);
    run_git(&seed, &["config", "user.email", "codex@example.com"]);
    fs::write(seed.join("README.md"), "repo\n").expect("seed readme");
    run_git(&seed, &["add", "README.md"]);
    run_git(&seed, &["commit", "-qm", "init"]);
    run_git_with_paths(&["clone", "--bare"], &[&seed, &origin]);
    run_git_with_paths(&["clone"], &[&origin, repo]);
    run_git(repo, &["config", "user.name", "Codex"]);
    run_git(repo, &["config", "user.email", "codex@example.com"]);
    run_git(repo, &["remote", "set-head", "origin", "-a"]);
    origin
}

fn init_managed_workspace_with_develop_worktree(workspace_home: &Path) -> (PathBuf, PathBuf) {
    fs::create_dir_all(workspace_home).expect("create workspace home");
    let seed = workspace_home.join(".seed");
    let bare_repo = workspace_home.join("repo.git");
    let develop_worktree = workspace_home.join("develop");

    fs::create_dir_all(&seed).expect("create seed");
    run_git(&seed, &["init", "-q", "-b", "develop"]);
    run_git(&seed, &["config", "user.name", "Codex"]);
    run_git(&seed, &["config", "user.email", "codex@example.com"]);
    fs::write(seed.join("README.md"), "repo\n").expect("seed readme");
    run_git(&seed, &["add", "README.md"]);
    run_git(&seed, &["commit", "-qm", "init"]);
    run_git_with_paths(&["clone", "--bare"], &[&seed, &bare_repo]);

    let develop_arg = develop_worktree.to_string_lossy().to_string();
    let output = gwt_core::process::hidden_command("git")
        .args(["worktree", "add", "-q", &develop_arg, "develop"])
        .current_dir(&bare_repo)
        .output()
        .expect("git worktree add develop");
    assert!(
        output.status.success(),
        "git worktree add develop failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    run_git(&develop_worktree, &["config", "user.name", "Codex"]);
    run_git(
        &develop_worktree,
        &["config", "user.email", "codex@example.com"],
    );

    (bare_repo, develop_worktree)
}

fn append_workspace_resume_journal(
    repo: &Path,
    journal_id: &str,
    project_root: PathBuf,
    owner: &str,
    summary: &str,
) {
    let path = gwt_core::paths::gwt_workspace_journal_path_for_repo_path(repo);
    let entry = gwt_core::workspace_projection::WorkspaceJournalEntry {
        id: journal_id.to_string(),
        project_root,
        title: Some("Suspended review".to_string()),
        status_category: Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Idle),
        status_text: Some("Suspended".to_string()),
        owner: Some(owner.to_string()),
        next_action: Some("Resume the review".to_string()),
        summary: Some(summary.to_string()),
        progress_summary: None,
        agent_session_id: None,
        agent_current_focus: None,
        agent_title_summary: Some("Suspended review".to_string()),
        updated_at: chrono::Utc::now(),
    };
    gwt_core::workspace_projection::append_workspace_journal_entry_to_path(&path, &entry)
        .expect("append journal");
}

fn sample_no_agent_launch_wizard_session(tab_id: &str, project_root: &Path) -> LaunchWizardSession {
    LaunchWizardSession {
        tab_id: tab_id.to_string(),
        wizard_id: "wizard-unavailable-agent".to_string(),
        wizard: LaunchWizardState::open_with(
            LaunchWizardContext {
                selected_branch: BranchListEntry {
                    name: "feature/demo".to_string(),
                    scope: BranchScope::Local,
                    is_head: false,
                    upstream: None,
                    ahead: 0,
                    behind: 0,
                    last_commit_date: None,
                    cleanup_ready: true,
                    cleanup: BranchCleanupInfo::default(),
                    resume: gwt::BranchResumeInfo::unavailable(),
                    start_work_eligibility: None,
                },
                normalized_branch_name: "feature/demo".to_string(),
                worktree_path: Some(project_root.to_path_buf()),
                quick_start_root: project_root.to_path_buf(),
                live_sessions: Vec::new(),
                docker_context: None,
                docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
                linked_issue_number: Some(42),
                linked_issue_kind: None,
                ultracode_supported: false,
                claude_workflows_enabled: false,
                ephemeral_base_ref: None,
            },
            Vec::new(),
            Vec::new(),
        ),
        workspace_resume_context: None,
        agent_kanban_target: None,
        auto_submit_after_runtime_resolution: None,
        issue_monitor_profile_save: None,
        issue_monitor_launch_issue_number: None,
    }
}

fn sample_start_work_confirm_session(tab_id: &str, project_root: &Path) -> LaunchWizardSession {
    let base_branch = "origin/develop".to_string();
    let work_branch = "work/20260625-1702".to_string();
    let mut wizard = LaunchWizardState::open_start_work_with_previous_profiles(
        LaunchWizardContext {
            selected_branch: BranchListEntry {
                name: base_branch.clone(),
                scope: BranchScope::Remote,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                last_commit_date: None,
                cleanup_ready: false,
                cleanup: BranchCleanupInfo::default(),
                resume: gwt::BranchResumeInfo::unavailable(),
                start_work_eligibility: None,
            },
            normalized_branch_name: work_branch.clone(),
            worktree_path: None,
            quick_start_root: project_root.to_path_buf(),
            live_sessions: Vec::new(),
            docker_context: None,
            docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
            linked_issue_number: None,
            linked_issue_kind: None,
            ultracode_supported: false,
            claude_workflows_enabled: false,
            ephemeral_base_ref: None,
        },
        base_branch,
        sample_agent_options(),
        Vec::new(),
        Default::default(),
    );
    wizard.mark_runtime_context_unresolved();
    wizard.apply(LaunchWizardAction::UseStartMethod {
        method: gwt::LaunchWizardStartMethodKind::ConfigureAndStart,
    });
    wizard.apply(LaunchWizardAction::Submit);
    wizard.completion = None;
    wizard.apply_runtime_context(gwt::LaunchWizardHydration {
        selected_branch: None,
        normalized_branch_name: work_branch,
        worktree_path: None,
        quick_start_root: project_root.to_path_buf(),
        docker_context: None,
        docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
        agent_options: sample_agent_options(),
        quick_start_entries: Vec::new(),
        previous_profiles: Some(Default::default()),
        open_branch_candidates: Vec::new(),
    });
    wizard.apply(LaunchWizardAction::Submit);
    assert!(wizard.view().show_confirm);

    LaunchWizardSession {
        tab_id: tab_id.to_string(),
        wizard_id: "wizard-start-work-confirm".to_string(),
        wizard,
        workspace_resume_context: None,
        agent_kanban_target: None,
        auto_submit_after_runtime_resolution: None,
        issue_monitor_profile_save: None,
        issue_monitor_launch_issue_number: None,
    }
}

fn sample_ready_agent_launch_wizard_session(
    tab_id: &str,
    project_root: &Path,
) -> LaunchWizardSession {
    LaunchWizardSession {
        tab_id: tab_id.to_string(),
        wizard_id: "wizard-ready-agent".to_string(),
        wizard: LaunchWizardState::open_with(
            LaunchWizardContext {
                selected_branch: BranchListEntry {
                    name: "feature/demo".to_string(),
                    scope: BranchScope::Local,
                    is_head: false,
                    upstream: None,
                    ahead: 0,
                    behind: 0,
                    last_commit_date: None,
                    cleanup_ready: true,
                    cleanup: BranchCleanupInfo::default(),
                    resume: gwt::BranchResumeInfo::unavailable(),
                    start_work_eligibility: None,
                },
                normalized_branch_name: "feature/demo".to_string(),
                worktree_path: Some(project_root.to_path_buf()),
                quick_start_root: project_root.to_path_buf(),
                live_sessions: Vec::new(),
                docker_context: None,
                docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
                linked_issue_number: Some(42),
                linked_issue_kind: None,
                ultracode_supported: false,
                claude_workflows_enabled: false,
                ephemeral_base_ref: None,
            },
            sample_agent_options(),
            Vec::new(),
        ),
        workspace_resume_context: None,
        agent_kanban_target: None,
        auto_submit_after_runtime_resolution: None,
        issue_monitor_profile_save: None,
        issue_monitor_launch_issue_number: None,
    }
}

#[test]
fn app_runtime_frontend_ready_replies_only_to_requesting_client_and_starts_with_workspace() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    runtime
        .window_details
        .insert(window_id.clone(), "Shell ready".to_string());
    runtime.launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));
    runtime.pending_update = Some(gwt_core::update::UpdateState::UpToDate { checked_at: None });

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

    assert!(matches!(
        events.first(),
        Some(event)
            if matches!(&event.target, DispatchTarget::Client(client_id) if client_id == "client-1")
                && matches!(event.event, BackendEvent::WindowCanvasState { .. })
    ));
    assert!(events.iter().all(|event| matches!(
        &event.target,
        DispatchTarget::Client(client_id) if client_id == "client-1"
    )));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::TerminalStatus { id, status, detail }
            if id == &window_id
                && *status == WindowProcessStatus::Ready
                && detail.as_deref() == Some("Shell ready")
    )));
    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::LaunchWizardState { wizard: Some(_) }
    )));
    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::UpdateState(gwt_core::update::UpdateState::UpToDate { .. })
    )));
}

#[test]
fn app_runtime_frontend_ready_replies_launch_wizard_tombstone_when_closed() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

    let tombstone = events
        .iter()
        .find(|event| {
            matches!(
                event.event,
                BackendEvent::LaunchWizardState { wizard: None }
            )
        })
        .expect("FrontendReady must clear stale Launch Wizard state after reconnect");
    assert!(
        matches!(&tombstone.target, DispatchTarget::Client(client_id) if client_id == "client-1"),
        "Launch Wizard tombstone must be scoped to the reconnecting client"
    );
}

#[test]
fn app_runtime_apply_update_uses_pending_available_update_state() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().join("repo"),
        ProjectKind::Git,
        &[WindowPreset::Shell],
    );
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.pending_update = Some(gwt_core::update::UpdateState::Available {
        current: "9.20.1".to_string(),
        latest: "9.20.2".to_string(),
        release_url: "https://example.invalid/releases/v9.20.2".to_string(),
        asset_url: Some("https://example.invalid/gwt-macos-arm64.dmg".to_string()),
        checked_at: chrono::Utc::now(),
    });

    let outbound =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::ApplyUpdate);

    assert!(outbound.is_empty(), "apply worker dispatch is internal");
    wait_for_recorded_event("pending update apply", &events, |events| {
        events.iter().any(|event| {
            matches!(
                event,
                UserEvent::ApplyUpdate {
                    client_id,
                    state: gwt_core::update::UpdateState::Available {
                        latest,
                        asset_url: Some(asset_url),
                        ..
                    },
                } if client_id == "client-1"
                    && latest == "9.20.2"
                    && asset_url == "https://example.invalid/gwt-macos-arm64.dmg"
            )
        })
    });
}

#[test]
fn app_runtime_apply_update_without_applicable_pending_update_reports_error() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().join("repo"),
        ProjectKind::Git,
        &[WindowPreset::Shell],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.pending_update = Some(gwt_core::update::UpdateState::Available {
        current: "9.20.1".to_string(),
        latest: "9.20.2".to_string(),
        release_url: "https://example.invalid/releases/v9.20.2".to_string(),
        asset_url: None,
        checked_at: chrono::Utc::now(),
    });

    let outbound =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::ApplyUpdate);

    assert!(outbound.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::UpdateApplyError { message: Some(message), .. }
                if message.contains("No applicable update asset")
        )
    }));
}

#[test]
fn app_runtime_frontend_ready_replays_terminal_snapshot_only_to_requesting_client() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };
    let mut pane = Pane::new(
        window_id.clone(),
        command,
        args,
        80,
        24,
        HashMap::new(),
        test_pane_cwd(),
    )
    .expect("pane");
    pane.process_bytes(b"hello from frontend ready\n");
    runtime.runtimes.insert(
        window_id.clone(),
        WindowRuntime {
            pane: Arc::new(Mutex::new(pane)),
            output_thread: None,
            status_thread: None,
        },
    );

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

    assert!(events.iter().all(|event| matches!(
        &event.target,
        DispatchTarget::Client(client_id) if client_id == "client-1"
    )));
    let snapshot = events.iter().find_map(|event| match &event.event {
        BackendEvent::TerminalSnapshot { id, data_base64 } if id == &window_id => Some(data_base64),
        _ => None,
    });
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(snapshot.expect("terminal snapshot event"))
        .expect("decode terminal snapshot");
    assert!(String::from_utf8_lossy(&decoded).contains("hello from frontend ready"));
}

#[test]
fn app_runtime_frontend_ready_replays_terminal_snapshot_with_sgr_attributes() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };
    let mut pane = Pane::new(
        window_id.clone(),
        command,
        args,
        80,
        24,
        HashMap::new(),
        test_pane_cwd(),
    )
    .expect("pane");
    // Write red foreground + bold "ALERT" then reset, then default-color text.
    pane.process_bytes(b"\x1b[31;1mALERT\x1b[0m normal\n");

    runtime.runtimes.insert(
        window_id.clone(),
        WindowRuntime {
            pane: Arc::new(Mutex::new(pane)),
            output_thread: None,
            status_thread: None,
        },
    );

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

    let snapshot = events.iter().find_map(|event| match &event.event {
        BackendEvent::TerminalSnapshot { id, data_base64 } if id == &window_id => Some(data_base64),
        _ => None,
    });
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(snapshot.expect("terminal snapshot event"))
        .expect("decode terminal snapshot");
    // Visible text must be present.
    assert!(
        String::from_utf8_lossy(&decoded).contains("ALERT"),
        "expected ALERT text in snapshot bytes, got: {:?}",
        String::from_utf8_lossy(&decoded)
    );
    // SGR escape sequence introducing a styled run (CSI ... m) must be present
    // so that xterm.js can replay foreground / bold / etc. from the snapshot.
    let has_sgr = decoded.windows(2).enumerate().any(|(idx, win)| {
        win == [0x1b, b'['] && {
            let tail = &decoded[idx + 2..];
            tail.iter().take(16).any(|b| *b == b'm')
        }
    });
    assert!(
            has_sgr,
            "expected SGR escape (CSI ... m) in TerminalSnapshot bytes so xterm.js can replay color/style; raw snapshot bytes: {:?}",
            decoded
        );
}

#[test]
fn app_runtime_frontend_ready_replays_terminal_snapshot_with_scrollback_history() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };
    let mut pane = Pane::new(
        window_id.clone(),
        command,
        args,
        80,
        6,
        HashMap::new(),
        test_pane_cwd(),
    )
    .expect("pane");
    for line in 1..=18 {
        pane.process_bytes(format!("SCROLLBACK-LINE-{line:03}\r\n").as_bytes());
    }

    runtime.runtimes.insert(
        window_id.clone(),
        WindowRuntime {
            pane: Arc::new(Mutex::new(pane)),
            output_thread: None,
            status_thread: None,
        },
    );

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

    let snapshot = events.iter().find_map(|event| match &event.event {
        BackendEvent::TerminalSnapshot { id, data_base64 } if id == &window_id => Some(data_base64),
        _ => None,
    });
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(snapshot.expect("terminal snapshot event"))
        .expect("decode terminal snapshot");
    let text = String::from_utf8_lossy(&decoded);
    assert!(
        text.contains("SCROLLBACK-LINE-001"),
        "expected frontend reconnect snapshot to include old scrollback history, got: {text:?}"
    );
    assert!(
        text.contains("SCROLLBACK-LINE-018"),
        "expected frontend reconnect snapshot to include current visible screen, got: {text:?}"
    );
}

#[test]
fn app_runtime_dock_window_tab_resizes_group_runtimes() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[WindowPreset::Shell, WindowPreset::Claude],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let shell_id = combined_window_id("tab-1", "shell-1");
    let claude_id = combined_window_id("tab-1", "claude-1");
    for window_id in [&shell_id, &claude_id] {
        let pane = Pane::new(
            window_id.clone(),
            if cfg!(windows) { "cmd" } else { "/bin/sh" }.to_string(),
            if cfg!(windows) {
                vec![
                    "/d".to_string(),
                    "/s".to_string(),
                    "/c".to_string(),
                    "exit /b 0".to_string(),
                ]
            } else {
                vec!["-lc".to_string(), "exit 0".to_string()]
            },
            80,
            24,
            HashMap::new(),
            test_pane_cwd(),
        )
        .expect("pane");
        runtime.runtimes.insert(
            window_id.clone(),
            WindowRuntime {
                pane: Arc::new(Mutex::new(pane)),
                output_thread: None,
                status_thread: None,
            },
        );
    }

    let target_geometry = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .window("claude-1")
        .expect("claude")
        .geometry
        .clone();
    let (expected_cols, expected_rows) = geometry_to_pty_size(&target_geometry);

    let events = runtime.dock_window_tab_events(&shell_id, &claude_id);

    assert_eq!(events.len(), 1);
    for window_id in [&shell_id, &claude_id] {
        let pane = runtime
            .runtimes
            .get(window_id)
            .expect("runtime")
            .pane
            .lock()
            .expect("pane");
        assert_eq!(pane.screen().size(), (expected_rows, expected_cols));
    }
}

#[test]
fn app_runtime_places_agent_window_in_kanban_from_frontend_event() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[WindowPreset::AgentKanban, WindowPreset::Agent],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let agent_id = combined_window_id("tab-1", "agent-1");
    let board_id = combined_window_id("tab-1", "agent-kanban-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::PlaceAgentWindowInKanban {
            id: agent_id,
            board_id,
            lane_id: gwt::AgentKanbanLane::Active,
            order: None,
        },
    );

    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "Kanban placement must broadcast workspace state"
    );
    let agent = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .window("agent-1")
        .expect("agent");
    assert_eq!(
        agent.placement,
        WindowPlacement::AgentKanban {
            board_id: "agent-kanban-1".to_string(),
            lane_id: gwt::AgentKanbanLane::Active,
            order: 0,
            collapsed: false,
        }
    );
}

#[test]
fn app_runtime_open_agent_kanban_launch_wizard_records_launch_target() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::Git,
        &[WindowPreset::AgentKanban],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let board_id = combined_window_id("tab-1", "agent-kanban-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::OpenAgentKanbanLaunchWizard {
            board_id,
            lane_id: gwt::AgentKanbanLane::Blocked,
        },
    );

    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::LaunchWizardState { .. })),
        "Kanban Launch Agent must open the normal Launch Agent wizard"
    );
    let session = runtime.launch_wizard.as_ref().expect("launch wizard");
    let view = session.wizard.view();
    assert_eq!(view.title, "Launch Agent");
    assert_ne!(
        view.mode,
        gwt::LaunchWizardMode::StartWork,
        "Kanban Launch Agent must not require Start Work branch materialization"
    );
    let target = session
        .agent_kanban_target
        .as_ref()
        .expect("agent kanban launch target");
    assert_eq!(target.board_id, "agent-kanban-1");
    assert_eq!(target.lane_id, gwt::AgentKanbanLane::Blocked);
}

#[test]
fn app_runtime_spawn_agent_window_in_agent_kanban_places_new_window() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::Git,
        &[WindowPreset::AgentKanban],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/20260619-kanban")
        .build();

    runtime
        .spawn_agent_window_in_agent_kanban(
            "tab-1",
            config,
            canvas_bounds(),
            None,
            None,
            AgentKanbanLaunchTarget {
                board_id: "agent-kanban-1".to_string(),
                lane_id: gwt::AgentKanbanLane::Active,
            },
        )
        .expect("spawn agent in kanban");

    let agent = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window");
    assert_eq!(
        agent.placement,
        WindowPlacement::AgentKanban {
            board_id: "agent-kanban-1".to_string(),
            lane_id: gwt::AgentKanbanLane::Active,
            order: 0,
            collapsed: false,
        }
    );
}

#[test]
fn app_runtime_spawn_agent_window_in_agent_kanban_falls_back_to_canvas_when_board_missing() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::Git,
        &[WindowPreset::AgentKanban],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/20260619-kanban-fallback")
        .build();

    runtime
        .spawn_agent_window_in_agent_kanban(
            "tab-1",
            config,
            canvas_bounds(),
            None,
            None,
            AgentKanbanLaunchTarget {
                board_id: "missing-board".to_string(),
                lane_id: gwt::AgentKanbanLane::Active,
            },
        )
        .expect("spawn agent even when kanban placement is unavailable");

    let agent = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window");
    assert_eq!(agent.placement, WindowPlacement::Canvas);
}

#[test]
fn app_runtime_update_terminal_grid_resizes_runtime_without_workspace_broadcast() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[WindowPreset::Agent],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let agent_id = combined_window_id("tab-1", "agent-1");
    insert_test_pane_runtime(&mut runtime, &agent_id);

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::UpdateTerminalGrid {
            id: agent_id.clone(),
            cols: 112,
            rows: 34,
        },
    );

    assert!(
        events
            .iter()
            .all(|event| !matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "embedded terminal grid resize must not mutate canvas geometry"
    );
    let pane = runtime
        .runtimes
        .get(&agent_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane");
    assert_eq!(pane.screen().size(), (34, 112));
}

#[test]
fn app_runtime_cycle_focus_preserves_real_fit_pty_size() {
    // Issue #2937: cycle_focus must NOT clobber the PTY size that the
    // frontend established via its real xterm fit. The backend's
    // geometry_to_pty_size approximation is only a spawn bootstrap;
    // reverting an already-fitted PTY back to it on every window switch
    // is what desyncs the child's grid from xterm and corrupts the
    // rendered terminal (recovers on manual resize).
    let temp = tempdir().expect("tempdir");
    let bounds = canvas_bounds();
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[WindowPreset::Shell, WindowPreset::Claude],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let shell_id = combined_window_id("tab-1", "shell-1");
    let claude_id = combined_window_id("tab-1", "claude-1");
    insert_test_pane_runtime(&mut runtime, &shell_id);
    insert_test_pane_runtime(&mut runtime, &claude_id);

    // Sentinel that differs from geometry_to_pty_size for any open
    // window, so a clobber via the approximation is detectable.
    const REAL_COLS: u16 = 137;
    const REAL_ROWS: u16 = 41;
    for raw_id in ["shell-1", "claude-1"] {
        let geometry = runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .window(raw_id)
            .expect("window")
            .geometry
            .clone();
        assert_ne!(
            geometry_to_pty_size(&geometry),
            (REAL_COLS, REAL_ROWS),
            "sentinel must differ from the approximation to be meaningful",
        );
    }

    // Simulate the frontend's real xterm fit having sized each PTY.
    for window_id in [&shell_id, &claude_id] {
        runtime
            .runtimes
            .get(window_id)
            .expect("runtime")
            .pane
            .lock()
            .expect("pane")
            .resize(REAL_COLS, REAL_ROWS)
            .expect("resize");
    }

    assert_eq!(
        runtime
            .cycle_focus_events(FocusCycleDirection::Forward, bounds)
            .len(),
        1
    );

    for window_id in [&shell_id, &claude_id] {
        let pane = runtime
            .runtimes
            .get(window_id)
            .expect("runtime")
            .pane
            .lock()
            .expect("pane");
        assert_eq!(
            pane.screen().size(),
            (REAL_ROWS, REAL_COLS),
            "cycle_focus must not clobber the frontend-fitted PTY size via geometry_to_pty_size",
        );
    }
}

#[test]
fn app_runtime_activate_window_tab_preserves_real_fit_pty_size() {
    // SPEC-2008 Phase 34 / Issue #2937 companion: tab activation changes only
    // the active marker. The backend must not resize the PTY from the shared
    // group geometry, because the frontend's visible xterm fit owns the real
    // cols/rows for the revealed tab.
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[WindowPreset::Shell, WindowPreset::Claude],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let shell_id = combined_window_id("tab-1", "shell-1");
    let claude_id = combined_window_id("tab-1", "claude-1");

    assert_eq!(
        runtime.dock_window_tab_events(&shell_id, &claude_id).len(),
        1
    );
    let _ = runtime.activate_window_tab_events(&shell_id);

    insert_test_pane_runtime(&mut runtime, &shell_id);
    insert_test_pane_runtime(&mut runtime, &claude_id);

    const REAL_COLS: u16 = 173;
    const REAL_ROWS: u16 = 31;
    for raw_id in ["shell-1", "claude-1"] {
        let geometry = runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .window(raw_id)
            .expect("window")
            .geometry
            .clone();
        assert_ne!(
            geometry_to_pty_size(&geometry),
            (REAL_COLS, REAL_ROWS),
            "sentinel must differ from the shared-geometry approximation",
        );
    }

    for window_id in [&shell_id, &claude_id] {
        runtime
            .runtimes
            .get(window_id)
            .expect("runtime")
            .pane
            .lock()
            .expect("pane")
            .resize(REAL_COLS, REAL_ROWS)
            .expect("resize");
    }

    let events = runtime.activate_window_tab_events(&claude_id);

    assert_eq!(events.len(), 1);
    let workspace = &runtime.tab("tab-1").expect("tab").workspace;
    assert!(
        workspace
            .window("claude-1")
            .expect("claude")
            .tab_group_active
    );
    assert!(!workspace.window("shell-1").expect("shell").tab_group_active);
    for window_id in [&shell_id, &claude_id] {
        let pane = runtime
            .runtimes
            .get(window_id)
            .expect("runtime")
            .pane
            .lock()
            .expect("pane");
        assert_eq!(
            pane.screen().size(),
            (REAL_ROWS, REAL_COLS),
            "tab activation must not clobber the frontend-fitted PTY size via geometry_to_pty_size",
        );
    }
}

#[test]
fn app_runtime_arrange_windows_does_not_clobber_real_fit_pty_size() {
    // Issue #2937 companion: arrange_windows shares the same all-window
    // resize fan-out as cycle_focus. The frontend re-fit (driven by the
    // geometry_revision bump) is the single source of truth for PTY
    // size; the backend must not revert PTYs to the approximation here.
    let temp = tempdir().expect("tempdir");
    let bounds = canvas_bounds();
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[WindowPreset::Shell, WindowPreset::Claude],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let shell_id = combined_window_id("tab-1", "shell-1");
    let claude_id = combined_window_id("tab-1", "claude-1");
    insert_test_pane_runtime(&mut runtime, &shell_id);
    insert_test_pane_runtime(&mut runtime, &claude_id);

    const REAL_COLS: u16 = 151;
    const REAL_ROWS: u16 = 47;
    for window_id in [&shell_id, &claude_id] {
        runtime
            .runtimes
            .get(window_id)
            .expect("runtime")
            .pane
            .lock()
            .expect("pane")
            .resize(REAL_COLS, REAL_ROWS)
            .expect("resize");
    }

    assert_eq!(
        runtime
            .arrange_windows_events(ArrangeMode::Tile, bounds)
            .len(),
        1
    );

    for window_id in [&shell_id, &claude_id] {
        let pane = runtime
            .runtimes
            .get(window_id)
            .expect("runtime")
            .pane
            .lock()
            .expect("pane");
        assert_eq!(
                pane.screen().size(),
                (REAL_ROWS, REAL_COLS),
                "arrange_windows must not clobber the frontend-fitted PTY size via geometry_to_pty_size",
            );
    }
}

#[test]
fn app_runtime_frontend_ready_replays_active_work_projection_separately_from_workspace() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Board],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.active_agent_sessions.insert(
        "tab-1::agent-1".to_string(),
        ActiveAgentSession {
            window_id: "tab-1::agent-1".to_string(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260504-1234".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.join("../repo-work-20260504-1234"),
            agent_project_root: repo
                .join("../repo-work-20260504-1234")
                .display()
                .to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

    assert!(matches!(
        events.first().map(|event| &event.event),
        Some(BackendEvent::WindowCanvasState { .. })
    ));
    let projection = events.iter().find_map(|event| match &event.event {
        BackendEvent::ActiveWorkProjection { projection } => Some(projection),
        _ => None,
    });
    let projection = projection.expect("active work projection event");
    assert_eq!(projection.status_category, "active");
    assert_eq!(projection.active_agents, 1);
    assert_eq!(projection.branch.as_deref(), Some("work/20260504-1234"));
    assert_eq!(projection.agents.len(), 1);
    assert_eq!(projection.agents[0].session_id, "session-1");
    assert_eq!(projection.agents[0].display_name, "Codex");
    assert_eq!(projection.agents[0].status_category, "active");
    assert_eq!(
        projection.agents[0].branch.as_deref(),
        Some("work/20260504-1234")
    );
    assert!(
        events.iter().all(|event| matches!(
            &event.target,
            DispatchTarget::Client(client_id) if client_id == "client-1"
        )),
        "frontend-ready projection replay must remain client-scoped"
    );
}

#[test]
fn app_runtime_select_project_tab_broadcasts_workspace_before_clearing_wizard() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    let other = temp.path().join("other");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&other).expect("create other");
    let tabs = vec![
        sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::NonRepo,
            &[WindowPreset::Branches],
        ),
        sample_project_tab(
            "tab-2",
            "Other",
            other,
            ProjectKind::NonRepo,
            &[WindowPreset::FileTree],
        ),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));
    runtime.launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));

    let events = runtime.select_project_tab_events("tab-2");

    assert_eq!(events.len(), 3);
    assert!(matches!(events[0].target, DispatchTarget::Broadcast));
    assert!(matches!(
        events[0].event,
        BackendEvent::WindowCanvasState { .. }
    ));
    assert!(matches!(events[1].target, DispatchTarget::Broadcast));
    assert!(matches!(
        events[1].event,
        BackendEvent::ActiveWorkProjection { .. }
    ));
    assert!(matches!(events[2].target, DispatchTarget::Broadcast));
    assert!(matches!(
        events[2].event,
        BackendEvent::LaunchWizardState { wizard: None }
    ));
}

#[test]
fn app_runtime_select_project_tab_emits_active_work_projection_for_new_active_tab() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    let other = temp.path().join("other");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&other).expect("create other");
    let tabs = vec![
        sample_project_tab("tab-1", "Repo", repo, ProjectKind::NonRepo, &[]),
        sample_project_tab("tab-2", "Other", other, ProjectKind::NonRepo, &[]),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));

    let events = runtime.select_project_tab_events("tab-2");

    let projection = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::ActiveWorkProjection { projection } => Some(projection),
            _ => None,
        })
        .expect("ActiveWorkProjection broadcast for newly selected tab");
    assert_eq!(
        projection.id, "tab-2",
        "projection must reflect the newly active tab"
    );
}

#[test]
fn app_runtime_close_project_tab_emits_active_work_projection_when_active_changes() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    let other = temp.path().join("other");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&other).expect("create other");
    let tabs = vec![
        sample_project_tab("tab-1", "Repo", repo, ProjectKind::NonRepo, &[]),
        sample_project_tab("tab-2", "Other", other, ProjectKind::NonRepo, &[]),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));

    let events = runtime.close_project_tab_events("tab-1");

    let projection = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::ActiveWorkProjection { projection } => Some(projection),
            _ => None,
        })
        .expect("ActiveWorkProjection broadcast after closing the active tab");
    assert_eq!(
        projection.id, "tab-2",
        "projection must reflect the new active tab after close"
    );
}

#[test]
fn app_runtime_open_project_path_emits_active_work_projection_for_new_tab() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tabs = vec![sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::NonRepo,
        &[],
    )];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));

    let other = temp.path().join("other-project");
    fs::create_dir_all(&other).expect("create other");

    let events = runtime.open_project_path_events(other.clone());

    assert!(
        events
            .iter()
            .any(|event| matches!(&event.event, BackendEvent::ActiveWorkProjection { .. })),
        "opening a new project must emit ActiveWorkProjection for the new active tab"
    );
}

#[test]
fn app_runtime_runtime_status_uses_lightweight_events_for_non_structural_status() {
    // Scoped HOME: with FR-382 the projection broadcast also fires when home
    // work records exist, so this lightweight-path assertion must not depend
    // on whatever ~/.gwt state the developer machine has accumulated (#3022).
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");

    let events = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("boom".to_string()),
    );

    assert_eq!(events.len(), 2);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "non-structural runtime status changes must not force a full workspace_state"
    );
    assert!(matches!(events[0].target, DispatchTarget::Broadcast));
    assert!(matches!(
        &events[0].event,
        BackendEvent::WindowState { window_id: id, state }
            if id == &window_id && *state == WindowProcessStatus::Error
    ));
    assert!(matches!(events[1].target, DispatchTarget::Broadcast));
    assert!(matches!(
        &events[1].event,
        BackendEvent::TerminalStatus { id, status, detail }
            if id == &window_id
                && *status == WindowProcessStatus::Error
                && detail.as_deref() == Some("boom")
    ));
}

#[test]
fn app_runtime_open_launch_wizard_uses_cached_previous_profile_without_hydrating() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");

    let mut session = gwt_agent::Session::new(&repo, "feature/demo", gwt_agent::AgentId::Codex);
    session.model = Some("gpt-5.4".to_string());
    session.reasoning_level = Some("high".to_string());
    session.tool_version = Some("latest".to_string());
    session.session_mode = gwt_agent::SessionMode::Continue;
    session.skip_permissions = true;
    session.codex_fast_mode = true;
    session.save(&sessions_dir).expect("save session");

    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_launch_wizard_for_branch("tab-1", &repo, "feature/demo", None, None)
        .expect("open launch wizard");

    let view = runtime
        .launch_wizard
        .as_ref()
        .expect("launch wizard")
        .wizard
        .view();
    assert!(!view.is_hydrating);
    assert_eq!(view.selected_agent_id, "codex");
    assert_eq!(view.selected_model, "gpt-5.4");
    assert_eq!(view.selected_reasoning, "high");
    assert_eq!(view.selected_version, "latest");
    assert_eq!(view.selected_execution_mode, "continue");
    assert!(view.skip_permissions);
    assert!(view.codex_fast_mode);
}

#[test]
fn app_runtime_open_launch_wizard_does_not_probe_branch_worktree_for_docker_context() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    run_git(&repo, &["init", "-q", "-b", "develop"]);
    run_git(&repo, &["config", "user.name", "Codex"]);
    run_git(&repo, &["config", "user.email", "codex@example.com"]);
    fs::write(repo.join("README.md"), "repo\n").expect("readme");
    run_git(&repo, &["add", "README.md"]);
    run_git(&repo, &["commit", "-qm", "init"]);
    run_git(&repo, &["branch", "feature/docker"]);

    let branch_worktree = temp.path().join("repo-feature-docker");
    let branch_worktree_arg = branch_worktree.to_string_lossy().to_string();
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            &branch_worktree_arg,
            "feature/docker",
        ],
    );
    fs::write(
        branch_worktree.join("docker-compose.yml"),
        "services:\n  app:\n    image: alpine:3.20\n",
    )
    .expect("compose");

    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_launch_wizard_for_branch("tab-1", &repo, "feature/docker", None, None)
        .expect("open launch wizard");

    let wizard = &runtime.launch_wizard.as_ref().expect("wizard").wizard;
    assert!(wizard.context.worktree_path.is_none());
    assert!(same_worktree_path(&wizard.context.quick_start_root, &repo));
    let view = wizard.view();
    assert!(!view.runtime_context_resolved);
    assert!(!view.show_runtime_target);
    assert!(view.selected_docker_service.is_none());
}

#[test]
fn app_runtime_launch_wizard_continue_resolves_runtime_context_from_worktree() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    run_git(&repo, &["init", "-q", "-b", "develop"]);
    run_git(&repo, &["config", "user.name", "Codex"]);
    run_git(&repo, &["config", "user.email", "codex@example.com"]);
    fs::write(repo.join("README.md"), "repo\n").expect("readme");
    fs::write(
        repo.join("docker-compose.yml"),
        "services:\n  app:\n    image: alpine:3.20\n",
    )
    .expect("compose");
    run_git(&repo, &["add", "README.md", "docker-compose.yml"]);
    run_git(&repo, &["commit", "-qm", "init"]);

    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_launch_wizard_for_branch("tab-1", &repo, "develop", None, None)
        .expect("open launch wizard");
    assert!(
        !runtime
            .launch_wizard
            .as_ref()
            .expect("wizard")
            .wizard
            .view()
            .runtime_context_resolved
    );

    let events = runtime.handle_launch_wizard_action(LaunchWizardAction::Submit, None);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].event,
        BackendEvent::LaunchWizardState { wizard: Some(_) }
    ));
    let pending_view = runtime
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .view();
    assert!(pending_view.runtime_resolution_pending);
    assert!(!pending_view.runtime_context_resolved);
    assert_eq!(pending_view.primary_action_label, "Preparing...");

    wait_for_recorded_event(
        "launch wizard runtime resolution",
        &recorded_events,
        |events| {
            events
                .iter()
                .any(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
        },
    );
    let resolved_event = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
            .map(|index| events.remove(index))
            .expect("runtime resolved event")
    };
    let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
        unreachable!("matched above")
    };
    let resolved_events = runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
    assert_eq!(resolved_events.len(), 1);

    let wizard = &runtime.launch_wizard.as_ref().expect("wizard").wizard;
    assert!(wizard
        .context
        .worktree_path
        .as_ref()
        .is_some_and(|path| same_worktree_path(path, &repo)));
    let view = wizard.view();
    assert!(!view.runtime_resolution_pending);
    assert!(view.runtime_context_resolved);
    assert!(view.show_runtime_target);
    assert_eq!(view.selected_runtime_target, "docker");
    assert_eq!(view.selected_docker_service.as_deref(), Some("app"));
}

#[test]
fn app_runtime_launch_wizard_continue_does_not_materialize_missing_worktree() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    let _origin = init_git_clone_with_origin(&repo);
    fs::write(
        repo.join("docker-compose.yml"),
        "services:\n  app:\n    image: alpine:3.20\n",
    )
    .expect("compose");
    run_git(&repo, &["add", "docker-compose.yml"]);
    run_git(&repo, &["commit", "-qm", "add compose"]);
    run_git(&repo, &["push", "origin", "develop"]);

    let branch_name = "work/runtime-deferral";
    let expected_worktree = gwt_git::worktree::sibling_worktree_path(&repo, branch_name);
    assert!(
        !expected_worktree.exists(),
        "fixture branch worktree should start absent"
    );

    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_launch_wizard_for_branch("tab-1", &repo, branch_name, None, None)
        .expect("open launch wizard");

    let events = runtime.handle_launch_wizard_action(LaunchWizardAction::Submit, None);
    assert_eq!(events.len(), 1);
    let pending_view = runtime
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .view();
    assert!(pending_view.runtime_resolution_pending);
    assert_eq!(
        pending_view.runtime_resolution_message.as_deref(),
        Some("Preparing runtime context...")
    );

    wait_for_recorded_event(
        "launch wizard runtime deferral",
        &recorded_events,
        |events| {
            events
                .iter()
                .any(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
        },
    );
    let resolved_event = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
            .map(|index| events.remove(index))
            .expect("runtime resolved event")
    };
    let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
        unreachable!("matched above")
    };
    let resolved_events = runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
    assert_eq!(resolved_events.len(), 1);

    let wizard = &runtime.launch_wizard.as_ref().expect("wizard").wizard;
    assert!(
        wizard.context.worktree_path.is_none(),
        "runtime confirmation should not resolve a newly-created target worktree"
    );
    assert!(
        !expected_worktree.exists(),
        "Runtime confirmation must not create {expected_worktree:?}"
    );
    let view = wizard.view();
    assert!(!view.runtime_resolution_pending);
    assert!(view.runtime_context_resolved);
    assert!(view.show_runtime_target);
    assert_eq!(view.selected_runtime_target, "docker");
    assert_eq!(view.selected_docker_service.as_deref(), Some("app"));
}

#[test]
fn app_runtime_start_work_parent_root_uses_develop_checkout_for_docker_context() {
    let temp = tempdir().expect("tempdir");
    let workspace_home = temp.path().join("workspace");
    let (bare_repo, develop_worktree) =
        init_managed_workspace_with_develop_worktree(&workspace_home);
    fs::write(
        develop_worktree.join("docker-compose.yml"),
        "services:\n  gwt:\n    image: alpine:3.20\n",
    )
    .expect("compose");

    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        workspace_home.clone(),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_start_work_for_project("tab-1", &workspace_home)
        .expect("open start work");
    let branch_name = runtime
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .context
        .normalized_branch_name
        .clone();
    let expected_worktree = gwt_git::worktree::sibling_worktree_path(&bare_repo, &branch_name);
    assert!(
        !expected_worktree.exists(),
        "fixture branch worktree should start absent"
    );

    resolve_launch_wizard_runtime_confirmation(
        &mut runtime,
        &recorded_events,
        "start work parent root docker context",
    );

    let wizard = &runtime.launch_wizard.as_ref().expect("wizard").wizard;
    assert!(
        wizard.context.worktree_path.is_none(),
        "Runtime confirmation must not resolve a missing target worktree"
    );
    assert!(
        !expected_worktree.exists(),
        "Runtime confirmation must not create {expected_worktree:?}"
    );
    let view = wizard.view();
    assert!(!view.runtime_resolution_pending);
    assert!(view.runtime_context_resolved);
    assert!(view.show_runtime_target);
    assert_eq!(view.selected_runtime_target, "docker");
    assert_eq!(view.selected_docker_service.as_deref(), Some("gwt"));
}

#[test]
fn app_runtime_start_work_parent_root_preserves_saved_host_while_showing_runtime_target() {
    let temp = tempdir().expect("tempdir");
    let workspace_home = temp.path().join("workspace");
    let (_bare_repo, develop_worktree) =
        init_managed_workspace_with_develop_worktree(&workspace_home);
    fs::write(
        develop_worktree.join("docker-compose.yml"),
        "services:\n  gwt:\n    image: alpine:3.20\n",
    )
    .expect("compose");

    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut session =
        gwt_agent::Session::new(&develop_worktree, "develop", gwt_agent::AgentId::Codex);
    session.runtime_target = gwt_agent::LaunchRuntimeTarget::Host;
    session.docker_service = None;
    session.save(&sessions_dir).expect("save session");

    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        workspace_home.clone(),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_start_work_for_project("tab-1", &workspace_home)
        .expect("open start work");
    resolve_launch_wizard_runtime_confirmation(
        &mut runtime,
        &recorded_events,
        "start work parent root saved host",
    );

    let view = runtime
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .view();
    assert!(view.runtime_context_resolved);
    assert!(view.show_runtime_target);
    assert_eq!(view.selected_runtime_target, "host");
    assert!(view.selected_docker_service.is_none());
    assert_eq!(
        view.docker_service_options
            .iter()
            .map(|option| option.value.as_str())
            .collect::<Vec<_>>(),
        vec!["gwt"]
    );
}

#[test]
fn app_runtime_launch_wizard_continue_falls_back_to_host_without_resolved_docker_context() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    run_git(&repo, &["init", "-q", "-b", "develop"]);
    run_git(&repo, &["config", "user.name", "Codex"]);
    run_git(&repo, &["config", "user.email", "codex@example.com"]);
    fs::write(repo.join("README.md"), "repo\n").expect("readme");
    run_git(&repo, &["add", "README.md"]);
    run_git(&repo, &["commit", "-qm", "init"]);

    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut session = gwt_agent::Session::new(&repo, "develop", gwt_agent::AgentId::Codex);
    session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
    session.docker_service = Some("app".to_string());
    session.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;
    session.save(&sessions_dir).expect("save session");

    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_launch_wizard_for_branch("tab-1", &repo, "develop", None, None)
        .expect("open launch wizard");
    let phase_one = runtime
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .view();
    assert!(!phase_one.runtime_context_resolved);
    assert_eq!(phase_one.selected_runtime_target, "host");
    assert!(!phase_one.show_runtime_target);

    let events = runtime.handle_launch_wizard_action(LaunchWizardAction::Submit, None);
    assert_eq!(events.len(), 1);
    let pending_view = runtime
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .view();
    assert!(pending_view.runtime_resolution_pending);
    assert!(!pending_view.runtime_context_resolved);

    wait_for_recorded_event("launch wizard host fallback", &recorded_events, |events| {
        events
            .iter()
            .any(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
    });
    let resolved_event = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
            .map(|index| events.remove(index))
            .expect("runtime resolved event")
    };
    let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
        unreachable!("matched above")
    };
    let resolved_events = runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
    assert_eq!(resolved_events.len(), 1);

    let view = runtime
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .view();
    assert!(!view.runtime_resolution_pending);
    assert!(view.runtime_context_resolved);
    assert_eq!(view.selected_runtime_target, "host");
    assert!(!view.show_runtime_target);
    assert!(view.selected_docker_service.is_none());
}

#[test]
fn app_runtime_workspace_add_agent_opens_branch_launch_without_branches_window() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Board],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.active_agent_sessions.insert(
        "tab-1::agent-1".to_string(),
        ActiveAgentSession {
            window_id: "tab-1::agent-1".to_string(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260504-1234".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.join("../repo-work-20260504-1234"),
            agent_project_root: repo
                .join("../repo-work-20260504-1234")
                .display()
                .to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::OpenActiveWorkLaunchWizard {
            branch_name: "work/20260504-1234".to_string(),
            linked_issue_number: None,
        },
    );

    assert!(matches!(
        events.first().map(|event| &event.event),
        Some(BackendEvent::LaunchWizardState { wizard: Some(_) })
    ));
    let view = runtime
        .launch_wizard
        .as_ref()
        .expect("active work launch wizard")
        .wizard
        .view();
    assert_eq!(view.mode, gwt::LaunchWizardMode::Branch);
    assert_eq!(view.branch_name, "work/20260504-1234");
    assert!(view.show_start_methods);
    assert!(!view.show_branch_controls);
    assert_eq!(view.live_sessions.len(), 1);
    assert_eq!(view.live_sessions[0].name, "Codex");

    let _ = runtime.handle_launch_wizard_action(
        gwt::LaunchWizardAction::UseStartMethod {
            method: gwt::LaunchWizardStartMethodKind::ConfigureAndStart,
        },
        None,
    );
    let configured_view = runtime
        .launch_wizard
        .as_ref()
        .expect("configured active work launch wizard")
        .wizard
        .view();
    assert!(!configured_view.show_start_methods);
    assert!(configured_view.show_branch_controls);
}

#[test]
fn app_runtime_live_sessions_report_composed_idle_runtime_status() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260504-1234".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: PathBuf::from("E:/gwt/test-repo"),
            agent_project_root: "E:/gwt/test-repo".to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    runtime.handle_runtime_hook_event(runtime_hook_state("Idle", "session-1"));
    let sessions = runtime.live_sessions_for_branch("tab-1", "work/20260504-1234");

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].runtime_status, WindowProcessStatus::Idle);
}

#[test]
fn app_runtime_live_sessions_report_idle_after_launch_before_first_hook() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260504-1234".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: PathBuf::from("E:/gwt/test-repo"),
            agent_project_root: "E:/gwt/test-repo".to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let sessions = runtime.live_sessions_for_branch("tab-1", "work/20260504-1234");

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].runtime_status, WindowProcessStatus::Idle);

    runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Idle",
        "SessionStart",
        "session-1",
    ));
    let sessions = runtime.live_sessions_for_branch("tab-1", "work/20260504-1234");

    assert_eq!(sessions[0].runtime_status, WindowProcessStatus::Idle);
}

#[test]
fn app_runtime_workspace_state_reports_idle_for_launched_agent_without_hook_state() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260504-1234".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: PathBuf::from("E:/gwt/test-repo"),
            agent_project_root: "E:/gwt/test-repo".to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let view = runtime.app_state_view();
    let tab = view.tabs.iter().find(|tab| tab.id == "tab-1").unwrap();
    let window = tab
        .workspace
        .windows
        .iter()
        .find(|window| window.id == "tab-1::agent-1")
        .unwrap();

    assert_eq!(window.status, WindowProcessStatus::Idle);
}

#[test]
fn app_runtime_workspace_state_normalizes_pre_lifecycle_agent_windows() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime.app_state_view();
    let tab = view.tabs.iter().find(|tab| tab.id == "tab-1").unwrap();
    let window = tab
        .workspace
        .windows
        .iter()
        .find(|window| window.id == "tab-1::agent-1")
        .unwrap();

    assert_eq!(window.status, WindowProcessStatus::Starting);
    assert_eq!(tab.running_agent_count, 0);
    assert!(tab.running_agents.is_empty());
}

#[test]
fn app_runtime_workspace_state_normalizes_agent_kanban_board_ids() {
    let temp = tempdir().expect("tempdir");
    let mut tab = sample_project_tab(
        "tab-1",
        "Repo",
        PathBuf::from("E:/gwt/test-repo"),
        ProjectKind::Git,
        &[WindowPreset::AgentKanban, WindowPreset::Agent],
    );
    assert!(tab.workspace.place_agent_window_in_kanban(
        "agent-1",
        "agent-kanban-1",
        gwt::AgentKanbanLane::Active,
        None,
    ));
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime.app_state_view();
    let tab = view.tabs.iter().find(|tab| tab.id == "tab-1").unwrap();
    let agent = tab
        .workspace
        .windows
        .iter()
        .find(|window| window.id == "tab-1::agent-1")
        .expect("agent window");

    assert_eq!(
        agent.placement,
        WindowPlacement::AgentKanban {
            board_id: "tab-1::agent-kanban-1".to_string(),
            lane_id: gwt::AgentKanbanLane::Active,
            order: 0,
            collapsed: false,
        },
        "workspace wire state must use the same combined IDs for windows and Kanban board references"
    );
}

#[test]
fn app_runtime_window_list_normalizes_pre_lifecycle_agent_windows() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let BackendEvent::WindowList { windows } = runtime.list_windows_event() else {
        panic!("expected window list");
    };
    let window = windows
        .iter()
        .find(|window| window.id == "tab-1::agent-1")
        .unwrap();

    assert_eq!(window.status, WindowProcessStatus::Starting);
}

#[test]
fn app_runtime_window_list_enumerates_all_project_tabs() {
    // SPEC-3038 (2026-06-20): the Command Rail Windows popover lists windows
    // from every project tab, not just the active one, so the list matches the
    // cross-tab open-window badge.
    let temp = tempdir().expect("tempdir");
    let tab_a = sample_project_tab_with_window(
        "tab-a",
        "win-a1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let tab_b = sample_project_tab_with_window(
        "tab-b",
        "win-b1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let runtime = sample_runtime(temp.path(), vec![tab_a, tab_b], Some("tab-a"));

    let BackendEvent::WindowList { windows } = runtime.list_windows_event() else {
        panic!("expected window list");
    };
    let ids: Vec<String> = windows.iter().map(|window| window.id.clone()).collect();
    assert!(
        ids.contains(&"tab-a::win-a1".to_string()),
        "active tab window must be listed: {ids:?}"
    );
    assert!(
        ids.contains(&"tab-b::win-b1".to_string()),
        "non-active tab window must also be listed: {ids:?}"
    );
    assert_eq!(windows.len(), 2, "all project-tab windows must be listed");
}

#[test]
fn app_runtime_open_intake_session_without_active_project_uses_intake_error_copy() {
    let temp = tempdir().expect("tempdir");
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::OpenIntakeSession);

    assert!(runtime.launch_wizard.is_none());
    assert!(matches!(
        events.first().map(|event| &event.target),
        Some(DispatchTarget::Client(client_id)) if client_id == "client-1"
    ));
    assert!(matches!(
        events.first().map(|event| &event.event),
        Some(BackendEvent::LaunchWizardOpenError { title, message })
            if title == "Intake" && message == "Open a project before starting an intake session"
    ));
}

#[test]
fn app_runtime_open_intake_session_failure_surfaces_launch_wizard_open_error() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::Git,
        &[WindowPreset::Board],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::OpenIntakeSession);

    assert!(runtime.launch_wizard.is_none());
    assert!(matches!(
        events.first().map(|event| &event.target),
        Some(DispatchTarget::Client(client_id)) if client_id == "client-1"
    ));
    assert!(matches!(
        events.first().map(|event| &event.event),
        Some(BackendEvent::LaunchWizardOpenError { title, message })
            if title == "Intake" && !message.is_empty()
    ));
}

#[test]
fn app_runtime_open_launch_wizard_failure_surfaces_launch_wizard_open_error() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().join("repo"),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::OpenLaunchWizard {
            id: "missing-window".to_string(),
            branch_name: "main".to_string(),
            linked_issue_number: None,
        },
    );

    assert!(runtime.launch_wizard.is_none());
    assert!(matches!(
        events.first().map(|event| &event.target),
        Some(DispatchTarget::Client(client_id)) if client_id == "client-1"
    ));
    assert!(matches!(
        events.first().map(|event| &event.event),
        Some(BackendEvent::LaunchWizardOpenError { title, message })
            if title == "Launch Agent" && message == "Window not found"
    ));
}

#[test]
fn app_runtime_open_launch_wizard_accepts_work_window_preset() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "work-1",
        repo,
        WindowPreset::Work,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "work-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::OpenLaunchWizard {
            id: window_id,
            branch_name: "main".to_string(),
            linked_issue_number: None,
        },
    );

    assert!(
        runtime.launch_wizard.is_some(),
        "Launch wizard should open from a Work window preset"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(&event.event, BackendEvent::LaunchWizardOpenError { .. })),
        "Work preset must not be rejected as 'not a Work surface'"
    );
}

#[test]
fn app_runtime_resume_branch_latest_agent_accepts_work_window_preset() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "work-1",
        repo,
        WindowPreset::Work,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "work-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeBranchLatestAgent {
            id: window_id,
            branch_name: "main".to_string(),
            bounds: canvas_bounds(),
        },
    );

    let has_surface_error = events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::BranchError { message, .. }
                if message.contains("is not a Work surface")
        )
    });
    assert!(
        !has_surface_error,
        "Work preset must not be rejected as 'not a Work surface'"
    );
}

#[test]
fn app_runtime_resume_workspace_failure_surfaces_launch_wizard_open_error() {
    // SPEC-2359 / Issue #2757: Resume クリックで `resume_workspace_events`
    // が早期 return / Start Work fallback 失敗を起こした場合、frontend で
    // 可視な `LaunchWizardOpenError` を return しなければならない。
    // 旧経路は `ProjectOpenError` を broadcast していたが、project 開放中は
    // `renderProjectPicker` が hidden なので silent failure になっていた。
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::Git,
        &[WindowPreset::Board],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspace {
            source: gwt::WorkspaceResumeSource::Current,
            journal_id: None,
        },
    );

    assert!(runtime.launch_wizard.is_none());
    assert!(
            matches!(
                events.first().map(|event| &event.target),
                Some(DispatchTarget::Client(client_id)) if client_id == "client-1"
            ),
            "Resume failure must be replied to the originating client, not broadcast as ProjectOpenError"
        );
    assert!(
            matches!(
                events.first().map(|event| &event.event),
                Some(BackendEvent::LaunchWizardOpenError { title, message })
                    if title == "Resume Work" && !message.is_empty()
            ),
            "Resume failure must surface as LaunchWizardOpenError so Work Overview can render a visible overlay"
        );
}

#[test]
fn app_runtime_resume_workspace_without_active_tab_returns_launch_wizard_open_error() {
    // Same contract for the `Open a project before resuming work` early return.
    let temp = tempdir().expect("tempdir");
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspace {
            source: gwt::WorkspaceResumeSource::Current,
            journal_id: None,
        },
    );

    assert!(matches!(
        events.first().map(|event| &event.target),
        Some(DispatchTarget::Client(client_id)) if client_id == "client-1"
    ));
    assert!(matches!(
        events.first().map(|event| &event.event),
        Some(BackendEvent::LaunchWizardOpenError { title, message })
            if title == "Resume Work"
                && message == "Open a project before resuming work"
    ));
}

#[test]
fn app_runtime_custom_agent_cache_refresh_rebroadcasts_open_wizard_state() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    runtime.launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));

    let events = runtime.custom_agent_reply_with_cache_refresh(
        "client-1".to_string(),
        BackendEvent::CustomAgentDeleted {
            agent_id: "custom-agent".to_string(),
        },
    );

    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[0].event,
        BackendEvent::CustomAgentDeleted { .. }
    ));
    assert!(matches!(
        events[1].event,
        BackendEvent::LaunchWizardState { wizard: Some(_) }
    ));
}

#[test]
fn app_runtime_launch_wizard_submit_failure_emits_structured_error_log() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::NonRepo,
        &[WindowPreset::Branches],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.launch_wizard = Some(sample_no_agent_launch_wizard_session("tab-1", &repo));

    let events = capture_tracing_events(|| {
        let _ =
            runtime.handle_launch_wizard_action(LaunchWizardAction::Submit, Some(canvas_bounds()));
    });

    let event = events
        .iter()
        .find(|event| {
            event.level == Level::ERROR
                && event.target == "gwt::agent_launch"
                && event.fields.get("stage").map(String::as_str) == Some("wizard_submit")
        })
        .expect("launch wizard submit failure log");
    assert_eq!(
        event.fields.get("wizard_id").map(String::as_str),
        Some("wizard-unavailable-agent")
    );
    assert_eq!(
        event.fields.get("tab_id").map(String::as_str),
        Some("tab-1")
    );
    assert_eq!(
        event.fields.get("selected_agent_id").map(String::as_str),
        Some("")
    );
    assert_eq!(
        event.fields.get("requested_agent_id").map(String::as_str),
        Some("none")
    );
    assert_eq!(
        event
            .fields
            .get("selected_launch_target")
            .map(String::as_str),
        Some("agent")
    );
    assert_eq!(
        event.fields.get("error").map(String::as_str),
        Some("Agent option is unavailable")
    );
}

#[test]
fn app_runtime_launch_submit_returns_materialization_pending_before_dispatch() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.launch_wizard = Some(sample_start_work_confirm_session("tab-1", &repo));

    let events = runtime.handle_launch_wizard_action_for_client(
        Some("client-1"),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );

    assert_eq!(events.len(), 1);
    let BackendEvent::LaunchWizardState {
        wizard: Some(wizard),
    } = &events[0].event
    else {
        panic!("expected pending wizard state before launch dispatch: {events:?}");
    };
    assert!(wizard.launch_materialization_pending);
    assert_eq!(
        wizard.launch_materialization_message.as_deref(),
        Some("Preparing worktree...")
    );
    assert_eq!(wizard.primary_action_label, "Launching...");
    assert!(!wizard.primary_action_enabled);

    let recorded = recorded_events.lock().expect("event log");
    assert_eq!(
        recorded
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    UserEvent::LaunchWizardLaunchMaterializationRequested { .. }
                )
            })
            .count(),
        1,
        "actual launch must be deferred to exactly one internal event",
    );
    drop(recorded);

    let duplicate_events =
        runtime.handle_launch_wizard_action(LaunchWizardAction::Submit, Some(canvas_bounds()));
    assert!(
        duplicate_events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::LaunchWizardState { .. })),
        "duplicate submit should keep returning the pending wizard state",
    );
    let recorded = recorded_events.lock().expect("event log");
    assert_eq!(
        recorded
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    UserEvent::LaunchWizardLaunchMaterializationRequested { .. }
                )
            })
            .count(),
        1,
        "duplicate submit while pending must not enqueue a second launch",
    );
}

#[test]
fn app_runtime_launch_wizard_set_agent_failure_logs_requested_agent() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::NonRepo,
        &[WindowPreset::Branches],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.launch_wizard = Some(sample_no_agent_launch_wizard_session("tab-1", &repo));

    let events = capture_tracing_events(|| {
        let _ = runtime.handle_launch_wizard_action(
            LaunchWizardAction::SetAgent {
                agent_id: "codex".to_string(),
            },
            Some(canvas_bounds()),
        );
    });

    let event = events
        .iter()
        .find(|event| {
            event.level == Level::ERROR
                && event.target == "gwt::agent_launch"
                && event.fields.get("stage").map(String::as_str) == Some("agent_select")
        })
        .expect("set agent failure log");
    assert_eq!(
        event.fields.get("requested_agent_id").map(String::as_str),
        Some("codex")
    );
    assert_eq!(
        event
            .fields
            .get("selected_runtime_target")
            .map(String::as_str),
        Some("host")
    );
    assert_eq!(
        event
            .fields
            .get("selected_tool_version")
            .map(String::as_str),
        Some("")
    );
}

#[test]
fn app_runtime_agent_launch_completion_failure_emits_structured_error_log() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");

    let events = capture_tracing_events(|| {
        let _ = runtime.handle_launch_complete(
            window_id.clone(),
            Err("launch failed before process spawn".to_string()),
        );
    });

    let event = events
        .iter()
        .find(|event| {
            event.level == Level::ERROR
                && event.target == "gwt::agent_launch"
                && event.fields.get("stage").map(String::as_str) == Some("launch_complete")
        })
        .expect("agent launch completion failure log");
    assert_eq!(
        event.fields.get("window_id").map(String::as_str),
        Some(window_id.as_str())
    );
    assert_eq!(
        event.fields.get("tab_id").map(String::as_str),
        Some("tab-1")
    );
    assert_eq!(
        event.fields.get("error").map(String::as_str),
        Some("launch failed before process spawn")
    );
}

#[test]
fn app_runtime_agent_launch_completion_failure_writes_diagnostic_to_terminal() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");

    let events = runtime.handle_launch_complete(
        window_id.clone(),
        Err("launch failed before process spawn".to_string()),
    );

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::TerminalStatus { id, status, detail }
                if id == &window_id
                    && *status == WindowProcessStatus::Error
                    && detail.as_deref() == Some("launch failed before process spawn")
        )
    }));
    let diagnostic = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::TerminalOutput { id, data_base64 } if id == &window_id => {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(data_base64)
                    .expect("decode terminal diagnostic");
                Some(String::from_utf8_lossy(&decoded).to_string())
            }
            _ => None,
        })
        .expect("launch failure diagnostic terminal output");
    assert!(
        diagnostic.contains("Launch failed before PTY started"),
        "diagnostic must explain that no PTY output exists yet: {diagnostic:?}"
    );
    assert!(
        diagnostic.contains("launch failed before process spawn"),
        "diagnostic must include the launch error detail: {diagnostic:?}"
    );
}

#[test]
fn app_runtime_antigravity_missing_binary_launch_error_is_actionable() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let raw_error = "PTY creation failed: Unable to spawn agy because: \
No viable candidates found in PATH \
\"/private/var/folders/tmp/node_modules/.bin:/opt/homebrew/bin:/Users/example/.local/bin\"";

    let events = runtime.handle_launch_complete(window_id.clone(), Err(raw_error.to_string()));

    let detail = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::TerminalStatus { id, status, detail }
                if id == &window_id && *status == WindowProcessStatus::Error =>
            {
                detail.as_deref()
            }
            _ => None,
        })
        .expect("terminal status detail");
    assert!(detail.contains("Antigravity CLI (`agy`) was not found"));
    assert!(detail.contains("https://antigravity.google/cli/install.sh"));
    assert!(detail.contains("~/.local/bin"));
    assert!(!detail.contains("No viable candidates found in PATH"));
    assert!(!detail.contains("/private/var/folders"));

    let diagnostic = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::TerminalOutput { id, data_base64 } if id == &window_id => {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(data_base64)
                    .expect("decode terminal diagnostic");
                Some(String::from_utf8_lossy(&decoded).to_string())
            }
            _ => None,
        })
        .expect("launch failure diagnostic terminal output");
    assert!(diagnostic.contains("Antigravity CLI (`agy`) was not found"));
    assert!(diagnostic.contains("https://antigravity.google/cli/install.sh"));
    assert!(!diagnostic.contains("No viable candidates found in PATH"));
    assert!(!diagnostic.contains("node_modules/.bin"));
}

#[test]
fn app_runtime_antigravity_missing_binary_launch_wizard_error_is_actionable() {
    let temp = tempdir().expect("tempdir");
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let raw_error = "PTY creation failed: Unable to spawn agy because: \
No viable candidates found in PATH \
\"/private/var/folders/tmp/node_modules/.bin:/opt/homebrew/bin:/Users/example/.local/bin\"";

    let events = runtime.launch_error_events(
        "tab-1::agent-1".to_string(),
        raw_error.to_string(),
        Some(LaunchFeedbackContext {
            client_id: "client-1".to_string(),
            title: "Launch failed".to_string(),
            issue_monitor_issue_number: None,
        }),
    );

    let message = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::LaunchWizardOpenError { title, message } if title == "Launch failed" => {
                Some(message.as_str())
            }
            _ => None,
        })
        .expect("launch wizard open error");
    assert!(message.contains("Antigravity CLI (`agy`) was not found"));
    assert!(message.contains("https://antigravity.google/cli/install.sh"));
    assert!(message.contains("~/.local/bin"));
    assert!(!message.contains("No viable candidates found in PATH"));
    assert!(!message.contains("/private/var/folders"));
}

// SPEC-3151 FR-003 / AS-3: when neither a native `opencode` binary nor a
// package runner is available, the raw PTY error must be rewritten into an
// actionable install hint, matching the Antigravity treatment.
#[test]
fn app_runtime_opencode_missing_binary_launch_error_is_actionable() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let raw_error = "PTY creation failed: Unable to spawn opencode because: \
No viable candidates found in PATH \
\"/private/var/folders/tmp/node_modules/.bin:/opt/homebrew/bin:/Users/example/.local/bin\"";

    let events = runtime.handle_launch_complete(window_id.clone(), Err(raw_error.to_string()));

    let detail = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::TerminalStatus { id, status, detail }
                if id == &window_id && *status == WindowProcessStatus::Error =>
            {
                detail.as_deref()
            }
            _ => None,
        })
        .expect("terminal status detail");
    assert!(detail.contains("OpenCode (`opencode`) was not found"));
    assert!(detail.contains("npm i -g opencode-ai"));
    assert!(detail.contains("bunx/npx"));
    assert!(!detail.contains("No viable candidates found in PATH"));
    assert!(!detail.contains("/private/var/folders"));
}

#[test]
fn app_runtime_opencode_missing_binary_launch_wizard_error_is_actionable() {
    let temp = tempdir().expect("tempdir");
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let raw_error = "PTY creation failed: Unable to spawn opencode because: \
No viable candidates found in PATH \
\"/private/var/folders/tmp/node_modules/.bin:/opt/homebrew/bin:/Users/example/.local/bin\"";

    let events = runtime.launch_error_events(
        "tab-1::agent-1".to_string(),
        raw_error.to_string(),
        Some(LaunchFeedbackContext {
            client_id: "client-1".to_string(),
            title: "Launch failed".to_string(),
            issue_monitor_issue_number: None,
        }),
    );

    let message = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::LaunchWizardOpenError { title, message } if title == "Launch failed" => {
                Some(message.as_str())
            }
            _ => None,
        })
        .expect("launch wizard open error");
    assert!(message.contains("OpenCode (`opencode`) was not found"));
    assert!(message.contains("npm i -g opencode-ai"));
    assert!(message.contains("bunx/npx"));
    assert!(!message.contains("No viable candidates found in PATH"));
    assert!(!message.contains("/private/var/folders"));
}

#[test]
fn app_runtime_issue_monitor_launch_error_emits_monitor_failure_events() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");

    let events = runtime.launch_error_events(
        window_id,
        "binary missing".to_string(),
        Some(LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(42),
        }),
    );

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorLaunchFailed {
                issue_number,
                message,
            } if *issue_number == 42 && message == "binary missing"
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast {
                level,
                message,
                issue_number
            } if level == "error" && message == "binary missing" && *issue_number == Some(42)
        )
    }));
}

#[test]
fn app_runtime_issue_monitor_git_auth_launch_failure_is_actionable() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");

    let events = runtime.launch_error_events(
        window_id,
        "fatal: could not read Username for 'https://github.com': terminal prompts disabled"
            .to_string(),
        Some(LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(42),
        }),
    );

    let message = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorLaunchFailed {
                issue_number,
                message,
            } if *issue_number == 42 => Some(message.as_str()),
            _ => None,
        })
        .expect("issue monitor launch failure message");
    assert!(message.contains("Git HTTPS credentials are required"));
    assert!(message.contains("gh auth setup-git"));
    assert!(message.contains("git ls-remote origin HEAD"));
    assert!(message.contains("Original error: fatal: could not read Username"));
}

#[test]
fn app_runtime_issue_monitor_launch_complete_marks_issue_launched_and_keeps_active_capacity() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Issue Monitor launch success",
            &["bug"],
            "Issue body",
            "2026-06-23T00:00:00Z",
        ))
        .expect("write issue cache");
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            enabled: true,
            max_active_agents: 1,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("save issue monitor prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.pending_launch_feedback_contexts.insert(
        window_id.clone(),
        LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(42),
        },
    );
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };

    let events = runtime.handle_launch_complete(
        window_id.clone(),
        Ok((
            ProcessLaunch {
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(repo.clone()),
            },
            "session-issue-42".to_string(),
            "work/issue-42".to_string(),
            "Codex".to_string(),
            repo.clone(),
            gwt_agent::AgentId::Codex,
            Some(42),
            Some("origin/develop".to_string()),
            gwt_agent::LaunchRuntimeTarget::Host,
            repo.display().to_string(),
        )),
    );

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    assert_eq!(status.state, "active");
    assert_eq!(status.active_count, 1);
    assert_eq!(status.active_issue_number, Some(42));

    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    let item = inbox
        .iter()
        .find(|item| item.issue.number == 42)
        .expect("launched issue row");
    assert_eq!(item.state, gwt::MonitorInboxState::Launched);
    assert_eq!(item.launched_window_id.as_deref(), Some(window_id.as_str()));

    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("load issue monitor prefs");
    assert_eq!(
        prefs.launched_issues,
        vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id,
        }]
    );
}

#[test]
fn app_runtime_closing_issue_monitor_window_returns_issue_to_pending() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Issue Monitor close returns to pending",
            &["bug"],
            "Issue body",
            "2026-06-23T00:00:00Z",
        ))
        .expect("write issue cache");
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            enabled: true,
            max_active_agents: 1,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("save issue monitor prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.pending_launch_feedback_contexts.insert(
        window_id.clone(),
        LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(42),
        },
    );
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };
    let _ = runtime.handle_launch_complete(
        window_id.clone(),
        Ok((
            ProcessLaunch {
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(repo.clone()),
            },
            "session-issue-42".to_string(),
            "work/issue-42".to_string(),
            "Codex".to_string(),
            repo.clone(),
            gwt_agent::AgentId::Codex,
            Some(42),
            Some("origin/develop".to_string()),
            gwt_agent::LaunchRuntimeTarget::Host,
            repo.display().to_string(),
        )),
    );

    // Closing the launched window must free the active slot and return the
    // (unmerged) Issue to pending — never a fabricated completion state.
    let events = runtime.close_window_events(&window_id);
    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status after close");
    assert_eq!(
        status.active_count, 0,
        "closing the launched window frees the active slot"
    );
    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox after close");
    let item = inbox
        .iter()
        .find(|item| item.issue.number == 42)
        .expect("issue row after close");
    assert_eq!(
        item.state,
        gwt::MonitorInboxState::Queued,
        "an unmerged close returns the Issue to pending"
    );

    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("load issue monitor prefs");
    assert!(
        prefs.launched_issues.is_empty(),
        "closed window is no longer persisted as an active launch"
    );
}

#[test]
fn app_runtime_runtime_error_marks_issue_monitor_launched_issue_failed() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Issue Monitor runtime failure",
            &["bug"],
            "Issue body",
            "2026-06-23T00:00:00Z",
        ))
        .expect("write issue cache");
    let window_id = combined_window_id("tab-1", "agent-1");
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            enabled: true,
            max_active_agents: 5,
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: window_id.clone(),
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("save issue monitor prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("Stop-block hit an error".to_string()),
    );

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    assert_eq!(status.state, "error");
    assert_eq!(status.active_count, 0);
    assert_eq!(
        status.last_error.as_deref(),
        Some("issue #42: Stop-block hit an error")
    );

    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    let item = inbox
        .iter()
        .find(|item| item.issue.number == 42)
        .expect("failed issue row");
    assert_eq!(item.state, gwt::MonitorInboxState::AgentFailed);
    assert_eq!(item.launched_window_id, None);
    assert_eq!(
        item.error_message.as_deref(),
        Some("Stop-block hit an error")
    );
    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast {
                level,
                message,
                issue_number,
            } if level == "error"
                && message == "Stop-block hit an error"
                && *issue_number == Some(42)
        )
    }));

    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("load issue monitor prefs");
    assert!(prefs.launched_issues.is_empty());
    assert_eq!(
        prefs.failed_issues,
        vec![gwt::IssueMonitorFailedIssue {
            issue_number: 42,
            message: "Stop-block hit an error".to_string(),
            // #3165 error-window lifecycle: the failed agent window id is
            // retained so an explicit Launch Now can close the stale window.
            window_id: Some(window_id.clone()),
        }]
    );
}

#[test]
fn app_runtime_hook_error_marks_issue_monitor_launched_issue_failed_with_hook_message() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Issue Monitor hook failure",
            &["bug"],
            "Issue body",
            "2026-06-23T00:00:00Z",
        ))
        .expect("write issue cache");
    let window_id = combined_window_id("tab-1", "agent-1");
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            enabled: true,
            max_active_agents: 5,
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: window_id.clone(),
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("save issue monitor prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let mut hook = runtime_hook_state("Error", "session-1");
    hook.project_root = Some(repo.display().to_string());
    hook.message = Some("Stop-block hit an error".to_string());

    let events = runtime.handle_runtime_hook_event(hook);

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    assert_eq!(status.state, "error");
    assert_eq!(
        status.last_error.as_deref(),
        Some("issue #42: Stop-block hit an error")
    );
    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    let item = inbox
        .iter()
        .find(|item| item.issue.number == 42)
        .expect("failed issue row");
    assert_eq!(item.state, gwt::MonitorInboxState::AgentFailed);
    assert_eq!(
        item.error_message.as_deref(),
        Some("Stop-block hit an error")
    );
}

#[test]
fn app_runtime_frontend_ready_replays_launch_error_diagnostic_snapshot_without_runtime() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let _ = runtime.handle_launch_complete(
        window_id.clone(),
        Err("launch failed before process spawn".to_string()),
    );

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

    let snapshot = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::TerminalSnapshot { id, data_base64 } if id == &window_id => {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(data_base64)
                    .expect("decode terminal diagnostic snapshot");
                Some(String::from_utf8_lossy(&decoded).to_string())
            }
            _ => None,
        })
        .expect("launch failure diagnostic terminal snapshot");
    assert!(
        snapshot.contains("Launch failed before PTY started"),
        "snapshot must replay the launch diagnostic after reconnect: {snapshot:?}"
    );
    assert!(
        snapshot.contains("launch failed before process spawn"),
        "snapshot must include the launch error detail: {snapshot:?}"
    );
}

#[test]
fn app_runtime_launch_wizard_submit_emits_agent_window_launching_status() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));

    let submit_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LaunchWizardAction {
            action: LaunchWizardAction::Submit,
            bounds: Some(canvas_bounds()),
        },
    );
    assert!(submit_events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::LaunchWizardState {
                wizard: Some(wizard)
            } if wizard.launch_materialization_pending
        )
    }));

    let events = dispatch_launch_materialization_request(
        &mut runtime,
        &recorded_events,
        "launch wizard submit materialization",
    );

    let workspace = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::WindowCanvasState { workspace } => Some(workspace),
            _ => None,
        })
        .expect("workspace state after wizard submit");
    let agent_window = workspace
        .tabs
        .iter()
        .find(|tab| tab.id == "tab-1")
        .and_then(|tab| {
            tab.workspace
                .windows
                .iter()
                .find(|window| window.preset == WindowPreset::Agent)
        })
        .expect("agent placeholder window");
    assert_eq!(agent_window.title, "Codex");
    assert_eq!(agent_window.agent_id.as_deref(), Some("codex"));

    let launch_status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::TerminalStatus { id, detail, .. }
                if detail.as_deref() == Some("Launching...") =>
            {
                Some(id)
            }
            _ => None,
        })
        .expect("launching terminal status");
    assert!(launch_status.ends_with("::agent-1"));
    assert!(events.iter().any(|event| {
        matches!(
            event.event,
            BackendEvent::LaunchWizardState { wizard: None }
        )
    }));
}

#[test]
fn app_runtime_launch_complete_missing_wizard_window_surfaces_open_error() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));

    let _submit_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LaunchWizardAction {
            action: LaunchWizardAction::Submit,
            bounds: Some(canvas_bounds()),
        },
    );
    let launch_events = dispatch_launch_materialization_request(
        &mut runtime,
        &recorded_events,
        "launch wizard complete materialization",
    );
    let window_id = launch_events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::TerminalStatus { id, detail, .. }
                if detail.as_deref() == Some("Launching...") =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .expect("wizard launch window id");
    let address = runtime
        .window_lookup
        .remove(&window_id)
        .expect("registered agent window");
    let tab = runtime.tab_mut(&address.tab_id).expect("tab");
    assert!(tab.workspace.close_window(&address.raw_id));

    let completion_events =
        runtime.handle_launch_complete(window_id, Err("Window not found".to_string()));

    assert!(completion_events.iter().any(|event| {
        matches!(
            (&event.target, &event.event),
            (
                DispatchTarget::Client(client_id),
                BackendEvent::LaunchWizardOpenError { title, message }
            ) if client_id == "client-1"
                && title == "Launch Agent"
                && message == "Window not found"
        )
    }));
}

#[test]
fn app_runtime_start_work_launch_completion_registers_unassigned_agent() {
    let _env_guard = env_test_lock().lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-20260504-1234");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };

    let _events = runtime.handle_launch_complete(
        window_id,
        Ok((
            ProcessLaunch {
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(worktree.clone()),
            },
            "session-1".to_string(),
            "work/20260504-1234".to_string(),
            "Codex".to_string(),
            worktree.clone(),
            gwt_agent::AgentId::Codex,
            None,
            Some("origin/main".to_string()),
            gwt_agent::LaunchRuntimeTarget::Host,
            worktree.display().to_string(),
        )),
    );

    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load projection")
        .expect("projection");
    assert_eq!(
        projection.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Unknown,
        "Start Work must not make an unassigned Agent an active Workspace"
    );
    assert!(projection.git_details.is_none());
    assert_eq!(projection.agents.len(), 1);
    assert_eq!(projection.agents[0].session_id, "session-1");
    assert_eq!(
        projection.agents[0].affiliation_status,
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned
    );
    assert_eq!(
        projection.agents[0].branch.as_deref(),
        Some("work/20260504-1234")
    );
    assert_eq!(
        projection.agents[0].worktree_path.as_deref(),
        Some(worktree.as_path())
    );
    let work_items =
        gwt_core::workspace_projection::load_workspace_work_items(&repo).expect("load work items");
    assert!(
        work_items.is_none(),
        "Start Work launch must not create Workspace history before explicit assignment"
    );
}

#[test]
fn app_runtime_non_work_launch_registers_unassigned_agent() {
    let _env_guard = env_test_lock().lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };

    let _events = runtime.handle_launch_complete(
        window_id,
        Ok((
            ProcessLaunch {
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(repo.clone()),
            },
            "session-develop".to_string(),
            "develop".to_string(),
            "Codex".to_string(),
            repo.clone(),
            gwt_agent::AgentId::Codex,
            None,
            Some("origin/develop".to_string()),
            gwt_agent::LaunchRuntimeTarget::Host,
            repo.display().to_string(),
        )),
    );

    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load projection")
        .expect("projection");
    assert_eq!(projection.agents.len(), 1);
    assert_eq!(projection.agents[0].session_id, "session-develop");
    assert_eq!(
        projection.agents[0].affiliation_status,
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned
    );
    assert_eq!(projection.agents[0].branch.as_deref(), Some("develop"));
}

#[test]
fn app_runtime_workspace_resume_launch_completion_carries_context_to_projection() {
    let _env_guard = env_test_lock().lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-20260507-0001");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.pending_workspace_resume_contexts.insert(
        window_id.clone(),
        WorkspaceResumeContext {
            title: Some("Suspended review".to_string()),
            owner: Some("SPEC-2359".to_string()),
            summary: Some("Resume the suspended Work card.".to_string()),
            next_action: Some("Resume the review".to_string()),
        },
    );
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };

    let _events = runtime.handle_launch_complete(
        window_id,
        Ok((
            ProcessLaunch {
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(worktree.clone()),
            },
            "session-1".to_string(),
            "work/20260507-0001".to_string(),
            "Codex".to_string(),
            worktree.clone(),
            gwt_agent::AgentId::Codex,
            Some(2359),
            None,
            gwt_agent::LaunchRuntimeTarget::Host,
            worktree.display().to_string(),
        )),
    );

    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load projection")
        .expect("projection");
    let details = projection.git_details.expect("git details");
    assert_eq!(projection.title, "Suspended review");
    assert_eq!(projection.owner.as_deref(), Some("SPEC-2359"));
    assert_eq!(
        projection.summary.as_deref(),
        Some("Resume the suspended Work card.")
    );
    assert_eq!(projection.next_action.as_deref(), Some("Resume the review"));
    assert_eq!(details.branch.as_deref(), Some("work/20260507-0001"));
    assert_eq!(details.worktree_path.as_deref(), Some(worktree.as_path()));
    assert!(details.created_by_start_work);
    assert_eq!(projection.agents.len(), 1);
    assert_eq!(projection.agents[0].session_id, "session-1");
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load work items")
        .expect("work items");
    assert_eq!(
        work_items.work_items[0].events[0].kind,
        gwt_core::workspace_projection::WorkEventKind::Resume
    );
}

#[test]
fn app_runtime_issue_launch_wizard_seeds_issue_workspace_context() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3096,
            "Fix Launch Agent trace",
            &["bug"],
            "Launch trace missing",
            "2026-06-20T00:00:00Z",
        ))
        .expect("write issue cache");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_knowledge_launch_wizard_for_base_branch(
            "tab-1",
            &repo,
            "develop",
            3096,
            LinkedIssueKind::Issue,
        )
        .expect("open issue launch wizard");

    let context = runtime
        .launch_wizard
        .as_ref()
        .and_then(|session| session.workspace_resume_context.as_ref())
        .expect("issue launch wizard should carry workspace context");
    assert_eq!(context.owner.as_deref(), Some("Issue #3096"));
    assert_eq!(context.title.as_deref(), Some("Fix Launch Agent trace"));
}

#[test]
fn app_runtime_issue_launch_completion_records_issue_owned_start_work_event() {
    let _env_guard = env_test_lock().lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-issue-3096");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    init_repo(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.pending_workspace_resume_contexts.insert(
        window_id.clone(),
        WorkspaceResumeContext {
            title: Some("Fix Launch Agent trace".to_string()),
            owner: Some("Issue #3096".to_string()),
            summary: None,
            next_action: None,
        },
    );
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };

    let _events = runtime.handle_launch_complete(
        window_id,
        Ok((
            ProcessLaunch {
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(worktree.clone()),
            },
            "session-issue-3096".to_string(),
            "work/issue-3096".to_string(),
            "Codex".to_string(),
            worktree.clone(),
            gwt_agent::AgentId::Codex,
            Some(3096),
            Some("develop".to_string()),
            gwt_agent::LaunchRuntimeTarget::Host,
            worktree.display().to_string(),
        )),
    );

    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load work items")
        .expect("work items");
    let item = work_items
        .work_items
        .iter()
        .find(|item| item.owner.as_deref() == Some("Issue #3096"))
        .expect("issue-owned work item");
    assert_eq!(item.title, "Fix Launch Agent trace");
    assert_eq!(item.agents[0].session_id, "session-issue-3096");
    assert_eq!(
        item.events[0].kind,
        gwt_core::workspace_projection::WorkEventKind::Start
    );
    assert_eq!(
        item.execution_containers[0].branch.as_deref(),
        Some("work/issue-3096")
    );
}

#[test]
fn app_runtime_start_work_launch_completion_registers_multiple_unassigned_agents() {
    let _env_guard = env_test_lock().lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree_one = temp.path().join("repo-work-20260504-1234");
    let worktree_two = temp.path().join("repo-work-20260504-1235");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree_one).expect("create worktree one");
    fs::create_dir_all(&worktree_two).expect("create worktree two");
    let mut persisted = empty_workspace_state();
    persisted.windows.push(sample_window(
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    ));
    persisted.windows.push(sample_window(
        "agent-2",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    ));
    persisted.next_z_index = 3;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let launch = |cwd: PathBuf| {
        let (command, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec![
                    "/d".to_string(),
                    "/s".to_string(),
                    "/c".to_string(),
                    "exit /b 0".to_string(),
                ],
            )
        } else {
            (
                "/bin/sh".to_string(),
                vec!["-lc".to_string(), "exit 0".to_string()],
            )
        };
        ProcessLaunch {
            command,
            args,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: Some(cwd),
        }
    };

    let _first_events = runtime.handle_launch_complete(
        combined_window_id("tab-1", "agent-1"),
        Ok((
            launch(worktree_one.clone()),
            "session-1".to_string(),
            "work/20260504-1234".to_string(),
            "Codex 1".to_string(),
            worktree_one.clone(),
            gwt_agent::AgentId::Codex,
            None,
            Some("origin/main".to_string()),
            gwt_agent::LaunchRuntimeTarget::Host,
            worktree_one.display().to_string(),
        )),
    );
    let second_events = runtime.handle_launch_complete(
        combined_window_id("tab-1", "agent-2"),
        Ok((
            launch(worktree_two.clone()),
            "session-2".to_string(),
            "work/20260504-1235".to_string(),
            "Codex 2".to_string(),
            worktree_two.clone(),
            gwt_agent::AgentId::Codex,
            None,
            Some("origin/main".to_string()),
            gwt_agent::LaunchRuntimeTarget::Host,
            worktree_two.display().to_string(),
        )),
    );

    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load projection")
        .expect("projection");
    let session_ids = projection
        .agents
        .iter()
        .map(|agent| agent.session_id.as_str())
        .collect::<std::collections::HashSet<_>>();

    assert_eq!(projection.agents.len(), 2);
    assert!(session_ids.contains("session-1"));
    assert!(session_ids.contains("session-2"));
    assert!(projection
        .agents
        .iter()
        .all(gwt_core::workspace_projection::WorkspaceAgentSummary::is_unassigned));
    assert!(second_events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Broadcast,
            event: BackendEvent::ActiveWorkProjection { projection },
        } if projection.active_agents == 2
            && projection.active_work_count == 2
            && projection.agents.len() == 2
            && projection.unassigned_agents.is_empty()
    )));
}

#[test]
fn app_runtime_active_work_projection_groups_live_assigned_agents_by_work_id() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-a", Some("work-a"));
        agent.window_id = Some("tab-1::agent-a".to_string());
        agent.branch = Some("work/a".to_string());
        agent.title_summary = Some("Parser cleanup".to_string());
        agent
    });
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-b", Some("work-b"));
        agent.window_id = Some("tab-1::agent-b".to_string());
        agent.branch = Some("work/b".to_string());
        agent.title_summary = Some("UI polish".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    for (window_id, session_id, branch) in [
        ("tab-1::agent-a", "session-a", "work/a"),
        ("tab-1::agent-b", "session-b", "work/b"),
    ] {
        let mut session = sample_active_agent_session("tab-1", window_id);
        session.session_id = session_id.to_string();
        session.branch_name = branch.to_string();
        session.window_id = window_id.to_string();
        runtime
            .active_agent_sessions
            .insert(window_id.to_string(), session);
    }
    // SPEC-2359 Phase W-12 Slice 2 (FR-348): Work identity is
    // `agent_session_id`-derived, so each live session owns its own row.
    let expected_a = "work-session-session-a";
    let expected_b = "work-session-session-b";

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_work_count, 2);
    assert_eq!(view.active_works.len(), 2);
    assert_eq!(view.active_works[0].agents.len(), 1);
    assert_eq!(view.active_works[1].agents.len(), 1);
    assert!(view.active_works.iter().any(|work| work.id == expected_a
        && work
            .agents
            .iter()
            .any(|agent| agent.session_id == "session-a")));
    assert!(view.active_works.iter().any(|work| work.id == expected_b
        && work
            .agents
            .iter()
            .any(|agent| agent.session_id == "session-b")));
}

#[test]
fn app_runtime_active_work_projection_includes_managed_hook_health() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    gwt_skills::generate_codex_hooks(&repo).expect("generate codex hooks");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-a");
    session.session_id = "session-a".to_string();
    session.worktree_path = repo.clone();
    session.agent_project_root = repo.display().to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::agent-a".to_string(), session);
    let runtime_path = gwt_agent::runtime_state_path(&runtime.sessions_dir, "session-a");
    gwt::cli::hook::runtime_state::write_for_event(&runtime_path, "PreToolUse")
        .expect("runtime state");

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");

    let health = view
        .managed_hook_health
        .as_ref()
        .expect("managed hook health");
    assert_eq!(health.status, "ready");
    assert_eq!(health.last_event.as_deref(), Some("PreToolUse"));
    assert!(health.issues.is_empty(), "{:?}", health.issues);
}

/// SPEC-2359 Phase W-12 Slice 2 (FR-348): "1 agent session : 1 Work". When
/// the *same* `session_id` surfaces under multiple windows, the agents
/// collapse into a single Work row keyed by that session. The Workspace detail
/// is then normalized to the latest visible entry per agent identity.
#[test]
fn app_runtime_active_work_projection_groups_same_session_windows_in_one_work_row() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    for window_id in ["tab-1::agent-a", "tab-1::agent-b"] {
        let mut agent = workspace_agent_summary_for_test("session-shared", Some("work-shared"));
        agent.window_id = Some(window_id.to_string());
        agent.branch = Some("work/shared".to_string());
        projection.agents.push(agent);
    }
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    for window_id in ["tab-1::agent-a", "tab-1::agent-b"] {
        let mut session = sample_active_agent_session("tab-1", window_id);
        session.session_id = "session-shared".to_string();
        session.branch_name = "work/shared".to_string();
        session.window_id = window_id.to_string();
        runtime
            .active_agent_sessions
            .insert(window_id.to_string(), session);
    }

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_work_count, 1);
    assert_eq!(view.active_works.len(), 1);
    assert_eq!(view.active_works[0].id, "work-session-session-shared");
    assert_eq!(view.active_works[0].agents.len(), 1);
    assert!(view.active_works[0]
        .agents
        .iter()
        .all(|agent| agent.session_id == "session-shared"));
    assert_eq!(
        view.active_works[0].session_agent_total, 2,
        "hidden same-agent candidates stay counted for the session summary"
    );
}

/// SPEC-2359 Phase W-12 Slice 2 (FR-348): `agent_session_id` is the storage
/// identity, but the Workspace detail shows only the latest visible entry per
/// agent identity after same-branch Works are grouped into one row.
#[test]
fn app_runtime_active_work_projection_separates_sessions_on_same_branch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    for (session_id, window_id) in [
        ("session-a", "tab-1::agent-a"),
        ("session-b", "tab-1::agent-b"),
    ] {
        let mut agent = workspace_agent_summary_for_test(session_id, Some("work-shared"));
        agent.window_id = Some(window_id.to_string());
        agent.branch = Some("work/shared".to_string());
        agent.updated_at = if session_id == "session-a" {
            Utc.with_ymd_and_hms(2026, 6, 17, 9, 0, 0).unwrap()
        } else {
            Utc.with_ymd_and_hms(2026, 6, 17, 10, 0, 0).unwrap()
        };
        projection.agents.push(agent);
    }
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    for (window_id, session_id) in [
        ("tab-1::agent-a", "session-a"),
        ("tab-1::agent-b", "session-b"),
    ] {
        let mut session = sample_active_agent_session("tab-1", window_id);
        session.session_id = session_id.to_string();
        session.branch_name = "work/shared".to_string();
        session.window_id = window_id.to_string();
        runtime
            .active_agent_sessions
            .insert(window_id.to_string(), session);
    }

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");

    // SPEC-2359 W16-2 (FR-389 / SC-259) supersedes the original two-row
    // contract here: storage still keys one Work per agent session (FR-348),
    // but the VIEW groups same-branch Works into one Workspace row carrying
    // both live agents.
    assert_eq!(
        view.active_works.len(),
        1,
        "same branch groups into one row"
    );
    let row = &view.active_works[0];
    assert_eq!(
        row.agents.len(),
        1,
        "same agent identity collapses to the newest visible session"
    );
    assert!(row
        .agents
        .iter()
        .any(|agent| agent.session_id == "session-b"));
    assert!(!row
        .agents
        .iter()
        .any(|agent| agent.session_id == "session-a"));
    assert_eq!(
        row.session_agent_total, 2,
        "hidden same-agent candidates stay counted for the session summary"
    );
    assert_eq!(
        row.works.len(),
        2,
        "both launch-scoped Works remain addressable"
    );
    assert!(row
        .works
        .iter()
        .any(|work| work.id == "work-session-session-a"));
    assert!(row
        .works
        .iter()
        .any(|work| work.id == "work-session-session-b"));
    assert!(row.workspace_key.is_some());
}

/// SPEC-2359 Phase W-12 Slice 2 (FR-349): each `active_works` item carries a
/// `lifecycle_state` derived from the agent-session Work lifecycle. Active
/// Work rows group a live, assigned, running agent session, so the wire
/// state is `"active"` and `closed_at` is None (agent stop alone never
/// closes a Work — FR-350).
#[test]
fn app_runtime_active_work_projection_sets_lifecycle_state_active() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-a", Some("work-a"));
        agent.window_id = Some("tab-1::agent-a".to_string());
        agent.branch = Some("work/a".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-a");
    session.session_id = "session-a".to_string();
    session.branch_name = "work/a".to_string();
    session.window_id = "tab-1::agent-a".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::agent-a".to_string(), session);

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_works.len(), 1);
    assert_eq!(view.active_works[0].lifecycle_state, "active");
    assert_eq!(view.active_works[0].closed_at, None);
}

/// SPEC-2359 Phase W-12 Slice 2 (FR-349): the `lifecycle_state` field is
/// back-compat — a serialized `ActiveWorkItemView` payload that predates the
/// field deserializes with `lifecycle_state = "active"` and `closed_at =
/// None` via the serde defaults.
#[test]
fn active_work_item_view_lifecycle_state_back_compat_default() {
    let legacy = serde_json::json!({
        "id": "work-1",
        "title": "Legacy Work",
        "status_category": "active",
        "status_text": "1 active agent",
        "summary": null,
        "owner": null,
        "next_action": null,
        "active_agents": 1,
        "blocked_agents": 0,
        "branch": "work/legacy",
        "worktree_path": null,
        "pr_number": null,
        "pr_url": null,
        "pr_state": null,
        "board_refs": [],
        "agents": []
    });
    let view: gwt::ActiveWorkItemView =
        serde_json::from_value(legacy).expect("deserialize legacy active work item");
    assert_eq!(view.lifecycle_state, "active");
    assert_eq!(view.closed_at, None);
}

/// SPEC-2359 Phase W-12 Slice 2 (FR-348): `agent_session_id` is the primary
/// Work identity, taking priority over both the branch-derived
/// `canonical_work_id` and the raw `workspace_id`. The resulting Work id is
/// `work-session-<session_id>`, and the branch-derived id is *not* used when
/// a session is present.
#[test]
fn app_runtime_active_work_projection_uses_agent_session_id_over_branch_and_workspace_id() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let branch_derived_work_id =
        gwt_core::workspace_projection::canonical_work_id(&repo, Some("work/test"), None)
            .expect("canonical work id");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-a", Some("legacy-work-id"));
        agent.window_id = Some("tab-1::agent-a".to_string());
        agent.branch = Some("work/test".to_string());
        agent.title_summary = Some("Parser cleanup".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-a");
    session.session_id = "session-a".to_string();
    session.branch_name = "work/test".to_string();
    session.window_id = "tab-1::agent-a".to_string();
    runtime
        .active_agent_sessions
        .insert(session.window_id.clone(), session);

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_work_count, 1);
    assert_eq!(view.active_works[0].id, "work-session-session-a");
    assert_ne!(view.active_works[0].id, branch_derived_work_id);
    assert_ne!(view.active_works[0].id, "legacy-work-id");
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): when the owning agent session
/// stops, the Work must not vanish from the Work surface. It is retained as
/// a `paused` `active_works` row (keyed by the session-derived Work id) until
/// the user explicitly closes it. Agent stop alone never closes a Work.
#[test]
fn app_runtime_active_work_projection_retains_stopped_agent_work_as_paused() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-paused", Some("work-paused"));
        agent.window_id = Some("tab-1::agent-paused".to_string());
        agent.branch = Some("work/paused".to_string());
        agent.title_summary = Some("Paused persistence".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-paused");
    session.session_id = "session-paused".to_string();
    session.branch_name = "work/paused".to_string();
    session.worktree_path = repo.join("work/paused");
    session.window_id = "tab-1::agent-paused".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::agent-paused".to_string(), session);

    // While the agent is live the Work is Active.
    let live_view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("live projection view");
    assert_eq!(live_view.active_works.len(), 1);
    assert_eq!(live_view.active_works[0].lifecycle_state, "active");

    // Agent stops: session leaves active_agent_sessions and a paused marker
    // is persisted to the work history.
    runtime.mark_agent_session_stopped("tab-1::agent-paused");
    assert!(!runtime
        .active_agent_sessions
        .contains_key("tab-1::agent-paused"));

    let paused_view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("paused projection view");
    assert_eq!(
        paused_view.active_works.len(),
        1,
        "stopped agent Work must remain in active_works as paused"
    );
    let paused = &paused_view.active_works[0];
    assert_eq!(paused.id, "work-session-session-paused");
    assert_eq!(paused.lifecycle_state, "paused");
    assert_eq!(paused.closed_at, None);
    assert_eq!(paused.active_agents, 0);
    assert_eq!(paused.branch.as_deref(), Some("work/paused"));
}

// SPEC-3214 T-005/T-007: an ephemeral intake session leaves NO Work identity
// and its throwaway `.intake-*` worktree is removed when it ends (clean), while
// a dirty intake worktree is kept so no in-progress work is lost.
#[test]
fn ephemeral_intake_session_stop_removes_clean_worktree_and_emits_no_paused_work() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "user.name", "Test User"]);
    run_git(&repo, &["commit", "--allow-empty", "-m", "init"]);

    let intake = temp.path().join(".intake-clean");
    gwt_git::WorktreeManager::new(&repo)
        .create_detached("HEAD", &intake)
        .expect("intake worktree");
    assert!(intake.exists());

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::intake");
    session.session_id = "session-intake".to_string();
    session.branch_name = String::new();
    session.worktree_path = intake.clone();
    session.window_id = "tab-1::intake".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::intake".to_string(), session);

    runtime.mark_agent_session_stopped("tab-1::intake");

    assert!(!runtime.active_agent_sessions.contains_key("tab-1::intake"));
    assert!(
        !intake.exists(),
        "clean intake worktree is removed when the session ends"
    );
    let active_work_count = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .map(|view| view.active_works.len())
        .unwrap_or(0);
    assert_eq!(
        active_work_count, 0,
        "an ephemeral intake session emits no Work identity (paused or otherwise)"
    );
    let recorded = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .ok()
        .flatten();
    assert!(
        recorded.is_none_or(|projection| projection.work_items.is_empty()),
        "no Work event is recorded for an ephemeral intake session"
    );
}

#[test]
fn ephemeral_intake_session_stop_keeps_dirty_worktree() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "user.name", "Test User"]);
    run_git(&repo, &["commit", "--allow-empty", "-m", "init"]);

    let intake = temp.path().join(".intake-dirty");
    gwt_git::WorktreeManager::new(&repo)
        .create_detached("HEAD", &intake)
        .expect("intake worktree");
    // Uncommitted work must not be destroyed.
    fs::write(intake.join("wip.txt"), "unsaved intake work").expect("write wip");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::intake");
    session.session_id = "session-intake-dirty".to_string();
    session.branch_name = String::new();
    session.worktree_path = intake.clone();
    session.window_id = "tab-1::intake".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::intake".to_string(), session);

    runtime.mark_agent_session_stopped("tab-1::intake");

    assert!(
        intake.exists() && intake.join("wip.txt").exists(),
        "a dirty intake worktree is kept so uncommitted work is never lost"
    );
}

// SPEC-3214 (codex #3235 review): a NORMAL branch worktree that a user happens
// to name `.intake-*` must NOT be misclassified as an ephemeral intake session
// — it keeps its worktree and its Paused-Work behavior. Classification requires
// the worktree to be branchless (detached), not just `.intake-*`-named.
#[test]
fn branch_worktree_named_intake_is_not_treated_as_ephemeral() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "user.name", "Test User"]);
    run_git(&repo, &["commit", "--allow-empty", "-m", "init"]);

    // A real BRANCH worktree that merely happens to be named `.intake-real`.
    let branch_wt = temp.path().join(".intake-real");
    gwt_git::WorktreeManager::new(&repo)
        .create_from_base("HEAD", "feature/real", &branch_wt)
        .expect("branch worktree");
    assert!(branch_wt.exists());

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::real");
    session.session_id = "session-real".to_string();
    session.branch_name = "feature/real".to_string();
    session.worktree_path = branch_wt.clone();
    session.window_id = "tab-1::real".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::real".to_string(), session);

    runtime.mark_agent_session_stopped("tab-1::real");

    assert!(
        branch_wt.exists(),
        "a real branch worktree named .intake-* must not be removed as ephemeral"
    );
    let works = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .ok()
        .flatten();
    assert!(
        works.is_some_and(|projection| !projection.work_items.is_empty()),
        "a real branch session still records a Paused Work"
    );
}

// #3065: a stopped session's Pause record must not inherit the repo-shared
// projection's owner/title — those belong to whatever Work last wrote the
// projection. Owner/summary come from the session's own Work item (matched
// by branch container); the title fallback is the matched item's title.
#[test]
fn paused_work_does_not_inherit_shared_projection_owner() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");

    // Shared projection poisoned with a foreign Work's identity. The agent
    // summary carries no title of its own, so the old code fell back to the
    // shared title and copied the shared owner verbatim.
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.id = "work-foreign-99999999".to_string();
    projection.title = "gwt-manage-pr".to_string();
    projection.owner = Some("SPEC-2359".to_string());
    projection.summary = Some("foreign summary".to_string());
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-paused", Some("work-paused"));
        agent.window_id = Some("tab-1::agent-paused".to_string());
        agent.branch = Some("work/paused".to_string());
        agent.title_summary = None;
        agent.current_focus = None;
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    // The session's own Work item with its own identity.
    let now = chrono::Utc::now();
    let work_id =
        gwt_core::workspace_projection::canonical_work_id(&repo, Some("work/paused"), None)
            .expect("canonical id");
    let mut event = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id,
        now,
    );
    event.title = Some("own work title".to_string());
    event.owner = Some("Issue #7".to_string());
    event.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/paused".to_string()),
            worktree_path: Some(repo.join("work/paused")),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, event)
        .expect("record work event");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-paused");
    session.session_id = "session-paused".to_string();
    session.branch_name = "work/paused".to_string();
    session.worktree_path = repo.join("work/paused");
    session.window_id = "tab-1::agent-paused".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::agent-paused".to_string(), session);

    runtime.mark_agent_session_stopped("tab-1::agent-paused");

    let works = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load works")
        .expect("works projection");
    let paused = works
        .work_items
        .iter()
        .find(|item| item.id == "work-session-session-paused")
        .expect("paused session work item");
    assert_eq!(
        paused.owner.as_deref(),
        Some("Issue #7"),
        "pause must carry the session's own work owner, not the shared projection's"
    );
    assert_eq!(
        paused.title, "own work title",
        "pause title falls back to the session's own work item, not the shared projection"
    );
}

/// SPEC-2359 Phase W-12 Slice 4 (FR-352): closing a Paused Work with
/// `close_kind = "done"` records a terminal Done close and removes the Work
/// from the active Work surface. No live agent owns the Work, so the close
/// is not blocked.
#[test]
fn app_runtime_close_work_done_removes_paused_work_from_active_surface() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-done", Some("work-done"));
        agent.window_id = Some("tab-1::agent-done".to_string());
        agent.branch = Some("work/done".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-done");
    session.session_id = "session-done".to_string();
    session.branch_name = "work/done".to_string();
    session.worktree_path = repo.join("work/done");
    session.window_id = "tab-1::agent-done".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::agent-done".to_string(), session);

    // Stop → Paused row retained on the active surface.
    runtime.mark_agent_session_stopped("tab-1::agent-done");
    let paused_view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("paused projection view");
    assert_eq!(paused_view.active_works.len(), 1);
    assert_eq!(paused_view.active_works[0].lifecycle_state, "paused");

    // Close (Done): the Work leaves the active surface.
    let events = runtime.close_work("work-session-session-done", "done");
    assert!(
        !events.is_empty(),
        "close_work should broadcast a refreshed projection"
    );

    let closed_view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("closed projection view");
    assert!(
        closed_view
            .active_works
            .iter()
            .all(|work| work.id != "work-session-session-done"),
        "Done-closed Work must not appear in active_works"
    );

    // The retained work history records the Done terminal close.
    let works = gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(&repo)
        .expect("load work items");
    let item = works
        .work_items
        .iter()
        .find(|item| item.id == "work-session-session-done")
        .expect("work item exists");
    assert_eq!(
        item.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Done
    );
    assert!(!item.discarded);
    assert!(item.is_terminal());
}

/// SPEC-2359 Phase W-12 Slice 4 (FR-352): closing a Paused Work with
/// `close_kind = "discarded"` records a terminal Discard close (distinct from
/// Done) and removes the Work from the active surface.
#[test]
fn app_runtime_close_work_discarded_marks_terminal_and_removes_from_surface() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-discard", Some("work-discard"));
        agent.window_id = Some("tab-1::agent-discard".to_string());
        agent.branch = Some("work/discard".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-discard");
    session.session_id = "session-discard".to_string();
    session.branch_name = "work/discard".to_string();
    session.worktree_path = repo.join("work/discard");
    session.window_id = "tab-1::agent-discard".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::agent-discard".to_string(), session);

    runtime.mark_agent_session_stopped("tab-1::agent-discard");

    let events = runtime.close_work("work-session-session-discard", "discarded");
    assert!(!events.is_empty());

    let closed_view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("closed projection view");
    assert!(
        closed_view
            .active_works
            .iter()
            .all(|work| work.id != "work-session-session-discard"),
        "Discarded Work must not appear in active_works"
    );

    let works = gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(&repo)
        .expect("load work items");
    let item = works
        .work_items
        .iter()
        .find(|item| item.id == "work-session-session-discard")
        .expect("work item exists");
    assert!(item.discarded, "Discard close must mark the Work discarded");
    assert_ne!(
        item.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Done,
        "Discard is distinct from Done"
    );
    assert!(item.is_terminal());
}

/// SPEC-2359 Phase W-12 Slice 4 (FR-352): a close request for a Work whose
/// owning agent session is still live must be blocked. The worktree is not
/// removed and the Work stays Active on the surface — the agent must be
/// stopped first.
#[test]
fn app_runtime_close_work_blocks_when_owning_agent_is_live() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    // A real worktree directory so we can assert it is NOT removed.
    let worktree_path = repo.join("work/live");
    fs::create_dir_all(&worktree_path).expect("create worktree dir");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-live", Some("work-live"));
        agent.window_id = Some("tab-1::agent-live".to_string());
        agent.branch = Some("work/live".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-live");
    session.session_id = "session-live".to_string();
    session.branch_name = "work/live".to_string();
    session.worktree_path = worktree_path.clone();
    session.window_id = "tab-1::agent-live".to_string();
    runtime
        .active_agent_sessions
        .insert("tab-1::agent-live".to_string(), session);

    // Agent is live: close must be blocked.
    let events = runtime.close_work("work-session-session-live", "done");
    assert!(
        events.is_empty(),
        "a blocked close must not broadcast a projection change"
    );
    assert!(
        worktree_path.exists(),
        "blocked close must never remove the live worktree"
    );

    // No terminal close was recorded; the Work remains live/active.
    let live_view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("live projection view");
    let work = live_view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-session-live")
        .expect("live Work still present");
    assert_eq!(work.lifecycle_state, "active");
    let works = gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(&repo)
        .expect("load work items");
    assert!(
        works
            .work_items
            .iter()
            .find(|item| item.id == "work-session-session-live")
            .is_none_or(|item| !item.is_terminal()),
        "a blocked close must not record a terminal event"
    );
}

/// SPEC-2359 W-21 (FR-463): Work close records lifecycle history only. The
/// actual worktree and branch remain until the independent cleanup transport
/// removes them.
#[test]
fn app_runtime_close_work_retains_worktree_and_branch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");

    // Initialize a real git repo with an initial commit.
    let run_git = |args: &[&str], cwd: &Path| {
        let output = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git command");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run_git(&["init"], &repo);
    run_git(&["config", "user.email", "test@example.com"], &repo);
    run_git(&["config", "user.name", "Test User"], &repo);
    run_git(&["commit", "--allow-empty", "-m", "init"], &repo);

    // Create a real worktree on a dedicated branch.
    let manager = gwt_git::WorktreeManager::new(&repo);
    let base = if crate::runtime_support::local_branch_exists(&repo, "main").unwrap_or(false) {
        "main"
    } else {
        "master"
    };
    let worktree_path = temp.path().join("work-cleanup");
    manager
        .create_from_base(base, "work/cleanup", &worktree_path)
        .expect("create worktree");
    assert!(worktree_path.exists());

    // A saved (empty) Workspace projection so the close broadcast can build
    // the active-work projection view for the tab.
    let projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    // Persist a Paused Work whose execution container points at the worktree.
    let now = chrono::Utc::now();
    gwt_core::workspace_projection::record_workspace_work_paused_event(
        &repo,
        "work-session-session-cleanup",
        Some("Cleanup work"),
        None,
        None,
        &[],
        Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/cleanup".to_string()),
                worktree_path: Some(worktree_path.clone()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        ),
        Some("session-cleanup"),
        now,
    )
    .expect("record paused work");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    // No live agent session: close the Work without cleaning its materialization.
    let events = runtime.close_work("work-session-session-cleanup", "done");
    assert!(!events.is_empty());

    assert!(
        worktree_path.exists(),
        "Work close must retain the worktree directory"
    );
    assert!(
        crate::runtime_support::local_branch_exists(&repo, "work/cleanup").unwrap_or(false),
        "worktree-only cleanup must retain the branch (branch / PR are preserved)"
    );
    let works = gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(&repo)
        .expect("load Work history");
    let closed = works
        .work_items
        .iter()
        .find(|item| item.id == "work-session-session-cleanup")
        .expect("closed Work remains in history");
    assert!(closed.is_terminal());
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): resuming a paused Work (a live
/// agent session for the same `session_id` returns) must surface a single
/// Active row — the live grouping wins and no duplicate Paused row is
/// emitted from the retained work history.
#[test]
fn app_runtime_active_work_projection_resumed_paused_work_is_single_active_row() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let worktree = repo.join("work/resume");
    fs::create_dir_all(&worktree).expect("create worktree path");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-resume", Some("work-resume"));
        agent.window_id = Some("tab-1::agent-resume".to_string());
        agent.branch = Some("work/resume".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let build_session = |runtime: &mut AppRuntime, worktree_path: PathBuf| {
        let mut session = sample_active_agent_session("tab-1", "tab-1::agent-resume");
        session.session_id = "session-resume".to_string();
        session.branch_name = "work/resume".to_string();
        session.worktree_path = worktree_path;
        session.window_id = "tab-1::agent-resume".to_string();
        runtime
            .active_agent_sessions
            .insert("tab-1::agent-resume".to_string(), session);
    };
    build_session(&mut runtime, worktree.clone());

    // Stop → paused marker persisted to work history.
    runtime.mark_agent_session_stopped("tab-1::agent-resume");
    let paused_view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("paused projection view");
    assert_eq!(paused_view.active_works.len(), 1);
    assert_eq!(paused_view.active_works[0].lifecycle_state, "paused");

    // Resume: the same session returns to active_agent_sessions.
    let alternate_worktree_path = worktree.join("..").join("resume");
    assert_ne!(worktree, alternate_worktree_path);
    assert!(same_worktree_path(&worktree, &alternate_worktree_path));
    build_session(&mut runtime, alternate_worktree_path);
    let resumed_view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("resumed projection view");
    assert_eq!(
        resumed_view.active_works.len(),
        1,
        "resumed Work must dedupe to a single Active row (no Paused duplicate)"
    );
    assert_eq!(
        resumed_view.active_works[0].id,
        "work-session-session-resume"
    );
    assert_eq!(resumed_view.active_works[0].lifecycle_state, "active");
    assert_eq!(
        resumed_view.active_works[0].works.len(),
        1,
        "one resumed Session must stay one child Work across equivalent worktree paths"
    );
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): a live Work and an unrelated
/// paused Work coexist as two rows — the live one Active, the retained one
/// Paused — without the merge collapsing or dropping either.
#[test]
fn app_runtime_active_work_projection_merges_live_and_paused_work_rows() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    for (session_id, branch) in [("session-live", "work/live"), ("session-stop", "work/stop")] {
        let mut agent = workspace_agent_summary_for_test(session_id, Some(session_id));
        agent.window_id = Some(format!("tab-1::agent-{session_id}"));
        agent.branch = Some(branch.to_string());
        projection.agents.push(agent);
    }
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    for (session_id, branch) in [("session-live", "work/live"), ("session-stop", "work/stop")] {
        let window_id = format!("tab-1::agent-{session_id}");
        let mut session = sample_active_agent_session("tab-1", &window_id);
        session.session_id = session_id.to_string();
        session.branch_name = branch.to_string();
        session.worktree_path = repo.join(branch);
        session.window_id = window_id.clone();
        runtime.active_agent_sessions.insert(window_id, session);
    }

    // Stop only the second agent; the first remains live.
    runtime.mark_agent_session_stopped("tab-1::agent-session-stop");

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");
    assert_eq!(view.active_works.len(), 2, "live + paused Work rows");
    let live = view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-session-live")
        .expect("live Work row");
    assert_eq!(live.lifecycle_state, "active");
    let paused = view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-session-stop")
        .expect("paused Work row");
    assert_eq!(paused.lifecycle_state, "paused");
    assert_eq!(paused.active_agents, 0);
}

fn history_agent_ref_view(
    session_id: &str,
    agent_id: Option<&str>,
    updated_at: &str,
) -> gwt::WorkspaceHistoryAgentView {
    gwt::WorkspaceHistoryAgentView {
        session_id: session_id.to_string(),
        agent_id: agent_id.map(str::to_string),
        display_name: agent_id.map(str::to_string),
        updated_at: updated_at.to_string(),
        sessions: Vec::new(),
    }
}

fn history_work_view(
    id: &str,
    branch: &str,
    worktree: &str,
    agents: Vec<gwt::WorkspaceHistoryAgentView>,
) -> gwt::WorkspaceHistoryView {
    gwt::WorkspaceHistoryView {
        id: id.to_string(),
        title: branch.to_string(),
        intent: None,
        summary: None,
        progress_summary: None,
        status_category: "active".to_string(),
        owner: None,
        created_at: "2026-06-29T07:45:56Z".to_string(),
        updated_at: "2026-06-30T08:45:48Z".to_string(),
        completed_at: None,
        agents,
        execution_containers: vec![gwt::WorkspaceExecutionContainerView {
            branch: Some(branch.to_string()),
            worktree_path: Some(worktree.to_string()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        }],
        board_refs: Vec::new(),
        related_workspace_ids: Vec::new(),
        events: Vec::new(),
    }
}

fn git_details_for_active_work_test(
    branch: &str,
    worktree: &str,
) -> gwt_core::workspace_projection::GitDetails {
    gwt_core::workspace_projection::GitDetails {
        branch: Some(branch.to_string()),
        worktree_path: Some(PathBuf::from(worktree)),
        base_branch: Some("origin/develop".to_string()),
        pr_number: None,
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    }
}

/// Issue #3213 regression (PR #3205 orphaned): a stray agent ref that shares a
/// session id with ANOTHER branch's Work must not swallow that Work's row.
/// Reproduces the affected project's works.json: the issue-3184 item carried a
/// mis-attributed (empty-identity) ref to issue-3197's session, and the
/// issue-3197 row — the session's legitimate owner — vanished from the
/// Workspace list with no remaining surface to resume it from.
#[test]
fn app_runtime_paused_work_row_survives_stray_shared_session_on_other_branch() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    let works = vec![
        history_work_view(
            "work-work-issue-3184-9431c779",
            "work/issue-3184",
            "/home/user/gwt/work/issue-3184",
            vec![
                history_agent_ref_view(
                    "810665c4-2e03-46b3-879b-7eec0064d038",
                    Some("Claude Code"),
                    "2026-06-29T07:46:15Z",
                ),
                // The stray ref: recorded under issue-3184 with an empty
                // identity, but the session belongs to issue-3197.
                history_agent_ref_view(
                    "0fe1b919-09dd-47e5-8976-76f4478aa907",
                    None,
                    "2026-06-29T08:24:29Z",
                ),
            ],
        ),
        history_work_view(
            "work-work-issue-3197-00504508",
            "work/issue-3197",
            "/home/user/gwt/work/issue-3197",
            vec![history_agent_ref_view(
                "0fe1b919-09dd-47e5-8976-76f4478aa907",
                Some("Claude Code"),
                "2026-06-29T07:45:56Z",
            )],
        ),
    ];

    let view =
        super::active_work_projection_from_saved_with_journal(projection, Vec::new(), works, None);

    assert_eq!(
        view.active_works.len(),
        2,
        "both branch rows must surface despite the shared session id"
    );
    let issue_3197 = view
        .active_works
        .iter()
        .find(|work| work.id == "work-work-issue-3197-00504508")
        .expect("work/issue-3197 row must not be swallowed by the stray session ref");
    assert_eq!(issue_3197.branch.as_deref(), Some("work/issue-3197"));
    assert_eq!(issue_3197.lifecycle_state, "paused");
}

/// SPEC-2359 FR-471: a shared Session id cannot override a conflicting
/// worktree identity even when the branch identity agrees.
#[test]
fn app_runtime_shared_session_with_conflicting_worktree_keeps_both_works() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.git_details = Some(git_details_for_active_work_test(
        "work/shared",
        "/home/user/gwt/work/paused",
    ));
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("shared-session", Some("work-live"));
        agent.branch = Some("work/shared".to_string());
        agent.worktree_path = Some(PathBuf::from("/home/user/gwt/work/live"));
        agent
    });
    let mut paused = history_work_view(
        "legacy-work-paused",
        "work/shared",
        "/home/user/gwt/work/paused",
        vec![history_agent_ref_view(
            "shared-session",
            Some("Codex"),
            "2026-07-12T01:00:00Z",
        )],
    );
    paused.title = "Paused worktree-conflict history".to_string();
    paused.summary = Some("Paused worktree-conflict summary".to_string());
    paused.owner = Some("Issue #471".to_string());
    paused.execution_containers[0].pr_number = Some(471);
    paused.execution_containers[0].pr_url = Some("https://example.test/pr/471".to_string());
    paused.board_refs = vec!["paused-board-ref".to_string()];

    let view = super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![paused],
        None,
    );

    assert_eq!(
        view.active_works.len(),
        2,
        "a worktree conflict must prevent shared-Session deduplication"
    );
    let live = view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-shared-session")
        .expect("live Work");
    assert_eq!(live.title, "Board audience follow-up");
    assert_eq!(live.summary, None);
    assert_eq!(live.owner, None);
    assert_eq!(live.pr_number, None);
    assert!(live.board_refs.is_empty());
}

/// SPEC-2359 FR-471: a shared Session id cannot override a conflicting
/// branch identity even when the worktree identity agrees.
#[test]
fn app_runtime_shared_session_with_conflicting_branch_keeps_both_works() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let shared_worktree = "/home/user/gwt/work/shared";
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.git_details = Some(git_details_for_active_work_test(
        "work/paused",
        shared_worktree,
    ));
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("shared-session", Some("work-live"));
        agent.branch = Some("work/live".to_string());
        agent.worktree_path = Some(PathBuf::from(shared_worktree));
        agent
    });
    let mut paused = history_work_view(
        "legacy-work-paused",
        "work/paused",
        shared_worktree,
        vec![history_agent_ref_view(
            "shared-session",
            Some("Codex"),
            "2026-07-12T01:00:00Z",
        )],
    );
    paused.title = "Paused branch-conflict history".to_string();
    paused.summary = Some("Paused branch-conflict summary".to_string());
    paused.owner = Some("Issue #472".to_string());
    paused.execution_containers[0].pr_number = Some(472);
    paused.execution_containers[0].pr_url = Some("https://example.test/pr/472".to_string());
    paused.board_refs = vec!["paused-board-ref".to_string()];

    let view = super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![paused],
        None,
    );

    assert_eq!(
        view.active_works.len(),
        2,
        "a branch conflict must prevent shared-Session deduplication"
    );
    let live = view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-shared-session")
        .expect("live Work");
    assert_eq!(live.title, "Board audience follow-up");
    assert_eq!(live.summary, None);
    assert_eq!(live.owner, None);
    assert_eq!(live.pr_number, None);
    assert!(live.board_refs.is_empty());
}

/// SPEC-2359 FR-471: projection fallback must participate in history matching
/// when the live Agent omits branch identity, so foreign history metadata does
/// not leak through a matching worktree.
#[test]
fn app_runtime_projection_branch_fallback_blocks_foreign_history_metadata() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let shared_worktree = "/home/user/gwt/work/shared";
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.title = "Live projection title".to_string();
    projection.git_details = Some(git_details_for_active_work_test(
        "work/live",
        shared_worktree,
    ));
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("shared-session", Some("work-live"));
        agent.branch = None;
        agent.worktree_path = Some(PathBuf::from(shared_worktree));
        agent
    });
    let mut foreign = history_work_view(
        "legacy-work-foreign",
        "work/foreign",
        shared_worktree,
        vec![history_agent_ref_view(
            "shared-session",
            Some("Codex"),
            "2026-07-12T01:00:00Z",
        )],
    );
    foreign.title = "Foreign history title".to_string();
    foreign.summary = Some("Foreign history summary".to_string());

    let view = super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![foreign],
        None,
    );

    assert_eq!(view.active_works.len(), 2);
    let live = view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-shared-session")
        .expect("live Work");
    assert_eq!(live.branch.as_deref(), Some("work/live"));
    assert_eq!(live.title, "Live projection title");
    assert_eq!(live.summary, None);
}

/// SPEC-2359 FR-471: projection fallback must participate in history matching
/// when the live Agent omits worktree identity, so foreign history metadata
/// does not leak through a matching branch.
#[test]
fn app_runtime_projection_worktree_fallback_blocks_foreign_history_metadata() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let live_worktree = "/home/user/gwt/work/live";
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.title = "Live projection title".to_string();
    projection.git_details = Some(git_details_for_active_work_test(
        "work/shared",
        live_worktree,
    ));
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("shared-session", Some("work-live"));
        agent.branch = Some("work/shared".to_string());
        agent.worktree_path = None;
        agent
    });
    let mut foreign = history_work_view(
        "legacy-work-foreign",
        "work/shared",
        "/home/user/gwt/work/foreign",
        vec![history_agent_ref_view(
            "shared-session",
            Some("Codex"),
            "2026-07-12T01:00:00Z",
        )],
    );
    foreign.title = "Foreign history title".to_string();
    foreign.summary = Some("Foreign history summary".to_string());

    let view = super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![foreign],
        None,
    );

    assert_eq!(view.active_works.len(), 2);
    let live = view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-shared-session")
        .expect("live Work");
    assert_eq!(live.worktree_path.as_deref(), Some(live_worktree));
    assert_eq!(live.title, "Live projection title");
    assert_eq!(live.summary, None);
}

/// SPEC-2359 FR-470/FR-471: branch/worktree can group a parent Workspace but
/// cannot assign a legacy history record to a live Work when neither exact
/// Work id nor a non-empty shared Session identifies that launch.
#[test]
fn app_runtime_sessionless_live_work_does_not_claim_legacy_history_by_git_identity() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let shared_worktree = "/home/user/gwt/work/shared";
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("", Some("work-live"));
        agent.branch = Some("work/shared".to_string());
        agent.worktree_path = Some(PathBuf::from(shared_worktree));
        agent
    });
    let mut legacy = history_work_view(
        "legacy-work-paused",
        "work/shared",
        shared_worktree,
        Vec::new(),
    );
    legacy.title = "Legacy history metadata".to_string();
    legacy.summary = Some("Legacy history summary".to_string());

    let view = super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![legacy],
        None,
    );

    assert_eq!(
        view.active_works.len(),
        2,
        "git identity alone must not collapse or assign launch-scoped Work metadata"
    );
    let live = view
        .active_works
        .iter()
        .find(|work| work.id != "legacy-work-paused")
        .expect("sessionless live Work");
    assert_eq!(live.title, "Board audience follow-up");
    assert_eq!(live.summary, None);
}

/// SPEC-2359 US-85 / SC-313: Work identity is launch-scoped even when two
/// stopped launches share one branch/worktree Workspace. They must survive
/// the history projection as separate child Works so each keeps its Session.
#[test]
fn app_runtime_paused_works_on_same_branch_remain_distinct_children() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);

    let mut older = history_work_view(
        "work-session-older",
        "work/shared",
        "/home/user/gwt/work/shared",
        vec![history_agent_ref_view(
            "older-session",
            Some("Codex"),
            "2026-07-12T01:00:00Z",
        )],
    );
    older.title = "Older Work".to_string();
    older.updated_at = "2026-07-12T01:00:00Z".to_string();
    older.agents[0].sessions = vec![gwt::WorkspaceHistorySessionView {
        agent_session_id: "older-conversation".to_string(),
        started_at: "2026-07-12T01:00:00Z".to_string(),
        is_active: true,
        resumable: true,
    }];

    let mut newer = history_work_view(
        "work-session-newer",
        "work/shared",
        "/home/user/gwt/work/shared",
        vec![history_agent_ref_view(
            "newer-session",
            Some("Codex"),
            "2026-07-13T01:00:00Z",
        )],
    );
    newer.title = "Newer Work".to_string();
    newer.updated_at = "2026-07-13T01:00:00Z".to_string();
    newer.agents[0].sessions = vec![gwt::WorkspaceHistorySessionView {
        agent_session_id: "newer-conversation".to_string(),
        started_at: "2026-07-13T01:00:00Z".to_string(),
        is_active: true,
        resumable: true,
    }];

    let mut view = super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![older, newer],
        None,
    );

    assert_eq!(
        view.active_works.len(),
        2,
        "branch equality groups a Workspace later; it must not erase a distinct Work"
    );

    super::assign_and_merge_workspace_groups(&mut view.active_works, &repo);
    super::attach_registry_sessions_to_active_works(
        &mut view.active_works,
        &[],
        None,
        &std::collections::HashMap::new(),
        &repo,
    );

    assert_eq!(view.active_works.len(), 1, "one branch is one Workspace");
    let workspace = &view.active_works[0];
    assert_eq!(workspace.works.len(), 2, "both launch-scoped Works remain");
    for (work_id, session_id, conversation_id) in [
        ("work-session-older", "older-session", "older-conversation"),
        ("work-session-newer", "newer-session", "newer-conversation"),
    ] {
        let child = workspace
            .works
            .iter()
            .find(|child| child.id == work_id)
            .expect("child Work");
        assert_eq!(child.agents.len(), 1);
        assert_eq!(child.agents[0].session_id, session_id);
        assert_eq!(child.agents[0].sessions.len(), 1);
        assert_eq!(
            child.agents[0].sessions[0].agent_session_id,
            conversation_id
        );
    }
}

/// SPEC-2359 FR-470/FR-471: branch/worktree identify the parent Workspace,
/// not one launch-scoped Work. A live Session must not hide an older Paused
/// Work on the same execution container when their Session identities differ.
#[test]
fn app_runtime_live_work_keeps_distinct_paused_work_on_same_branch() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let shared_worktree = "/home/user/gwt/work/shared";
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("live-session", Some("work-live"));
        agent.branch = Some("work/shared".to_string());
        agent.worktree_path = Some(std::path::PathBuf::from(shared_worktree));
        agent
    });

    let mut paused = history_work_view(
        "work-session-paused-session",
        "work/shared",
        shared_worktree,
        vec![history_agent_ref_view(
            "paused-session",
            Some("Codex"),
            "2026-07-12T01:00:00Z",
        )],
    );
    paused.title = "Paused Work".to_string();
    paused.summary = Some("Paused Work summary".to_string());
    paused.owner = Some("Issue #470".to_string());
    paused.execution_containers[0].pr_number = Some(470);
    paused.execution_containers[0].pr_url = Some("https://example.test/pr/470".to_string());
    paused.board_refs = vec!["paused-work-board-ref".to_string()];
    paused.agents[0].sessions = vec![gwt::WorkspaceHistorySessionView {
        agent_session_id: "paused-conversation".to_string(),
        started_at: "2026-07-12T01:00:00Z".to_string(),
        is_active: true,
        resumable: true,
    }];

    let mut view = super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![paused],
        None,
    );

    assert_eq!(
        view.active_works.len(),
        2,
        "same Workspace identity must not collapse different live/paused Sessions"
    );
    let live = view
        .active_works
        .iter()
        .find(|work| work.id == "work-session-live-session")
        .expect("live child Work");
    assert_eq!(live.title, "Board audience follow-up");
    assert_eq!(live.summary, None);
    assert_eq!(live.owner, None);
    assert_eq!(live.pr_number, None);
    assert!(live.board_refs.is_empty());

    super::assign_and_merge_workspace_groups(&mut view.active_works, &repo);
    super::attach_registry_sessions_to_active_works(
        &mut view.active_works,
        &[],
        None,
        &std::collections::HashMap::new(),
        &repo,
    );

    assert_eq!(view.active_works.len(), 1, "one branch is one Workspace");
    let workspace = &view.active_works[0];
    assert_eq!(workspace.works.len(), 2, "both Session-owned Works remain");
    let paused = workspace
        .works
        .iter()
        .find(|work| work.id == "work-session-paused-session")
        .expect("Paused child Work");
    assert_eq!(paused.lifecycle_state, "paused");
    assert_eq!(paused.agents.len(), 1);
    assert_eq!(paused.agents[0].session_id, "paused-session");
}

/// SPEC-2359 FR-471: a legacy history id and a live Session-derived Work id
/// still represent one resumed Work when their worktree paths are equivalent
/// filesystem paths with different lexical spellings.
#[test]
fn app_runtime_resumed_work_dedupes_equivalent_worktree_paths() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    let worktree = repo.join("work/resume");
    fs::create_dir_all(&worktree).expect("create worktree");
    let alternate_worktree_path = worktree.join("..").join("resume");
    assert_ne!(worktree, alternate_worktree_path);
    assert!(same_worktree_path(&worktree, &alternate_worktree_path));

    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-resume-alias", Some("work-live"));
        agent.branch = None;
        agent.worktree_path = Some(alternate_worktree_path);
        agent
    });

    let mut paused = history_work_view(
        "legacy-work-resume-alias",
        "",
        worktree.to_string_lossy().as_ref(),
        vec![history_agent_ref_view(
            "session-resume-alias",
            Some("Codex"),
            "2026-07-12T01:00:00Z",
        )],
    );
    paused.execution_containers[0].branch = None;
    paused.title = "Resumed history metadata".to_string();
    paused.summary = Some("Resumed history summary".to_string());
    paused.owner = Some("SPEC-2359".to_string());
    paused.execution_containers[0].pr_number = Some(2359);
    paused.execution_containers[0].pr_url = Some("https://example.test/pr/2359".to_string());
    paused.board_refs = vec!["resumed-board-ref".to_string()];

    let view = super::active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        vec![paused],
        None,
    );

    assert_eq!(
        view.active_works.len(),
        1,
        "shared Session must dedupe equivalent worktree path spellings"
    );
    assert_eq!(view.active_works[0].id, "work-session-session-resume-alias");
    assert_eq!(view.active_works[0].title, "Resumed history metadata");
    assert_eq!(
        view.active_works[0].summary.as_deref(),
        Some("Resumed history summary")
    );
    assert_eq!(view.active_works[0].owner.as_deref(), Some("SPEC-2359"));
    assert_eq!(view.active_works[0].pr_number, Some(2359));
    assert_eq!(
        view.active_works[0].board_refs,
        vec!["resumed-board-ref".to_string()]
    );
}

/// FR-348 compatibility guard: Work identity is agent-session-derived, so a
/// legacy and canonical history row for the same Session remain one Work even
/// after distinct same-Workspace Sessions stop collapsing by git identity.
#[test]
fn app_runtime_duplicate_paused_history_for_same_session_stays_one_work() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    let works = vec![
        history_work_view(
            "legacy-work-id",
            "work/shared",
            "/home/user/gwt/work/shared",
            vec![history_agent_ref_view(
                "shared-session",
                Some("Codex"),
                "2026-07-12T01:00:00Z",
            )],
        ),
        history_work_view(
            "work-session-shared-session",
            "work/shared",
            "/home/user/gwt/work/shared",
            vec![history_agent_ref_view(
                "shared-session",
                Some("Codex"),
                "2026-07-13T01:00:00Z",
            )],
        ),
    ];

    let view =
        super::active_work_projection_from_saved_with_journal(projection, Vec::new(), works, None);

    assert_eq!(
        view.active_works.len(),
        1,
        "the same launch Session must not become two Paused Works"
    );
    assert_eq!(view.active_works[0].agents[0].session_id, "shared-session");
}

/// FR-350 contract guard for the #3213 fix: a live Work synthesized without
/// git_details (no branch / worktree on the row) still dedupes the
/// session-sharing history item — the original purpose of the session-id
/// fallback in `active_work_already_present`.
#[test]
fn app_runtime_paused_work_dedups_by_session_when_live_row_has_no_git_identity() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.push({
        let mut agent = workspace_agent_summary_for_test("session-shared", Some("work-x"));
        agent.branch = None;
        agent.worktree_path = None;
        agent
    });
    // The history row carries a full git identity and the same session.
    let works = vec![history_work_view(
        "work-work-other-0000abcd",
        "work/other",
        "/home/user/gwt/work/other",
        vec![history_agent_ref_view(
            "session-shared",
            Some("Claude Code"),
            "2026-06-29T07:45:56Z",
        )],
    )];

    let view =
        super::active_work_projection_from_saved_with_journal(projection, Vec::new(), works, None);

    assert_eq!(
        view.active_works.len(),
        1,
        "identity-less live row + session-sharing history must stay one row"
    );
}

#[test]
fn app_runtime_active_work_projection_resolves_branch_known_unassigned_agents_as_work() {
    let _env_guard = env_test_lock().lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.register_unassigned_agent({
        let mut agent = workspace_agent_summary_for_test("session-unassigned", None);
        agent.window_id = Some("tab-1::agent-unassigned".to_string());
        agent.branch = Some("work/unassigned-but-known".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-unassigned");
    session.session_id = "session-unassigned".to_string();
    session.branch_name = "work/unassigned-but-known".to_string();
    runtime
        .active_agent_sessions
        .insert(session.window_id.clone(), session);

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_work_count, 1);
    assert_eq!(view.active_works.len(), 1);
    assert_eq!(view.active_works[0].agents.len(), 1);
    assert!(view.unassigned_agents.is_empty());
}

#[test]
fn app_runtime_open_active_work_launch_wizard_focuses_existing_agent_for_branch() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.branch_name = "work/test".to_string();
    session.window_id = window_id.clone();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);

    let events = runtime.open_active_work_launch_wizard("client-1", "work/test", None);

    assert!(runtime.launch_wizard.is_none());
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Broadcast,
            event: BackendEvent::WindowCanvasState { .. },
        }
    )));
}

#[test]
fn app_runtime_live_work_agent_lookup_ignores_stopped_or_error_windows() {
    for status in [WindowProcessStatus::Stopped, WindowProcessStatus::Error] {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "agent-1",
            repo,
            WindowPreset::Agent,
            status,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");
        let mut session = sample_active_agent_session("tab-1", &window_id);
        session.branch_name = "work/test".to_string();
        session.window_id = window_id.clone();
        runtime
            .active_agent_sessions
            .insert(window_id.clone(), session);

        assert_eq!(
            runtime.live_agent_window_for_work("tab-1", Some("work/test"), None),
            None,
            "{status:?} windows must not block a later launch"
        );
    }
}

#[test]
fn app_runtime_active_work_projection_promotes_branch_known_unassigned_agents_to_active_work() {
    let _env_guard = env_test_lock().lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.register_unassigned_agent({
        let mut agent = workspace_agent_summary_for_test("session-unassigned", None);
        agent.window_id = Some("tab-1::agent-unassigned".to_string());
        agent
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = sample_active_agent_session("tab-1", "tab-1::agent-unassigned");
    session.session_id = "session-unassigned".to_string();
    runtime
        .active_agent_sessions
        .insert(session.window_id.clone(), session);

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_work_count, 1);
    assert_eq!(view.active_works.len(), 1);
    assert_eq!(view.active_works[0].agents.len(), 1);
    assert!(view.unassigned_agents.is_empty());
}

#[test]
fn app_runtime_launch_failure_log_redacts_sensitive_error_values() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");

    let events = capture_tracing_events(|| {
        let _ = runtime.handle_launch_complete(
                window_id,
                Err("failed OPENAI_API_KEY=sk-test --api-key sk-other GWT_HOOK_TOKEN=hook-secret --token plain-token".to_string()),
            );
    });

    let event = events
        .iter()
        .find(|event| {
            event.level == Level::ERROR
                && event.target == "gwt::agent_launch"
                && event.fields.get("stage").map(String::as_str) == Some("launch_complete")
        })
        .expect("redacted launch completion failure log");
    let error = event.fields.get("error").expect("error field");
    assert!(!error.contains("sk-test"));
    assert!(!error.contains("sk-other"));
    assert!(!error.contains("hook-secret"));
    assert!(!error.contains("plain-token"));
    assert!(error.contains("OPENAI_API_KEY=[REDACTED]"));
    assert!(error.contains("--api-key [REDACTED]"));
    assert!(error.contains("GWT_HOOK_TOKEN=[REDACTED]"));
    assert!(error.contains("--token [REDACTED]"));
}

#[test]
fn app_runtime_runtime_status_stopped_keeps_active_agent_window_for_diagnostics() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let events = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Stopped,
        Some("Process exited".to_string()),
    );

    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::TerminalStatus { .. })),
        "PTY stop must still update the terminal status"
    );
    assert!(
        runtime.window_lookup.contains_key(&window_id),
        "PTY stop alone must keep the agent window open so diagnostics remain visible"
    );
    assert!(
        runtime.tabs[0].workspace.window("codex-1").is_some(),
        "workspace must retain the stopped agent window"
    );
    assert!(!runtime.active_agent_sessions.contains_key(&window_id));
}

// Issue #3274 (SPEC-1921 exact session restore amendment): when a resumed
// agent process exits because the provider no longer has the conversation,
// the final screen output must survive into the persistent window detail.
// The vt100 state is dropped together with the runtime on Error, so a client
// that reconnects later would otherwise face an empty Error window with no
// clue why exact session restore failed.
#[test]
fn app_runtime_agent_error_exit_promotes_exact_resume_failure_diagnostic() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(b"No conversation found with session ID: resume-target-1\r\n");

    let _ = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("Process exited with status 1".to_string()),
    );

    let detail = runtime
        .window_details
        .get(&window_id)
        .cloned()
        .unwrap_or_default();
    assert!(
        detail.contains("Exact session restore failed"),
        "exact-resume failure must be promoted to an explicit diagnostic, got: {detail}"
    );
    assert!(
        detail.contains("resume-target-1"),
        "diagnostic must carry the reported session id, got: {detail}"
    );
    assert!(
        !runtime.runtimes.contains_key(&window_id),
        "errored runtime is still torn down after the tail is captured"
    );
}

// Issue #3274: any agent error exit keeps its last screen output in the
// window detail so the failure reason survives reconnects, while non-agent
// process windows keep the plain exit detail.
#[test]
fn app_runtime_agent_error_exit_keeps_last_output_in_window_detail() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let mut persisted = empty_workspace_state();
    persisted.windows.push(sample_window(
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    ));
    persisted.windows.push(sample_window(
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Running,
    ));
    persisted.next_z_index = 3;
    let agent_tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: temp.path().join("repo"),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![agent_tab], Some("tab-1"));
    let agent_window_id = combined_window_id("tab-1", "agent-1");
    let shell_window_id = combined_window_id("tab-1", "shell-1");
    insert_test_pane_runtime(&mut runtime, &agent_window_id);
    insert_test_pane_runtime(&mut runtime, &shell_window_id);
    runtime
        .runtimes
        .get(&agent_window_id)
        .expect("agent runtime")
        .pane
        .lock()
        .expect("agent pane")
        .process_bytes(b"unexpected fatal: config parse error\r\n");
    runtime
        .runtimes
        .get(&shell_window_id)
        .expect("shell runtime")
        .pane
        .lock()
        .expect("shell pane")
        .process_bytes(b"command not found: frobnicate\r\n");

    let _ = runtime.handle_runtime_status(
        agent_window_id.clone(),
        WindowProcessStatus::Error,
        Some("Process exited with status 1".to_string()),
    );
    let _ = runtime.handle_runtime_status(
        shell_window_id.clone(),
        WindowProcessStatus::Error,
        Some("Process exited with status 1".to_string()),
    );

    let agent_detail = runtime
        .window_details
        .get(&agent_window_id)
        .cloned()
        .unwrap_or_default();
    assert!(
        agent_detail.contains("Process exited with status 1")
            && agent_detail.contains("unexpected fatal: config parse error"),
        "agent error detail must keep the last screen output, got: {agent_detail}"
    );
    assert_eq!(
        runtime
            .window_details
            .get(&shell_window_id)
            .map(String::as_str),
        Some("Process exited with status 1"),
        "non-agent windows keep the plain exit detail"
    );
}

#[test]
fn app_runtime_runtime_hook_running_recovers_active_agent_after_pty_error() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let _ = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Running",
        "PreToolUse",
        "session-1",
    ));

    let error_events = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("pty stream interrupted".to_string()),
    );

    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    assert_eq!(
        runtime.window_details.get(&window_id).map(String::as_str),
        Some("pty stream interrupted")
    );
    assert!(error_events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Error,
            ..
        }
    )));

    let recovered_events = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Running",
        "PreToolUse",
        "session-1",
    ));

    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Running)
    );
    assert!(
        !runtime.window_details.contains_key(&window_id),
        "live hook recovery clears the stale PTY error detail"
    );
    assert!(recovered_events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Running,
            ..
        }
    )));
}

#[test]
fn app_runtime_runtime_status_error_without_live_hook_stops_active_agent() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let error_events = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("process failed".to_string()),
    );

    assert!(!runtime.active_agent_sessions.contains_key(&window_id));
    assert!(error_events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Error,
            ..
        }
    )));
}

#[test]
fn app_runtime_duplicate_pty_error_after_live_hook_keeps_active_agent_for_recovery() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let _ = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Running",
        "PreToolUse",
        "session-1",
    ));

    let _ = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("transient pty error".to_string()),
    );
    assert!(runtime.active_agent_sessions.contains_key(&window_id));

    let duplicate_error_events = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("transient pty error".to_string()),
    );

    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    assert!(duplicate_error_events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Error,
            ..
        }
    )));
}

#[test]
fn app_runtime_live_hook_recovery_clears_recoverable_pty_error_marker() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let _ = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Running",
        "PreToolUse",
        "session-1",
    ));
    let _ = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("transient pty error".to_string()),
    );
    let _ = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("transient pty error".to_string()),
    );
    assert!(runtime.recoverable_agent_error_windows.contains(&window_id));

    let _ = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Running",
        "PreToolUse",
        "session-1",
    ));

    assert!(
        !runtime.recoverable_agent_error_windows.contains(&window_id),
        "live hook recovery must end the stale PTY Error duplicate window"
    );
    runtime.window_hook_states.remove(&window_id);

    let _ = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("process exited".to_string()),
    );

    assert!(!runtime.active_agent_sessions.contains_key(&window_id));
}

#[test]
fn app_runtime_active_work_projection_filters_stale_saved_agents_when_no_agent_is_live() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
    projection.status_text = "Old agent is running".to_string();
    projection
        .agents
        .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: "stale-session".to_string(),
            window_id: Some("tab-1::agent-1".to_string()),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: Some("Old focus".to_string()),
            title_summary: None,
            worktree_path: None,
            branch: Some("work/old".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at: chrono::Utc::now(),
        });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save stale projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_agents, 0);
    assert_eq!(view.blocked_agents, 0);
    assert!(view.agents.is_empty());
    assert_eq!(view.status_category, "idle");
    assert_eq!(view.status_text, "No active work");
}

#[test]
fn app_runtime_active_work_projection_resets_stale_current_identity_when_no_agent_is_live() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.title = "PR-2525".to_string();
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
    projection.status_text = "Old PR is active".to_string();
    projection.summary = Some("Old PR summary".to_string());
    projection.owner = Some("PR-2525".to_string());
    projection.next_action = Some("Review old PR".to_string());
    projection.board_refs = vec!["board-old".to_string()];
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some("work/old".to_string()),
        worktree_path: Some(repo.join("work-old")),
        base_branch: Some("origin/develop".to_string()),
        pr_number: Some(2525),
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    projection
        .agents
        .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: "stale-session".to_string(),
            window_id: Some("tab-1::agent-1".to_string()),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: Some("Old focus".to_string()),
            title_summary: Some("Old title".to_string()),
            worktree_path: None,
            branch: Some("work/old".to_string()),
            last_board_entry_id: Some("board-old".to_string()),
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at: chrono::Utc::now(),
        });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save stale projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.title, "Repo Work");
    assert_eq!(view.status_category, "idle");
    assert_eq!(view.status_text, "No active work");
    assert_eq!(view.summary, None);
    assert_eq!(view.owner, None);
    assert_eq!(view.next_action, None);
    assert_eq!(view.branch, None);
    assert_eq!(view.worktree_path, None);
    assert_eq!(view.pr_number, None);
    assert!(view.board_refs.is_empty());
}

#[test]
fn app_runtime_active_work_projection_filters_stale_agent_when_window_id_is_reused() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let window_id = "tab-1::agent-1";
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
    projection.status_text = "Old agent is running".to_string();
    projection
        .agents
        .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: "stale-session".to_string(),
            window_id: Some(window_id.to_string()),
            agent_id: "codex".to_string(),
            display_name: "Old Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: Some("Old focus".to_string()),
            title_summary: Some("Old title".to_string()),
            worktree_path: None,
            branch: Some("work/old".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at: chrono::Utc::now(),
        });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save stale projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.active_agent_sessions.insert(
        window_id.to_string(),
        ActiveAgentSession {
            window_id: window_id.to_string(),
            session_id: "live-session".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/live".to_string(),
            display_name: "Live Codex".to_string(),
            worktree_path: repo.join("../repo-work-live"),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_agents, 1);
    assert_eq!(view.blocked_agents, 0);
    assert_eq!(view.agents.len(), 1);
    assert_eq!(view.agents[0].session_id, "live-session");
    assert_eq!(view.agents[0].display_name, "Live Codex");
    assert!(!view
        .agents
        .iter()
        .any(|agent| agent.session_id == "stale-session"));
}

#[test]
fn app_runtime_active_work_projection_includes_recent_workspace_journal_entries() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    gwt_core::workspace_projection::update_workspace_projection_with_journal(
        &repo,
        gwt_core::workspace_projection::WorkspaceProjectionUpdate {
            title: Some("Work".to_string()),
            status_category: Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Idle),
            status_text: Some("Ready for review".to_string()),
            owner: Some("SPEC-2359".to_string()),
            next_action: Some("Review summary".to_string()),
            summary: Some("Overview summary is persisted.".to_string()),
            progress_summary: None,
            agent_session_id: None,
            agent_current_focus: None,
            agent_title_summary: None,
        },
    )
    .expect("workspace update");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(
        view.summary.as_deref(),
        Some("Overview summary is persisted.")
    );
    assert_eq!(view.journal_entries.len(), 1);
    assert_eq!(
        view.journal_entries[0].summary.as_deref(),
        Some("Overview summary is persisted.")
    );
    assert_eq!(
        view.journal_entries[0].next_action.as_deref(),
        Some("Review summary")
    );
}

#[test]
fn app_runtime_resume_workspace_journal_reuses_existing_branch_as_execution_container() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let branch = "work/20260507-0001";
    run_git(&repo, &["branch", branch]);
    gwt_core::workspace_projection::save_workspace_projection(
        &repo,
        &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
    )
    .expect("save projection");
    append_workspace_resume_journal(
        &repo,
        "journal-reuse",
        temp.path().join("work").join("20260507-0001"),
        "SPEC-2359",
        "Resume the suspended Work card.",
    );
    assert_eq!(
        super::workspace_resume_branch_from_journal_project_root(
            &temp.path().join("work").join("20260507-0001"),
            &repo
        )
        .as_deref(),
        Some(branch)
    );
    assert!(super::workspace_resume_branch_exists(&repo, branch));
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspace {
            source: gwt::WorkspaceResumeSource::Journal,
            journal_id: Some("journal-reuse".to_string()),
        },
    );

    let session = runtime.launch_wizard.as_ref().expect("launch wizard");
    let view = session.wizard.view();
    assert_eq!(view.title, "Launch Agent");
    assert_eq!(view.branch_name, branch);
    let context = session
        .workspace_resume_context
        .as_ref()
        .expect("workspace resume context");
    assert_eq!(context.owner.as_deref(), Some("SPEC-2359"));
    assert_eq!(
        context.summary.as_deref(),
        Some("Resume the suspended Work card.")
    );
}

// SPEC-2359 US-42 — Workspace Resume Picker tests.

fn write_resumable_session_for_test(
    sessions_dir: &Path,
    session_id: &str,
    repo: &Path,
    branch: &str,
    agent_id: gwt_agent::AgentId,
    agent_session_id: Option<&str>,
) {
    let mut session = gwt_agent::Session::new(repo, branch, agent_id);
    session.id = session_id.to_string();
    session.display_name = "Codex".to_string();
    session.tool_version = Some("installed".to_string());
    session.agent_session_id = agent_session_id.map(str::to_string);
    std::fs::create_dir_all(sessions_dir).expect("sessions dir");
    session.save(sessions_dir).expect("session toml");
}

fn projection_with_assigned_agent(
    repo: &Path,
    session_id: &str,
) -> gwt_core::workspace_projection::WorkspaceProjection {
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(repo);
    projection.title = "Work with Resume candidate".to_string();
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
    projection
        .agents
        .push(workspace_agent_summary_for_test(session_id, None));
    projection
}

#[test]
fn app_runtime_list_resumable_agents_returns_assigned_with_session_toml() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let session_id = "session-resumable-1";
    let projection = projection_with_assigned_agent(&repo, session_id);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let sessions_dir = temp.path().join("sessions");
    write_resumable_session_for_test(
        &sessions_dir,
        session_id,
        &repo,
        "work/test",
        gwt_agent::AgentId::Codex,
        Some("prior-codex-uuid"),
    );

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ListResumableAgents { workspace_id: None },
    );

    let event = events
        .first()
        .expect("backend should respond to ListResumableAgents");
    assert!(matches!(
        &event.target,
        DispatchTarget::Client(client_id) if client_id == "client-1"
    ));
    match &event.event {
        BackendEvent::WorkspaceResumableAgents { agents, .. } => {
            assert_eq!(agents.len(), 1, "single assigned agent must surface");
            assert_eq!(agents[0].session_id, session_id);
            assert!(matches!(
                agents[0].resume_kind,
                gwt::ResumableAgentResumeKind::Session
            ));
        }
        other => panic!("unexpected backend event: {other:?}"),
    }
}

#[test]
fn app_runtime_list_resumable_agents_includes_unassigned_agents_with_session_toml() {
    // SPEC-2359 US-42 follow-up: production projections often store
    // agents with `affiliation_status = unassigned` (no explicit
    // `workspace join` step). Resume Picker must still offer them as
    // candidates when a Session toml is on disk; otherwise users see
    // "No resumable agents" for every Workspace they did not
    // manually ensure / join.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let session_id = "session-unassigned-1";
    let mut projection = projection_with_assigned_agent(&repo, session_id);
    projection.agents[0].affiliation_status =
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned;
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let sessions_dir = temp.path().join("sessions");
    write_resumable_session_for_test(
        &sessions_dir,
        session_id,
        &repo,
        "work/test",
        gwt_agent::AgentId::Codex,
        Some("agent-session-uuid"),
    );

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ListResumableAgents { workspace_id: None },
    );

    match events.first().map(|outbound| &outbound.event) {
        Some(BackendEvent::WorkspaceResumableAgents { agents, .. }) => {
            assert_eq!(
                agents.len(),
                1,
                "Unassigned agent with a backing Session toml must still surface",
            );
            assert_eq!(agents[0].session_id, session_id);
        }
        other => panic!("unexpected backend event: {other:?}"),
    }
}

#[test]
fn app_runtime_list_resumable_agents_includes_live_session_as_running() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let session_id = "session-live-1";
    let projection = projection_with_assigned_agent(&repo, session_id);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let sessions_dir = temp.path().join("sessions");
    write_resumable_session_for_test(
        &sessions_dir,
        session_id,
        &repo,
        "work/test",
        gwt_agent::AgentId::Codex,
        Some("agent-session-uuid"),
    );

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut live = sample_active_agent_session("tab-1", "window-1");
    live.session_id = session_id.to_string();
    runtime
        .active_agent_sessions
        .insert("window-1".to_string(), live);

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ListResumableAgents { workspace_id: None },
    );

    match events.first().map(|outbound| &outbound.event) {
        Some(BackendEvent::WorkspaceResumableAgents { agents, .. }) => {
            assert_eq!(
                agents.len(),
                1,
                "live session should appear with Running status"
            );
            assert_eq!(
                agents[0].lifecycle_status,
                Some(gwt::ResumableAgentLifecycleStatus::Running),
            );
            assert_eq!(
                agents[0].resume_kind,
                gwt::ResumableAgentResumeKind::Session
            );
        }
        other => panic!("unexpected backend event: {other:?}"),
    }
}

#[test]
fn app_runtime_list_resumable_agents_marks_idless_codex_as_native_picker() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let session_id = "session-native-picker-1";
    let projection = projection_with_assigned_agent(&repo, session_id);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let sessions_dir = temp.path().join("sessions");
    write_resumable_session_for_test(
        &sessions_dir,
        session_id,
        &repo,
        "work/test",
        gwt_agent::AgentId::Codex,
        None,
    );

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ListResumableAgents { workspace_id: None },
    );

    match events.first().map(|outbound| &outbound.event) {
        Some(BackendEvent::WorkspaceResumableAgents { agents, .. }) => {
            assert_eq!(agents.len(), 1);
            assert_eq!(
                agents[0].resume_kind,
                gwt::ResumableAgentResumeKind::NativePicker,
                "Codex without an exact id should open the provider-native resume picker"
            );
        }
        other => panic!("unexpected backend event: {other:?}"),
    }
}

#[test]
fn app_runtime_list_resumable_agents_uses_workspace_branch_ledger_candidates() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let branch = "work/ledger-resume";
    run_git(&repo, &["branch", branch]);

    let projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection without raw agents");

    let work_id = gwt_core::workspace_projection::canonical_work_id(&repo, Some(branch), None)
        .expect("canonical work id");
    let updated_at = Utc.with_ymd_and_hms(2026, 6, 17, 9, 0, 0).unwrap();
    let work_item = gwt_core::workspace_projection::WorkItem {
        id: work_id.clone(),
        title: branch.to_string(),
        intent: None,
        summary: None,
        progress_summary: None,
        status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Idle,
        owner: None,
        created_at: updated_at,
        updated_at,
        completed_at: None,
        agents: Vec::new(),
        execution_containers: vec![
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some(branch.to_string()),
                worktree_path: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        ],
        board_refs: Vec::new(),
        related_work_item_ids: Vec::new(),
        events: Vec::new(),
        legacy_metadata_snapshot: None,
        legacy_metadata_authoritative: false,
        legacy_metadata_snapshot_at: None,
        duplicate_event_containers: Default::default(),
        discarded: false,
        discarded_at: None,
    };
    let work_items = gwt_core::workspace_projection::WorkItemsProjection {
        updated_at,
        work_items: vec![work_item],
    };
    let work_items_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
    fs::create_dir_all(work_items_path.parent().expect("work items parent"))
        .expect("create work items parent");
    gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
        &work_items_path,
        &work_items,
    )
    .expect("save work items projection");

    let sessions_dir = temp.path().join("sessions");
    write_resumable_session_for_test(
        &sessions_dir,
        "session-ledger-codex",
        &repo,
        branch,
        gwt_agent::AgentId::Codex,
        Some("codex-thread-ledger"),
    );

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ListResumableAgents {
            workspace_id: Some(work_id),
        },
    );

    match events.first().map(|outbound| &outbound.event) {
        Some(BackendEvent::WorkspaceResumableAgents { agents, .. }) => {
            assert_eq!(
                agents.len(),
                1,
                "Resume picker must use the same branch ledger candidates as Workspace detail",
            );
            assert_eq!(agents[0].session_id, "session-ledger-codex");
            assert_eq!(agents[0].display_name, "Codex");
            assert_eq!(
                agents[0].resume_kind,
                gwt::ResumableAgentResumeKind::Session
            );
        }
        other => panic!("unexpected backend event: {other:?}"),
    }
}

#[test]
fn app_runtime_resume_workspace_agent_replies_error_when_session_toml_missing() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspaceAgent {
            session_id: "missing-session".to_string(),
            agent_session_id: None,
            bounds: canvas_bounds(),
        },
    );

    let event = events
        .first()
        .expect("ResumeWorkspaceAgent must reply on missing session");
    assert!(matches!(
        &event.target,
        DispatchTarget::Client(client_id) if client_id == "client-1"
    ));
    assert!(matches!(
        &event.event,
        BackendEvent::WorkspaceResumeAgentError { session_id, message }
            if session_id == "missing-session" && !message.is_empty()
    ));
}

#[test]
fn app_runtime_resume_workspace_agent_ignores_stopped_same_session_window() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo,
        WindowPreset::Agent,
        WindowProcessStatus::Stopped,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.session_id = "stopped-session".to_string();
    session.window_id = window_id.clone();
    runtime.active_agent_sessions.insert(window_id, session);

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspaceAgent {
            session_id: "stopped-session".to_string(),
            agent_session_id: None,
            bounds: canvas_bounds(),
        },
    );

    let event = events
        .first()
        .expect("ResumeWorkspaceAgent should proceed past stopped window");
    assert!(matches!(
        &event.event,
        BackendEvent::WorkspaceResumeAgentError { session_id, message }
            if session_id == "stopped-session" && !message.is_empty()
    ));
}

// SPEC-2359 US-79: a Session whose worktree was removed on this machine can
// still be resumed when the branch is available locally/remotely. The launch
// path must be allowed to materialize the worktree again.
#[test]
fn app_runtime_resume_workspace_agent_materializes_missing_worktree_when_branch_exists() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let branch = "feature/ghost";
    run_git(&repo, &["branch", branch]);
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let ghost_worktree = temp.path().join("ghost-worktree");
    let mut session = gwt_agent::Session::new(&ghost_worktree, branch, gwt_agent::AgentId::Codex);
    session.id = "session-ghost".to_string();
    session.agent_session_id = Some("conv-ghost".to_string());
    session.save(&runtime.sessions_dir).expect("save session");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspaceAgent {
            session_id: "session-ghost".to_string(),
            agent_session_id: None,
            bounds: canvas_bounds(),
        },
    );

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::WorkspaceResumeAgentStarted { session_id, branch: Some(started_branch) }
                if session_id == "session-ghost" && started_branch == branch
        )),
        "branch-materializable missing worktree should enter launch materialization and ack"
    );
    assert!(
        events.iter().all(|event| !matches!(
            &event.event,
            BackendEvent::WorkspaceResumeAgentError { message, .. }
                if message.contains("Worktree path not found")
        )),
        "missing worktree alone must not be a synchronous resume error"
    );
}

#[test]
fn app_runtime_list_resumable_agents_filters_session_when_worktree_and_branch_missing() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let session_id = "session-branch-gone";
    let projection = projection_with_assigned_agent(&repo, session_id);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let sessions_dir = temp.path().join("sessions");
    write_resumable_session_for_test(
        &sessions_dir,
        session_id,
        &temp.path().join("deleted-worktree"),
        "feature/deleted",
        gwt_agent::AgentId::Codex,
        Some("conv-deleted"),
    );

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ListResumableAgents { workspace_id: None },
    );

    match events.first().map(|outbound| &outbound.event) {
        Some(BackendEvent::WorkspaceResumableAgents { agents, .. }) => {
            assert!(
                agents.is_empty(),
                "exact Session Resume candidates require an existing worktree or materializable branch"
            );
        }
        other => panic!("unexpected backend event: {other:?}"),
    }
}

// SPEC-2359 D1: requesting a *specific* past conversation while a live window
// is running a *different* conversation must surface a visible error, not
// silently focus the live window (which would drop the requested resume).
#[test]
fn app_runtime_resume_workspace_agent_errors_when_live_window_runs_other_conversation() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut active = sample_active_agent_session("tab-1", &window_id);
    active.session_id = "work-live".to_string();
    active.window_id = window_id.clone();
    runtime.active_agent_sessions.insert(window_id, active);

    // The live window is running "conv-current"; the user clicked Resume on
    // an older conversation ("conv-old").
    let mut session = gwt_agent::Session::new(&repo, "feature/live", gwt_agent::AgentId::Codex);
    session.id = "work-live".to_string();
    session.agent_session_id = Some("conv-current".to_string());
    session.save(&runtime.sessions_dir).expect("save session");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspaceAgent {
            session_id: "work-live".to_string(),
            agent_session_id: Some("conv-old".to_string()),
            bounds: canvas_bounds(),
        },
    );

    let event = events
        .first()
        .expect("resume must reply on conversation conflict");
    assert!(matches!(
        &event.event,
        BackendEvent::WorkspaceResumeAgentError { session_id, message }
            if session_id == "work-live" && message.contains("different conversation")
    ));
}

#[test]
fn app_runtime_latest_branch_resume_picks_newest_resumable_session() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("sessions dir");

    let mut older = gwt_agent::Session::new(&repo, "work/manual-resume", gwt_agent::AgentId::Codex);
    older.id = "session-older".to_string();
    older.agent_session_id = Some("native-older".to_string());
    older.last_activity_at = Utc.with_ymd_and_hms(2026, 5, 21, 9, 0, 0).unwrap();
    older.updated_at = older.last_activity_at;
    older.created_at = older.last_activity_at;
    older.save(&sessions_dir).expect("save older session");

    let mut newer = gwt_agent::Session::new(&repo, "work/manual-resume", gwt_agent::AgentId::Codex);
    newer.id = "session-newer".to_string();
    newer.agent_session_id = Some("native-newer".to_string());
    newer.last_activity_at = Utc.with_ymd_and_hms(2026, 5, 21, 10, 0, 0).unwrap();
    newer.updated_at = newer.last_activity_at;
    newer.created_at = newer.last_activity_at;
    newer.save(&sessions_dir).expect("save newer session");

    let mut metadata_only =
        gwt_agent::Session::new(&repo, "work/manual-resume", gwt_agent::AgentId::Codex);
    metadata_only.id = "session-metadata-only".to_string();
    metadata_only.agent_session_id = None;
    metadata_only.last_activity_at = Utc.with_ymd_and_hms(2026, 5, 21, 11, 0, 0).unwrap();
    metadata_only.updated_at = metadata_only.last_activity_at;
    metadata_only.created_at = metadata_only.last_activity_at;
    metadata_only
        .save(&sessions_dir)
        .expect("save metadata-only session");

    let runtime = sample_runtime(temp.path(), Vec::new(), None);

    let selected = runtime
        .latest_resumable_branch_session(&repo, "work/manual-resume")
        .expect("latest resumable session");

    assert_eq!(selected.id, "session-newer");
    assert_eq!(selected.agent_session_id.as_deref(), Some("native-newer"));
}

#[test]
fn app_runtime_latest_branch_resume_reflects_sessions_refreshed_after_cache_load() {
    // #2995 regression: the managed hook CLI persists a session's real
    // agent_session_id out-of-process *after* the GUI loaded its in-memory
    // session cache. The branch load's disk-fresh refresh
    // (apply_refreshed_launch_wizard_sessions, dispatched off-thread) must
    // make such a session resumable without a full process restart — the
    // gwt daemon/tray process otherwise keeps the stale cache alive.
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("sessions dir");

    // Runtime constructed first, so its in-memory cache starts empty.
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    assert!(
        runtime
            .latest_resumable_branch_session(&repo, "work/late-write")
            .is_none(),
        "no session exists yet"
    );

    // Session TOML appears on disk afterwards (hook CLI writing the native
    // agent_session_id post-launch). The stale cache still cannot see it.
    let mut session = gwt_agent::Session::new(&repo, "work/late-write", gwt_agent::AgentId::Codex);
    session.id = "session-late".to_string();
    session.agent_session_id = Some("native-late".to_string());
    session.save(&sessions_dir).expect("save late session");
    assert!(
        runtime
            .latest_resumable_branch_session(&repo, "work/late-write")
            .is_none(),
        "stale cache must not yet see the late on-disk session"
    );

    // The off-thread branch load refreshes the cache from disk (no main
    // thread session-dir scan); resolution now finds the session.
    runtime
        .apply_refreshed_launch_wizard_sessions(gwt::launch_wizard::load_sessions(&sessions_dir));
    let selected = runtime
        .latest_resumable_branch_session(&repo, "work/late-write")
        .expect("disk-fresh refresh makes the late session resumable");
    assert_eq!(selected.id, "session-late");
    assert_eq!(selected.agent_session_id.as_deref(), Some("native-late"));
}

#[test]
fn app_runtime_open_launch_wizard_shows_only_latest_resume_and_focus_methods() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("sessions dir");

    for (session_id, native_session_id, hour) in [
        ("session-older", "native-older", 9),
        ("session-newer", "native-newer", 10),
    ] {
        let mut session =
            gwt_agent::Session::new(&repo, "work/manual-resume", gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.agent_session_id = Some(native_session_id.to_string());
        session.last_activity_at = Utc.with_ymd_and_hms(2026, 5, 21, hour, 0, 0).unwrap();
        session.updated_at = session.last_activity_at;
        session.created_at = session.last_activity_at;
        session.save(&sessions_dir).expect("save session");
    }
    for (session_id, hour) in [("session-live-older", 11), ("session-live-newer", 12)] {
        let mut session =
            gwt_agent::Session::new(&repo, "work/manual-resume", gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.agent_session_id = None;
        session.last_activity_at = Utc.with_ymd_and_hms(2026, 5, 21, hour, 0, 0).unwrap();
        session.updated_at = session.last_activity_at;
        session.created_at = session.last_activity_at;
        session
            .save(&sessions_dir)
            .expect("save live session metadata");
    }

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "branches-1",
        repo,
        WindowPreset::Branches,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "branches-1");
    for (window, session, display) in [
        ("agent-older", "session-live-older", "Codex Older"),
        ("agent-newer", "session-live-newer", "Codex Newer"),
    ] {
        let agent_window_id = combined_window_id("tab-1", window);
        runtime.active_agent_sessions.insert(
            agent_window_id.clone(),
            ActiveAgentSession {
                window_id: agent_window_id,
                session_id: session.to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/manual-resume".to_string(),
                display_name: display.to_string(),
                worktree_path: PathBuf::from("/tmp/repo"),
                agent_project_root: "/tmp/repo".to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );
    }

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::OpenLaunchWizard {
            id: window_id,
            branch_name: "work/manual-resume".to_string(),
            linked_issue_number: None,
        },
    );

    let view = runtime
        .launch_wizard
        .as_ref()
        .expect("launch wizard")
        .wizard
        .view();
    assert_eq!(
        view.quick_start_entries
            .iter()
            .map(|entry| entry.resume_session_id.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("native-newer")]
    );
    let continue_method = view
        .start_methods
        .iter()
        .find(|method| method.kind == "continue_last_session")
        .expect("continue method");
    assert!(
        continue_method
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("native-newer"),
        "continue method should describe the latest resumable session"
    );
    let focus_method = view
        .start_methods
        .iter()
        .find(|method| method.kind == "focus_running_session")
        .expect("focus method");
    assert!(
        focus_method.summary.contains("Codex Newer"),
        "focus method should target the latest live session"
    );
}

#[test]
fn app_runtime_resume_branch_latest_returns_branch_error_when_no_resumable_session_exists() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "branches-1",
        repo,
        WindowPreset::Branches,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "branches-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeBranchLatestAgent {
            id: window_id.clone(),
            branch_name: "work/manual-resume".to_string(),
            bounds: canvas_bounds(),
        },
    );

    assert!(matches!(
        events.first().map(|event| &event.event),
        Some(BackendEvent::BranchError { id, message })
            if id == &window_id && message.contains("No resumable session")
    ));
}

#[test]
fn app_runtime_bootstrap_auto_resumes_clean_waiting_input_session() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("auto-resume");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/auto-resume",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab(
        "tab-auto",
        "Auto Resume",
        worktree.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-auto"));
    for (session_id, native_session_id) in [
        ("session-auto-one", "native-session-one"),
        ("session-auto-two", "native-session-two"),
    ] {
        let mut session =
            gwt_agent::Session::new(&worktree, "work/auto-resume", gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.agent_session_id = Some(native_session_id.to_string());
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");
    }

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    assert_eq!(
        runtime.tabs.len(),
        1,
        "bootstrap should reopen the worktree tab"
    );
    assert!(same_worktree_path(&runtime.tabs[0].project_root, &worktree));
    let agent_windows = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
        agent_windows, 2,
        "all exact-resumable sessions for a worktree should restart"
    );
}

#[test]
fn app_runtime_bootstrap_resumes_session_in_linked_worktree_of_workspace_home_tab() {
    // Issue #2942 root cause: the open tab's project_root is the gwt
    // workspace home / main repo, while a resumable agent session lives in
    // a *linked worktree*. `repo_hash` / `project_scope_hash` differ between
    // the two, so scope-hash matching failed and the session never resumed
    // on startup. It must match via the shared main worktree root instead.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("linked-resume");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/linked-resume",
            worktree.to_str().expect("worktree path"),
        ],
    );
    // Tab project_root is the workspace home / main repo, NOT the worktree.
    let tab = sample_project_tab("tab-home", "Home", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-home"));
    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/linked-resume",
        gwt_agent::AgentId::ClaudeCode,
    );
    session.id = "sess-linked".to_string();
    session.agent_session_id = Some("native-linked".to_string());
    // A non-matching persisted repo_hash guarantees the scope-hash fallback
    // cannot match; only the main-worktree-root association can.
    session.repo_hash = Some("zz-nonmatching-scope-hash".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save resumable session");

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let agent_windows = runtime
        .tab("tab-home")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
        agent_windows, 1,
        "a session in a linked worktree must resume into the workspace-home tab"
    );
}

#[test]
fn app_runtime_bootstrap_resumes_unclosed_window_despite_stopped_status_and_age() {
    // Issue #2942: a session whose status drifted to Stopped (idle timeout)
    // AND is older than the 24h freshness window must STILL resume on
    // startup when its agent window is still present in the workspace (the
    // user did not explicitly close it). Both the status-candidate gate and
    // the freshness gate would exclude this session on the orphan path; only
    // the "unclosed placeholder" path can restore it.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("unclosed-resume");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/unclosed-resume",
            worktree.to_str().expect("worktree path"),
        ],
    );

    // Tab still holds the paused agent placeholder (not closed by the user).
    let mut persisted = empty_workspace_state();
    let mut agent_window =
        sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Stopped);
    agent_window.agent_id = Some("claude".to_string());
    agent_window.session_id = Some("sess-unclosed".to_string());
    persisted.windows.push(agent_window);
    persisted.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-unclosed".to_string(),
        title: "Unclosed".to_string(),
        project_root: worktree.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-unclosed"));

    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/unclosed-resume",
        gwt_agent::AgentId::ClaudeCode,
    );
    session.id = "sess-unclosed".to_string();
    session.agent_session_id = Some("native-unclosed".to_string());
    session.record_hook_event("Stop");
    session.record_completed_stop();
    // Status drifted to Stopped (would fail the candidate gate)...
    session.update_status(gwt_agent::AgentStatus::Stopped);
    // ...and the session is older than the 24h freshness window.
    session.last_activity_at = chrono::Utc::now() - chrono::Duration::hours(30);
    session
        .save(&runtime.sessions_dir)
        .expect("save stale stopped session");

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let agent_windows = runtime
        .tab("tab-unclosed")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
        agent_windows, 1,
        "an unclosed agent window must resume despite Stopped status and >24h age"
    );
    assert_eq!(
        runtime.pending_auto_resume_sources.len(),
        1,
        "the resumed unclosed window must track its source session"
    );
}

#[test]
fn app_runtime_bootstrap_queues_startup_auto_resume_until_canvas_ready() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("queued-auto-resume");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/queued-auto-resume",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab(
        "tab-auto",
        "Auto Resume",
        worktree.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-auto"));
    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/queued-auto-resume",
        gwt_agent::AgentId::Codex,
    );
    session.id = "session-queued-auto".to_string();
    session.agent_session_id = Some("native-queued-auto".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save resumable session");

    runtime.bootstrap();

    assert!(
        runtime.tabs[0]
            .workspace
            .persisted()
            .windows
            .iter()
            .all(|window| window.preset != WindowPreset::Agent),
        "bootstrap should wait for the frontend canvas bounds before placing restored windows"
    );

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let agent_windows = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(agent_windows, 1);
    assert_eq!(runtime.pending_auto_resume_sources.len(), 1);
}

#[test]
fn open_project_restore_resumes_paused_agent_even_after_stopped_drift() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("open-project-resume");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/open-project-resume",
            worktree.to_str().expect("worktree path"),
        ],
    );

    // A paused (Stopped) agent placeholder persisted in the workspace means
    // the user never closed it (closing removes it from the list).
    let mut persisted = empty_workspace_state();
    let mut agent_window =
        sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Stopped);
    agent_window.agent_id = Some("claude".to_string());
    agent_window.session_id = Some("session-open-resume".to_string());
    persisted.windows.push(agent_window);
    persisted.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-open".to_string(),
        title: "Open Resume".to_string(),
        project_root: worktree.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-open"));

    // The backing session drifted to `Stopped` via idle timeout but was
    // never explicitly closed; it must still be restored (Issue #2942).
    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/open-project-resume",
        gwt_agent::AgentId::ClaudeCode,
    );
    session.id = "session-open-resume".to_string();
    session.agent_session_id = Some("native-open-resume".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session.update_status(gwt_agent::AgentStatus::Stopped);
    session
        .save(&runtime.sessions_dir)
        .expect("save resumable session");

    let events = runtime.restore_open_project_windows("tab-open");

    assert!(
        !events.is_empty(),
        "Open Project restore should spawn the resumable agent window"
    );
    let agent_windows = runtime
        .tab("tab-open")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
        agent_windows, 1,
        "the paused placeholder should be replaced by one live agent window"
    );
    assert_eq!(
        runtime.pending_auto_resume_sources.len(),
        1,
        "the source session must be tracked so it is retired on launch complete"
    );
    assert!(runtime
        .pending_auto_resume_sources
        .values()
        .any(|source| source == "session-open-resume"));
}

// SPEC-1921 Phase 65 (T335): restored Agent-family windows with exact
// provider session ids are startup auto-resumed and their stopped
// placeholders are removed across the legacy `Agent`, `Claude`, and `Codex`
// presets — the removal must not be limited to `WindowPreset::Agent`.
#[test]
fn app_runtime_startup_auto_resume_removes_stale_placeholders_across_agent_family_presets() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("family-restore");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/family-restore",
            worktree.to_str().expect("worktree path"),
        ],
    );

    let mut persisted = empty_workspace_state();
    let legacy_window = sample_window(
        "agent-legacy",
        WindowPreset::Agent,
        WindowProcessStatus::Stopped,
    );
    let mut claude_window = sample_window(
        "agent-claude",
        WindowPreset::Claude,
        WindowProcessStatus::Stopped,
    );
    claude_window.agent_id = Some("claude".to_string());
    let mut codex_window = sample_window(
        "agent-codex",
        WindowPreset::Codex,
        WindowProcessStatus::Stopped,
    );
    codex_window.agent_id = Some("codex".to_string());
    let mut legacy_window = legacy_window;
    legacy_window.session_id = Some("session-family-legacy".to_string());
    claude_window.session_id = Some("session-family-claude".to_string());
    codex_window.session_id = Some("session-family-codex".to_string());
    persisted.windows.push(legacy_window);
    persisted.windows.push(claude_window);
    persisted.windows.push(codex_window);
    persisted.next_z_index = 4;
    let tab = ProjectTabRuntime {
        id: "tab-family".to_string(),
        title: "Family Restore".to_string(),
        project_root: worktree.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-family"));

    for (session_id, native_id, agent_id) in [
        (
            "session-family-legacy",
            "native-family-legacy",
            gwt_agent::AgentId::ClaudeCode,
        ),
        (
            "session-family-claude",
            "native-family-claude",
            gwt_agent::AgentId::ClaudeCode,
        ),
        (
            "session-family-codex",
            "native-family-codex",
            gwt_agent::AgentId::Codex,
        ),
    ] {
        let mut session = gwt_agent::Session::new(&worktree, "work/family-restore", agent_id);
        session.id = session_id.to_string();
        session.agent_session_id = Some(native_id.to_string());
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");
    }

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let agent_windows = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| crate::runtime_support::window_is_agent_pane(window))
        .count();
    assert_eq!(
        agent_windows, 3,
        "each Agent-family placeholder must be replaced by exactly one resumed window; \
         a stale Claude/Codex placeholder must not survive next to its resumed window"
    );
    assert_eq!(runtime.pending_auto_resume_sources.len(), 3);
    for source in [
        "session-family-legacy",
        "session-family-claude",
        "session-family-codex",
    ] {
        assert!(
            runtime
                .pending_auto_resume_sources
                .values()
                .any(|value| value == source),
            "resumed window must track source session {source}"
        );
    }
}

// SPEC-1921 Phase 65 (T336): a restored Agent-family window whose persisted
// session has no exact provider session id must stay a stopped placeholder
// with an explicit "exact session restore is unavailable" diagnostic — and
// must never fall back to Continue / latest / new-session launches.
#[test]
fn app_runtime_startup_auto_resume_without_exact_id_keeps_placeholder_with_diagnostic() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("no-exact-id");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/no-exact-id",
            worktree.to_str().expect("worktree path"),
        ],
    );

    let mut persisted = empty_workspace_state();
    let mut codex_window = sample_window(
        "agent-noid",
        WindowPreset::Codex,
        WindowProcessStatus::Stopped,
    );
    codex_window.agent_id = Some("codex".to_string());
    codex_window.session_id = Some("session-no-exact-id".to_string());
    persisted.windows.push(codex_window);
    persisted.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-noid".to_string(),
        title: "No Exact Id".to_string(),
        project_root: worktree.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-noid"));

    // The Codex placeholder session id is structurally unusable for exact
    // resume, so this session has no exact provider id.
    let mut session =
        gwt_agent::Session::new(&worktree, "work/no-exact-id", gwt_agent::AgentId::Codex);
    session.id = "session-no-exact-id".to_string();
    session.agent_session_id = Some("agent-session".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save session without exact id");

    runtime.seed_restored_window_details();
    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let windows = runtime.tabs[0].workspace.persisted().windows.clone();
    assert_eq!(windows.len(), 1, "no fallback window may be spawned");
    assert_eq!(windows[0].id, "agent-noid");
    assert_eq!(windows[0].status, WindowProcessStatus::Stopped);
    assert!(
        runtime.pending_auto_resume_sources.is_empty(),
        "a session without an exact provider id must not auto-resume"
    );
    assert!(
        runtime.runtimes.is_empty(),
        "no Continue/latest/new-session process may be launched as a fallback"
    );

    let detail = runtime
        .window_details
        .get(&combined_window_id("tab-noid", "agent-noid"))
        .cloned()
        .unwrap_or_default();
    assert!(
        detail.contains("Exact session restore is unavailable"),
        "placeholder must explain exact session restore is unavailable, got: {detail}"
    );
    assert!(
        !detail.contains("Restored window is paused"),
        "the generic paused message must be replaced by the exact-restore diagnostic"
    );
}

// SPEC-1921 Phase 65 (T336/T337): an exact auto-resume candidate must not be
// labeled with the generic paused-placeholder detail (it resumes as soon as
// the canvas is ready), while non-agent process windows keep the generic
// message.
#[test]
fn app_runtime_startup_auto_resume_candidate_skips_generic_paused_detail() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("candidate-detail");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/candidate-detail",
            worktree.to_str().expect("worktree path"),
        ],
    );

    let mut persisted = empty_workspace_state();
    let mut claude_window = sample_window(
        "agent-candidate",
        WindowPreset::Claude,
        WindowProcessStatus::Stopped,
    );
    claude_window.agent_id = Some("claude".to_string());
    claude_window.session_id = Some("session-candidate-detail".to_string());
    persisted.windows.push(claude_window);
    persisted.windows.push(sample_window(
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Stopped,
    ));
    persisted.next_z_index = 3;
    let tab = ProjectTabRuntime {
        id: "tab-candidate".to_string(),
        title: "Candidate Detail".to_string(),
        project_root: worktree.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-candidate"));

    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/candidate-detail",
        gwt_agent::AgentId::ClaudeCode,
    );
    session.id = "session-candidate-detail".to_string();
    session.agent_session_id = Some("native-candidate-detail".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save resumable session");

    runtime.seed_restored_window_details();

    assert!(
        !runtime
            .window_details
            .contains_key(&combined_window_id("tab-candidate", "agent-candidate")),
        "an exact auto-resume candidate must not carry the generic paused detail"
    );
    let shell_detail = runtime
        .window_details
        .get(&combined_window_id("tab-candidate", "shell-1"))
        .cloned()
        .unwrap_or_default();
    assert!(
        shell_detail.contains("Restored window is paused"),
        "non-agent process windows keep the generic paused message, got: {shell_detail}"
    );
}

#[test]
fn app_runtime_startup_auto_resume_uses_centered_stack_bounds() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("centered-stack");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/centered-stack",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab(
        "tab-auto",
        "Auto Resume",
        worktree.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-auto"));
    for (index, native_session_id) in ["native-stack-one", "native-stack-two", "native-stack-three"]
        .into_iter()
        .enumerate()
    {
        let mut session =
            gwt_agent::Session::new(&worktree, "work/centered-stack", gwt_agent::AgentId::Codex);
        session.id = format!("session-centered-stack-{index}");
        session.agent_session_id = Some(native_session_id.to_string());
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session.last_activity_at = chrono::Utc::now() - chrono::Duration::seconds(index as i64);
        session.updated_at = session.last_activity_at;
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");
    }

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let geometries = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .map(|window| window.geometry.clone())
        .collect::<Vec<_>>();
    assert_eq!(geometries.len(), 3);
    assert_eq!(
        geometries
            .iter()
            .map(|geometry| (geometry.x, geometry.y, geometry.width, geometry.height))
            .collect::<Vec<_>>(),
        vec![
            (32.0, 26.0, 1280.0, 800.0),
            (60.0, 50.0, 1280.0, 800.0),
            (88.0, 74.0, 1280.0, 800.0),
        ],
        "restored agent windows should form a stack centered in the startup canvas"
    );
}

#[test]
fn app_runtime_startup_auto_resume_excludes_closed_stopped_windows() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("restore-flag");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/restore-flag",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab(
        "tab-auto",
        "Auto Resume",
        worktree.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-auto"));
    for (session_id, native_session_id, restore_window_on_startup, stopped) in [
        ("session-open-window", "native-open-window", true, false),
        ("session-closed-window", "native-closed-window", false, true),
    ] {
        let mut session =
            gwt_agent::Session::new(&worktree, "work/restore-flag", gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.agent_session_id = Some(native_session_id.to_string());
        session.restore_window_on_startup = restore_window_on_startup;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        if stopped {
            session.update_status(gwt_agent::AgentStatus::Stopped);
        }
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");
    }

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let resumed_sources = runtime
        .pending_auto_resume_sources
        .values()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        resumed_sources,
        std::collections::HashSet::from(["session-open-window".to_string()])
    );
}

#[test]
fn app_runtime_startup_auto_resume_includes_legacy_non_stopped_sessions() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("legacy-restore");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/legacy-restore",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab(
        "tab-auto",
        "Auto Resume",
        worktree.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-auto"));
    let mut session =
        gwt_agent::Session::new(&worktree, "work/legacy-restore", gwt_agent::AgentId::Codex);
    session.id = "session-legacy-open-window".to_string();
    session.agent_session_id = Some("native-legacy-open-window".to_string());
    session.restore_window_on_startup = false;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save legacy open session");

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let resumed_sources = runtime
        .pending_auto_resume_sources
        .values()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
            resumed_sources,
            std::collections::HashSet::from(["session-legacy-open-window".to_string()]),
            "legacy sessions without the new restore flag should still restore when they were not explicitly stopped"
        );
}

#[test]
fn app_runtime_close_agent_window_clears_startup_restore_eligibility() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some(tab_id));
    let mut session =
        gwt_agent::Session::new(&worktree, "work/restore-flag", gwt_agent::AgentId::Codex);
    session.id = "session-close-clears-restore".to_string();
    session.restore_window_on_startup = true;
    session
        .save(&runtime.sessions_dir)
        .expect("save active session");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: session.id.clone(),
            agent_id: "codex".to_string(),
            branch_name: "work/restore-flag".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );

    runtime.close_window_events(&window_id);

    let loaded = gwt_agent::Session::load(
        &runtime
            .sessions_dir
            .join("session-close-clears-restore.toml"),
    )
    .expect("load session");
    assert!(!loaded.restore_window_on_startup);
}

#[test]
fn app_runtime_bootstrap_auto_resumes_same_repo_worktree_session_from_restored_project_tab() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("same-repo-session");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/same-repo-session",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab("tab-repo", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/same-repo-session",
        gwt_agent::AgentId::Codex,
    );
    session.id = "session-same-repo-worktree".to_string();
    session.agent_session_id = Some("native-same-repo-worktree".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save resumable session");

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    assert_eq!(
        runtime.tabs.len(),
        1,
        "bootstrap should keep the restored project tab instead of opening a hidden worktree tab"
    );
    assert!(same_worktree_path(&runtime.tabs[0].project_root, &repo));
    let agent_windows = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
            agent_windows, 1,
            "resumable sessions from local worktrees in the restored repo should restart inside the project tab"
        );
}

#[test]
fn app_runtime_bootstrap_ignores_same_repo_worktree_session_without_lifecycle() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("no-lifecycle-session");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/no-lifecycle-session",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab("tab-repo", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/no-lifecycle-session",
        gwt_agent::AgentId::Codex,
    );
    session.id = "session-same-repo-no-lifecycle".to_string();
    session.agent_session_id = Some("native-no-lifecycle".to_string());
    session.restore_window_on_startup = true;
    session.update_status(gwt_agent::AgentStatus::Running);
    session
        .save(&runtime.sessions_dir)
        .expect("save stale session");

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let agent_windows = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
            agent_windows, 0,
            "same-repo fallback must still require lifecycle evidence so old session history does not mass launch"
        );
}

#[test]
fn app_runtime_bootstrap_ignores_same_repo_worktree_session_with_placeholder_resume_id() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("placeholder-session");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/placeholder-session",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab("tab-repo", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/placeholder-session",
        gwt_agent::AgentId::Codex,
    );
    session.id = "session-same-repo-placeholder".to_string();
    session.agent_session_id = Some("agent-session".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save placeholder session");

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let agent_windows = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
        agent_windows, 0,
        "placeholder Codex hook ids must not launch `codex resume agent-session`"
    );
}

#[test]
fn app_runtime_bootstrap_does_not_auto_resume_sessions_outside_restored_tabs() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("unlisted-auto-resume");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/unlisted-auto-resume",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/unlisted-auto-resume",
        gwt_agent::AgentId::Codex,
    );
    session.id = "session-unlisted-auto".to_string();
    session.agent_session_id = Some("native-unlisted-auto".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(&runtime.sessions_dir)
        .expect("save resumable session");

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    assert!(
            runtime.tabs.is_empty(),
            "bootstrap must not open project tabs from old session TOMLs that were not restored from session.json"
        );
    assert!(
            runtime.pending_auto_resume_sources.is_empty(),
            "unlisted sessions must remain manual resume candidates instead of launching hidden agent windows"
        );
}

#[test]
fn app_runtime_bootstrap_auto_resume_dedupes_and_skips_stale_without_count_cap() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.path().join("worktrees").join("auto-resume-guard");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/auto-resume-guard",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab(
        "tab-worktree",
        "Worktree",
        worktree.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-worktree"));
    let now = chrono::Utc::now();
    let cases = [
        ("session-fresh-1", "native-one", 1_i64),
        ("session-duplicate-native-one", "native-one", 2_i64),
        ("session-fresh-2", "native-two", 3_i64),
        ("session-fresh-3", "native-three", 4_i64),
        ("session-fresh-4", "native-four", 5_i64),
        ("session-stale", "native-stale", 60 * 60 * 48_i64),
    ];
    for (session_id, native_session_id, age_secs) in cases {
        let mut session = gwt_agent::Session::new(
            &worktree,
            "work/auto-resume-guard",
            gwt_agent::AgentId::Codex,
        );
        session.id = session_id.to_string();
        session.agent_session_id = Some(native_session_id.to_string());
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session.last_activity_at = now - chrono::Duration::seconds(age_secs);
        session.updated_at = session.last_activity_at;
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");
    }

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    assert_eq!(
        runtime.pending_auto_resume_sources.len(),
        4,
        "startup auto-resume must restore every fresh unique exact-resumable session"
    );
    let resumed_sources = runtime
        .pending_auto_resume_sources
        .values()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    assert!(
        !resumed_sources.contains("session-duplicate-native-one"),
        "duplicate native agent session ids must not launch twice"
    );
    assert!(
        !resumed_sources.contains("session-stale"),
        "stale persisted sessions must stay available for manual resume instead of auto-launching"
    );
    assert!(
        resumed_sources.contains("session-fresh-4"),
        "startup auto-resume must not drop fresh unique sessions due to an arbitrary count cap"
    );
}

#[test]
fn app_runtime_resume_workspace_journal_populates_quick_start_entries_from_prior_sessions() {
    // SPEC-2359 US-44 (Issue #2757) follow-on: when the user clicks
    // `Resume` on a Workspace journal card whose branch already has a
    // prior session on disk, the Launch Wizard should expose that prior
    // session through the Quick Start panel immediately. Without this,
    // the Quick Start panel only fills after the user completes the
    // runtime resolution step, which forces a multi-click resume path
    // even though the resumable session metadata is already known.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let branch = "work/20260518-resume-qs";
    run_git(&repo, &["branch", branch]);
    gwt_core::workspace_projection::save_workspace_projection(
        &repo,
        &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
    )
    .expect("save projection");
    append_workspace_resume_journal(
        &repo,
        "journal-quickstart",
        temp.path().join("work").join("20260518-resume-qs"),
        "SPEC-2359",
        "Resume with prior Codex session available",
    );

    // Pre-seed a Session toml that matches the resumable branch so the
    // wizard cache exposes it through Quick Start entries.
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("sessions dir");
    let mut session = gwt_agent::Session::new(&repo, branch, gwt_agent::AgentId::Codex);
    session.display_name = "Codex".to_string();
    session.agent_session_id = Some("prior-codex-uuid".to_string());
    session.tool_version = Some("installed".to_string());
    session.save(&sessions_dir).expect("save session toml");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspace {
            source: gwt::WorkspaceResumeSource::Journal,
            journal_id: Some("journal-quickstart".to_string()),
        },
    );

    let session_ref = runtime
        .launch_wizard
        .as_ref()
        .expect("launch wizard opened");
    let view = session_ref.wizard.view();
    assert_eq!(view.branch_name, branch);
    assert!(
            !view.quick_start_entries.is_empty(),
            "Resume must pre-populate Quick Start entries so the prior session is immediately resumable; saw empty Quick Start panel"
        );
    assert!(
            view.quick_start_entries
                .iter()
                .any(|entry| entry.resume_session_id.as_deref() == Some("prior-codex-uuid")),
            "Expected the prior Codex session to appear in Quick Start entries with its resume_session_id surfaced"
        );
}

#[test]
fn app_runtime_resume_workspace_journal_falls_back_to_new_work_branch_when_branch_is_missing() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    gwt_core::workspace_projection::save_workspace_projection(
        &repo,
        &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
    )
    .expect("save projection");
    append_workspace_resume_journal(
        &repo,
        "journal-new-work",
        temp.path().join("work").join("20260507-0002"),
        "Issue #2359",
        "Carry this suspended context into a new work branch.",
    );
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspace {
            source: gwt::WorkspaceResumeSource::Journal,
            journal_id: Some("journal-new-work".to_string()),
        },
    );

    let session = runtime.launch_wizard.as_ref().expect("launch wizard");
    let view = session.wizard.view();
    assert_eq!(view.title, "Start Work");
    assert!(view.branch_name.starts_with("work/"));
    assert_ne!(view.branch_name, "work/20260507-0002");
    assert_eq!(view.linked_issue_number, Some(2359));
    let context = session
        .workspace_resume_context
        .as_ref()
        .expect("workspace resume context");
    assert_eq!(context.owner.as_deref(), Some("Issue #2359"));
    assert_eq!(
        context.summary.as_deref(),
        Some("Carry this suspended context into a new work branch.")
    );
}

#[test]
fn app_runtime_resume_workspace_current_ignores_idle_stale_git_details() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let stale_branch = "work/20260507-stale";
    run_git(&repo, &["branch", stale_branch]);
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.title = "Stale Workspace".to_string();
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Idle;
    projection.status_text = "No active work".to_string();
    projection.summary = Some("Old work should not be resumed".to_string());
    projection.owner = Some("Issue #2359".to_string());
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some(stale_branch.to_string()),
        worktree_path: Some(temp.path().join("work/20260507-stale")),
        base_branch: Some("origin/develop".to_string()),
        pr_number: None,
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspace {
            source: gwt::WorkspaceResumeSource::Current,
            journal_id: None,
        },
    );

    let session = runtime.launch_wizard.as_ref().expect("launch wizard");
    let view = session.wizard.view();
    assert_eq!(view.title, "Start Work");
    assert!(view.branch_name.starts_with("work/"));
    assert_ne!(view.branch_name, stale_branch);
    let context = session
        .workspace_resume_context
        .as_ref()
        .expect("workspace resume context");
    assert_eq!(context.title.as_deref(), Some("Repo Work"));
    assert_eq!(context.owner, None);
    assert_eq!(context.summary, None);
}

#[test]
fn app_runtime_resume_workspace_journal_derives_feature_branch_under_work_named_repo_parent() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("work").join("repo");
    init_git_clone_with_origin(&repo);
    let branch = "feature/resume-existing";
    run_git(&repo, &["branch", branch]);
    gwt_core::workspace_projection::save_workspace_projection(
        &repo,
        &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
    )
    .expect("save projection");
    append_workspace_resume_journal(
        &repo,
        "journal-feature",
        temp.path()
            .join("work")
            .join("feature")
            .join("resume-existing"),
        "Issue #2359",
        "Resume a non-work branch from a deleted worktree path.",
    );
    assert_eq!(
        super::workspace_resume_branch_from_journal_project_root(
            &temp
                .path()
                .join("work")
                .join("feature")
                .join("resume-existing"),
            &repo
        )
        .as_deref(),
        Some(branch)
    );
    assert!(super::workspace_resume_branch_exists(&repo, branch));
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspace {
            source: gwt::WorkspaceResumeSource::Journal,
            journal_id: Some("journal-feature".to_string()),
        },
    );

    let session = runtime.launch_wizard.as_ref().expect("launch wizard");
    let view = session.wizard.view();
    assert_eq!(view.title, "Launch Agent");
    assert_eq!(view.branch_name, branch);
}

#[test]
fn app_runtime_active_work_projection_exposes_done_workspace_cleanup_candidate() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Done;
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some("work/20260507-0200".to_string()),
        worktree_path: Some(repo.join("work/20260507-0200")),
        base_branch: Some("origin/main".to_string()),
        pr_number: Some(2525),
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let candidate = view.cleanup_candidate.expect("cleanup candidate");

    assert_eq!(candidate.branch, "work/20260507-0200");
    assert_eq!(candidate.reason, "workspace_done");
    assert!(!candidate.default_delete_remote);
}

#[test]
fn app_runtime_active_work_projection_does_not_spawn_git_for_cleanup_candidate() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let fake_bin = temp.path().join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("create fake bin");
    let fake_git = write_fake_git_recorder(&fake_bin);
    let git_log = temp.path().join("git-invocations.log");
    let _path = prepend_tool_parent_to_path(&fake_git);
    let _git_log = ScopedEnvVar::set("GWT_FAKE_GIT_LOG", &git_log);
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Done;
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some("work/20260507-0200".to_string()),
        worktree_path: Some(repo.join("work/20260507-0200")),
        base_branch: Some("origin/main".to_string()),
        pr_number: Some(2525),
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert!(view.cleanup_candidate.is_some());
    let invocations = fs::read_to_string(&git_log).unwrap_or_default();
    assert!(
            invocations.trim().is_empty(),
            "active-work projection must not spawn git on the GUI hot path; invocations:\n{invocations}"
        );
}

#[test]
fn workspace_cleanup_failure_does_not_emit_done_work_item() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let branch = "work/missing";
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some(branch.to_string()),
        worktree_path: Some(repo.join("work/missing")),
        base_branch: Some("origin/develop".to_string()),
        pr_number: Some(2828),
        pr_state: Some("MERGED".to_string()),
        pr_url: Some("https://github.com/akiojin/gwt/pull/2828".to_string()),
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    let work_item_id = projection.id.clone();
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let start = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        &work_item_id,
        chrono::Utc::now(),
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, start)
        .expect("record start");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let immediate_events = runtime.run_workspace_cleanup_events("client-1", branch, false, false);

    assert!(immediate_events.is_empty());
    wait_for_recorded_event("workspace cleanup failure", &events, |events| {
        events.iter().any(|event| {
            matches!(
                event,
                UserEvent::Dispatch(outbound_events)
                    if outbound_events.iter().any(|outbound| matches!(
                        outbound.event,
                        BackendEvent::BranchCleanupResult { .. }
                            | BackendEvent::BranchError { .. }
                    ))
            )
        })
    });
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load work items")
        .expect("work items");
    let item = work_items
        .work_items
        .iter()
        .find(|item| item.id == work_item_id)
        .expect("work item");

    assert!(
        !item
            .events
            .iter()
            .any(|event| event.kind == gwt_core::workspace_projection::WorkEventKind::Done),
        "failed cleanup must not mark the Workspace work item done"
    );
}

#[test]
fn app_runtime_active_work_projection_exposes_saved_pr_metadata_without_live_agents() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.title = "Active Work PR display".to_string();
    projection.status_text = "No active work".to_string();
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some("work/20260507-0808".to_string()),
        worktree_path: Some(repo.join("work/20260507-0808")),
        base_branch: Some("origin/develop".to_string()),
        pr_number: Some(2538),
        pr_state: Some("OPEN".to_string()),
        pr_url: Some("https://github.com/akiojin/gwt/pull/2538".to_string()),
        pr_created_at: Some("2026-05-07T08:20:00Z".parse().expect("pr created_at")),
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.active_agents, 0);
    assert_eq!(view.pr_number, Some(2538));
    assert_eq!(view.pr_state.as_deref(), Some("OPEN"));
    assert_eq!(
        view.pr_url.as_deref(),
        Some("https://github.com/akiojin/gwt/pull/2538")
    );
    assert_eq!(
        view.pr_created_at.as_deref(),
        Some("2026-05-07T08:20:00+00:00")
    );
}

#[test]
fn app_runtime_active_work_projection_hides_cleanup_candidate_for_live_agent_branch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "codex-1",
        repo.clone(),
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Done;
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some("work/20260507-0200".to_string()),
        worktree_path: Some(repo.join("work/20260507-0200")),
        base_branch: Some("origin/main".to_string()),
        pr_number: Some(2525),
        pr_state: Some("merged".to_string()),
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id,
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260507-0200".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.join("work/20260507-0200"),
            agent_project_root: repo.join("work/20260507-0200").display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");

    assert_eq!(view.cleanup_candidate, None);
}

#[test]
fn app_runtime_active_work_projection_hides_row_cleanup_candidate_for_live_agent_branch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "codex-1",
        repo.clone(),
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some("work/20260615-live-cleanup".to_string()),
        worktree_path: Some(repo.join("work/20260615-live-cleanup")),
        base_branch: Some("origin/develop".to_string()),
        pr_number: Some(3099),
        pr_state: Some("MERGED".to_string()),
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id,
            session_id: "session-live-cleanup".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260615-live-cleanup".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.join("work/20260615-live-cleanup"),
            agent_project_root: repo
                .join("work/20260615-live-cleanup")
                .display()
                .to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let row = view
        .active_works
        .iter()
        .find(|work| work.branch.as_deref() == Some("work/20260615-live-cleanup"))
        .expect("live Workspace row");

    assert!(row.merged_into_base, "merged badge remains visible");
    assert_eq!(
        row.cleanup_candidate, None,
        "live Agent branch must be absent from row-level cleanup candidates"
    );
}

#[test]
fn app_runtime_row_cleanup_candidate_exposes_merged_workspace_without_live_agent() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            "work-merged-cleanup-row",
            chrono::Utc::now(),
        );
        event.title = Some("Merged cleanup row".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/20260615-merged-cleanup".to_string()),
                worktree_path: Some(repo.join("work/20260615-merged-cleanup")),
                pr_number: Some(3100),
                pr_url: None,
                pr_state: Some("MERGED".to_string()),
            },
        );
        event
    })
    .expect("record work");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let row = view
        .active_works
        .iter()
        .find(|work| work.branch.as_deref() == Some("work/20260615-merged-cleanup"))
        .expect("merged Workspace row");
    let candidate = row
        .cleanup_candidate
        .as_ref()
        .expect("eligible merged row should expose cleanup candidate");

    assert_eq!(candidate.branch, "work/20260615-merged-cleanup");
    assert_eq!(candidate.reason, "pr_merged");
    assert!(!candidate.default_delete_remote);
}

#[test]
fn app_runtime_row_cleanup_candidate_hides_grouped_live_agent_branch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            "work-merged-grouped-row",
            chrono::Utc::now(),
        );
        event.title = Some("Merged grouped row".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/20260615-grouped-cleanup".to_string()),
                worktree_path: Some(repo.join("work/20260615-grouped-cleanup")),
                pr_number: Some(3101),
                pr_url: None,
                pr_state: Some("MERGED".to_string()),
            },
        );
        event
    })
    .expect("record work");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "codex-1",
        repo.clone(),
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id,
            session_id: "session-grouped-cleanup".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260615-grouped-cleanup".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.join("work/20260615-grouped-cleanup"),
            agent_project_root: repo
                .join("work/20260615-grouped-cleanup")
                .display()
                .to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let row = view
        .active_works
        .iter()
        .find(|work| work.branch.as_deref() == Some("work/20260615-grouped-cleanup"))
        .expect("grouped Workspace row");

    assert!(row.merged_into_base, "merged badge remains visible");
    assert_eq!(
        row.cleanup_candidate, None,
        "grouped row with live Agent on the same branch must not be cleanable"
    );
}

#[cfg(unix)]
#[test]
fn app_runtime_row_cleanup_candidate_hides_workspace_with_live_cwd_process() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = repo.join("work/20260616-0203");
    fs::create_dir_all(&worktree).expect("create worktree");
    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            "work-live-cwd-cleanup-row",
            chrono::Utc::now(),
        );
        event.title = Some("Merged live cwd row".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/20260616-0203".to_string()),
                worktree_path: Some(worktree.clone()),
                pr_number: Some(3108),
                pr_url: None,
                pr_state: Some("MERGED".to_string()),
            },
        );
        event
    })
    .expect("record work");
    let _child = KillOnDrop(
        gwt_core::process::hidden_command("sh")
            .arg("-c")
            .arg("sleep 30")
            .current_dir(&worktree)
            .spawn()
            .expect("spawn cwd process"),
    );
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let row = view
        .active_works
        .iter()
        .find(|work| work.branch.as_deref() == Some("work/20260616-0203"))
        .expect("merged Workspace row");

    assert!(row.merged_into_base, "merged badge remains visible");
    assert_eq!(
        row.cleanup_candidate, None,
        "Workspace whose worktree is still an agent process cwd must not be cleanable"
    );
}

#[test]
fn app_runtime_stopped_agent_cleans_saved_projection_and_broadcasts_active_work_idle() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "codex-1",
        repo.clone(),
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    let session = ActiveAgentSession {
        window_id: window_id.clone(),
        session_id: "session-1".to_string(),
        agent_id: "codex".to_string(),
        branch_name: "work/20260506-1652".to_string(),
        display_name: "Codex".to_string(),
        worktree_path: temp.path().join("work/20260506-1652"),
        agent_project_root: temp.path().join("work/20260506-1652").display().to_string(),
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        tab_id: "tab-1".to_string(),
    };
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session.clone());
    save_start_work_workspace_projection(
        &repo,
        &session,
        "origin/main",
        None,
        None,
        &std::collections::HashSet::new(),
    )
    .expect("save projection");

    let events = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Stopped,
        Some("Process exited".to_string()),
    );

    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load projection")
        .expect("projection");
    assert!(projection.agents.is_empty());
    assert_eq!(
        projection.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Idle
    );
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Broadcast,
            event: BackendEvent::ActiveWorkProjection { projection },
        } if projection.active_agents == 0
            && projection.agents.is_empty()
            && projection.status_category == "idle"
    )));
}

#[test]
fn app_runtime_status_thread_reports_process_exit_without_reader_eof() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Running,
    );
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    let captured_events = match &runtime.proxy {
        AppEventProxy::Stub(events) => events.clone(),
        AppEventProxy::Real(_) => panic!("sample runtime must use stub proxy"),
    };
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/D".to_string(),
                "/S".to_string(),
                "/C".to_string(),
                "exit /B 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };
    let pane = Arc::new(Mutex::new(
        Pane::new(
            window_id.clone(),
            command,
            args,
            80,
            24,
            HashMap::new(),
            test_pane_cwd(),
        )
        .expect("pane"),
    ));
    if cfg!(windows) {
        // Windows ConPTY may wait for this CPR response before exposing exit state.
        if let Ok(pane) = pane.lock() {
            let _ = pane.pty().write_input(b"\x1b[1;1R");
        }
    }
    let status_thread = runtime.spawn_status_thread(window_id.clone(), pane.clone());

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut observed_status = None;
    while Instant::now() < deadline {
        if let Ok(events) = captured_events.lock() {
            observed_status = events.iter().find_map(|event| match event {
                UserEvent::RuntimeStatus { id, status, detail }
                    if id == &window_id && *status == WindowProcessStatus::Stopped =>
                {
                    Some(detail.clone())
                }
                _ => None,
            });
        }
        if observed_status.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    if observed_status.is_none() {
        if let Ok(pane) = pane.lock() {
            let _ = pane.kill();
        }
    }
    let _ = status_thread.join();

    assert_eq!(observed_status.flatten().as_deref(), Some("Process exited"));
}

#[test]
fn app_runtime_runtime_hook_stopped_auto_closes_active_agent_window() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let events = runtime.handle_runtime_hook_event(runtime_hook_state("Stopped", "session-1"));

    // SPEC-2359 Phase W-15 (FR-382): the stop records a Pause work item, and
    // the surface must update without a saved current.json or live agents —
    // so the projection broadcast accompanies the WindowCanvasState event.
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[0].event,
        BackendEvent::WindowCanvasState { .. }
    ));
    assert!(matches!(
        events[1].event,
        BackendEvent::ActiveWorkProjection { .. }
    ));
    assert!(!runtime.active_agent_sessions.contains_key(&window_id));
    assert!(!runtime.window_lookup.contains_key(&window_id));
    assert!(runtime.tabs[0].workspace.window("codex-1").is_none());
}

#[test]
fn app_runtime_workspace_projection_surface_helper_groups_state_and_active_work_events() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let mut events = Vec::new();
    runtime.push_workspace_and_active_work_projection_broadcasts(&mut events);

    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[0].event,
        BackendEvent::WindowCanvasState { .. }
    ));
    assert!(matches!(
        events[1].event,
        BackendEvent::ActiveWorkProjection { .. }
    ));
}

#[test]
fn app_runtime_runtime_hook_stopped_without_active_session_keeps_window_open() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");

    let events = runtime.handle_runtime_hook_event(runtime_hook_state("Stopped", "session-1"));

    assert!(events.is_empty());
    assert!(runtime.window_lookup.contains_key(&window_id));
    assert!(runtime.tabs[0].workspace.window("codex-1").is_some());
}

#[test]
fn app_runtime_runtime_state_hooks_use_status_events_without_browser_hook_event() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let events = runtime.handle_runtime_hook_event(runtime_hook_state("Waiting", "session-1"));

    assert!(
            !events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::RuntimeHookEvent { .. })),
            "runtime_state hooks are browser-internal noise; status events carry the visible chrome state"
        );
    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowState { .. })));
    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::TerminalStatus { .. })));
}

#[test]
fn app_runtime_coordination_hooks_still_emit_browser_hook_event() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "board-1",
        WindowPreset::Board,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_runtime_hook_event(runtime_hook_coordination_event("session-1"));

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].event,
        BackendEvent::RuntimeHookEvent { .. }
    ));
}

#[test]
fn app_runtime_runtime_state_bursts_emit_no_browser_hook_events() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let browser_hook_events = (0..1_000)
        .flat_map(|_| runtime.handle_runtime_hook_event(runtime_hook_state("Waiting", "session-1")))
        .filter(|event| matches!(event.event, BackendEvent::RuntimeHookEvent { .. }))
        .count();

    assert_eq!(browser_hook_events, 0);
}

#[test]
fn app_runtime_duplicate_runtime_state_hooks_emit_status_events_only_once() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let events = (0..1_000)
        .flat_map(|_| runtime.handle_runtime_hook_event(runtime_hook_state("Waiting", "session-1")))
        .collect::<Vec<_>>();

    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.event, BackendEvent::RuntimeHookEvent { .. }))
            .count(),
        0
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.event, BackendEvent::WindowState { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.event, BackendEvent::TerminalStatus { .. }))
            .count(),
        1
    );
}

#[test]
fn app_runtime_runtime_state_change_after_duplicate_burst_emits_status_events() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let first_events =
        runtime.handle_runtime_hook_event(runtime_hook_state("Waiting", "session-1"));
    let duplicate_events =
        runtime.handle_runtime_hook_event(runtime_hook_state("Waiting", "session-1"));
    let changed_events =
        runtime.handle_runtime_hook_event(runtime_hook_state("Running", "session-1"));

    assert!(first_events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::TerminalStatus { .. })));
    assert!(
        duplicate_events.is_empty(),
        "unchanged RuntimeState hooks should not fan out status events"
    );
    assert!(changed_events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Running,
            ..
        }
    )));
    assert!(changed_events.iter().any(|event| matches!(
        event.event,
        BackendEvent::TerminalStatus {
            status: WindowProcessStatus::Running,
            ..
        }
    )));
}

#[test]
fn app_runtime_stopped_runtime_state_after_prior_state_still_auto_closes() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let _ = runtime.handle_runtime_hook_event(runtime_hook_state("Waiting", "session-1"));

    let events = runtime.handle_runtime_hook_event(runtime_hook_state("Stopped", "session-1"));

    assert!(matches!(
        events.first().map(|event| &event.event),
        Some(BackendEvent::WindowCanvasState { .. })
    ));
    assert!(!runtime.active_agent_sessions.contains_key(&window_id));
    assert!(!runtime.window_lookup.contains_key(&window_id));
    assert!(runtime.tabs[0].workspace.window("codex-1").is_none());
}

#[test]
fn app_runtime_start_window_registers_running_process_runtime_and_pty_writer() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::NonRepo,
        &[WindowPreset::Shell],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window = runtime.tabs[0].workspace.persisted().windows[0].clone();
    let window_id = combined_window_id("tab-1", &window.id);

    let events = runtime.start_window("tab-1", &window.id, window.preset, window.geometry.clone());

    assert_eq!(events.len(), 2);
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::WindowState { window_id: id, state }
            if id == &window_id && *state == WindowProcessStatus::Running
    )));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::TerminalStatus { id, status, detail }
            if id == &window_id
                && *status == WindowProcessStatus::Running
                && detail.is_none()
    )));
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Running)
    );
    assert!(runtime.runtimes.contains_key(&window_id));
    assert!(runtime
        .pty_writers
        .read()
        .expect("pty writer registry")
        .contains_key(&window_id));

    runtime.stop_window_runtime(&window_id);
}

// SPEC-2356 安心 Addendum (FR-041): StopWindow tears down the runtime but KEEPS
// the window on the canvas, rendered as Stopped, unlike CloseWindow.
#[test]
fn stop_window_events_keeps_window_and_marks_stopped() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::NonRepo,
        &[WindowPreset::Shell],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window = runtime.tabs[0].workspace.persisted().windows[0].clone();
    let window_id = combined_window_id("tab-1", &window.id);
    runtime.register_window("tab-1", &window.id);
    insert_test_pane_runtime(&mut runtime, &window_id);
    runtime
        .window_pty_statuses
        .insert(window_id.clone(), WindowProcessStatus::Running);

    let events = runtime.stop_window_events(&window_id);

    // The runtime is gone (PTY killed) but the window record survives.
    assert!(!runtime.runtimes.contains_key(&window_id));
    assert!(runtime.window_lookup.contains_key(&window_id));
    assert!(
        runtime.tabs[0].workspace.window(&window.id).is_some(),
        "StopWindow must keep the window on the canvas, unlike CloseWindow"
    );
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Stopped),
        "stopped window must render as Stopped"
    );
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::WindowState { window_id: id, state }
            if id == &window_id && *state == WindowProcessStatus::Stopped
    )));
}

// SPEC-2356 安心 Addendum (FR-041): StopWindow is idempotent — stopping an
// already-stopped window keeps it on the canvas and stays Stopped.
#[test]
fn stop_window_events_is_idempotent_when_already_stopped() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "shell-1",
        repo,
        WindowPreset::Shell,
        WindowProcessStatus::Stopped,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    runtime.register_window("tab-1", "shell-1");

    let events = runtime.stop_window_events(&window_id);

    assert!(runtime.tabs[0].workspace.window("shell-1").is_some());
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Stopped)
    );
    // Idempotent: it still emits the authoritative Stopped status, never an error.
    assert!(events.iter().all(|event| !matches!(
        &event.event,
        BackendEvent::WindowState { state, .. } if *state == WindowProcessStatus::Error
    )));
}

// SPEC-2356 安心 Addendum (FR-041): CloseWindow vs StopWindow contract — Close
// removes the window, Stop keeps it. This guards the distinction directly.
#[test]
fn close_window_removes_while_stop_window_keeps() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::NonRepo,
        &[WindowPreset::Shell, WindowPreset::Shell],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let windows: Vec<_> = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .map(|window| window.id.clone())
        .collect();
    let stop_raw = windows[0].clone();
    let close_raw = windows[1].clone();
    let stop_id = combined_window_id("tab-1", &stop_raw);
    let close_id = combined_window_id("tab-1", &close_raw);
    runtime.register_window("tab-1", &stop_raw);
    runtime.register_window("tab-1", &close_raw);

    runtime.stop_window_events(&stop_id);
    runtime.close_window_events(&close_id);

    assert!(
        runtime.tabs[0].workspace.window(&stop_raw).is_some(),
        "StopWindow keeps the window"
    );
    assert!(
        runtime.tabs[0].workspace.window(&close_raw).is_none(),
        "CloseWindow removes the window"
    );
}

// SPEC-2356 安心 Addendum (FR-042): StopAllWindows stops every running agent
// window's runtime while keeping all windows on the canvas.
#[test]
fn stop_all_windows_events_stops_every_runtime_keeping_windows() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::NonRepo,
        &[WindowPreset::Claude, WindowPreset::Codex],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let raw_ids: Vec<_> = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .map(|window| window.id.clone())
        .collect();
    for raw in &raw_ids {
        let window_id = combined_window_id("tab-1", raw);
        runtime.register_window("tab-1", raw);
        insert_test_pane_runtime(&mut runtime, &window_id);
        runtime
            .window_pty_statuses
            .insert(window_id.clone(), WindowProcessStatus::Running);
    }

    runtime.stop_all_windows_events();

    for raw in &raw_ids {
        let window_id = combined_window_id("tab-1", raw);
        assert!(
            !runtime.runtimes.contains_key(&window_id),
            "every agent runtime must be torn down"
        );
        assert!(
            runtime.tabs[0].workspace.window(raw).is_some(),
            "StopAllWindows must keep all windows on the canvas"
        );
        assert_eq!(
            runtime.window_status(&window_id),
            Some(WindowProcessStatus::Stopped)
        );
    }
}

// SPEC-2356 安心 Addendum (FR-044): RestartWindow relaunches a stopped process
// preset in place, preserving the window id and re-registering its runtime.
#[test]
fn restart_window_events_relaunches_stopped_process_window_in_place() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "shell-1",
        repo,
        WindowPreset::Shell,
        WindowProcessStatus::Stopped,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    runtime.register_window("tab-1", "shell-1");
    assert!(!runtime.runtimes.contains_key(&window_id));

    let events = runtime.restart_window_events(&window_id);

    // Same window id, freshly-running runtime + PTY writer registered.
    assert!(
        runtime.tabs[0].workspace.window("shell-1").is_some(),
        "RestartWindow must preserve the window id"
    );
    assert!(runtime.runtimes.contains_key(&window_id));
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Running)
    );
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::WindowState { window_id: id, state }
            if id == &window_id && *state == WindowProcessStatus::Running
    )));

    runtime.stop_window_runtime(&window_id);
}

// SPEC-2356 安心 Addendum (FR-044): RestartWindow only acts on stopped/errored
// windows — restarting a window that is already running is a no-op so a live
// agent is never double-spawned.
#[test]
fn restart_window_events_is_noop_when_window_already_running() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::NonRepo,
        &[WindowPreset::Shell],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window = runtime.tabs[0].workspace.persisted().windows[0].clone();
    let window_id = combined_window_id("tab-1", &window.id);
    runtime.register_window("tab-1", &window.id);
    insert_test_pane_runtime(&mut runtime, &window_id);
    runtime
        .window_pty_statuses
        .insert(window_id.clone(), WindowProcessStatus::Running);
    let original_pane = runtime
        .runtimes
        .get(&window_id)
        .map(|runtime| Arc::as_ptr(&runtime.pane) as usize)
        .expect("runtime present");

    let events = runtime.restart_window_events(&window_id);

    assert!(events.is_empty(), "restarting a running window is a no-op");
    let after_pane = runtime
        .runtimes
        .get(&window_id)
        .map(|runtime| Arc::as_ptr(&runtime.pane) as usize)
        .expect("runtime still present");
    assert_eq!(
        original_pane, after_pane,
        "running window's runtime must not be replaced"
    );

    runtime.stop_window_runtime(&window_id);
}

#[test]
fn app_runtime_stop_all_runtimes_kills_every_pane_before_join_waits() {
    let temp = tempdir().expect("tempdir");
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let blocker_id = "a-blocking-runtime".to_string();
    let observed_id = "b-observed-runtime".to_string();
    let blocking_pane = Arc::new(Mutex::new(long_running_test_pane(&blocker_id)));
    let observed_pane = Arc::new(Mutex::new(long_running_test_pane(&observed_id)));
    let observed_pane_for_assertion = observed_pane.clone();
    let blocking_join = thread::spawn(|| thread::sleep(Duration::from_secs(2)));

    runtime.runtimes.insert(
        blocker_id.clone(),
        WindowRuntime {
            pane: blocking_pane,
            output_thread: Some(blocking_join),
            status_thread: None,
        },
    );
    runtime.runtimes.insert(
        observed_id.clone(),
        WindowRuntime {
            pane: observed_pane,
            output_thread: None,
            status_thread: None,
        },
    );

    let stop_thread = thread::spawn(move || {
        runtime.stop_runtimes_in_shutdown_order(vec![blocker_id, observed_id]);
    });

    let deadline = Instant::now() + Duration::from_millis(400);
    let mut observed_exited = false;
    while Instant::now() < deadline {
        observed_exited = observed_pane_for_assertion
            .lock()
            .expect("observed pane")
            .pty()
            .try_wait()
            .expect("observed try_wait")
            .is_some();
        if observed_exited {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    if !observed_exited {
        let _ = observed_pane_for_assertion
            .lock()
            .expect("observed pane cleanup")
            .kill();
    }
    stop_thread.join().expect("stop thread");

    assert!(
        observed_exited,
        "shutdown must kill all panes before waiting for any runtime join handle"
    );
}

#[test]
fn app_runtime_viewport_and_geometry_updates_persist_workspace_state() {
    // Persistence flows through `workspace_state_path()` which is
    // HOME-based, so we must serialize against other HOME-touching
    // tests and pin HOME to this test's tempdir for the duration.
    // Without this guard, parallel tests that mutate HOME race
    // with our persist + load pair and the workspace file ends up
    // missing (or pointing at another test's already-cleaned
    // tempdir).
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "shell-1",
        repo.clone(),
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");

    assert_eq!(
        runtime
            .update_viewport_events(gwt::CanvasViewport {
                x: 12.0,
                y: 34.0,
                zoom: 1.25,
            })
            .len(),
        1
    );
    assert_eq!(
        runtime
            .update_window_geometry_events(
                &window_id,
                WindowGeometry {
                    x: 56.0,
                    y: 78.0,
                    width: 720.0,
                    height: 480.0,
                },
                100,
                30,
                None,
            )
            .len(),
        1
    );

    assert!(
        runtime
            .persist_dispatcher
            .wait_idle(std::time::Duration::from_secs(5)),
        "persist dispatcher should drain before disk readback",
    );
    let session = load_session_state(&temp.path().join("session-state.json"))
        .expect("load persisted session state");
    assert_eq!(session.active_tab_id.as_deref(), Some("tab-1"));
    assert_eq!(session.tabs.len(), 1);
    assert_eq!(session.tabs[0].project_root, repo);

    let workspace = load_restored_workspace_state(&repo).expect("load persisted workspace");
    assert_eq!(workspace.viewport.x, 12.0);
    assert_eq!(workspace.viewport.y, 34.0);
    assert_eq!(workspace.viewport.zoom, 1.25);
    let window = workspace
        .windows
        .iter()
        .find(|window| window.id == "shell-1")
        .expect("persisted window");
    assert_eq!(window.geometry.x, 56.0);
    assert_eq!(window.geometry.y, 78.0);
    assert_eq!(window.geometry.width, 720.0);
    assert_eq!(window.geometry.height, 480.0);
}

#[test]
fn app_runtime_duplicate_viewport_update_skips_workspace_broadcast_and_persist() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "shell-1",
        repo,
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let viewport = gwt::CanvasViewport {
        x: 12.0,
        y: 34.0,
        zoom: 1.25,
    };

    assert_eq!(runtime.update_viewport_events(viewport.clone()).len(), 1);
    assert_eq!(runtime.persist_dispatcher.enqueued_count(), 1);

    assert!(
        runtime.update_viewport_events(viewport).is_empty(),
        "duplicate viewport payload should not broadcast a workspace_state",
    );
    assert_eq!(
        runtime.persist_dispatcher.enqueued_count(),
        1,
        "duplicate viewport payload should not enqueue another persist snapshot",
    );

    assert_eq!(
        runtime
            .update_viewport_events(gwt::CanvasViewport {
                x: 12.0,
                y: 34.0,
                zoom: 1.5,
            })
            .len(),
        1,
        "changed zoom must still broadcast workspace_state",
    );
    assert_eq!(
        runtime.persist_dispatcher.enqueued_count(),
        2,
        "changed viewport must still enqueue persistence",
    );
}

#[test]
fn app_runtime_geometry_update_rejects_stale_base_revision() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "shell-1",
        repo.clone(),
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");

    assert_eq!(
        runtime
            .update_window_geometry_events(
                &window_id,
                WindowGeometry {
                    x: 56.0,
                    y: 78.0,
                    width: 720.0,
                    height: 480.0,
                },
                100,
                30,
                Some(0),
            )
            .len(),
        1
    );

    assert_eq!(
        runtime
            .update_window_geometry_events(
                &window_id,
                WindowGeometry {
                    x: 90.0,
                    y: 120.0,
                    width: 960.0,
                    height: 640.0,
                },
                120,
                40,
                Some(0),
            )
            .len(),
        1,
        "stale updates should return the current workspace state so the frontend can resync"
    );

    assert!(
        runtime
            .persist_dispatcher
            .wait_idle(std::time::Duration::from_secs(5)),
        "persist dispatcher should drain before disk readback",
    );
    let workspace = load_restored_workspace_state(&repo).expect("load persisted workspace");
    let window = workspace
        .windows
        .iter()
        .find(|window| window.id == "shell-1")
        .expect("persisted window");
    assert_eq!(window.geometry.x, 56.0);
    assert_eq!(window.geometry.y, 78.0);
    assert_eq!(window.geometry.width, 720.0);
    assert_eq!(window.geometry.height, 480.0);
    assert_eq!(window.geometry_revision, 1);
}

#[test]
fn app_runtime_load_board_replies_with_repo_scoped_snapshot() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "Need review",
            Some("running".to_string()),
            None,
            vec!["coordination".to_string()],
            vec!["2018".to_string()],
        ),
    )
    .expect("seed board snapshot");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo,
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "board-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadBoard {
            id: window_id.clone(),
            all: false,
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::BoardEntries { id, entries, .. },
        }] if client_id == "client-1"
            && id == &window_id
            && entries.len() == 1
            && entries[0].body == "Need review"
    ));
}

#[test]
fn app_runtime_load_board_defaults_to_current_workspace_audience() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.id = "workspace-current".to_string();
    projection
        .agents
        .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: "session-current".to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: Some("Board audience".to_string()),
            title_summary: Some("Board audience".to_string()),
            worktree_path: Some(repo.clone()),
            branch: Some("work/board-audience".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: Some("workspace-current".to_string()),
            updated_at: chrono::Utc::now(),
        });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "broadcast entry",
            None,
            None,
            vec![],
            vec![],
        ),
    )
    .expect("seed broadcast");
    post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "current workspace entry",
            None,
            None,
            vec![],
            vec![],
        )
        .with_audience(vec!["workspace-current"]),
    )
    .expect("seed current");
    post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "other workspace entry",
            None,
            None,
            vec![],
            vec![],
        )
        .with_audience(vec!["workspace-other"]),
    )
    .expect("seed other");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo,
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "board-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadBoard {
            id: window_id.clone(),
            all: false,
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            event: BackendEvent::BoardEntries { entries, .. },
            ..
        }] if entries.iter().map(|entry| entry.body.as_str()).collect::<Vec<_>>()
            == vec!["broadcast entry", "current workspace entry"]
    ));
}

#[test]
fn app_runtime_load_board_history_replies_with_older_page() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    for idx in 0..4 {
        let mut entry = BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            format!("entry-{idx}"),
            None,
            None,
            vec![],
            vec![],
        );
        entry.id = format!("entry-{idx}");
        entry.created_at = chrono::Utc::now() + chrono::Duration::seconds(idx);
        entry.updated_at = entry.created_at;
        post_entry(&repo, entry).expect("seed board entry");
    }
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo,
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "board-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadBoardHistory {
            id: window_id.clone(),
            before_entry_id: Some("entry-3".to_string()),
            limit: 2,
            all: false,
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::BoardHistoryPage {
                id,
                entries,
                has_more_before,
            },
        }] if client_id == "client-1"
            && id == &window_id
            && entries.iter().map(|entry| entry.body.as_str()).collect::<Vec<_>>() == vec!["entry-1", "entry-2"]
            && *has_more_before
    ));
}

#[test]
fn app_runtime_open_board_origin_agent_focuses_live_origin_session_window() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let board_raw_id = tab
        .workspace
        .add_window(WindowPreset::Board, canvas_bounds())
        .id;
    let agent_raw_id = tab
        .workspace
        .add_window(WindowPreset::Agent, canvas_bounds())
        .id;
    let board_window_id = combined_window_id("tab-1", &board_raw_id);
    let agent_window_id = combined_window_id("tab-1", &agent_raw_id);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.active_agent_sessions.insert(
        agent_window_id.clone(),
        ActiveAgentSession {
            window_id: agent_window_id.clone(),
            session_id: "session-origin".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/board-origin".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::OpenBoardOriginAgent {
            id: board_window_id,
            origin_session_id: "session-origin".to_string(),
            bounds: Some(canvas_bounds()),
        },
    );

    let workspace = runtime.tab("tab-1").expect("tab").workspace.persisted();
    let focused = workspace
        .windows
        .iter()
        .max_by_key(|window| window.z_index)
        .expect("focused window");
    assert_eq!(focused.id, agent_raw_id);
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            event: BackendEvent::WindowCanvasState { .. },
            ..
        }
    )));
}

#[test]
fn board_origin_agent_resume_config_uses_exact_saved_session() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let runtime = sample_runtime(temp.path(), Vec::new(), None);
    let mut session =
        gwt_agent::Session::new(&repo, "work/board-origin", gwt_agent::AgentId::Codex);
    session.id = "session-origin".to_string();
    session.agent_session_id = Some("codex-resume-123".to_string());
    session.model = Some("gpt-5.4".to_string());
    session.reasoning_level = Some("high".to_string());
    session.tool_version = Some("latest".to_string());
    session.skip_permissions = true;
    session.codex_fast_mode = true;
    session.save(&runtime.sessions_dir).expect("save session");

    let config = runtime
        .board_origin_agent_resume_config("session-origin")
        .expect("resume config");

    assert_eq!(config.branch.as_deref(), Some("work/board-origin"));
    assert_eq!(config.working_dir.as_deref(), Some(repo.as_path()));
    assert_eq!(
        config.resume_session_id.as_deref(),
        Some("codex-resume-123")
    );
    assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
    assert_eq!(config.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(config.reasoning_level.as_deref(), Some("high"));
    assert!(config.skip_permissions);
    assert!(config.codex_fast_mode);
}

#[test]
fn board_origin_agent_resume_config_supports_builtin_agent_descriptors() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let runtime = sample_runtime(temp.path(), Vec::new(), None);

    for agent_id in [gwt_agent::AgentId::OpenClaw, gwt_agent::AgentId::Hermes] {
        let session_id = format!("session-{}", agent_id.command());
        let resume_id = format!("resume-{}", agent_id.command());
        let mut session = gwt_agent::Session::new(
            &repo,
            format!("work/{}", agent_id.command()),
            agent_id.clone(),
        );
        session.id = session_id.clone();
        session.agent_session_id = Some(resume_id.clone());
        session.save(&runtime.sessions_dir).expect("save session");

        let config = runtime
            .board_origin_agent_resume_config(&session_id)
            .expect("resume config");

        assert_eq!(config.agent_id, agent_id);
        assert_eq!(
            config.resume_session_id.as_deref(),
            Some(resume_id.as_str())
        );
        assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
    }
}

#[test]
fn app_runtime_open_board_origin_agent_rejects_missing_exact_resume_session() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo,
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let board_window_id = combined_window_id("tab-1", "board-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::OpenBoardOriginAgent {
            id: board_window_id.clone(),
            origin_session_id: "missing-session".to_string(),
            bounds: Some(canvas_bounds()),
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::BoardError { id, message },
        }] if client_id == "client-1"
            && id == &board_window_id
            && message.contains("missing-session")
    ));
}

#[test]
fn app_runtime_load_knowledge_bridge_replies_with_cache_backed_issue_and_spec_views() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let cache = Cache::new(issue_cache_root(&repo));
    cache
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Issue bridge",
            &["bug"],
            "Issue body",
            "2026-04-20T10:00:00Z",
        ))
        .expect("write issue snapshot");
    cache
        .write_snapshot(&sample_issue_snapshot(
            1930,
            "SPEC-1930: Cache-backed SPEC bridge",
            &["gwt-spec", "phase/implementation"],
            concat!(
                "<!-- gwt-spec id=1930 version=1 -->\n",
                "<!-- sections:\n",
                "spec=body\n",
                "tasks=body\n",
                "-->\n\n",
                "<!-- artifact:spec BEGIN -->\n",
                "# SPEC bridge\n",
                "## Summary\n",
                "Cache-backed issue view\n",
                "<!-- artifact:spec END -->\n\n",
                "<!-- artifact:tasks BEGIN -->\n",
                "- [x] T-001\n",
                "<!-- artifact:tasks END -->\n"
            ),
            "2026-04-20T09:00:00Z",
        ))
        .expect("write spec snapshot");
    write_issue_link_store(
        &repo,
        HashMap::from([("feature/issue-bridge".to_string(), 42)]),
    );

    let mut persisted = empty_workspace_state();
    persisted.windows.push(sample_window(
        "issue-1",
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    ));
    persisted.windows.push(sample_window(
        "spec-1",
        WindowPreset::Spec,
        WindowProcessStatus::Ready,
    ));
    persisted.next_z_index = 3;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo,
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let issue_window_id = combined_window_id("tab-1", "issue-1");
    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &issue_window_id,
            kind: gwt::KnowledgeKind::Issue,
            request_id: None,
            selected_number: Some(42),
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let issue_events = wait_for_knowledge_view_dispatch(&recorded_events, &issue_window_id);
    assert_eq!(issue_events.len(), 2);
    assert!(matches!(
        &issue_events[0].event,
        BackendEvent::KnowledgeEntries {
            knowledge_kind,
            entries,
            selected_number,
            refresh_enabled,
            ..
        } if *knowledge_kind == gwt::KnowledgeKind::Issue
            && entries.len() == 2
            && entries.iter().any(|entry| entry.number == 42
                && entry.linked_branch_count == 1
                && !entry.is_spec)
            && entries.iter().any(|entry| entry.number == 1930 && entry.is_spec)
            && *selected_number == Some(42)
            && *refresh_enabled
    ));
    assert!(matches!(
        &issue_events[1].event,
        BackendEvent::KnowledgeDetail { detail, .. }
            if detail.launch_issue_number == Some(42)
                && detail.sections.iter().any(|section| section.title == "Linked branches"
                    && section.body.contains("feature/issue-bridge"))
    ));

    let spec_window_id = combined_window_id("tab-1", "spec-1");
    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &spec_window_id,
            kind: gwt::KnowledgeKind::Issue,
            request_id: None,
            selected_number: Some(1930),
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let spec_events = wait_for_knowledge_view_dispatch(&recorded_events, &spec_window_id);
    assert_eq!(spec_events.len(), 2);
    assert!(matches!(
        &spec_events[0].event,
        BackendEvent::KnowledgeEntries {
            knowledge_kind,
            entries,
            selected_number,
            refresh_enabled,
            ..
        } if *knowledge_kind == gwt::KnowledgeKind::Issue
            && entries.len() == 2
            && entries.iter().any(|entry| entry.number == 1930 && entry.is_spec)
            && *selected_number == Some(1930)
            && *refresh_enabled
    ));
    assert!(matches!(
        &spec_events[1].event,
        BackendEvent::KnowledgeDetail { detail, .. }
            if detail.sections.iter().any(|section| section.title == "spec"
                && section.body.contains("Cache-backed issue view"))
    ));
}

/// Issue #3297: the cache-backed knowledge load must not run on the GUI
/// event loop. On slow machines the synchronous read blocked the loop past
/// the frontend's 5s recovery timer, the late reply was discarded, and the
/// retry escalated into a minutes-long forced sync. The load replies through
/// the blocking-task proxy instead, like the semantic search path.
#[test]
fn app_runtime_load_knowledge_bridge_replies_off_the_gui_event_loop() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let cache = Cache::new(issue_cache_root(&repo));
    cache
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Issue bridge",
            &["bug"],
            "Issue body",
            "2026-04-20T10:00:00Z",
        ))
        .expect("write issue snapshot");

    let mut persisted = empty_workspace_state();
    persisted.windows.push(sample_window(
        "issue-1",
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    ));
    persisted.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo,
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let (runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "issue-1");

    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &window_id,
            kind: gwt::KnowledgeKind::Issue,
            request_id: Some(7),
            selected_number: Some(42),
            refresh: false,
        },
    );

    assert!(
        immediate.is_empty(),
        "cache-backed knowledge load must not reply on the frontend event loop"
    );
    wait_for_recorded_event("knowledge entries dispatch", &events, |events| {
        events.iter().any(|event| {
            matches!(
                event,
                UserEvent::Dispatch(dispatched)
                    if dispatched.iter().any(|outbound| {
                        matches!(
                            &outbound.target,
                            DispatchTarget::Client(client_id) if client_id == "client-1"
                        ) && matches!(
                            &outbound.event,
                            BackendEvent::KnowledgeEntries {
                                id,
                                request_id,
                                entries,
                                selected_number,
                                ..
                            } if id == &window_id
                                && *request_id == Some(7)
                                && entries.iter().any(|entry| entry.number == 42)
                                && *selected_number == Some(42)
                        )
                    })
            )
        })
    });
    wait_for_recorded_event("knowledge detail dispatch", &events, |events| {
        events.iter().any(|event| {
            matches!(
                event,
                UserEvent::Dispatch(dispatched)
                    if dispatched.iter().any(|outbound| matches!(
                        &outbound.event,
                        BackendEvent::KnowledgeDetail { id, .. } if id == &window_id
                    ))
            )
        })
    });
}

#[test]
fn app_runtime_load_knowledge_bridge_projects_related_issue_work_sessions() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-issue-3096");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3096,
            "Fix Launch Agent trace",
            &["bug"],
            "Issue body",
            "2026-06-20T09:00:00Z",
        ))
        .expect("write issue snapshot");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo.clone(),
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let mut session =
        gwt_agent::Session::new(&worktree, "work/issue-3096", gwt_agent::AgentId::Codex);
    session.id = "session-issue-3096".to_string();
    session.agent_session_id = Some("conv-issue-3096".to_string());
    session.display_name = "Codex".to_string();
    session.linked_issue_number = Some(3096);
    session.save(&runtime.sessions_dir).expect("save session");

    let now = Utc.with_ymd_and_hms(2026, 6, 20, 9, 5, 0).unwrap();
    let work_id = gwt_core::workspace_projection::canonical_work_id(
        &repo,
        Some("work/issue-3096"),
        Some(&worktree),
    )
    .expect("work id");
    let mut event = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id,
        now,
    );
    event.title = Some("Fix Launch Agent trace".to_string());
    event.owner = Some("Issue #3096".to_string());
    event.agent_session_id = Some("session-issue-3096".to_string());
    event.agent_id = Some("codex".to_string());
    event.display_name = Some("Codex".to_string());
    event.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-3096".to_string()),
            worktree_path: Some(worktree.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, event)
        .expect("record work event");

    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &combined_window_id("tab-1", "issue-1"),
            kind: gwt::KnowledgeKind::Issue,
            request_id: None,
            selected_number: Some(3096),
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let events =
        wait_for_knowledge_view_dispatch(&recorded_events, &combined_window_id("tab-1", "issue-1"));

    let entries = match &events[0].event {
        BackendEvent::KnowledgeEntries { entries, .. } => entries,
        other => panic!("unexpected entries event: {other:?}"),
    };
    assert_eq!(entries[0].number, 3096);
    assert_eq!(entries[0].related_work_count, 1);
    assert_eq!(entries[0].related_session_count, 1);

    let detail = match &events[1].event {
        BackendEvent::KnowledgeDetail { detail, .. } => detail,
        other => panic!("unexpected detail event: {other:?}"),
    };
    assert_eq!(detail.related_works.len(), 1);
    let work = &detail.related_works[0];
    assert_eq!(work.title, "Fix Launch Agent trace");
    assert_eq!(work.branch.as_deref(), Some("work/issue-3096"));
    assert_eq!(work.agents[0].session_id, "session-issue-3096");
    assert_eq!(
        work.agents[0].sessions[0].agent_session_id,
        "conv-issue-3096"
    );
}

#[test]
fn app_runtime_load_knowledge_bridge_marks_session_only_stopped_related_session_past() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-issue-3133");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3133,
            "Resume historical Launch Agent session",
            &["bug"],
            "Issue body",
            "2026-06-20T09:00:00Z",
        ))
        .expect("write issue snapshot");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo.clone(),
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let stopped_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 5, 0).unwrap();
    let mut session =
        gwt_agent::Session::new(&worktree, "work/issue-3133", gwt_agent::AgentId::Codex);
    session.id = "session-issue-3133-stopped".to_string();
    session.agent_session_id = Some("conv-issue-3133-stopped".to_string());
    session.status = gwt_agent::AgentStatus::Stopped;
    session.linked_issue_number = Some(3133);
    session.created_at = stopped_at;
    session.updated_at = stopped_at;
    session.last_activity_at = stopped_at;
    session.save(&runtime.sessions_dir).expect("save session");

    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &combined_window_id("tab-1", "issue-1"),
            kind: gwt::KnowledgeKind::Issue,
            request_id: None,
            selected_number: Some(3133),
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let events =
        wait_for_knowledge_view_dispatch(&recorded_events, &combined_window_id("tab-1", "issue-1"));

    let detail = match &events[1].event {
        BackendEvent::KnowledgeDetail { detail, .. } => detail,
        other => panic!("unexpected detail event: {other:?}"),
    };
    assert_eq!(detail.related_works.len(), 1);
    assert_eq!(detail.related_works[0].status_category, "idle");
    assert_eq!(detail.related_works[0].agents[0].sessions.len(), 1);
    assert!(
        !detail.related_works[0].agents[0].sessions[0].is_active,
        "session-only stopped related sessions must render as Past, not Current"
    );
}

#[test]
fn app_runtime_load_knowledge_bridge_dedupes_related_issue_sessions_by_conversation() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-issue-3133");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3133,
            "Resume Launch Agent session",
            &["bug"],
            "Issue body",
            "2026-06-20T09:00:00Z",
        ))
        .expect("write issue snapshot");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo.clone(),
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let conversation_id = "conv-issue-3133";
    let old_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 0, 0).unwrap();
    let new_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 5, 0).unwrap();

    let mut stale_session =
        gwt_agent::Session::new(&worktree, "work/issue-3133", gwt_agent::AgentId::Codex);
    stale_session.id = "session-issue-3133-stale".to_string();
    stale_session.agent_session_id = Some(conversation_id.to_string());
    stale_session.status = gwt_agent::AgentStatus::Stopped;
    stale_session.linked_issue_number = Some(3133);
    stale_session.created_at = old_at;
    stale_session.updated_at = old_at;
    stale_session.last_activity_at = old_at;
    stale_session
        .save(&runtime.sessions_dir)
        .expect("save stale session");

    let mut current_session =
        gwt_agent::Session::new(&worktree, "work/issue-3133", gwt_agent::AgentId::Codex);
    current_session.id = "session-issue-3133-current".to_string();
    current_session.agent_session_id = Some(conversation_id.to_string());
    current_session.status = gwt_agent::AgentStatus::Running;
    current_session.linked_issue_number = Some(3133);
    current_session.created_at = new_at;
    current_session.updated_at = new_at;
    current_session.last_activity_at = new_at;
    current_session
        .save(&runtime.sessions_dir)
        .expect("save current session");

    let work_id = gwt_core::workspace_projection::canonical_work_id(
        &repo,
        Some("work/issue-3133"),
        Some(&worktree),
    )
    .expect("work id");
    let mut event = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id,
        new_at,
    );
    event.title = Some("Resume Launch Agent session".to_string());
    event.owner = Some("Issue #3133".to_string());
    event.agent_session_id = Some("session-issue-3133-current".to_string());
    event.agent_id = Some("codex".to_string());
    event.display_name = Some("Codex".to_string());
    event.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-3133".to_string()),
            worktree_path: Some(worktree.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, event)
        .expect("record work event");

    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &combined_window_id("tab-1", "issue-1"),
            kind: gwt::KnowledgeKind::Issue,
            request_id: None,
            selected_number: Some(3133),
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let events =
        wait_for_knowledge_view_dispatch(&recorded_events, &combined_window_id("tab-1", "issue-1"));

    let entries = match &events[0].event {
        BackendEvent::KnowledgeEntries { entries, .. } => entries,
        other => panic!("unexpected entries event: {other:?}"),
    };
    assert_eq!(entries[0].number, 3133);
    assert_eq!(entries[0].related_session_count, 1);

    let detail = match &events[1].event {
        BackendEvent::KnowledgeDetail { detail, .. } => detail,
        other => panic!("unexpected detail event: {other:?}"),
    };
    assert_eq!(detail.related_works.len(), 1);
    assert_eq!(detail.related_works[0].agents.len(), 1);
    assert_eq!(detail.related_works[0].agents[0].sessions.len(), 1);
    assert_eq!(
        detail.related_works[0].agents[0].session_id,
        "session-issue-3133-current"
    );
    assert_eq!(
        detail.related_works[0].agents[0].sessions[0].agent_session_id,
        conversation_id
    );
}

#[test]
fn app_runtime_load_knowledge_bridge_collapses_related_issue_session_actions_to_live_latest() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-issue-3133");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3133,
            "Resume Launch Agent session",
            &["bug"],
            "Issue body",
            "2026-06-20T09:00:00Z",
        ))
        .expect("write issue snapshot");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo.clone(),
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let stale_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 0, 0).unwrap();
    let past_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 1, 0).unwrap();
    let live_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 5, 0).unwrap();
    let empty_agent_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 6, 0).unwrap();

    for (id, conversation_id, status, timestamp) in [
        (
            "session-issue-3133-stale-live",
            "conv-issue-3133-live",
            gwt_agent::AgentStatus::Stopped,
            stale_at,
        ),
        (
            "session-issue-3133-past",
            "conv-issue-3133-past",
            gwt_agent::AgentStatus::Stopped,
            past_at,
        ),
        (
            "session-issue-3133-current-live",
            "conv-issue-3133-live",
            gwt_agent::AgentStatus::Running,
            live_at,
        ),
    ] {
        let mut session =
            gwt_agent::Session::new(&worktree, "work/issue-3133", gwt_agent::AgentId::Codex);
        session.id = id.to_string();
        session.agent_session_id = Some(conversation_id.to_string());
        session.status = status;
        session.linked_issue_number = Some(3133);
        session.created_at = timestamp;
        session.updated_at = timestamp;
        session.last_activity_at = timestamp;
        session.save(&runtime.sessions_dir).expect("save session");
    }

    let work_id = gwt_core::workspace_projection::canonical_work_id(
        &repo,
        Some("work/issue-3133"),
        Some(&worktree),
    )
    .expect("work id");
    for (session_id, timestamp) in [
        ("session-issue-3133-stale-live", stale_at),
        ("session-issue-3133-past", past_at),
        ("session-issue-3133-current-live", live_at),
    ] {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Start,
            work_id.clone(),
            timestamp,
        );
        event.title = Some("Resume Launch Agent session".to_string());
        event.owner = Some("Issue #3133".to_string());
        event.agent_session_id = Some(session_id.to_string());
        event.agent_id = Some("codex".to_string());
        event.display_name = Some("Codex".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/issue-3133".to_string()),
                worktree_path: Some(worktree.clone()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        gwt_core::workspace_projection::record_workspace_work_event(&repo, event)
            .expect("record work event");
    }
    let mut empty_agent_event = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id.clone(),
        empty_agent_at,
    );
    empty_agent_event.title = Some("Resume Launch Agent session".to_string());
    empty_agent_event.owner = Some("Issue #3133".to_string());
    empty_agent_event.agent_session_id = Some("session-issue-3133-empty".to_string());
    empty_agent_event.agent_id = Some("codex".to_string());
    empty_agent_event.display_name = Some("Codex".to_string());
    empty_agent_event.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-3133".to_string()),
            worktree_path: Some(worktree.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, empty_agent_event)
        .expect("record empty agent work event");

    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &combined_window_id("tab-1", "issue-1"),
            kind: gwt::KnowledgeKind::Issue,
            request_id: None,
            selected_number: Some(3133),
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let events =
        wait_for_knowledge_view_dispatch(&recorded_events, &combined_window_id("tab-1", "issue-1"));

    let entries = match &events[0].event {
        BackendEvent::KnowledgeEntries { entries, .. } => entries,
        other => panic!("unexpected entries event: {other:?}"),
    };
    assert_eq!(entries[0].number, 3133);
    assert_eq!(entries[0].related_session_count, 1);

    let detail = match &events[1].event {
        BackendEvent::KnowledgeDetail { detail, .. } => detail,
        other => panic!("unexpected detail event: {other:?}"),
    };
    assert_eq!(detail.related_works.len(), 1);
    assert_eq!(detail.related_works[0].agents.len(), 1);
    assert_eq!(
        detail.related_works[0].agents[0].session_id,
        "session-issue-3133-current-live"
    );
    assert_eq!(detail.related_works[0].agents[0].sessions.len(), 1);
    assert_eq!(
        detail.related_works[0].agents[0].sessions[0].agent_session_id,
        "conv-issue-3133-live"
    );
}

#[test]
fn app_runtime_load_knowledge_bridge_ignores_ambiguous_branch_only_related_work() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let develop_worktree = temp.path().join("repo-develop");
    let issue_worktree = temp.path().join("repo-work-issue-3133");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&develop_worktree).expect("create develop worktree");
    fs::create_dir_all(&issue_worktree).expect("create issue worktree");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3133,
            "Resume Launch Agent session",
            &["bug"],
            "Issue body",
            "2026-06-20T09:00:00Z",
        ))
        .expect("write issue snapshot");
    write_issue_link_store(
        &repo,
        HashMap::from([("work/issue-3133".to_string(), 3133)]),
    );

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo.clone(),
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let live_at = Utc.with_ymd_and_hms(2026, 6, 20, 9, 5, 0).unwrap();
    let mut session = gwt_agent::Session::new(
        &issue_worktree,
        "work/issue-3133",
        gwt_agent::AgentId::Codex,
    );
    session.id = "session-issue-3133-current".to_string();
    session.agent_session_id = Some("conv-issue-3133-current".to_string());
    session.status = gwt_agent::AgentStatus::Running;
    session.linked_issue_number = Some(3133);
    session.created_at = live_at;
    session.updated_at = live_at;
    session.last_activity_at = live_at;
    session.save(&runtime.sessions_dir).expect("save session");

    let issue_work_id = gwt_core::workspace_projection::canonical_work_id(
        &repo,
        Some("work/issue-3133"),
        Some(&issue_worktree),
    )
    .expect("issue work id");
    let mut issue_event = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        issue_work_id,
        live_at,
    );
    issue_event.title = Some("Issue #3133 visual verification".to_string());
    issue_event.owner = Some("Issue #3133".to_string());
    issue_event.agent_session_id = Some("session-issue-3133-current".to_string());
    issue_event.agent_id = Some("codex".to_string());
    issue_event.display_name = Some("Codex".to_string());
    issue_event.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-3133".to_string()),
            worktree_path: Some(issue_worktree.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, issue_event)
        .expect("record issue work event");

    let ambiguous_id = "legacy-ambiguous-branch-only";
    for (branch, worktree, at) in [
        (
            "develop",
            develop_worktree.as_path(),
            Utc.with_ymd_and_hms(2026, 6, 20, 9, 6, 0).unwrap(),
        ),
        (
            "work/issue-3133",
            issue_worktree.as_path(),
            Utc.with_ymd_and_hms(2026, 6, 20, 9, 7, 0).unwrap(),
        ),
    ] {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            ambiguous_id,
            at,
        );
        event.title = Some("Work progress summary detail".to_string());
        event.status_category =
            Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Unknown);
        event.agent_session_id = Some("missing-legacy-session".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some(branch.to_string()),
                worktree_path: Some(worktree.to_path_buf()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        gwt_core::workspace_projection::record_workspace_work_event(&repo, event)
            .expect("record ambiguous work event");
    }

    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &combined_window_id("tab-1", "issue-1"),
            kind: gwt::KnowledgeKind::Issue,
            request_id: None,
            selected_number: Some(3133),
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let events =
        wait_for_knowledge_view_dispatch(&recorded_events, &combined_window_id("tab-1", "issue-1"));

    let entries = match &events[0].event {
        BackendEvent::KnowledgeEntries { entries, .. } => entries,
        other => panic!("unexpected entries event: {other:?}"),
    };
    assert_eq!(entries[0].number, 3133);
    assert_eq!(entries[0].related_work_count, 1);
    assert_eq!(entries[0].related_session_count, 1);

    let detail = match &events[1].event {
        BackendEvent::KnowledgeDetail { detail, .. } => detail,
        other => panic!("unexpected detail event: {other:?}"),
    };
    assert_eq!(detail.related_works.len(), 1);
    assert_eq!(
        detail.related_works[0].title,
        "Issue #3133 visual verification"
    );
    assert!(
        detail
            .related_works
            .iter()
            .all(|work| work.title != "Work progress summary detail"),
        "ambiguous branch-only Work must not appear in Issue related work"
    );
}

#[test]
fn app_runtime_knowledge_search_errors_for_wrong_surface() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "shell-1",
        repo,
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    let events = runtime.search_knowledge_bridge_events(
        "client-1",
        KnowledgeSearchRequest {
            id: &window_id,
            kind: gwt::KnowledgeKind::Issue,
            query: "semantic query",
            request_id: 9,
            selected_number: None,
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::KnowledgeError {
                knowledge_kind,
                message,
                ..
            },
        }] if client_id == "client-1"
            && *knowledge_kind == gwt::KnowledgeKind::Issue
            && message == "Window is not a knowledge bridge"
    ));
}

#[cfg(unix)]
#[test]
fn app_runtime_knowledge_search_replies_through_async_dispatch() {
    let _lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    write_fake_project_index_runtime(temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let cache = Cache::new(issue_cache_root(&repo));
    cache
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Async semantic issue",
            &["bug"],
            "Search result body",
            "2026-04-20T10:00:00Z",
        ))
        .expect("write issue snapshot");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo,
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "issue-1");

    let immediate_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SearchKnowledgeBridge {
            id: window_id.clone(),
            knowledge_kind: gwt::KnowledgeKind::Issue,
            query: "semantic query".to_string(),
            request_id: 9,
            selected_number: None,
        },
    );

    assert!(
        immediate_events.is_empty(),
        "semantic search must not reply on the frontend event loop"
    );
    wait_for_recorded_event("knowledge search dispatch", &events, |events| {
        events.iter().any(|event| {
            matches!(
                event,
                UserEvent::Dispatch(dispatched)
                    if dispatched.iter().any(|outbound| {
                        matches!(
                            &outbound.target,
                            DispatchTarget::Client(client_id) if client_id == "client-1"
                        ) && matches!(
                            &outbound.event,
                            BackendEvent::KnowledgeSearchResults {
                                id,
                                knowledge_kind,
                                query,
                                request_id,
                                entries,
                                ..
                            } if id == &window_id
                                && *knowledge_kind == gwt::KnowledgeKind::Issue
                                && query == "semantic query"
                                && *request_id == 9
                                && entries.len() == 1
                                && entries[0].number == 42
                        )
                    })
            )
        })
    });
}

#[test]
fn app_runtime_manual_knowledge_refresh_replies_through_async_dispatch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let fake_gh = write_fake_gh_issue_list(temp.path());
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo,
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "issue-1");

    let immediate_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadKnowledgeBridge {
            id: window_id.clone(),
            knowledge_kind: gwt::KnowledgeKind::Issue,
            request_id: Some(31),
            selected_number: Some(43),
            refresh: true,
        },
    );

    assert!(
        immediate_events.is_empty(),
        "manual refresh must not block the frontend event loop"
    );
    wait_for_recorded_event("manual knowledge refresh dispatch", &events, |events| {
        events.iter().any(|event| {
            matches!(
                event,
                UserEvent::Dispatch(dispatched)
                    if dispatched.iter().any(|outbound| {
                        matches!(
                            &outbound.target,
                            DispatchTarget::Client(client_id) if client_id == "client-1"
                        ) && matches!(
                            &outbound.event,
                            BackendEvent::KnowledgeEntries {
                                id,
                                knowledge_kind,
                                request_id,
                                entries,
                                selected_number,
                                ..
                            } if id == &window_id
                                && *knowledge_kind == gwt::KnowledgeKind::Issue
                                && *request_id == Some(31)
                                && *selected_number == Some(43)
                                && entries.len() == 1
                                && entries[0].number == 43
                        )
                    }) && dispatched.iter().any(|outbound| {
                        matches!(
                            &outbound.event,
                            BackendEvent::KnowledgeDetail {
                                id,
                                request_id,
                                detail,
                                ..
                            } if id == &window_id
                                && *request_id == Some(31)
                                && detail.number == Some(43)
                        )
                    })
            )
        })
    });
}

#[cfg(unix)]
#[test]
fn app_runtime_manual_knowledge_refresh_uses_child_bare_repo_for_workspace_home() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let fake_gh = write_fake_gh_issue_list(temp.path());
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");

    let workspace_home = temp.path().join("workspace");
    let bare_repo = init_workspace_home_with_child_bare(&workspace_home);
    let expected_cwd = dunce::canonicalize(&bare_repo).expect("canonical bare repo");
    let _expected = ScopedEnvVar::set("GWT_FAKE_GH_EXPECT_CWD", &expected_cwd);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        workspace_home,
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "issue-1");

    let immediate_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadKnowledgeBridge {
            id: window_id.clone(),
            knowledge_kind: gwt::KnowledgeKind::Issue,
            request_id: Some(35),
            selected_number: Some(43),
            refresh: true,
        },
    );

    assert!(
        immediate_events.is_empty(),
        "manual refresh must stay asynchronous for workspace homes"
    );
    wait_for_recorded_event(
        "workspace home knowledge refresh dispatch",
        &events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    UserEvent::Dispatch(dispatched)
                        if dispatched.iter().any(|outbound| {
                            matches!(
                                &outbound.event,
                                BackendEvent::KnowledgeEntries {
                                    id,
                                    knowledge_kind,
                                    request_id,
                                    entries,
                                    selected_number,
                                    ..
                                } if id == &window_id
                                    && *knowledge_kind == gwt::KnowledgeKind::Issue
                                    && *request_id == Some(35)
                                    && *selected_number == Some(43)
                                    && entries.len() == 1
                                    && entries[0].number == 43
                            )
                        })
                )
            })
        },
    );
}

#[test]
fn app_runtime_manual_knowledge_refresh_error_preserves_request_context() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let fake_gh = write_fake_gh_issue_list(temp.path());
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo,
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "issue-1");

    let immediate_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadKnowledgeBridge {
            id: window_id.clone(),
            knowledge_kind: gwt::KnowledgeKind::Issue,
            request_id: Some(32),
            selected_number: None,
            refresh: true,
        },
    );

    assert!(
        immediate_events.is_empty(),
        "manual refresh errors must be reported asynchronously"
    );
    wait_for_recorded_event("manual knowledge refresh error", &events, |events| {
        events.iter().any(|event| {
            matches!(
                event,
                UserEvent::Dispatch(dispatched)
                    if dispatched.iter().any(|outbound| {
                        matches!(
                            &outbound.event,
                            BackendEvent::KnowledgeError {
                                id,
                                knowledge_kind,
                                request_id,
                                message,
                                ..
                            } if id == &window_id
                                && *knowledge_kind == gwt::KnowledgeKind::Issue
                                && *request_id == Some(32)
                                && message.contains("gh refresh failed")
                        )
                    })
            )
        })
    });
}

#[test]
fn app_runtime_background_knowledge_refresh_silent_paths_do_not_dispatch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let fake_gh = write_fake_gh_issue_list(temp.path());
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let marker = temp.path().join("fake-gh-called");
    let _marker = ScopedEnvVar::set("GWT_FAKE_GH_MARKER", &marker);

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "issue-1",
        repo.clone(),
        WindowPreset::Issue,
        WindowProcessStatus::Ready,
    );
    let (runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "issue-1");

    let mode_guard = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");
    runtime.spawn_knowledge_bridge_refresh(KnowledgeRefreshTask {
        client_id: "client-1".to_string(),
        id: window_id.clone(),
        project_root: repo,
        kind: gwt::KnowledgeKind::Issue,
        request_id: Some(33),
        selected_number: None,
        force: false,
        sessions_dir: runtime.sessions_dir.clone(),
        issue_link_cache_dir: runtime.issue_link_cache_dir.clone(),
    });
    wait_for_path("stale knowledge refresh gh invocation", &marker);
    assert!(
        events.lock().expect("event log").is_empty(),
        "background refresh errors should not overwrite the current cache view"
    );

    fs::remove_file(&marker).expect("remove marker");
    drop(mode_guard);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");

    runtime.spawn_knowledge_bridge_refresh(KnowledgeRefreshTask {
        client_id: "client-1".to_string(),
        id: window_id,
        project_root: temp.path().join("missing-repo"),
        kind: gwt::KnowledgeKind::Issue,
        request_id: Some(34),
        selected_number: Some(43),
        force: false,
        sessions_dir: runtime.sessions_dir.clone(),
        issue_link_cache_dir: runtime.issue_link_cache_dir.clone(),
    });
    thread::sleep(Duration::from_millis(250));
    assert!(
        events.lock().expect("event log").is_empty(),
        "noop background refresh should return silently without dispatch"
    );
}

#[test]
fn app_runtime_load_knowledge_bridge_keeps_pr_surface_disabled_until_cache_support_exists() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "pr-1",
        repo,
        WindowPreset::Pr,
        WindowProcessStatus::Ready,
    );
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let immediate = runtime.load_knowledge_bridge_events(
        "client-1",
        KnowledgeLoadRequest {
            id: &combined_window_id("tab-1", "pr-1"),
            kind: gwt::KnowledgeKind::Pr,
            request_id: None,
            selected_number: None,
            refresh: false,
        },
    );
    assert!(immediate.is_empty());
    let events =
        wait_for_knowledge_view_dispatch(&recorded_events, &combined_window_id("tab-1", "pr-1"));

    assert_eq!(events.len(), 2);
    assert!(matches!(
        &events[0].event,
        BackendEvent::KnowledgeEntries {
            knowledge_kind,
            entries,
            refresh_enabled,
            empty_message,
            ..
        } if *knowledge_kind == gwt::KnowledgeKind::Pr
            && entries.is_empty()
            && !*refresh_enabled
            && empty_message.as_deref().is_some_and(|message| message.contains("cache-backed PR list support"))
    ));
    assert!(matches!(
        &events[1].event,
        BackendEvent::KnowledgeDetail { detail, .. }
            if detail.sections.iter().any(|section| section.body.contains("cache-backed PR list support"))
    ));
}

#[test]
fn app_runtime_load_profile_replies_with_config_backed_snapshot() {
    let temp = tempdir().expect("tempdir");
    let config_path = temp.path().join("profile-config.toml");
    let mut settings = Settings::default();
    settings
        .profiles
        .add(Profile::new("dev"))
        .expect("add profile");
    settings.profiles.switch("dev").expect("switch active");
    settings
        .profiles
        .set_env_var("dev", "API_KEY", "override")
        .expect("set env var");
    write_profile_config(&config_path, &settings);

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "profile-1",
        repo,
        WindowPreset::Profile,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "profile-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadProfile {
            id: window_id.clone(),
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::ProfileSnapshot { id, snapshot },
        }] if client_id == "client-1"
            && id == &window_id
            && snapshot.active_profile == "dev"
            && snapshot.selected_profile == "dev"
            && snapshot.profiles.iter().any(|profile|
                profile.name == "dev"
                    && profile.is_active
                    && profile.env_vars.iter().any(|entry|
                        entry.key == "API_KEY" && entry.value == "override"
                    )
            )
    ));
}

#[test]
fn app_runtime_select_and_save_profile_broadcasts_snapshot_to_profile_windows() {
    let temp = tempdir().expect("tempdir");
    let config_path = temp.path().join("profile-config.toml");
    let mut settings = Settings::default();
    settings
        .profiles
        .add(Profile::new("dev"))
        .expect("add profile");
    write_profile_config(&config_path, &settings);

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut persisted = empty_workspace_state();
    persisted.windows.push(sample_window(
        "profile-1",
        WindowPreset::Profile,
        WindowProcessStatus::Ready,
    ));
    persisted.windows.push(sample_window(
        "profile-2",
        WindowPreset::Profile,
        WindowProcessStatus::Ready,
    ));
    persisted.next_z_index = 3;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo,
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let current_window_id = combined_window_id("tab-1", "profile-1");
    let sibling_window_id = combined_window_id("tab-1", "profile-2");

    let select_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SelectProfile {
            id: current_window_id.clone(),
            profile_name: "dev".to_string(),
        },
    );
    assert!(matches!(
        &select_events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::ProfileSnapshot { id, snapshot },
        }] if client_id == "client-1"
            && id == &current_window_id
            && snapshot.selected_profile == "dev"
    ));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SaveProfile {
            id: current_window_id.clone(),
            current_name: "dev".to_string(),
            name: "review".to_string(),
            description: "Review profile".to_string(),
            env_vars: vec![ProfileEnvEntryView {
                key: "API_KEY".to_string(),
                value: "override".to_string(),
            }],
            disabled_env: vec!["SECRET".to_string()],
        },
    );

    assert_eq!(events.len(), 2);
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Broadcast,
            event: BackendEvent::ProfileSnapshot { id, snapshot },
        } if id == &current_window_id
            && snapshot.selected_profile == "review"
            && snapshot.active_profile == "default"
            && snapshot.profiles.iter().any(|profile|
                profile.name == "review"
                    && profile.env_vars.iter().any(|entry|
                        entry.key == "API_KEY" && entry.value == "override"
                    )
            )
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Broadcast,
            event: BackendEvent::ProfileSnapshot { id, snapshot },
        } if id == &sibling_window_id
            && snapshot.selected_profile == "default"
            && snapshot.profiles.iter().any(|profile| profile.name == "review")
    )));

    let saved = Settings::load_from_path(&config_path).expect("load saved config");
    assert!(saved
        .profiles
        .profiles
        .iter()
        .any(|profile| profile.name == "review" && profile.description == "Review profile"));
}

#[test]
fn app_runtime_logs_profile_save_user_action_without_env_values() {
    let temp = tempdir().expect("tempdir");
    let _config_home = ScopedEnvVar::set("GWT_CONFIG_HOME", temp.path());
    Settings::default()
        .save(&temp.path().join("config.toml"))
        .expect("save settings");

    let mut runtime = sample_runtime(temp.path(), vec![], None);
    let events = capture_tracing_events(|| {
        let _ = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::SaveProfile {
                id: "profile-window".to_string(),
                current_name: "default".to_string(),
                name: "default".to_string(),
                description: String::new(),
                env_vars: vec![ProfileEnvEntryView {
                    key: "Test".to_string(),
                    value: "must-not-leak".to_string(),
                }],
                disabled_env: vec![],
            },
        );
    });

    let action = events
        .iter()
        .find(|event| event.target == "gwt_ui_action")
        .expect("profile save user action log");
    assert_eq!(action.level, Level::INFO);
    assert_eq!(
        action.fields.get("action").map(String::as_str),
        Some("save_profile")
    );
    assert_eq!(
        action.fields.get("profile_name").map(String::as_str),
        Some("default")
    );
    assert_eq!(
        action.fields.get("env_keys").map(String::as_str),
        Some("Test")
    );
    assert_eq!(
        action.fields.get("env_var_count").map(String::as_str),
        Some("1")
    );
    assert!(
        !action
            .fields
            .values()
            .any(|value| value.contains("must-not-leak")),
        "env values must not be written to the user action log: {action:?}"
    );
}

#[test]
fn frontend_user_action_redacts_backend_test_url_secrets() {
    let custom_agent_log = super::frontend_user_action_log(&FrontendEvent::TestBackendConnection {
        base_url: "https://user:pass@example.com/v1?token=secret#frag".to_string(),
        api_key: "api-key-must-not-leak".to_string(),
    })
    .expect("custom agent backend test action log");
    assert_eq!(custom_agent_log.ui_target, "https://example.com");

    let builtin_agent_log =
        super::frontend_user_action_log(&FrontendEvent::TestAgentBackendConnection {
            agent: gwt_agent::BuiltinAgentId::Codex,
            base_url: "http://token@example.net:11434/openai?signed=secret".to_string(),
            api_key: "agent-key-must-not-leak".to_string(),
        })
        .expect("builtin agent backend test action log");
    assert_eq!(builtin_agent_log.ui_target, "http://example.net:11434");

    let logged_values = [
        custom_agent_log.ui_target.as_str(),
        builtin_agent_log.ui_target.as_str(),
    ];
    assert!(
        !logged_values.iter().any(|value| value.contains("user")
            || value.contains("pass")
            || value.contains("token")
            || value.contains("secret")),
        "backend test URLs must not leak credentials or query strings: {logged_values:?}"
    );
}

#[test]
fn frontend_user_action_logs_project_index_search_without_query_values() {
    let log = super::frontend_user_action_log(&FrontendEvent::SearchProjectIndex {
        id: "index-window".to_string(),
        query: "secret query".to_string(),
        request_id: 7,
        scopes: vec![
            gwt::IndexSearchScope::Issues,
            gwt::IndexSearchScope::FilesDocs,
        ],
        worktree_hash: Some("worktree-hash".to_string()),
        match_mode: gwt::IndexSearchMatchMode::AllTerms,
    })
    .expect("project index search user action log");

    assert_eq!(log.action, "search_project_index");
    assert_eq!(log.surface, "index");
    assert_eq!(log.window_id, "index-window");
    assert_eq!(log.mode, "files-docs,issues");
    assert_eq!(log.agent_id, "worktree-hash");
    assert_eq!(log.count, "secret query".len());

    let logged_values = [
        log.window_id.as_str(),
        log.ui_target.as_str(),
        log.profile_name.as_str(),
        log.env_keys.as_str(),
        log.agent_id.as_str(),
        log.mode.as_str(),
    ];
    assert!(
        !logged_values.iter().any(|value| value.contains("secret")),
        "project index search query must not be written to the user action log: {logged_values:?}"
    );
}

#[test]
fn frontend_user_action_logs_issue_monitor_global_profile_configure() {
    let log = super::frontend_user_action_log(&FrontendEvent::IssueMonitorConfigureProfile)
        .expect("issue monitor configure profile user action log");

    assert_eq!(log.action, "issue_monitor_configure_profile");
    assert_eq!(log.surface, "issue_monitor");
    assert_eq!(log.ui_target, "");
}

#[test]
fn app_runtime_load_logs_replies_with_current_log_snapshot() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "logs-1",
        repo,
        WindowPreset::Logs,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "logs-1");
    let log_path = current_log_file(&runtime.log_dir);
    // Write the canonical on-disk JSONL shape produced by
    // `tracing_subscriber::fmt::layer().json()` (see
    // `crates/gwt-core/src/logging/fmt_layer.rs`) so the reader exercises
    // the production format end-to-end (SPEC-1924 FR-035).
    fs::write(
        &log_path,
        "{\"timestamp\":\"2026-05-20T09:00:00.000000+00:00\",\
             \"level\":\"WARN\",\
             \"fields\":{\"message\":\"runtime stalled\",\"detail\":\"retrying read\"},\
             \"target\":\"pty\"}\n",
    )
    .expect("write log snapshot");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadLogs {
            id: window_id.clone(),
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::LogEntries { id, entries },
        }] if client_id == "client-1"
            && id == &window_id
            && entries.len() == 1
            && entries[0].message == "runtime stalled"
            && entries[0].detail.as_deref() == Some("retrying read")
            && entries[0].source == "pty"
            && matches!(entries[0].severity, LogLevel::Warn)
    ));
}

/// SPEC-1924 US-14 / FR-036 / SC-010 — when canonical log file contains
/// malformed lines, the Logs window receives the surviving entries plus
/// exactly one Warning notice via `LogEntryAppended`.
#[test]
fn app_runtime_load_logs_emits_warning_for_skipped_lines() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "logs-1",
        repo,
        WindowPreset::Logs,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "logs-1");
    let log_path = current_log_file(&runtime.log_dir);
    let good = "{\"timestamp\":\"2026-05-20T09:00:00.000000+00:00\",\
            \"level\":\"INFO\",\"fields\":{\"message\":\"ok\"},\"target\":\"gwt\"}";
    let malformed = "{\"foo\":\"bar\"}";
    fs::write(&log_path, format!("{good}\n{malformed}\n{good}\n")).expect("write log snapshot");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LoadLogs {
            id: window_id.clone(),
        },
    );

    assert_eq!(
        events.len(),
        2,
        "expected LogEntries + LogEntryAppended for skipped notice, got {:?}",
        events
    );

    let entries_match = matches!(
        &events[0],
        OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::LogEntries { id, entries },
        } if client_id == "client-1"
            && id == &window_id
            && entries.len() == 2
            && entries.iter().all(|e| e.message == "ok")
    );
    assert!(
        entries_match,
        "first event must be LogEntries: {:?}",
        events[0]
    );

    let warning_match = matches!(
        &events[1],
        OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::LogEntryAppended { entry },
        } if client_id == "client-1"
            && entry.severity == LogLevel::Warn
            && entry.source == "gwt_core::logging::reader"
            && entry.message.contains("Skipped 1 malformed line")
    );
    assert!(
        warning_match,
        "second event must be a Warn LogEntryAppended notice: {:?}",
        events[1]
    );
}

#[test]
fn app_runtime_save_ui_trace_replies_with_artifact_path() {
    let temp = tempdir().expect("tempdir");
    let mut runtime = sample_runtime(temp.path(), vec![], None);

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SaveUiTrace {
            trace: serde_json::from_value::<UiTracePayload>(serde_json::json!({
                "session_id": "trace-1",
                "entries": [
                    { "kind": "trace_start", "ts": 1 }
                ]
            }))
            .expect("typed ui trace payload"),
        },
    );

    assert!(matches!(
        &events[..],
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::UiTraceSaved { path, entries },
        }] if client_id == "client-1" && *entries == 1 && Path::new(path).exists()
    ));
}

#[test]
fn app_runtime_post_board_entry_persists_reply_topics_and_owners() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let parent = post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Question,
            "Can someone verify this?",
            None,
            None,
            vec!["coordination".to_string()],
            vec!["2018".to_string()],
        ),
    )
    .expect("seed board parent")
    .board
    .entries
    .into_iter()
    .next()
    .expect("parent entry");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo.clone(),
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "board-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::PostBoardEntry {
            id: window_id.clone(),
            entry_kind: BoardEntryKind::Next,
            body: "I will take the next slice".to_string(),
            title: None,
            target_workspace: None,
            broadcast: false,
            parent_id: Some(parent.id.clone()),
            topics: vec!["coordination".to_string(), "phase-1b".to_string()],
            owners: vec!["2018".to_string()],
            targets: Vec::new(),
            mentions: vec![
                BoardMention::new(BoardMentionTargetKind::User, "akiojin").with_label("Akio")
            ],
        },
    );

    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::BoardEntries { id, entries, .. },
        } if client_id == "client-1"
            && id == &window_id
            && entries.iter().any(|entry|
                entry.body == "I will take the next slice"
                && entry.parent_id.as_deref() == Some(parent.id.as_str())
                && entry.related_topics == vec!["coordination".to_string(), "phase-1b".to_string()]
                && entry.related_owners == vec!["2018".to_string()]
                && entry.mentions.len() == 1
                && entry.mentions[0].typed_key() == "user:akiojin"
            )
    )));

    let snapshot = load_snapshot(&repo).expect("load board snapshot");
    assert!(snapshot
        .board
        .entries
        .iter()
        .any(|entry| entry.body == "I will take the next slice"
            && entry.parent_id.as_deref() == Some(parent.id.as_str())
            && entry.related_topics == vec!["coordination".to_string(), "phase-1b".to_string()]
            && entry.related_owners == vec!["2018".to_string()]
            && entry.mentions.len() == 1
            && entry.mentions[0].typed_key() == "user:akiojin"));
}

#[test]
fn app_runtime_post_board_entry_accepts_reply_to_history_parent() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let parent_id = "history-parent".to_string();
    let events_path = coordination_events_path(&repo);
    fs::create_dir_all(
        events_path
            .parent()
            .expect("coordination event log has parent"),
    )
    .expect("create coordination dir");
    let mut events = fs::File::create(&events_path).expect("create legacy event log");
    for idx in 0..505 {
        let mut entry = BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            format!("history entry {idx}"),
            None,
            None,
            vec![],
            vec![],
        );
        if idx == 0 {
            entry.id = parent_id.clone();
        }
        serde_json::to_writer(&mut events, &CoordinationEvent::MessageAppended { entry })
            .expect("write board seed event");
        events.write_all(b"\n").expect("write board seed newline");
    }
    events.flush().expect("flush board seed events");
    let snapshot = load_snapshot(&repo).expect("load board snapshot");
    assert!(!snapshot
        .board
        .entries
        .iter()
        .any(|entry| entry.id == parent_id));
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo.clone(),
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "board-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::PostBoardEntry {
            id: window_id.clone(),
            entry_kind: BoardEntryKind::Next,
            body: "Reply to older context".to_string(),
            title: None,
            target_workspace: None,
            broadcast: false,
            parent_id: Some(parent_id.clone()),
            topics: vec![],
            owners: vec![],
            targets: Vec::new(),
            mentions: Vec::new(),
        },
    );

    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::BoardEntries { id, entries, .. },
        } if client_id == "client-1"
            && id == &window_id
            && entries.iter().any(|entry|
                entry.body == "Reply to older context"
                && entry.parent_id.as_deref() == Some(parent_id.as_str())
            )
    )));
}

#[test]
fn app_runtime_post_board_entry_updates_workspace_projection_current_state() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo.clone(),
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "board-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::PostBoardEntry {
            id: window_id,
            entry_kind: BoardEntryKind::Next,
            body: "Run final verification".to_string(),
            title: None,
            target_workspace: None,
            broadcast: false,
            parent_id: None,
            topics: vec!["start-work".to_string()],
            owners: vec!["SPEC-2359".to_string()],
            targets: Vec::new(),
            mentions: Vec::new(),
        },
    );

    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load projection")
        .expect("projection");
    let board_entry_id = projection
        .board_refs
        .first()
        .expect("workspace board ref")
        .clone();

    assert_eq!(
        projection.next_action.as_deref(),
        Some("Run final verification")
    );
    assert_eq!(projection.owner.as_deref(), Some("SPEC-2359"));
    assert_eq!(projection.board_refs.len(), 1);
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load work items")
        .expect("work items");
    assert_eq!(
        work_items.work_items[0].board_refs,
        vec![board_entry_id.clone()]
    );
    assert_eq!(
        work_items.work_items[0].events[0].kind,
        gwt_core::workspace_projection::WorkEventKind::Update
    );
    let projected_event = events
        .iter()
        .find_map(|event| match event {
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::ActiveWorkProjection { projection },
            } => Some(projection),
            _ => None,
        })
        .expect("active work projection broadcast");
    assert_eq!(projected_event.board_refs, vec![board_entry_id.clone()]);
    assert_eq!(projected_event.active_agents, 0);
    assert_eq!(projected_event.status_category, "idle");
    assert_eq!(projected_event.next_action, None);
}

#[test]
fn app_runtime_board_milestone_from_unassigned_origin_does_not_create_workspace_history() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo.clone(),
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection
        .agents
        .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: "session-unassigned".to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: Some("Investigate Workspace materialization".to_string()),
            title_summary: Some("Work materialization".to_string()),
            worktree_path: None,
            branch: Some("work/unassigned".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned,
            workspace_id: None,
            updated_at: chrono::Utc::now(),
        });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let entry = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Claim,
        "Unassigned claim without materialization must not pollute current Workspace history.",
        None,
        None,
        vec!["workspace-materialization".to_string()],
        vec!["2359".to_string()],
    )
    .with_title_summary("Work materialization")
    .with_origin_session_id("session-unassigned");

    runtime.record_workspace_board_milestone_event("tab-1", &repo, &entry);

    let saved = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load projection")
        .expect("projection");
    let agent = saved
        .agents
        .iter()
        .find(|agent| agent.session_id == "session-unassigned")
        .expect("agent");
    assert_eq!(
        agent.affiliation_status,
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned
    );
    assert!(
        gwt_core::workspace_projection::load_workspace_work_items(&repo)
            .expect("load workspace history")
            .is_none(),
        "Unassigned origin Board entries must not append to unrelated Workspace history"
    );
}

#[test]
fn app_runtime_board_milestone_uses_latest_duplicate_session_assignment() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo.clone(),
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let old_at = chrono::Utc::now() - chrono::Duration::minutes(2);
    let current_at = chrono::Utc::now() - chrono::Duration::minutes(1);
    let stale = gwt_core::workspace_projection::WorkspaceAgentSummary {
        session_id: "session-duplicate-assigned".to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
        current_focus: Some("stale".to_string()),
        title_summary: None,
        worktree_path: None,
        branch: Some("feature/stale".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status:
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned,
        workspace_id: None,
        updated_at: old_at,
    };
    let mut assigned = stale.clone();
    assigned.current_focus = Some("current".to_string());
    assigned.branch = Some("feature/current".to_string());
    assigned.affiliation_status =
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned;
    assigned.workspace_id = Some("work-current-duplicate".to_string());
    assigned.updated_at = current_at;
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.append(&mut vec![stale, assigned]);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    gwt_core::workspace_projection::record_workspace_work_paused_event(
        &repo,
        "work-current-duplicate",
        Some("Current Work"),
        None,
        None,
        &[],
        None,
        Some("session-duplicate-assigned"),
        current_at,
    )
    .expect("seed current Work");
    let entry = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "Current assigned Session milestone.",
        None,
        None,
        vec!["workspace-assignment".to_string()],
        vec!["2359".to_string()],
    )
    .with_origin_session_id("session-duplicate-assigned");

    runtime.record_workspace_board_milestone_event("tab-1", &repo, &entry);

    let works = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load Work history")
        .expect("Work history");
    let current = works
        .work_items
        .iter()
        .find(|item| item.id == "work-current-duplicate")
        .expect("current assigned Work");
    assert!(
        current.board_refs.iter().any(|id| id == &entry.id),
        "the latest assigned duplicate row must receive the Board event"
    );
}

#[test]
fn app_runtime_old_board_milestone_does_not_rewind_latest_assigned_work_or_agent() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "board-1",
        repo.clone(),
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let stale_agent_at = chrono::Utc::now() - chrono::Duration::minutes(4);
    let assigned_agent_at = chrono::Utc::now() - chrono::Duration::minutes(3);
    let replayed_at = chrono::Utc::now() - chrono::Duration::minutes(2);
    let current_work_at = chrono::Utc::now() - chrono::Duration::minutes(1);
    let assigned = gwt_core::workspace_projection::WorkspaceAgentSummary {
        session_id: "session-duplicate-assigned".to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Blocked,
        current_focus: Some("Current blocked work".to_string()),
        title_summary: None,
        worktree_path: None,
        branch: Some("feature/current".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status:
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: Some("work-current-duplicate".to_string()),
        updated_at: assigned_agent_at,
    };
    let mut stale = assigned.clone();
    stale.current_focus = Some("Stale work".to_string());
    stale.branch = Some("feature/stale".to_string());
    stale.affiliation_status =
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned;
    stale.workspace_id = None;
    stale.updated_at = stale_agent_at;
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.agents.append(&mut vec![stale, assigned]);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    gwt_core::workspace_projection::record_workspace_work_paused_event(
        &repo,
        "work-current-duplicate",
        Some("Current Work"),
        None,
        None,
        &[],
        None,
        Some("session-duplicate-assigned"),
        current_work_at,
    )
    .expect("seed current Work");
    let mut replayed = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "Old assigned Session milestone.",
        None,
        None,
        vec!["workspace-assignment".to_string()],
        vec!["stale-owner".to_string()],
    )
    .with_origin_session_id("session-duplicate-assigned");
    replayed.updated_at = replayed_at;

    runtime.record_workspace_board_milestone_event("tab-1", &repo, &replayed);

    let works = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load Work history")
        .expect("Work history");
    let current = works
        .work_items
        .iter()
        .find(|item| item.id == "work-current-duplicate")
        .expect("current assigned Work");
    assert_eq!(
        current.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Idle
    );
    assert_eq!(current.title, "Current Work");
    assert_eq!(current.updated_at, current_work_at);
    assert!(!current.board_refs.iter().any(|id| id == &replayed.id));
    let current_projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load current projection")
        .expect("current projection");
    assert_eq!(
        current_projection.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Unknown
    );
    assert!(current_projection
        .board_refs
        .iter()
        .any(|id| id == &replayed.id));
    let current_agent = current_projection
        .latest_agent_for_session("session-duplicate-assigned")
        .expect("current assigned Agent");
    assert_eq!(
        current_agent.current_focus.as_deref(),
        Some("Current blocked work")
    );
    assert_eq!(
        current_agent.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Blocked
    );
    assert_eq!(current_agent.last_board_entry_id, None);
    assert_eq!(current_agent.last_board_entry_kind, None);
    assert_eq!(current_agent.updated_at, assigned_agent_at);
}

#[test]
fn app_runtime_active_work_projection_preserves_blocked_agent_board_state() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-20260504-1234");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Board],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let session = ActiveAgentSession {
        window_id: "tab-1::agent-1".to_string(),
        session_id: "session-1".to_string(),
        agent_id: "codex".to_string(),
        branch_name: "work/20260504-1234".to_string(),
        display_name: "Codex".to_string(),
        worktree_path: worktree.clone(),
        agent_project_root: worktree.display().to_string(),
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        tab_id: "tab-1".to_string(),
    };
    runtime
        .active_agent_sessions
        .insert(session.window_id.clone(), session.clone());
    save_assigned_workspace_projection_for_test(&repo, &session).expect("save initial projection");
    let blocked = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Blocked,
        "Waiting for API credentials",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1")
    .with_origin_agent_id("codex")
    .with_origin_branch("work/20260504-1234");

    let events = runtime.record_workspace_board_milestone_event("tab-1", &repo, &blocked);
    let event = events
        .iter()
        .find(|e| matches!(e.event, BackendEvent::ActiveWorkProjection { .. }))
        .cloned()
        .expect("active projection broadcast");

    assert!(matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Broadcast,
            event: BackendEvent::ActiveWorkProjection { projection },
        } if projection.status_category == "blocked"
            && projection.blocked_agents == 1
            && projection.agents.iter().any(|agent|
                agent.session_id == "session-1"
                    && agent.status_category == "blocked"
                    && agent.last_board_entry_id.as_deref() == Some(blocked.id.as_str())
            )
            && projection.board_refs == vec![blocked.id.clone()]
            && projection.next_action.as_deref() == Some("Resolve blocker")
    ));
}

#[test]
fn app_runtime_active_work_projection_prioritizes_handoff_agents() {
    use gwt_core::workspace_projection::{
        WorkspaceAgentSummary, WorkspaceProjection, WorkspaceStatusCategory,
    };

    let mut projection = WorkspaceProjection::default_for_project("/repo");
    let now = chrono::Utc::now();
    projection.agents.push(WorkspaceAgentSummary {
        session_id: "session-active".to_string(),
        window_id: Some("tab-1::agent-active".to_string()),
        agent_id: "codex".to_string(),
        display_name: "Alpha".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: Some("Implementing tests".to_string()),
        title_summary: None,
        worktree_path: None,
        branch: Some("work/20260504-1234".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status:
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: None,
        updated_at: now,
    });
    projection.agents.push(WorkspaceAgentSummary {
        session_id: "session-handoff".to_string(),
        window_id: Some("tab-1::agent-handoff".to_string()),
        agent_id: "codex".to_string(),
        display_name: "Zulu".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: Some("Review visual state coverage".to_string()),
        title_summary: None,
        worktree_path: None,
        branch: Some("work/20260504-1234".to_string()),
        last_board_entry_id: Some("board-handoff".to_string()),
        last_board_entry_kind: Some(BoardEntryKind::Handoff),
        coordination_scope: Some("SPEC-2359 / workspace-ux".to_string()),
        affiliation_status:
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: None,
        updated_at: now,
    });

    let view = active_work_projection_from_saved(projection);

    assert_eq!(view.agents[0].session_id, "session-handoff");
    assert_eq!(
        view.agents[0].last_board_entry_kind.as_deref(),
        Some("handoff")
    );
    assert_eq!(
        view.agents[0].coordination_scope.as_deref(),
        Some("SPEC-2359 / workspace-ux")
    );
}

#[test]
fn app_runtime_active_work_projection_recovers_blocked_agent_after_status_milestone() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-20260504-1234");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Board],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let session = ActiveAgentSession {
        window_id: "tab-1::agent-1".to_string(),
        session_id: "session-1".to_string(),
        agent_id: "codex".to_string(),
        branch_name: "work/20260504-1234".to_string(),
        display_name: "Codex".to_string(),
        worktree_path: worktree.clone(),
        agent_project_root: worktree.display().to_string(),
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        tab_id: "tab-1".to_string(),
    };
    runtime
        .active_agent_sessions
        .insert(session.window_id.clone(), session.clone());
    save_assigned_workspace_projection_for_test(&repo, &session).expect("save initial projection");
    let blocked = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Blocked,
        "Waiting for API credentials",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1");
    runtime.record_workspace_board_milestone_event("tab-1", &repo, &blocked);
    let status = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "API credentials configured",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1");

    let events = runtime.record_workspace_board_milestone_event("tab-1", &repo, &status);
    let event = events
        .iter()
        .find(|e| matches!(e.event, BackendEvent::ActiveWorkProjection { .. }))
        .cloned()
        .expect("active projection broadcast");

    assert!(matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Broadcast,
            event: BackendEvent::ActiveWorkProjection { projection },
        } if projection.status_category == "active"
            && projection.active_agents == 1
            && projection.blocked_agents == 0
            && projection.branch.as_deref() == Some("work/20260504-1234")
    ));
}

#[test]
fn app_runtime_active_work_projection_keeps_blocked_agent_after_next_milestone() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-20260504-1234");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Board],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let session = ActiveAgentSession {
        window_id: "tab-1::agent-1".to_string(),
        session_id: "session-1".to_string(),
        agent_id: "codex".to_string(),
        branch_name: "work/20260504-1234".to_string(),
        display_name: "Codex".to_string(),
        worktree_path: worktree.clone(),
        agent_project_root: worktree.display().to_string(),
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        tab_id: "tab-1".to_string(),
    };
    runtime
        .active_agent_sessions
        .insert(session.window_id.clone(), session.clone());
    save_assigned_workspace_projection_for_test(&repo, &session).expect("save initial projection");
    let blocked = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Blocked,
        "Waiting for API credentials",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1");
    runtime.record_workspace_board_milestone_event("tab-1", &repo, &blocked);
    let next = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Next,
        "Try alternate credential source",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1");

    let events = runtime.record_workspace_board_milestone_event("tab-1", &repo, &next);
    let event = events
        .iter()
        .find(|e| matches!(e.event, BackendEvent::ActiveWorkProjection { .. }))
        .cloned()
        .expect("blocked projection broadcast");

    assert!(matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Broadcast,
            event: BackendEvent::ActiveWorkProjection { projection },
        } if projection.status_category == "blocked"
            && projection.active_agents == 0
            && projection.blocked_agents == 1
            && projection.status_text == "Waiting for API credentials"
            && projection.next_action.as_deref() == Some("Try alternate credential source")
            && projection.branch.as_deref() == Some("work/20260504-1234")
    ));
}

#[test]
fn app_runtime_agent_window_initial_title_uses_linked_issue_title() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            2359,
            "SPEC: Workspace purpose titles",
            &["gwt-spec"],
            "Spec body",
            "2026-05-06T00:00:00Z",
        ))
        .expect("write issue cache");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/20260506-0736")
        .linked_issue_number(2359)
        .build();

    runtime
        .spawn_agent_window("tab-1", config, canvas_bounds(), None)
        .expect("spawn agent window");

    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window");
    assert_eq!(
        agent_window.purpose_title.as_deref(),
        Some("SPEC: Workspace purpose titles")
    );
    assert_eq!(agent_window.title, "Codex");
}

#[test]
fn app_runtime_issue_monitor_enable_opens_single_settings_wizard_without_launch_settings() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let fake_gh = write_fake_gh_issue_list(temp.path());
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3165,
            "SPEC: Issue auto-improve monitor",
            &["gwt-spec"],
            "Spec body",
            "2026-06-23T00:00:00Z",
        ))
        .expect("write issue cache");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetIssueMonitorEnabled { enabled: true },
    );

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("status resets optimistic enabled UI");
    assert!(
        !status.enabled,
        "Start without saved profile must return monitor status to stopped"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::IssueMonitorInbox { .. })),
        "Start without saved profile must not publish cached inbox yet"
    );
    assert!(
        runtime.launch_wizard.is_some(),
        "Start without launch settings should open one settings wizard"
    );
    assert!(runtime
        .launch_wizard
        .as_ref()
        .and_then(|session| session.issue_monitor_profile_save.as_ref())
        .is_some_and(|context| context.issue_number.is_none()));
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::LaunchWizardState { .. })),
        "settings wizard should be broadcast immediately"
    );
    assert!(
        runtime.window_details.is_empty(),
        "settings-required Start must not spawn an agent window"
    );
    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .unwrap_or_default();
    assert!(
        !prefs.enabled,
        "Start without saved profile must not persist enabled state"
    );
}

#[test]
fn app_runtime_issue_monitor_enable_reports_missing_origin_detail() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_without_origin(&repo);
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("save issue monitor prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetIssueMonitorEnabled { enabled: true },
    );

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    let error = status.last_error.as_deref().expect("origin error");
    assert!(
        error.starts_with("Git origin remote is not configured"),
        "unexpected error: {error}"
    );
    assert_ne!(error, "GitHub origin remote is unavailable");
}

#[test]
fn app_runtime_issue_monitor_reorder_persists_and_reorders_cached_inbox() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let fake_gh = write_fake_gh_issue_list(temp.path());
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let cache = Cache::new(issue_cache_root(&repo));
    for number in [3165, 3166, 3167] {
        cache
            .write_snapshot(&sample_issue_snapshot(
                number,
                &format!("Issue {number}"),
                &["bug"],
                "Issue body",
                "2026-06-23T00:00:00Z",
            ))
            .expect("write issue cache");
    }
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ReorderIssueMonitorIssues {
            issue_numbers: vec![3167, 3165, 3166],
        },
    );

    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    let visible_numbers: Vec<u64> = inbox.iter().map(|item| item.issue.number).collect();
    assert_eq!(visible_numbers, vec![3167, 3165, 3166]);

    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("load issue monitor prefs");
    assert_eq!(prefs.priority_order, vec![3167, 3165, 3166]);
}

#[test]
fn app_runtime_quick_register_issue_creates_issue_cache_and_inbox_entry() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let fake_client = Arc::new(FakeIssueClient::new());
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.issue_client_factory = Arc::new({
        let fake_client = Arc::clone(&fake_client);
        move |_owner, _repo| {
            let client: Arc<dyn IssueClient> = fake_client.clone();
            Ok(client)
        }
    });

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::QuickRegisterIssue {
            title: "Investigate Intake registration".to_string(),
            launch: false,
        },
    );

    assert_eq!(fake_client.call_log(), vec!["create_issue:#1"]);

    let cached = Cache::new(issue_cache_root(&repo))
        .load_entry(IssueNumber(1))
        .expect("quick issue written to cache");
    assert_eq!(cached.snapshot.title, "Investigate Intake registration");
    assert!(cached.snapshot.labels.is_empty());
    for heading in [
        "## Summary",
        "## Background",
        "## Spec Status",
        "## Related SPECs",
        "## Expected Outcome",
        "## Notes",
    ] {
        assert!(
            cached.snapshot.body.contains(heading),
            "quick issue body should contain {heading}: {}",
            cached.snapshot.body
        );
    }
    assert!(
        cached.snapshot.body.contains("compatibility path")
            && cached
                .snapshot
                .body
                .contains("gwt-register-issue remains the primary intake workflow"),
        "quick issue body must describe the withdrawn toolbar path as a compatibility guard: {}",
        cached.snapshot.body
    );

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue registered" && *issue_number == Some(1)
        )
    }));
    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    let item = inbox
        .iter()
        .find(|item| item.issue.number == 1)
        .expect("registered issue appears in monitor inbox");
    assert_eq!(item.issue.title, "Investigate Intake registration");
    assert_eq!(item.state, gwt::MonitorInboxState::Queued);
}

struct PermissionDeniedCreateIssueClient;

impl IssueClient for PermissionDeniedCreateIssueClient {
    fn fetch(
        &self,
        _number: IssueNumber,
        _since: Option<&UpdatedAt>,
    ) -> Result<FetchResult, ApiError> {
        unreachable!("quick register only creates issues")
    }

    fn patch_body(&self, _number: IssueNumber, _new_body: &str) -> Result<IssueSnapshot, ApiError> {
        unreachable!("quick register only creates issues")
    }

    fn patch_title(
        &self,
        _number: IssueNumber,
        _new_title: &str,
    ) -> Result<IssueSnapshot, ApiError> {
        unreachable!("quick register only creates issues")
    }

    fn patch_comment(
        &self,
        _comment_id: CommentId,
        _new_body: &str,
    ) -> Result<CommentSnapshot, ApiError> {
        unreachable!("quick register only creates issues")
    }

    fn create_comment(
        &self,
        _number: IssueNumber,
        _body: &str,
    ) -> Result<CommentSnapshot, ApiError> {
        unreachable!("quick register only creates issues")
    }

    fn delete_comment(&self, _comment_id: CommentId) -> Result<(), ApiError> {
        unreachable!("quick register only creates issues")
    }

    fn create_issue(
        &self,
        _title: &str,
        _body: &str,
        _labels: &[String],
    ) -> Result<IssueSnapshot, ApiError> {
        Err(ApiError::PermissionDenied {
            message: "Issues are disabled for this repository".to_string(),
        })
    }

    fn set_labels(
        &self,
        _number: IssueNumber,
        _labels: &[String],
    ) -> Result<IssueSnapshot, ApiError> {
        unreachable!("quick register only creates issues")
    }

    fn set_state(
        &self,
        _number: IssueNumber,
        _state: IssueState,
    ) -> Result<IssueSnapshot, ApiError> {
        unreachable!("quick register only creates issues")
    }

    fn list_spec_issues(&self, _filter: &SpecListFilter) -> Result<Vec<SpecSummary>, ApiError> {
        unreachable!("quick register only creates issues")
    }
}

#[test]
fn app_runtime_quick_register_issue_permission_error_includes_reason_and_fallback() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.issue_client_factory = Arc::new(|_owner, _repo| {
        Ok(Arc::new(PermissionDeniedCreateIssueClient) as Arc<dyn IssueClient>)
    });

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::QuickRegisterIssue {
            title: "Investigate Intake registration".to_string(),
            launch: false,
        },
    );

    let message = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorToast {
                level,
                message,
                issue_number: None,
            } if level == "error" => Some(message.as_str()),
            _ => None,
        })
        .expect("permission error toast");
    assert!(
        message.contains("Issues are disabled for this repository"),
        "toast must preserve the GitHub-provided reason: {message}"
    );
    assert!(
        message.contains("Fallback: create the Issue manually on GitHub"),
        "toast must include the FR-011 fallback path: {message}"
    );
}

#[test]
fn app_runtime_issue_monitor_auto_launch_uses_start_with_last_settings() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut previous = gwt_agent::Session::new(&repo, "develop", gwt_agent::AgentId::Codex);
    previous.model = Some("gpt-5.5".to_string());
    previous.reasoning_level = Some("high".to_string());
    previous.skip_permissions = true;
    previous.save(&sessions_dir).expect("save previous session");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, _recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.auto_launch_issue_monitor_request_events(3165, LinkedIssueKind::Spec);

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue Monitor launch requested" && *issue_number == Some(3165)
        )
    }));
    assert!(
        runtime.launch_wizard.is_none(),
        "auto launch with last settings must not open the Launch Agent window"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::LaunchWizardState { .. })),
        "silent auto launch must not broadcast LaunchWizardState"
    );
    assert!(
        runtime
            .pending_launch_feedback_contexts
            .values()
            .any(|context| context.issue_monitor_issue_number == Some(3165)),
        "auto launch errors must be wired back to Issue Monitor"
    );
    let workspace = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::WindowCanvasState { workspace } => Some(workspace),
            _ => None,
        })
        .expect("workspace broadcast");
    let agent_window = workspace.tabs[0]
        .workspace
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("silent auto launch agent window");
    assert_eq!(agent_window.agent_id.as_deref(), Some("codex"));
    assert_eq!(agent_window.geometry.x, 96.0);
    assert_eq!(agent_window.geometry.y, 96.0);
    assert_eq!(agent_window.geometry.width, 860.0);
    assert_eq!(agent_window.geometry.height, 520.0);
}

#[test]
fn app_runtime_issue_monitor_pending_launch_error_marks_issue_row_failed() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Issue Monitor pending launch failure",
            &["bug"],
            "Issue body",
            "2026-06-23T00:00:00Z",
        ))
        .expect("write issue cache");
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            enabled: true,
            max_active_agents: 5,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("save issue monitor prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.pending_launch_feedback_contexts.insert(
        window_id.clone(),
        LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(42),
        },
    );

    let events = runtime.handle_runtime_status(
        window_id,
        WindowProcessStatus::Error,
        Some("Stop-block hit an error".to_string()),
    );

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    assert_eq!(status.state, "error");
    assert_eq!(
        status.last_error.as_deref(),
        Some("issue #42: Stop-block hit an error")
    );
    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    let item = inbox
        .iter()
        .find(|item| item.issue.number == 42)
        .expect("failed issue row");
    assert_eq!(item.state, gwt::MonitorInboxState::AgentFailed);
    assert_eq!(
        item.error_message.as_deref(),
        Some("Stop-block hit an error")
    );
}

#[test]
fn app_runtime_issue_monitor_auto_launch_uses_last_settings_runtime_target() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    fs::write(
        repo.join("docker-compose.yml"),
        "services:\n  app:\n    image: alpine:3.20\n",
    )
    .expect("compose");
    run_git(&repo, &["add", "docker-compose.yml"]);
    run_git(&repo, &["commit", "-qm", "compose"]);
    run_git(&repo, &["push", "origin", "develop"]);

    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut previous =
        gwt_agent::Session::new(&repo, "feature/spec-3170", gwt_agent::AgentId::Codex);
    previous.model = Some("gpt-5.5".to_string());
    previous.reasoning_level = Some("xhigh".to_string());
    previous.runtime_target = gwt_agent::LaunchRuntimeTarget::Host;
    previous.docker_service = None;
    previous.save(&sessions_dir).expect("save previous session");

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let profiles = runtime.issue_monitor_previous_profiles(&runtime.tabs[0].project_root);
    let repo_profile = profiles.repo_local().expect("repo-local last settings");
    assert_eq!(
        repo_profile.runtime_target,
        gwt_agent::LaunchRuntimeTarget::Host
    );

    let _events = runtime.auto_launch_issue_monitor_request_events(3165, LinkedIssueKind::Spec);

    wait_for_recorded_event(
        "issue monitor last settings runtime launch",
        &recorded_events,
        |events| {
            events
                .iter()
                .any(|event| matches!(event, UserEvent::LaunchComplete { .. }))
        },
    );
    let result = {
        let events = recorded_events.lock().expect("event log");
        events
            .iter()
            .find_map(|event| match event {
                UserEvent::LaunchComplete { result, .. } => Some(result.clone()),
                _ => None,
            })
            .expect("launch complete")
    };
    let Ok((process, _, _, _, _, _, _, _, runtime_target, _)) = result else {
        panic!("Issue Monitor auto launch failed: {result:?}");
    };
    assert_eq!(runtime_target, gwt_agent::LaunchRuntimeTarget::Host);
    assert_eq!(
        process.args.last().map(String::as_str),
        Some("$gwt-execute #3165"),
        "Issue Monitor auto launch must pass the generated prompt to the agent"
    );
}

#[test]
fn app_runtime_issue_monitor_status_reports_last_settings_source() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let fake_gh = write_fake_gh_issue_list(temp.path());
    let _path = prepend_fake_gh_to_path(&fake_gh);

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut previous = gwt_agent::Session::new(&repo, "develop", gwt_agent::AgentId::Codex);
    previous.model = Some("gpt-5.5".to_string());
    previous.reasoning_level = Some("high".to_string());
    previous.runtime_target = gwt_agent::LaunchRuntimeTarget::Host;
    previous.save(&sessions_dir).expect("save previous session");

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::ListIssueMonitor);

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    assert_eq!(
        status.launch_profile_source,
        gwt::IssueMonitorLaunchProfileSource::LastSettings
    );
    assert_eq!(
        status.launch_profile_summary,
        "codex / gpt-5.5 / high / host"
    );
}

#[test]
fn app_runtime_issue_monitor_configure_saves_profile_without_launching() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorConfigureIssue {
            issue_number: 3165,
            linked_issue_kind: Some(LinkedIssueKind::Spec),
        },
    );

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue Monitor settings opened" && *issue_number == Some(3165)
        )
    }));
    let view = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::LaunchWizardState {
                wizard: Some(wizard),
            } => Some(wizard.as_ref()),
            _ => None,
        })
        .expect("launch wizard view");
    assert_eq!(view.primary_action_label, "Continue");
    assert_eq!(view.linked_issue_number, Some(3165));
    assert!(runtime
        .launch_wizard
        .as_ref()
        .expect("launch wizard")
        .issue_monitor_profile_save
        .is_some());
    assert_eq!(
        runtime
            .launch_wizard
            .as_ref()
            .expect("launch wizard")
            .wizard
            .initial_prompt,
        "$gwt-execute #3165"
    );

    runtime.handle_launch_wizard_action(
        LaunchWizardAction::SetModel {
            model: "gpt-5.5".to_string(),
        },
        None,
    );
    runtime.handle_launch_wizard_action(
        LaunchWizardAction::SetReasoning {
            reasoning: "high".to_string(),
        },
        None,
    );
    runtime.handle_launch_wizard_action(
        LaunchWizardAction::SetSkipPermissions { enabled: true },
        None,
    );
    runtime.handle_launch_wizard_action(LaunchWizardAction::Submit, None);
    wait_for_recorded_event(
        "issue monitor settings runtime resolution",
        &recorded_events,
        |events| {
            events
                .iter()
                .any(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
        },
    );
    let resolved_event = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
            .map(|index| events.remove(index))
            .expect("runtime resolved event")
    };
    let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
        unreachable!("matched above")
    };
    runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
    let confirm_events = runtime.handle_launch_wizard_action(LaunchWizardAction::Submit, None);
    let confirm_view = confirm_events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::LaunchWizardState {
                wizard: Some(wizard),
            } => Some(wizard.as_ref()),
            _ => None,
        })
        .expect("confirm wizard view");
    assert_eq!(confirm_view.primary_action_label, "Save settings");

    let saved_events = runtime.handle_launch_wizard_action(LaunchWizardAction::Submit, None);

    assert!(saved_events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue Monitor settings saved" && *issue_number == Some(3165)
        )
    }));
    assert!(runtime.launch_wizard.is_none());
    assert!(
        runtime.window_details.is_empty(),
        "saving Issue Monitor settings must not spawn an agent window"
    );

    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("load issue monitor prefs");
    let profile = prefs.launch_profile.expect("saved launch profile");
    assert_eq!(profile.agent_id, "codex");
    assert_eq!(profile.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(profile.reasoning.as_deref(), Some("high"));
    assert!(profile.skip_permissions);
}

#[test]
fn app_runtime_issue_monitor_configure_profile_saves_global_profile_without_launching() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let autonomous_tuning = gwt::issue_monitor::AutonomousTuning {
        max_attempts: 9,
        review_model: Some("gpt-5.5-review".to_string()),
        ..gwt::issue_monitor::AutonomousTuning::default()
    };
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            autonomous_tuning: autonomous_tuning.clone(),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed issue monitor prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorConfigureProfile,
    );

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue Monitor settings opened" && issue_number.is_none()
        )
    }));
    let view = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::LaunchWizardState {
                wizard: Some(wizard),
            } => Some(wizard.as_ref()),
            _ => None,
        })
        .expect("launch wizard view");
    assert_eq!(view.title, "Configure Issue Monitor");
    assert_eq!(view.linked_issue_number, None);
    assert_eq!(view.primary_action_label, "Continue");
    let save_context = runtime
        .launch_wizard
        .as_ref()
        .expect("launch wizard")
        .issue_monitor_profile_save
        .as_ref()
        .expect("profile save context");
    assert_eq!(save_context.issue_number, None);
    assert_eq!(
        runtime
            .launch_wizard
            .as_ref()
            .expect("launch wizard")
            .wizard
            .initial_prompt,
        ""
    );

    runtime.handle_launch_wizard_action(
        LaunchWizardAction::SetModel {
            model: "gpt-5.5".to_string(),
        },
        None,
    );
    runtime.handle_launch_wizard_action(
        LaunchWizardAction::SetReasoning {
            reasoning: "high".to_string(),
        },
        None,
    );
    runtime.handle_launch_wizard_action(LaunchWizardAction::Submit, None);
    wait_for_recorded_event(
        "global issue monitor settings runtime resolution",
        &recorded_events,
        |events| {
            events
                .iter()
                .any(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
        },
    );
    let resolved_event = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
            .map(|index| events.remove(index))
            .expect("runtime resolved event")
    };
    let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
        unreachable!("matched above")
    };
    runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
    let _confirm_events = runtime.handle_launch_wizard_action(LaunchWizardAction::Submit, None);
    let saved_events = runtime.handle_launch_wizard_action(LaunchWizardAction::Submit, None);

    assert!(saved_events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue Monitor settings saved" && issue_number.is_none()
        )
    }));
    assert!(runtime.launch_wizard.is_none());
    assert!(
        runtime.window_details.is_empty(),
        "saving global Issue Monitor settings must not spawn an agent window"
    );

    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("load issue monitor prefs");
    let profile = prefs.launch_profile.expect("saved launch profile");
    assert_eq!(profile.agent_id, "codex");
    assert_eq!(profile.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(profile.reasoning.as_deref(), Some("high"));
    assert_eq!(
        prefs.autonomous_tuning, autonomous_tuning,
        "Agent settings save must not rewrite autonomous tuning fields"
    );
}

#[test]
fn app_runtime_issue_monitor_start_without_saved_profile_opens_global_settings_before_enable() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut previous = gwt_agent::Session::new(&repo, "develop", gwt_agent::AgentId::Codex);
    previous.model = Some("gpt-5.5".to_string());
    previous.reasoning_level = Some("high".to_string());
    previous.save(&sessions_dir).expect("save previous session");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetIssueMonitorEnabled { enabled: true },
    );

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue Monitor settings opened" && issue_number.is_none()
        )
    }));
    assert!(runtime
        .launch_wizard
        .as_ref()
        .and_then(|session| session.issue_monitor_profile_save.as_ref())
        .is_some_and(|context| context.issue_number.is_none()));
    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("status resets optimistic enabled UI");
    assert!(!status.enabled);
    assert!(
        runtime.window_details.is_empty(),
        "missing saved profile must not spawn an agent window"
    );
    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .unwrap_or_default();
    assert!(
        !prefs.enabled,
        "Start with only last settings must not publish or persist enabled state"
    );
}

#[test]
fn app_runtime_issue_monitor_auto_launch_prefers_saved_profile() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs = gwt::IssueMonitorPrefs {
        launch_profile: Some(sample_issue_monitor_launch_profile()),
        ..Default::default()
    };
    gwt::save_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo), &prefs)
        .expect("save issue monitor prefs");

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let (mut runtime, _recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let mut agent_options = sample_agent_options();
    agent_options.push(gwt::AgentOption {
        id: "claude".to_string(),
        name: "Claude Code".to_string(),
        available: true,
        installed_version: Some("latest".to_string()),
        versions: vec!["latest".to_string()],
        custom_agent: None,
    });
    runtime.launch_wizard_cache = LaunchWizardMemoryCache::load_with_agent_options(
        &temp.path().join("sessions"),
        agent_options,
    );

    let _events = runtime.auto_launch_issue_monitor_request_events(3165, LinkedIssueKind::Spec);

    assert!(
        runtime.launch_wizard.is_none(),
        "saved Issue Monitor profile must launch silently"
    );
    let agent_window = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window");
    assert_eq!(
        agent_window.agent_id.as_deref(),
        Some("claude"),
        "saved profile should override last settings"
    );
    assert!(
        runtime
            .pending_launch_feedback_contexts
            .values()
            .any(|context| context.issue_monitor_issue_number == Some(3165)),
        "saved-profile auto launch errors must be wired back to Issue Monitor"
    );
}

#[test]
fn app_runtime_issue_monitor_auto_launch_without_previous_settings_opens_wizard() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let (mut runtime, _recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.auto_launch_issue_monitor_request_events(3165, LinkedIssueKind::Spec);

    assert!(
        runtime.launch_wizard.is_some(),
        "auto launch without saved or last settings must open one settings window"
    );
    assert!(runtime
        .launch_wizard
        .as_ref()
        .expect("launch wizard")
        .issue_monitor_profile_save
        .is_some());
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::LaunchWizardState { .. })),
        "wizard fallback must broadcast LaunchWizardState so the user can configure settings"
    );
    assert!(
        runtime.window_details.is_empty(),
        "wizard fallback must not silently spawn an agent window"
    );
    assert_eq!(
        runtime
            .launch_wizard
            .as_ref()
            .expect("launch wizard")
            .wizard
            .initial_prompt,
        "$gwt-execute #3165"
    );
}

#[test]
fn app_runtime_issue_monitor_auto_launch_keeps_existing_settings_wizard() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let (mut runtime, _recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let _first_events =
        runtime.auto_launch_issue_monitor_request_events(3165, LinkedIssueKind::Spec);
    let first_wizard_id = runtime
        .launch_wizard
        .as_ref()
        .expect("first settings wizard")
        .wizard_id
        .clone();

    let second_events =
        runtime.auto_launch_issue_monitor_request_events(3166, LinkedIssueKind::Spec);

    assert_eq!(
        runtime
            .launch_wizard
            .as_ref()
            .expect("existing settings wizard")
            .wizard_id,
        first_wizard_id,
        "additional auto launch requests must not replace the open settings wizard"
    );
    assert!(
        !second_events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::LaunchWizardState { .. })),
        "additional auto launch requests must not open another settings wizard"
    );
}

#[test]
fn app_runtime_issue_monitor_launch_now_ignores_auto_max_active_setting() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs = gwt::IssueMonitorPrefs {
        enabled: true,
        max_active_agents: 1,
        priority_order: Vec::new(),
        launch_profile: None,
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo), &prefs)
        .expect("save issue monitor prefs");

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorLaunchNow {
            issue_number: 3165,
            linked_issue_kind: Some(LinkedIssueKind::Spec),
        },
    );

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue Monitor launch prepared" && *issue_number == Some(3165)
        )
    }));
    assert!(
        runtime.launch_wizard.is_some(),
        "manual Issue Monitor launch should not be capped by max_active_agents"
    );
}

#[test]
fn app_runtime_issue_monitor_launch_now_wires_launch_feedback_to_issue_row() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.launch_wizard_cache = LaunchWizardMemoryCache::load_with_agent_options(
        &temp.path().join("sessions"),
        sample_agent_options(),
    );

    let _events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorLaunchNow {
            issue_number: 3165,
            linked_issue_kind: Some(LinkedIssueKind::Spec),
        },
    );

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LaunchWizardAction {
            action: LaunchWizardAction::UseStartMethod {
                method: gwt::LaunchWizardStartMethodKind::ConfigureAndStart,
            },
            bounds: None,
        },
    );
    resolve_launch_wizard_runtime_confirmation(
        &mut runtime,
        &recorded_events,
        "issue monitor launch now runtime resolution",
    );
    let confirm_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LaunchWizardAction {
            action: LaunchWizardAction::Submit,
            bounds: None,
        },
    );
    assert!(
        confirm_events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::LaunchWizardState { .. })),
        "runtime submit should move to the launch confirmation step"
    );

    let launch_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LaunchWizardAction {
            action: LaunchWizardAction::Submit,
            bounds: Some(canvas_bounds()),
        },
    );

    assert!(
        launch_events.iter().any(|event| {
            matches!(
                &event.event,
                BackendEvent::LaunchWizardState {
                    wizard: Some(wizard)
                } if wizard.launch_materialization_pending
            )
        }),
        "confirm submit should report materialization progress before creating an agent window"
    );
    let launch_events = dispatch_launch_materialization_request(
        &mut runtime,
        &recorded_events,
        "issue monitor launch now materialization",
    );
    assert!(
        launch_events.iter().any(|event| {
            matches!(
                event.event,
                BackendEvent::LaunchWizardState { wizard: None }
            )
        }),
        "materialization dispatch should close the wizard and create an agent window"
    );
    assert!(
        runtime
            .pending_launch_feedback_contexts
            .values()
            .any(|context| context.issue_monitor_issue_number == Some(3165)),
        "manual Issue Monitor launches must report launch completion/failure back to the row"
    );
}

#[test]
fn app_runtime_agent_window_initial_state_broadcast_includes_agent_id() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode).build();

    let events = runtime
        .spawn_agent_window("tab-1", config, canvas_bounds(), None)
        .expect("spawn agent window");

    let workspace = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::WindowCanvasState { workspace } => Some(workspace),
            _ => None,
        })
        .expect("initial WindowCanvasState broadcast");
    let tab = workspace
        .tabs
        .iter()
        .find(|tab| tab.id == "tab-1")
        .expect("tab in WindowCanvasState");
    let agent_window = tab
        .workspace
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window in WindowCanvasState");

    assert_eq!(agent_window.title, "Claude Code");
    assert_eq!(agent_window.agent_id.as_deref(), Some("claude"));
}

#[test]
fn app_state_view_projects_agent_window_lane_kind_without_guessing_restored_windows() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "user.name", "Test User"]);
    run_git(&repo, &["commit", "--allow-empty", "-m", "init"]);

    let mut tab_workspace = empty_workspace_state();
    let mut intake = sample_window(
        "agent-intake",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    intake.title = "Codex".to_string();
    intake.agent_id = Some("codex".to_string());
    let mut execution = sample_window(
        "agent-exec",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    execution.title = "Codex".to_string();
    execution.agent_id = Some("codex".to_string());
    let mut restored = sample_window(
        "agent-restored",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    restored.title = "Codex".to_string();
    restored.agent_id = Some("codex".to_string());
    tab_workspace.windows.extend([intake, execution, restored]);
    tab_workspace.next_z_index = 4;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let intake_id = combined_window_id("tab-1", "agent-intake");
    let execution_id = combined_window_id("tab-1", "agent-exec");
    let manager = gwt_git::WorktreeManager::new(&repo);
    let intake_worktree = temp.path().join(".intake-lane");
    manager
        .create_detached("HEAD", &intake_worktree)
        .expect("create detached intake worktree");
    let execution_worktree = temp.path().join(".intake-real-lane");
    manager
        .create_from_base("HEAD", "feature/lane-real", &execution_worktree)
        .expect("create branch worktree with intake-like basename");

    let mut intake_session = sample_active_agent_session("tab-1", &intake_id);
    intake_session.branch_name = "work".to_string();
    intake_session.worktree_path = intake_worktree;
    runtime
        .active_agent_sessions
        .insert(intake_id.clone(), intake_session);

    let mut execution_session = sample_active_agent_session("tab-1", &execution_id);
    execution_session.branch_name = "feature/lane-real".to_string();
    execution_session.worktree_path = execution_worktree;
    runtime
        .active_agent_sessions
        .insert(execution_id.clone(), execution_session);

    let view = runtime.app_state_view();
    let windows = &view
        .tabs
        .iter()
        .find(|tab| tab.id == "tab-1")
        .expect("tab")
        .workspace
        .windows;
    let lane = |raw_id: &str| {
        windows
            .iter()
            .find(|window| window.id == combined_window_id("tab-1", raw_id))
            .map(|window| window.lane_kind)
            .expect("projected window")
    };

    assert_eq!(lane("agent-intake"), gwt::WindowLaneKind::Intake);
    assert_eq!(
        lane("agent-exec"),
        gwt::WindowLaneKind::Execution,
        "a named branch worktree with an .intake-* basename must not be mislabeled as Intake",
    );
    assert_eq!(
        lane("agent-restored"),
        gwt::WindowLaneKind::Unknown,
        "restored agent windows without an active lane signal must not be mislabeled as Execution",
    );
}

#[test]
fn app_runtime_agent_window_initial_title_uses_branch_issue_link_title() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            2468,
            "SPEC: Branch linked purpose",
            &["gwt-spec"],
            "Spec body",
            "2026-05-06T00:00:00Z",
        ))
        .expect("write issue cache");
    write_issue_link_store(
        &repo,
        HashMap::from([("work/20260506-1257".to_string(), 2468)]),
    );
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/20260506-1257")
        .build();

    runtime
        .spawn_agent_window("tab-1", config, canvas_bounds(), None)
        .expect("spawn agent window");

    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window");
    assert_eq!(
        agent_window.purpose_title.as_deref(),
        Some("SPEC: Branch linked purpose")
    );
    assert_eq!(agent_window.title, "Codex");
}

#[test]
fn app_runtime_agent_window_initial_title_falls_back_to_projection_owner() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.owner = Some("SPEC-2008".to_string());
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some("work/20260506-1257".to_string()),
        worktree_path: Some(repo.join("work/20260506-1257")),
        base_branch: Some("origin/develop".to_string()),
        pr_number: None,
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/20260506-1257")
        .build();

    runtime
        .spawn_agent_window("tab-1", config, canvas_bounds(), None)
        .expect("spawn agent window");

    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window");
    assert_eq!(agent_window.purpose_title.as_deref(), Some("SPEC-2008"));
    assert_eq!(agent_window.title, "Codex");
}

#[test]
fn app_runtime_agent_window_initial_title_ignores_projection_owner_for_other_branch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.owner = Some("PR-2525".to_string());
    projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
        branch: Some("work/old".to_string()),
        worktree_path: Some(repo.join("work-old")),
        base_branch: Some("origin/develop".to_string()),
        pr_number: Some(2525),
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work: true,
        created_at: chrono::Utc::now(),
    });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/20260507-0714")
        .build();

    runtime
        .spawn_agent_window("tab-1", config, canvas_bounds(), None)
        .expect("spawn agent window");

    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window");
    assert_eq!(agent_window.purpose_title, None);
    assert_eq!(agent_window.title, "Codex");
}

#[test]
fn app_runtime_board_milestone_updates_same_session_agent_window_detail_only() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab_workspace = empty_workspace_state();
    let mut agent_one = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent_one.title = "Codex".to_string();
    agent_one.purpose_title = Some("Initial purpose".to_string());
    let mut agent_two = sample_window("agent-2", WindowPreset::Agent, WindowProcessStatus::Running);
    agent_two.title = "Claude".to_string();
    agent_two.purpose_title = Some("Other purpose".to_string());
    tab_workspace.windows.push(agent_one);
    tab_workspace.windows.push(agent_two);
    tab_workspace.next_z_index = 3;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let first_window_id = combined_window_id("tab-1", "agent-1");
    let second_window_id = combined_window_id("tab-1", "agent-2");
    runtime.active_agent_sessions.insert(
        first_window_id.clone(),
        ActiveAgentSession {
            window_id: first_window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260506-0736".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    runtime.active_agent_sessions.insert(
        second_window_id,
        ActiveAgentSession {
            window_id: combined_window_id("tab-1", "agent-2"),
            session_id: "session-2".to_string(),
            agent_id: "claude".to_string(),
            branch_name: "work/20260506-0737".to_string(),
            display_name: "Claude".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    save_start_work_workspace_projection(
        &repo,
        runtime
            .active_agent_sessions
            .get(&first_window_id)
            .expect("first session"),
        "develop",
        None,
        None,
        &std::collections::HashSet::new(),
    )
    .expect("save projection");
    let milestone = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "Implement dynamic title sync with detailed workspace context",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1")
    .with_title_summary("Implement dynamic title sync");

    runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);

    let tab = runtime.tab("tab-1").expect("tab");
    assert_eq!(
        tab.workspace
            .window("agent-1")
            .expect("agent 1")
            .dynamic_title
            .as_deref(),
        None
    );
    assert_eq!(
        tab.workspace
            .window("agent-1")
            .expect("agent 1")
            .dynamic_title_detail
            .as_deref(),
        Some("Implement dynamic title sync with detailed workspace context")
    );
    assert_eq!(
        tab.workspace
            .window("agent-2")
            .expect("agent 2")
            .dynamic_title
            .as_deref(),
        None
    );
}

/// Phase U-5 (SPEC-2359 US-38, FR-125, FR-126): a Board post that updates
/// an agent's current focus must broadcast both `WindowCanvasState` (so the
/// pane detail rehydrates on WS reconnect / GUI reload) and
/// `ActiveWorkProjection` (Active Work card) in the same batch. Board
/// `title_summary` is legacy history metadata; the live pane title comes
/// from Workspace purpose updates.
#[test]
fn app_runtime_board_milestone_broadcasts_workspace_state_for_focus_sync() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab_workspace = empty_workspace_state();
    let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent.title = "Codex".to_string();
    agent.purpose_title = Some("Initial purpose".to_string());
    tab_workspace.windows.push(agent);
    tab_workspace.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260513-0343".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    save_start_work_workspace_projection(
        &repo,
        runtime
            .active_agent_sessions
            .get(&window_id)
            .expect("session"),
        "develop",
        None,
        None,
        &std::collections::HashSet::new(),
    )
    .expect("save projection");
    let milestone = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "Implementing Phase U-5 Board path title sync hardening",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1")
    .with_title_summary("Implementing Phase U-5");

    let events = runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);

    assert!(
            events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
            "expected WindowCanvasState broadcast from Board path so pane heading refreshes on reconnect: {events:?}"
        );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjection { .. })),
        "expected ActiveWorkProjection broadcast from Board path: {events:?}"
    );
}

/// Phase U-5 (SPEC-2359 US-38, FR-129, FR-130): the WebSocket reconnect
/// path goes through `FrontendEvent::FrontendReady` → `frontend_sync_events`.
/// The replied `WindowCanvasState` must carry each window's `dynamic_title`
/// and `dynamic_title_detail` so the frontend's `windowDisplayTitle()` can
/// rehydrate the pane heading without waiting for another mutation. This
/// test fails if anyone strips `dynamic_title` from the projected
/// `WorkspaceView`, regressing the reconnect contract.
#[test]
fn frontend_sync_events_preserves_window_dynamic_title_for_reconnect_rehydrate() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab_workspace = empty_workspace_state();
    let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent.title = "Codex".to_string();
    agent.purpose_title = Some("Initial purpose".to_string());
    tab_workspace.windows.push(agent);
    tab_workspace.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let tab_mut = runtime.tab_mut("tab-1").expect("tab mut");
    tab_mut.workspace.set_dynamic_title_with_detail(
        "agent-1",
        Some("Phase U-5 rehydrate target".to_string()),
        Some("simulated stale state before reconnect".to_string()),
    );

    let events = runtime.frontend_sync_events("client-1");

    let workspace_event = events
        .iter()
        .find(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. }))
        .expect("WindowCanvasState reply for FrontendReady");
    let workspace = match &workspace_event.event {
        BackendEvent::WindowCanvasState { workspace } => workspace,
        _ => unreachable!(),
    };
    let projected_window = workspace
        .tabs
        .iter()
        .find(|tab| tab.id == "tab-1")
        .and_then(|tab| {
            tab.workspace
                .windows
                .iter()
                .find(|window| window.id == combined_window_id("tab-1", "agent-1"))
        })
        .expect("agent window in projected WindowCanvasState");
    assert_eq!(
            projected_window.dynamic_title.as_deref(),
            Some("Phase U-5 rehydrate target"),
            "frontend_sync_events must include dynamic_title so reconnect rehydrate restores pane heading"
        );
    assert_eq!(
        projected_window.dynamic_title_detail.as_deref(),
        Some("simulated stale state before reconnect"),
        "frontend_sync_events must include dynamic_title_detail so tooltip survives reconnect"
    );
}

/// Phase U-5: re-asserts the diff gate from
/// `apply_workspace_projection_title_sync_skips_workspace_state_when_same_title_resyncs`
/// at the Board entrypoint. Re-posting an identical milestone (same body for
/// `current_focus`) must not emit a duplicate `WindowCanvasState` broadcast
/// on busy projections.
#[test]
fn app_runtime_board_milestone_skips_workspace_state_on_identical_resync() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab_workspace = empty_workspace_state();
    let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent.title = "Codex".to_string();
    tab_workspace.windows.push(agent);
    tab_workspace.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260513-0343".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    save_start_work_workspace_projection(
        &repo,
        runtime
            .active_agent_sessions
            .get(&window_id)
            .expect("session"),
        "develop",
        None,
        None,
        &std::collections::HashSet::new(),
    )
    .expect("save projection");
    let milestone = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "Stable body for current_focus",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1")
    .with_title_summary("Stable title");

    let first = runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);
    assert!(
        first
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "first Board post should broadcast WindowCanvasState: {first:?}"
    );

    let second = runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);
    assert!(
            !second
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
            "second Board post with identical current_focus must not duplicate WindowCanvasState: {second:?}"
        );
    assert!(
        second
            .iter()
            .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjection { .. })),
        "ActiveWorkProjection should still broadcast on identical resync: {second:?}"
    );
}

#[test]
fn app_runtime_board_milestone_ignores_legacy_title_summary_for_window_title() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab_workspace = empty_workspace_state();
    let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent.title = "Codex".to_string();
    agent.purpose_title = Some("Initial purpose".to_string());
    tab_workspace.windows.push(agent);
    tab_workspace.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260507-0227".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    save_start_work_workspace_projection(
        &repo,
        runtime
            .active_agent_sessions
            .get(&window_id)
            .expect("session"),
        "develop",
        None,
        None,
        &std::collections::HashSet::new(),
    )
    .expect("save projection");
    let long_body = "Implementing the title-summary contract across Board, Workspace, runtime synchronization, CLI parsing, hook reminders, and frontend titlebar rendering";
    let mut entry_value = serde_json::to_value(
        BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            long_body,
            None,
            None,
            vec!["start-work".to_string()],
            vec!["SPEC-2359".to_string()],
        )
        .with_origin_session_id("session-1"),
    )
    .expect("entry json");
    entry_value["title_summary"] = serde_json::json!("Title summary contract");
    let milestone: BoardEntry = serde_json::from_value(entry_value).expect("milestone");

    runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);

    let tab = runtime.tab("tab-1").expect("tab");
    assert_eq!(
        tab.workspace
            .window("agent-1")
            .expect("agent")
            .dynamic_title
            .as_deref(),
        None
    );
    assert_eq!(
        tab.workspace
            .window("agent-1")
            .expect("agent")
            .dynamic_title_detail
            .as_deref(),
        Some(long_body)
    );
}

#[test]
fn app_runtime_board_milestone_without_title_summary_keeps_existing_agent_window_title() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab_workspace = empty_workspace_state();
    let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent.title = "Codex".to_string();
    agent.purpose_title = Some("Initial purpose".to_string());
    tab_workspace.windows.push(agent);
    tab_workspace.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260507-0227".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    save_start_work_workspace_projection(
        &repo,
        runtime
            .active_agent_sessions
            .get(&window_id)
            .expect("session"),
        "develop",
        None,
        None,
        &std::collections::HashSet::new(),
    )
    .expect("save projection");
    let milestone = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "This long body should remain detail only and should not become the titlebar text",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1");

    runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);

    let tab = runtime.tab("tab-1").expect("tab");
    assert_eq!(
        tab.workspace
            .window("agent-1")
            .expect("agent")
            .dynamic_title
            .as_deref(),
        None
    );
}

#[test]
fn app_runtime_workspace_projection_change_updates_agent_window_title_summary() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab_workspace = empty_workspace_state();
    let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent.title = "Codex".to_string();
    tab_workspace.windows.push(agent);
    tab_workspace.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260510-0900".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
    projection
        .agents
        .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: "session-1".to_string(),
            window_id: Some(window_id),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: Some(
                "Implement mandatory Agent title summary updates for Workspace".to_string(),
            ),
            title_summary: Some("Agent title summary guard".to_string()),
            worktree_path: Some(repo.clone()),
            branch: Some("work/20260510-0900".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at: chrono::Utc::now(),
        });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    let events = runtime.handle_workspace_projection_changed_events(&repo);

    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjection { .. })));
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert_eq!(
        agent_window.dynamic_title.as_deref(),
        Some("Agent title summary guard")
    );
    assert_eq!(
        agent_window.dynamic_title_detail.as_deref(),
        Some("Implement mandatory Agent title summary updates for Workspace")
    );
}

// ---------------------------------------------------------------------
// SPEC-2359 US-26 Phase U-1: canonical title-sync orchestration
// ---------------------------------------------------------------------

fn apply_title_sync_setup_tab_and_runtime(
    repo: PathBuf,
    active_tab: Option<&str>,
) -> (AppRuntime, String) {
    let mut tab_workspace = empty_workspace_state();
    let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent.title = "Codex".to_string();
    tab_workspace.windows.push(agent);
    tab_workspace.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    // Need the path so the temp directory survives until the runtime drops.
    let temp_root = repo.parent().expect("repo has parent").to_path_buf();
    let mut runtime = sample_runtime(&temp_root, vec![tab], active_tab);
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260510-0900".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo,
            agent_project_root: String::new(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    (runtime, window_id)
}

fn apply_title_sync_sample_projection(
    repo: &Path,
    window_id: &str,
    title_summary: Option<&str>,
    current_focus: Option<&str>,
) -> gwt_core::workspace_projection::WorkspaceProjection {
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(repo);
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
    projection
        .agents
        .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: "session-1".to_string(),
            window_id: Some(window_id.to_string()),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: current_focus.map(str::to_string),
            title_summary: title_summary.map(str::to_string),
            worktree_path: Some(repo.to_path_buf()),
            branch: Some("work/20260510-0900".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at: chrono::Utc::now(),
        });
    projection
}

#[test]
fn apply_workspace_projection_title_sync_writes_dynamic_title() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    let projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Canonical orchestration"),
        Some("Implement apply_workspace_projection_title_sync"),
    );

    let _events = runtime.apply_workspace_projection_title_sync(&repo, &projection);

    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert_eq!(
        agent_window.dynamic_title.as_deref(),
        Some("Canonical orchestration"),
        "dynamic_title should reflect projection.agents[<i>].title_summary"
    );
    assert_eq!(
        agent_window.dynamic_title_detail.as_deref(),
        Some("Implement apply_workspace_projection_title_sync"),
        "dynamic_title_detail should reflect projection.agents[<i>].current_focus"
    );
}

#[test]
fn apply_workspace_projection_title_sync_emits_active_work_projection_for_active_tab() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    // active_work_projection_for_tab loads from disk, so the projection
    // file must exist for the broadcast to populate.
    let projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Phase U-1 broadcast assertion"),
        None,
    );
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    let events = runtime.apply_workspace_projection_title_sync(&repo, &projection);

    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjection { .. })),
        "expected ActiveWorkProjection broadcast in events: {events:?}"
    );
}

#[test]
fn apply_workspace_projection_title_sync_returns_no_events_without_active_tab() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) = apply_title_sync_setup_tab_and_runtime(repo.clone(), None);
    let projection =
        apply_title_sync_sample_projection(&repo, &window_id, Some("No active tab"), None);

    let events = runtime.apply_workspace_projection_title_sync(&repo, &projection);

    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjection { .. })),
        "without an active tab, ActiveWorkProjection broadcast must be skipped"
    );
    // Even without an active tab, the in-memory dynamic_title should still
    // be synced so the next workspace_state broadcast carries it.
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert_eq!(
        agent_window.dynamic_title.as_deref(),
        Some("No active tab"),
        "dynamic_title must be set in-memory regardless of active_tab routing"
    );
}

#[test]
fn apply_workspace_projection_title_sync_emits_workspace_state_when_dynamic_title_changed() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    let projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Phase U-2 WindowCanvasState assertion"),
        None,
    );
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    let events = runtime.apply_workspace_projection_title_sync(&repo, &projection);

    // Phase U-2 (SPEC-2359 US-26): a workspace update path that mutates
    // an in-memory dynamic_title MUST broadcast WindowCanvasState in the
    // same batch so the frontend's `windowData.dynamic_title` and the
    // pane heading `windowDisplayTitle` refresh without waiting for the
    // next hook event or window structure change.
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "expected WindowCanvasState broadcast when dynamic_title changed: {events:?}"
    );
}

#[test]
fn apply_workspace_projection_title_sync_skips_workspace_state_when_nothing_changed() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, _window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    // Drop the active_agent_sessions entry AND erase the projection's
    // window_id so neither the fast path nor the Phase U-3 fallback
    // can resolve a window. The WindowCanvasState broadcast should be
    // skipped to avoid forcing a frontend re-render for a no-op update.
    runtime.active_agent_sessions.clear();
    let mut projection =
        apply_title_sync_sample_projection(&repo, "tab-1::agent-1", Some("No-op"), None);
    projection.agents[0].window_id = None;
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    let events = runtime.apply_workspace_projection_title_sync(&repo, &projection);

    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "WindowCanvasState must be skipped when in-memory dynamic_title did not change: {events:?}"
    );
}

#[test]
fn apply_workspace_projection_title_sync_skips_workspace_state_when_same_title_resyncs() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    let projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Stable title"),
        Some("Stable focus"),
    );
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    // First sync: dynamic_title transitions from None → "Stable title".
    let first = runtime.apply_workspace_projection_title_sync(&repo, &projection);
    assert!(
        first
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "first sync should broadcast WindowCanvasState: {first:?}"
    );

    // Second sync with the same projection: nothing diffs, so the
    // WindowCanvasState broadcast must be suppressed to avoid forcing a
    // full frontend re-render on busy projections (Codex review P2).
    let second = runtime.apply_workspace_projection_title_sync(&repo, &projection);
    assert!(
        !second
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "second sync with identical title must not broadcast WindowCanvasState: {second:?}"
    );
    // ActiveWorkProjection still fires (it's idempotent on the
    // frontend; the active card snapshot is harmless to re-send).
    assert!(
        second
            .iter()
            .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjection { .. })),
        "ActiveWorkProjection should still broadcast on resync: {second:?}"
    );
}

#[test]
fn handle_workspace_projection_changed_events_broadcasts_workspace_state_for_pane_heading() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    let projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Pane heading via WindowCanvasState"),
        Some("triggered by workspace.update params.purpose"),
    );
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    let events = runtime.handle_workspace_projection_changed_events(&repo);

    // The original handler returned only ActiveWorkProjection. Phase
    // U-2 promotes it to also broadcast WindowCanvasState in one batch so
    // the pane heading refreshes immediately after `workspace.update`.
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "handle_workspace_projection_changed_events must broadcast WindowCanvasState: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjection { .. })),
        "ActiveWorkProjection broadcast must still fire: {events:?}"
    );
}

#[test]
fn handle_workspace_projection_changed_events_syncs_title_from_canonical_project_root() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let project_root = temp.path().join("workspace-home");
    let worktree = project_root.join("work").join("20260601-0934");
    fs::create_dir_all(&worktree).expect("worktree");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(project_root.clone(), Some("tab-1"));
    runtime
        .active_agent_sessions
        .get_mut(&window_id)
        .expect("active session")
        .worktree_path = worktree.clone();
    let mut projection = apply_title_sync_sample_projection(
        &project_root,
        &window_id,
        Some("Canonical Project State title"),
        Some("Agent worktree differs from Project State root"),
    );
    projection.agents[0].worktree_path = Some(worktree);
    gwt_core::workspace_projection::save_workspace_projection(&project_root, &projection)
        .expect("save projection");

    let events = runtime.handle_workspace_projection_changed_events(&project_root);

    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "canonical Project State root updates must broadcast WindowCanvasState: {events:?}"
    );
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert_eq!(
        agent_window.dynamic_title.as_deref(),
        Some("Canonical Project State title")
    );
    assert_eq!(
        agent_window.dynamic_title_detail.as_deref(),
        Some("Agent worktree differs from Project State root")
    );
}

#[test]
fn sync_agent_window_titles_falls_back_to_projection_window_id_when_active_agent_sessions_missing()
{
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    // Phase U-3: simulate a session that gwt's launch tracking has not
    // registered (e.g. GUI restarted after launch, or session started
    // outside gwt's launch path). The window exists in workspace state
    // and the projection knows its window_id + worktree_path, but
    // active_agent_sessions is empty.
    runtime.active_agent_sessions.clear();
    let projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Phase U-3 backfill via projection window_id"),
        Some("ensures untracked sessions still update pane heading"),
    );

    let changed = runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

    assert!(
        changed,
        "Phase U-3: sync must return true when projection-driven fallback resolves the window"
    );
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert_eq!(
        agent_window.dynamic_title.as_deref(),
        Some("Phase U-3 backfill via projection window_id"),
        "dynamic_title must propagate even when active_agent_sessions is empty"
    );
    assert_eq!(
        agent_window.dynamic_title_detail.as_deref(),
        Some("ensures untracked sessions still update pane heading")
    );
}

#[test]
fn sync_agent_window_titles_skips_fallback_when_projection_worktree_mismatches() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let other_repo = temp.path().join("other-repo");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&other_repo).expect("create other repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    runtime.active_agent_sessions.clear();
    // Projection claims the agent lives in a *different* worktree.
    // Phase U-3 fallback must refuse to touch local windows in that
    // case, otherwise cross-worktree titles could leak.
    let mut projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Cross-worktree title leak guard"),
        None,
    );
    projection.agents[0].worktree_path = Some(other_repo);

    let changed = runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

    assert!(
        !changed,
        "fallback must refuse cross-worktree window updates"
    );
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert!(
        agent_window.dynamic_title.is_none(),
        "dynamic_title must stay None when projection.agents[<i>].worktree_path mismatches"
    );
}

#[test]
fn sync_agent_window_titles_skips_fallback_when_projection_window_id_unknown_locally() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, _window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    runtime.active_agent_sessions.clear();
    // Projection references a window_id that is NOT present in any
    // local tab. The fallback must short-circuit instead of producing
    // a phantom window update.
    let mut projection =
        apply_title_sync_sample_projection(&repo, "tab-1::agent-1", Some("phantom"), None);
    projection.agents[0].window_id = Some("tab-99::ghost".to_string());

    let changed = runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

    assert!(
        !changed,
        "fallback must require the projected window_id to exist locally"
    );
}

#[test]
fn sync_agent_window_titles_returns_false_when_no_resolution_path_exists() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, _window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    // Drop the active_agent_sessions entry AND erase the projection's
    // window_id so neither the fast path nor the Phase U-3 fallback
    // can resolve this agent's window. The sync must then be a no-op.
    runtime.active_agent_sessions.clear();
    let mut projection = apply_title_sync_sample_projection(
        &repo,
        "tab-1::agent-1",
        Some("Should not propagate"),
        None,
    );
    projection.agents[0].window_id = None;

    let changed = runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

    assert!(
        !changed,
        "sync must return false when no in-memory window was touched"
    );
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert!(
            agent_window.dynamic_title.is_none(),
            "dynamic_title must stay None when neither active_agent_sessions nor projection window_id resolve"
        );
}

// ---------------------------------------------------------------------
// SPEC-2359 Phase U-4: worktree-only fallback for SessionStart hook
// registered records (window_id is None, session_id does not match any
// active_agent_session). Resolves to the unique active session in the
// same worktree with the same agent_id when one exists.
// ---------------------------------------------------------------------

#[test]
fn sync_agent_window_titles_fast_path_resolves_across_unrelated_project_root() {
    // SPEC-2359 Phase U-7: when the watcher event project_root is for
    // tab A but a session lives in tab B (both share current.json
    // because they're worktrees of the same repo), the fast path must
    // still resolve B's window via session_id alone. Previously the
    // additional `same_worktree_path(session.worktree_path,
    // project_root)` filter prevented this and caused the user-visible
    // pane heading to stay on the agent_id fallback even after
    // `workspace.update` succeeded at the data
    // layer.
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    let unrelated = temp.path().join("unrelated");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&unrelated).expect("create unrelated");
    let (mut runtime, _window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));

    // Projection identifies the agent by session-1 with a worktree
    // that *does not* match the unrelated project_root we will pass
    // in. The fast path should still resolve.
    let projection = apply_title_sync_sample_projection(
        &repo,
        "tab-1::agent-1",
        Some("Phase U-7 cross-tab fast path"),
        None,
    );

    let changed =
        runtime.sync_agent_window_titles_from_workspace_projection(&unrelated, &projection);

    assert!(
        changed,
        "fast path must resolve by session_id alone, independent of project_root"
    );
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert_eq!(
        agent_window.dynamic_title.as_deref(),
        Some("Phase U-7 cross-tab fast path"),
    );
}

#[test]
fn sync_agent_window_titles_falls_back_to_worktree_when_session_id_not_active() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, _window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    let mut projection = apply_title_sync_sample_projection(
        &repo,
        "tab-1::agent-1",
        Some("Phase U-4 worktree fallback"),
        Some("SessionStart-hook registered records have no window_id"),
    );
    // Simulate a SessionStart-hook registration: same worktree, same
    // agent_id (codex), but a *different* session_id and no window_id.
    // The fast path won't match by session_id, but the worktree-only
    // fallback should resolve to the in-memory Codex window.
    projection.agents[0].session_id = "out-of-band-session".to_string();
    projection.agents[0].window_id = None;

    let changed = runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

    assert!(
        changed,
        "worktree fallback should resolve to the unique active session"
    );
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert_eq!(
        agent_window.dynamic_title.as_deref(),
        Some("Phase U-4 worktree fallback"),
    );
}

#[test]
fn sync_agent_window_titles_worktree_fallback_refuses_when_agent_id_mismatches() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, _window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    // Active session is `codex`, but the projection record (SessionStart
    // registered) claims `claude`. The fallback must refuse rather than
    // assigning a Claude title to the Codex pane.
    let mut projection = apply_title_sync_sample_projection(
        &repo,
        "tab-1::agent-1",
        Some("Wrong-agent title leak guard"),
        None,
    );
    projection.agents[0].session_id = "out-of-band-claude".to_string();
    projection.agents[0].window_id = None;
    projection.agents[0].agent_id = "claude".to_string();

    let changed = runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

    assert!(
        !changed,
        "worktree fallback must require matching agent_id to disambiguate"
    );
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert!(agent_window.dynamic_title.is_none());
}

#[test]
fn sync_agent_window_titles_worktree_fallback_refuses_when_multiple_sessions_match() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    // Add a second Codex session in the same worktree. The fallback
    // must refuse to pick one because the mapping would be ambiguous.
    runtime.active_agent_sessions.insert(
        "tab-1::agent-2".to_string(),
        ActiveAgentSession {
            window_id: "tab-1::agent-2".to_string(),
            session_id: "session-2".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260510-0900".to_string(),
            display_name: "Codex 2".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: String::new(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    let mut projection =
        apply_title_sync_sample_projection(&repo, &window_id, Some("Ambiguity guard"), None);
    projection.agents[0].session_id = "out-of-band-session".to_string();
    projection.agents[0].window_id = None;

    let changed = runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

    assert!(
        !changed,
        "worktree fallback must refuse when multiple sessions share the same worktree + agent_id"
    );
}

#[test]
fn app_runtime_runtime_hook_state_does_not_update_agent_window_dynamic_title() {
    let temp = tempdir().expect("tempdir");
    let mut tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    tab.workspace
        .set_dynamic_title("codex-1", Some("Board milestone focus".to_string()));
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let events = runtime.handle_runtime_hook_event(runtime_hook_state("Waiting", "session-1"));

    assert!(events
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::RuntimeHookEvent { .. })));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "non-structural runtime hook state changes must not force a full workspace_state"
    );
    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowState { .. })));
    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::TerminalStatus { .. })));
    let tab = runtime.tab("tab-1").expect("tab");
    assert_eq!(
        tab.workspace
            .window("codex-1")
            .expect("codex window")
            .dynamic_title
            .as_deref(),
        Some("Board milestone focus")
    );
}

#[test]
fn app_runtime_gui_board_scope_uses_active_workspace_id_not_first_agent_workspace() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.id = "workspace-active".to_string();
    projection.agents.push(workspace_agent_summary_for_test(
        "session-other",
        Some("workspace-other"),
    ));
    projection
        .agents
        .push(workspace_agent_summary_for_test("session-active", None));
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    let scope =
        super::board::gui_default_board_scope_for_project(&repo).expect("resolve GUI board scope");
    let post_audience = gwt::board_audience::post_audience_for_gui(&repo, &[], None, false)
        .expect("resolve post audience");

    assert_eq!(
        scope,
        BoardAudienceScope::Workspace("workspace-active".to_string())
    );
    assert_eq!(post_audience, Some(vec!["workspace-active".to_string()]));
}

#[test]
fn app_runtime_board_projection_change_preserves_all_view_for_live_updates() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.id = "workspace-a".to_string();
    projection
        .agents
        .push(workspace_agent_summary_for_test("session-a", None));
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "Workspace A update",
            None,
            None,
            vec![],
            vec![],
        )
        .with_audience(vec!["workspace-a"]),
    )
    .expect("seed workspace A update");
    post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "Workspace B update",
            None,
            None,
            vec![],
            vec![],
        )
        .with_audience(vec!["workspace-b"]),
    )
    .expect("seed workspace B update");

    let mut tab_workspace = empty_workspace_state();
    tab_workspace.windows.push(sample_window(
        "board-all",
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    ));
    tab_workspace.windows.push(sample_window(
        "board-workspace",
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    ));
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let all_window_id = combined_window_id("tab-1", "board-all");
    let workspace_window_id = combined_window_id("tab-1", "board-workspace");
    let _ = runtime.load_board_events("client-1", &all_window_id, true);
    let _ = runtime.load_board_events("client-1", &workspace_window_id, false);

    let events = runtime.handle_board_projection_changed_events(&repo);

    let all_entries = events
        .iter()
        .find_map(|event| match event {
            OutboundEvent {
                event: BackendEvent::BoardEntries { id, entries, .. },
                ..
            } if id == &all_window_id => Some(entries),
            _ => None,
        })
        .expect("all view board entries");
    let workspace_entries = events
        .iter()
        .find_map(|event| match event {
            OutboundEvent {
                event: BackendEvent::BoardEntries { id, entries, .. },
                ..
            } if id == &workspace_window_id => Some(entries),
            _ => None,
        })
        .expect("workspace view board entries");

    assert!(
        all_entries
            .iter()
            .any(|entry| entry.body == "Workspace B update"),
        "All view live update must include other Workspace entries"
    );
    assert!(
        !workspace_entries
            .iter()
            .any(|entry| entry.body == "Workspace B update"),
        "Workspace view live update must stay scoped"
    );
}

#[test]
fn app_runtime_board_projection_change_broadcasts_to_matching_board_windows_only() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let other_repo = temp.path().join("other-repo");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&other_repo).expect("create other repo");
    post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "External update",
            None,
            None,
            vec![],
            vec![],
        ),
    )
    .expect("seed matching board snapshot");
    post_entry(
        &other_repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "Other project update",
            None,
            None,
            vec![],
            vec![],
        ),
    )
    .expect("seed other board snapshot");

    let mut tab_workspace = empty_workspace_state();
    tab_workspace.windows.push(sample_window(
        "board-1",
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    ));
    tab_workspace.windows.push(sample_window(
        "board-2",
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    ));
    tab_workspace.windows.push(sample_window(
        "logs-1",
        WindowPreset::Logs,
        WindowProcessStatus::Ready,
    ));
    tab_workspace.next_z_index = 4;
    let matching_tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let other_tab = sample_project_tab_with_window_at(
        "tab-2",
        "board-3",
        other_repo,
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![matching_tab, other_tab], Some("tab-1"));

    let events = runtime.handle_board_projection_changed_events(&repo);

    assert_eq!(events.len(), 3);
    for expected_id in [
        combined_window_id("tab-1", "board-1"),
        combined_window_id("tab-1", "board-2"),
    ] {
        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::BoardEntries { id, entries, .. },
            } if *id == expected_id
                && entries.len() == 1
                && entries[0].body == "External update"
        )));
    }
    assert!(!events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            event: BackendEvent::BoardEntries { id, .. },
            ..
        } if *id == combined_window_id("tab-2", "board-3")
    )));
}

fn migration_pending_tab(tab_id: &str, project_root: PathBuf) -> ProjectTabRuntime {
    ProjectTabRuntime {
        id: tab_id.to_string(),
        title: "Repo".to_string(),
        project_root,
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(empty_workspace_state()),
        migration_pending: true,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    }
}

/// SPEC-2014 FR-PERF-003: ProjectTabRuntime caches `main_worktree_root`
/// resolution per tab so the Launch Wizard / Start Work paths do not
/// re-spawn `git rev-parse --git-common-dir` on every open.
#[test]
fn project_tab_runtime_main_worktree_root_caches_resolution() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    gwt_core::process::hidden_command("git")
        .args(["init", repo.to_str().unwrap()])
        .output()
        .expect("git init");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    assert!(
        tab.main_worktree_root_cache.get().is_none(),
        "cache must start empty"
    );

    let first = tab.main_worktree_root();
    let cached = tab
        .main_worktree_root_cache
        .get()
        .expect("cache populated after first access")
        .clone();
    assert_eq!(first, cached);

    let second = tab.main_worktree_root();
    assert_eq!(
        first, second,
        "second call must return the cached resolution"
    );
}

#[test]
fn migration_detected_broadcasts_only_for_pending_tabs() {
    let temp = tempdir().expect("tempdir");
    let repo_a = temp.path().join("repo-a");
    let repo_b = temp.path().join("repo-b");
    fs::create_dir_all(&repo_a).expect("repo-a");
    fs::create_dir_all(&repo_b).expect("repo-b");

    let pending = migration_pending_tab("tab-1", repo_a);
    let mut clean = sample_project_tab("tab-2", "Other", repo_b, ProjectKind::Git, &[]);
    clean.migration_pending = false;
    let runtime = sample_runtime(temp.path(), vec![pending, clean], Some("tab-1"));

    let events = runtime.migration_detected_broadcasts();

    assert_eq!(events.len(), 1, "only pending tabs should broadcast");
    assert!(matches!(
        &events[0],
        OutboundEvent {
            target: DispatchTarget::Broadcast,
            event: BackendEvent::MigrationDetected { tab_id, .. },
        } if tab_id == "tab-1"
    ));
}

#[test]
fn handle_migration_done_repoints_tab_and_emits_broadcast() {
    let temp = tempdir().expect("tempdir");
    let project = temp.path().join("project");
    let new_worktree = project.join("develop");
    fs::create_dir_all(&new_worktree).expect("new worktree");

    let tab = migration_pending_tab("tab-1", project);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_migration_done("tab-1", &new_worktree);

    let updated = runtime
        .tabs
        .iter()
        .find(|t| t.id == "tab-1")
        .expect("tab still present");
    let canonical_new = dunce::canonicalize(&new_worktree).unwrap_or_else(|_| new_worktree.clone());
    assert_eq!(updated.project_root, canonical_new);
    assert!(!updated.migration_pending, "pending flag must clear");

    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Broadcast,
            event: BackendEvent::MigrationDone { tab_id, .. },
        } if tab_id == "tab-1"
    )));
}

#[test]
fn handle_migration_error_clears_pending_and_broadcasts_recovery_label() {
    use gwt_core::migration::{MigrationPhase, RecoveryState};
    let temp = tempdir().expect("tempdir");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");

    let tab = migration_pending_tab("tab-1", project);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_migration_error(
        "tab-1",
        MigrationPhase::Bareify,
        "boom".to_string(),
        RecoveryState::RolledBack,
    );

    assert!(
        !runtime
            .tabs
            .iter()
            .find(|t| t.id == "tab-1")
            .unwrap()
            .migration_pending
    );
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Broadcast,
            event: BackendEvent::MigrationError { tab_id, recovery, phase, .. },
        } if tab_id == "tab-1" && recovery == "rolled_back" && phase == "bareify"
    )));
}

// SPEC-3214 Phase 3: OpenIntakeSession opens the Launch Wizard flagged as an
// ephemeral intake — the resulting launch will be branchless / detached.
#[test]
fn open_intake_session_opens_ephemeral_branchless_wizard() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "user.name", "Test User"]);
    run_git(&repo, &["commit", "--allow-empty", "-m", "init"]);

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::OpenIntakeSession);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::LaunchWizardOpenError { .. })),
        "intake session opens without error: {events:?}"
    );

    let wizard = &runtime
        .launch_wizard
        .as_ref()
        .expect("intake wizard")
        .wizard;
    assert_eq!(wizard.view().mode, gwt::LaunchWizardMode::Intake);
    assert_eq!(
        wizard.view().title,
        "Intake",
        "hydrated intake wizard must not fall back to Start Work copy"
    );
    assert_eq!(
        wizard.context.ephemeral_base_ref.as_deref(),
        Some(gwt::start_work::START_WORK_BASE_BRANCH_CANDIDATES[0]),
        "intake wizard is flagged ephemeral on the base ref"
    );
    assert!(
        wizard.context.normalized_branch_name.is_empty(),
        "intake wizard reserves no branch"
    );
}

#[test]
fn open_intake_session_refuses_while_migration_pending() {
    // SPEC-1934 US-7 / FR-034: Workspace Start Work must not run on a tab
    // whose Normal → Nested Bare+Worktree migration is still pending.
    // Without this gate, the launch path tries to fetch
    // `origin/work/<branch>` on a single-branch refspec and dies with
    // `fatal: invalid reference: origin/work/<branch>`.
    let temp = tempdir().expect("tempdir");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");

    let tab = migration_pending_tab("tab-1", project);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.open_intake_session("client-1");

    assert!(
        events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Client(_),
                event: BackendEvent::LaunchWizardOpenError { message, .. },
            } if message == "Complete the project migration before starting an intake session"
        )),
        "Start Work on a migration_pending tab must surface a clear error: {events:?}"
    );
}

#[test]
fn github_repository_search_parser_maps_gh_json_fields() {
    let raw = r#"[
          {
            "fullName": "akiojin/gwt",
            "description": "Git Worktree Manager",
            "url": "https://github.com/akiojin/gwt",
            "defaultBranch": "develop",
            "visibility": "public",
            "updatedAt": "2026-05-13T00:00:00Z"
          }
        ]"#;

    let repositories =
        super::parse_github_repository_search_results(raw).expect("parse gh search json");

    assert_eq!(repositories.len(), 1);
    assert_eq!(repositories[0].full_name, "akiojin/gwt");
    assert_eq!(
        repositories[0].description.as_deref(),
        Some("Git Worktree Manager")
    );
    assert_eq!(repositories[0].url, "https://github.com/akiojin/gwt");
    assert_eq!(repositories[0].default_branch.as_deref(), Some("develop"));
    assert_eq!(repositories[0].visibility.as_deref(), Some("public"));
    assert_eq!(
        repositories[0].updated_at.as_deref(),
        Some("2026-05-13T00:00:00Z")
    );
}

#[test]
fn clone_project_done_opens_workspace_home_and_broadcasts_done() {
    let temp = tempdir().expect("tempdir");
    let workspace_home = temp.path().join("sample");
    let bare_repo = workspace_home.join("sample.git");
    fs::create_dir_all(&workspace_home).expect("workspace home");
    let output = gwt_core::process::hidden_command("git")
        .args(["init", "--bare", bare_repo.to_str().expect("bare path")])
        .output()
        .expect("git init --bare");
    assert!(
        output.status.success(),
        "git init --bare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);

    let events = runtime.handle_clone_project_done(&workspace_home);

    assert_eq!(runtime.tabs.len(), 1);
    assert_eq!(
        runtime.tabs[0].project_root,
        dunce::canonicalize(&workspace_home).unwrap()
    );
    assert_eq!(runtime.recent_projects.len(), 1);
    assert_eq!(
        runtime.recent_projects[0].path,
        dunce::canonicalize(&workspace_home).unwrap()
    );
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Broadcast,
            event: BackendEvent::CloneProjectDone {
                workspace_home: emitted_workspace_home,
            },
        } if emitted_workspace_home == &workspace_home.display().to_string()
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Broadcast,
            event: BackendEvent::WindowCanvasState { .. },
        }
    )));
}

#[test]
fn open_project_path_for_worktree_remembers_workspace_home_only() {
    // Issue #2867: open_project_path で worktree path を渡したとき、tab は
    // worktree で開く (direct-pick) が、recent_projects は workspace home
    // (bare repo の親) に正規化されて 1 件だけ残る。同じ workspace の
    // 別 worktree を続けて開いても recent_projects は増えない。
    let temp = tempdir().expect("tempdir");
    let workspace_home = temp.path().join("workspace");
    let bare_repo = workspace_home.join("repo.git");
    fs::create_dir_all(&workspace_home).expect("workspace home");
    let output = gwt_core::process::hidden_command("git")
        .args(["init", "--bare", bare_repo.to_str().expect("bare path")])
        .output()
        .expect("git init --bare");
    assert!(
        output.status.success(),
        "git init --bare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bootstrap = workspace_home.join(".bootstrap");
    let clone = gwt_core::process::hidden_command("git")
        .args([
            "clone",
            bare_repo.to_str().unwrap(),
            bootstrap.to_str().unwrap(),
        ])
        .output()
        .expect("git clone");
    assert!(clone.status.success(), "git clone failed");
    for (key, value) in [
        ("user.email", "test@example.com"),
        ("user.name", "Test User"),
    ] {
        let cfg = gwt_core::process::hidden_command("git")
            .args(["config", key, value])
            .current_dir(&bootstrap)
            .output()
            .expect("git config");
        assert!(cfg.status.success(), "git config {key} failed");
    }
    for args in [
        vec!["checkout", "-b", "develop"],
        vec!["commit", "--allow-empty", "-m", "init"],
        vec!["push", "origin", "develop"],
    ] {
        let out = gwt_core::process::hidden_command("git")
            .args(&args)
            .current_dir(&bootstrap)
            .output()
            .expect("git command");
        assert!(out.status.success(), "git {args:?} failed");
    }
    fs::remove_dir_all(&bootstrap).expect("remove bootstrap");

    let develop_worktree = workspace_home.join("develop");
    let feature_worktree = workspace_home.join("feature/alpha");
    for (path, branch_args) in [
        (
            &develop_worktree,
            vec!["worktree", "add", "@PATH@", "develop"],
        ),
        (
            &feature_worktree,
            vec![
                "worktree",
                "add",
                "-b",
                "feature/alpha",
                "@PATH@",
                "develop",
            ],
        ),
    ] {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("worktree parent");
        }
        let path_str = path.to_str().unwrap().to_string();
        let resolved: Vec<&str> = branch_args
            .iter()
            .map(|a| {
                if *a == "@PATH@" {
                    path_str.as_str()
                } else {
                    *a
                }
            })
            .collect();
        let out = gwt_core::process::hidden_command("git")
            .args(&resolved)
            .current_dir(&bare_repo)
            .output()
            .expect("git worktree add");
        assert!(
            out.status.success(),
            "git {resolved:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let canonical_home = dunce::canonicalize(&workspace_home).unwrap();

    runtime
        .open_project_path(develop_worktree.clone())
        .expect("open develop worktree");
    assert_eq!(runtime.tabs.len(), 1);
    assert_eq!(
        runtime.tabs[0].project_root,
        dunce::canonicalize(&develop_worktree).unwrap(),
        "tab must open at the chosen worktree (SC-035 direct-pick preserved)"
    );
    assert_eq!(runtime.recent_projects.len(), 1);
    assert_eq!(
        runtime.recent_projects[0].path, canonical_home,
        "recent_projects must collapse to workspace home, not worktree path (Issue #2867)"
    );

    runtime
        .open_project_path(feature_worktree.clone())
        .expect("open feature worktree");
    assert_eq!(
        runtime.recent_projects.len(),
        1,
        "opening another worktree in the same workspace must not add a new recent entry"
    );
    assert_eq!(
        runtime.recent_projects[0].path, canonical_home,
        "second worktree open must keep workspace home as the canonical recent entry"
    );
}

#[test]
fn clone_project_start_validation_uses_clone_project_error_event() {
    let temp = tempdir().expect("tempdir");
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::CloneProjectStart {
            url: "".to_string(),
            parent_path: "".to_string(),
        },
    );

    assert!(events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::CloneProjectError { message },
        } if client_id == "client-1" && message.contains("repository URL")
    )));
    assert!(!events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            event: BackendEvent::ProjectOpenError { .. },
            ..
        }
    )));
}

#[test]
fn skip_migration_events_clears_pending_flag_without_broadcast() {
    let temp = tempdir().expect("tempdir");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");

    let tab = migration_pending_tab("tab-1", project);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.skip_migration_events("tab-1");
    assert!(events.is_empty(), "skip must not emit events itself");
    assert!(!runtime.tabs[0].migration_pending);
}

#[test]
fn skip_migration_events_keeps_normal_git_and_redetects_on_next_launch() {
    let temp = tempdir().expect("tempdir");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");
    init_repo(&project);

    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let open_events = runtime.open_project_path_events(project.clone());
    let tab_id = runtime.active_tab_id.clone().expect("active tab");

    assert!(open_events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            target: DispatchTarget::Broadcast,
            event: BackendEvent::MigrationDetected { .. },
        }
    )));

    let skip_events = runtime.skip_migration_events(&tab_id);
    assert!(skip_events.is_empty(), "skip must not mutate repository");
    assert!(matches!(
        gwt_git::detect_repo_type(&project),
        gwt_git::RepoType::Normal {
            needs_migration: true,
            ..
        }
    ));

    let mut next_runtime = sample_runtime(temp.path(), Vec::new(), None);
    let next_events = next_runtime.open_project_path_events(project);

    assert!(
        next_events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::MigrationDetected { .. },
            }
        )),
        "skip is launch-local; the modal must be shown again next launch"
    );
}

#[test]
fn quit_migration_events_requests_app_quit_without_repository_changes() {
    let temp = tempdir().expect("tempdir");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");
    init_repo(&project);

    let tab = migration_pending_tab("tab-1", project.clone());
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.quit_migration_events("tab-1");

    assert!(
        events.is_empty(),
        "quit is delivered through the event proxy"
    );
    let recorded_events = recorded_events.lock().expect("recorded events");
    assert!(recorded_events
        .iter()
        .any(|event| matches!(event, UserEvent::QuitApp)));
    assert!(matches!(
        gwt_git::detect_repo_type(&project),
        gwt_git::RepoType::Normal {
            needs_migration: true,
            ..
        }
    ));
}

#[test]
fn open_project_with_existing_migration_backup_emits_recovery_error() {
    let temp = tempdir().expect("tempdir");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project dir");
    init_repo(&project);
    fs::create_dir_all(project.join(gwt_core::migration::backup::BACKUP_DIR_NAME))
        .expect("migration backup dir");

    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let events = runtime.open_project_path_events(project.clone());

    assert!(
        events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::MigrationDetected { .. },
            }
        )),
        "Normal Git layout should still open a migration-pending tab"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::MigrationError {
                    phase,
                    recovery,
                    message,
                    ..
                },
            } if phase == "backup"
                && recovery == "partial"
                && message.contains(".gwt-migration-backup")
        )),
        "existing migration backup must be surfaced as a recovery error"
    );
}

// SPEC-2041 Phase 19 (FR-065 / CodeRabbit review on PR #2630): renderer-
// supplied log paths must canonicalize into the gwt update logs root.
// These tests cover the `validate_update_log_path` pure validator.
#[test]
fn validate_update_log_path_accepts_file_inside_logs_root() {
    let logs_root = tempfile::tempdir().expect("logs root tempdir");
    let log_file = logs_root.path().join("update-2026-05-10.log");
    std::fs::write(&log_file, b"{}\n").unwrap();

    let resolved = super::validate_update_log_path(log_file.to_str().unwrap(), logs_root.path());
    assert!(
        resolved.is_some(),
        "expected file inside logs_root to validate"
    );
    let resolved = resolved.unwrap();
    assert!(resolved.is_absolute());
    assert!(resolved.ends_with("update-2026-05-10.log"));
}

#[test]
fn validate_update_log_path_rejects_files_outside_logs_root() {
    let logs_root = tempfile::tempdir().expect("logs root tempdir");
    let outside = tempfile::tempdir().expect("outside tempdir");
    let outside_file = outside.path().join("evil.txt");
    std::fs::write(&outside_file, b"steal me").unwrap();

    let resolved =
        super::validate_update_log_path(outside_file.to_str().unwrap(), logs_root.path());
    assert!(resolved.is_none(), "outside-root paths must be rejected");
}

#[test]
fn validate_update_log_path_rejects_url_schemes_and_empty() {
    let logs_root = tempfile::tempdir().expect("logs root tempdir");
    for raw in [
        "",
        "   ",
        "http://evil.example/log",
        "https://evil.example/log",
        "file:///etc/passwd",
    ] {
        assert!(
            super::validate_update_log_path(raw, logs_root.path()).is_none(),
            "expected `{raw}` to be rejected",
        );
    }
}

#[test]
fn validate_update_log_path_rejects_directories() {
    let logs_root = tempfile::tempdir().expect("logs root tempdir");
    // Caller passes the logs root itself; a directory must not be opened
    // as a file.
    let resolved =
        super::validate_update_log_path(logs_root.path().to_str().unwrap(), logs_root.path());
    assert!(resolved.is_none(), "directories must be rejected");
}

#[test]
fn validate_update_log_path_rejects_missing_files() {
    let logs_root = tempfile::tempdir().expect("logs root tempdir");
    let missing = logs_root.path().join("does-not-exist.log");
    let resolved = super::validate_update_log_path(missing.to_str().unwrap(), logs_root.path());
    assert!(resolved.is_none(), "missing files must be rejected");
}

// SPEC-2785 FR-E: open_server_url requests must be gated by an exact
// same-origin match against the embedded server's bound URL. The shared
// validator function is reused by `AppRuntime::open_server_url_events`
// so a mismatched origin cannot smuggle an arbitrary URL into the OS
// opener.
#[test]
fn validate_server_url_accepts_exact_bound_url() {
    let allowed = Some("http://127.0.0.1:54321/");
    assert!(super::validate_server_url(
        allowed,
        "http://127.0.0.1:54321/"
    ));
}

#[test]
fn validate_server_url_rejects_different_port() {
    let allowed = Some("http://127.0.0.1:54321/");
    assert!(!super::validate_server_url(
        allowed,
        "http://127.0.0.1:54322/"
    ));
}

#[test]
fn validate_server_url_rejects_different_scheme() {
    let allowed = Some("http://127.0.0.1:54321/");
    assert!(!super::validate_server_url(
        allowed,
        "https://127.0.0.1:54321/"
    ));
}

#[test]
fn validate_server_url_rejects_when_allowed_is_none() {
    assert!(!super::validate_server_url(None, "http://127.0.0.1:54321/"));
}

#[test]
fn validate_server_url_rejects_external_origin() {
    let allowed = Some("http://127.0.0.1:54321/");
    assert!(!super::validate_server_url(allowed, "http://evil.example/"));
}

// SPEC-2785 SC-4: `open_server_url_events` returns an empty event list and
// performs no OS opener side effect when the requested URL does not match
// the configured server URL. The state mutation guard is `server_url`
// being None or unequal to the request.
#[test]
fn open_server_url_events_rejects_mismatched_origin() {
    let temp = tempdir().expect("tempdir");
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), Vec::new(), None);
    runtime.set_server_url("http://127.0.0.1:54321/".to_string());
    let outbound = runtime.open_server_url_events("client-1", "http://evil.example/".to_string());
    assert!(
        outbound.is_empty(),
        "mismatched origin must yield no outbound events"
    );
}

#[test]
fn open_server_url_events_rejects_when_server_url_unset() {
    let temp = tempdir().expect("tempdir");
    let (runtime, _events) = sample_runtime_with_events(temp.path(), Vec::new(), None);
    let outbound =
        runtime.open_server_url_events("client-1", "http://127.0.0.1:54321/".to_string());
    assert!(
        outbound.is_empty(),
        "unset server URL must reject any open request"
    );
}

#[test]
fn codex_hook_discovery_mode_switches_at_codex_0_131_alpha_21() {
    use gwt_skills::CodexHookDiscoveryMode;

    assert_eq!(
        super::codex_hook_discovery_mode_from_selected_codex_version(Some("0.130.0")),
        Some(CodexHookDiscoveryMode::WorktreeLocal)
    );
    assert_eq!(
        super::codex_hook_discovery_mode_from_selected_codex_version(Some("0.131.0-alpha.9")),
        Some(CodexHookDiscoveryMode::WorktreeLocal)
    );
    assert_eq!(
        super::codex_hook_discovery_mode_from_selected_codex_version(Some("0.131.0-alpha.21")),
        Some(CodexHookDiscoveryMode::WorkspaceHome)
    );
    assert_eq!(
        super::codex_hook_discovery_mode_from_selected_codex_version(Some("0.131.0")),
        Some(CodexHookDiscoveryMode::WorkspaceHome)
    );
    assert_eq!(
        super::codex_hook_discovery_mode_from_selected_codex_version(Some("latest")),
        Some(CodexHookDiscoveryMode::WorkspaceHome)
    );
    assert_eq!(
        super::codex_hook_discovery_mode_from_selected_codex_version(Some("installed")),
        None
    );
}

#[test]
fn codex_hook_discovery_mode_extracts_installed_codex_version_output() {
    use gwt_skills::CodexHookDiscoveryMode;

    assert_eq!(
        super::codex_hook_discovery_mode_from_codex_version_output("codex-cli 0.133.0\n"),
        Some(CodexHookDiscoveryMode::WorkspaceHome)
    );
    assert_eq!(
        super::codex_hook_discovery_mode_from_codex_version_output("codex 0.130.0\n"),
        Some(CodexHookDiscoveryMode::WorktreeLocal)
    );
    assert_eq!(
        super::codex_hook_discovery_mode_from_codex_version_output("unexpected output\n"),
        None
    );
}

#[test]
fn codex_hook_trust_launch_enabled_registers_host_codex_hooks() {
    let home = tempdir().expect("home tempdir");
    let profile_config_path = home.path().join(".gwt/config.toml");
    let mut settings = Settings::default();
    settings.agent.codex_trust_managed_hooks = Some(true);
    settings.save(&profile_config_path).unwrap();

    let worktree = tempdir().expect("worktree tempdir");
    gwt_skills::generate_codex_hooks(worktree.path()).unwrap();

    let report = super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &gwt_agent::AgentId::Codex,
        gwt_agent::LaunchRuntimeTarget::Host,
        None,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
    )
    .unwrap()
    .expect("enabled host Codex launch should register trust");

    assert_eq!(report.trusted_entries.len(), 5);
    let codex_config_path = home.path().join(".codex/config.toml");
    let config = fs::read_to_string(&codex_config_path).unwrap();
    assert!(
        config.contains("trusted_hash"),
        "Codex config should contain trusted hashes, got: {config}"
    );
    assert_eq!(report.config_path, codex_config_path);
}

#[test]
fn codex_hook_trust_launch_uses_effective_codex_home_config() {
    let home = tempdir().expect("home tempdir");
    let profile_config_path = home.path().join(".gwt/config.toml");
    let worktree = tempdir().expect("worktree tempdir");
    let codex_home = tempdir().expect("codex home");
    gwt_skills::generate_codex_hooks(worktree.path()).unwrap();

    let report = super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &gwt_agent::AgentId::Codex,
        gwt_agent::LaunchRuntimeTarget::Host,
        None,
        Some(codex_home.path()),
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
    )
    .unwrap()
    .expect("Codex launch should register trust into the effective CODEX_HOME");

    let codex_home_config = codex_home.path().join("config.toml");
    assert_eq!(report.config_path, codex_home_config);
    let config = fs::read_to_string(&codex_home_config).unwrap();
    assert!(
        config.contains("trusted_hash"),
        "effective CODEX_HOME config should contain trusted hashes, got: {config}"
    );
    assert!(
        !home.path().join(".codex/config.toml").exists(),
        "backend override launches must not write trust state to the profile-derived Codex home"
    );
}

#[test]
fn codex_hook_trust_launch_defaults_to_host_codex_registration_and_false_opts_out() {
    let home = tempdir().expect("home tempdir");
    let profile_config_path = home.path().join(".gwt/config.toml");
    let worktree = tempdir().expect("worktree tempdir");
    gwt_skills::generate_codex_hooks(worktree.path()).unwrap();

    let unset = super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &gwt_agent::AgentId::Codex,
        gwt_agent::LaunchRuntimeTarget::Host,
        None,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
    )
    .unwrap();
    assert_eq!(
        unset
            .expect("unset config should default to trusting managed Codex hooks")
            .trusted_entries
            .len(),
        5
    );
    fs::remove_file(home.path().join(".codex/config.toml")).unwrap();

    let mut settings = Settings::default();
    settings.agent.codex_trust_managed_hooks = Some(false);
    settings.save(&profile_config_path).unwrap();
    let disabled = super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &gwt_agent::AgentId::Codex,
        gwt_agent::LaunchRuntimeTarget::Host,
        None,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
    )
    .unwrap();
    assert!(disabled.is_none());

    assert!(
        !home.path().join(".codex/config.toml").exists(),
        "false opt-out must not recreate Codex config"
    );

    settings.agent.codex_trust_managed_hooks = Some(true);
    settings.save(&profile_config_path).unwrap();
    let enabled = super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &gwt_agent::AgentId::Codex,
        gwt_agent::LaunchRuntimeTarget::Host,
        None,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
    )
    .unwrap();
    assert_eq!(
        enabled
            .expect("true config should register managed Codex hooks")
            .trusted_entries
            .len(),
        5
    );

    let claude = super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &gwt_agent::AgentId::ClaudeCode,
        gwt_agent::LaunchRuntimeTarget::Host,
        None,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
    )
    .unwrap();
    assert!(claude.is_none());

    assert!(
        home.path().join(".codex/config.toml").exists(),
        "host default/true paths must create Codex config"
    );
}

#[test]
fn codex_hook_trust_launch_is_warning_only_when_registration_fails() {
    let home = tempdir().expect("home tempdir");
    let profile_config_path = home.path().join(".gwt/config.toml");
    let worktree = tempdir().expect("worktree tempdir");
    gwt_skills::generate_codex_hooks(worktree.path()).unwrap();

    let codex_config_parent = home.path().join(".codex");
    fs::write(&codex_config_parent, "not a directory").unwrap();

    let result = super::maybe_register_codex_managed_hook_trust_for_launch(
        &profile_config_path,
        worktree.path(),
        &gwt_agent::AgentId::Codex,
        gwt_agent::LaunchRuntimeTarget::Host,
        None,
        None,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
    );

    assert!(
        result.is_ok(),
        "optional trust registration must not abort launch: {result:?}"
    );
    assert!(
        result.unwrap().is_none(),
        "skip paths must not create Codex config"
    );
}

#[test]
fn workspace_view_for_tab_omits_work_item_history_from_workspace_state() {
    // SPEC-2359 CPU/power follow-up: workspace_state is broadcast frequently
    // and must stay structural. Workspace history/work items are carried by
    // active_work_projection so every window/status update does not serialize
    // the full work item event log.
    let _env_guard = env_test_lock().lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");

    use chrono::TimeZone as _;
    let completed_at = chrono::Utc.with_ymd_and_hms(2026, 5, 14, 10, 0, 0).unwrap();
    let work_item = gwt_core::workspace_projection::WorkItem {
        id: "work-item-done".to_string(),
        title: "Test Done Item".to_string(),
        intent: None,
        summary: None,
        progress_summary: None,
        status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Done,
        owner: None,
        created_at: completed_at,
        updated_at: completed_at,
        completed_at: Some(completed_at),
        agents: Vec::new(),
        execution_containers: Vec::new(),
        board_refs: Vec::new(),
        related_work_item_ids: Vec::new(),
        events: Vec::new(),
        legacy_metadata_snapshot: None,
        legacy_metadata_authoritative: false,
        legacy_metadata_snapshot_at: None,
        duplicate_event_containers: Default::default(),
        discarded: false,
        discarded_at: None,
    };
    let projection = gwt_core::workspace_projection::WorkItemsProjection {
        updated_at: completed_at,
        work_items: vec![work_item],
    };
    let work_items_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
    fs::create_dir_all(work_items_path.parent().expect("parent dir"))
        .expect("create workspace dir");
    gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
        &work_items_path,
        &projection,
    )
    .expect("save work items projection");

    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(empty_workspace_state()),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };

    let view = crate::runtime_support::workspace_view_for_tab(&tab);
    assert!(
            view.work_items.is_empty(),
            "WorkspaceView.work_items must stay empty because workspace_state is a hot broadcast path; active_work_projection owns Workspace history"
        );
}

// SPEC-1924 US-14 FR-035 / FR-036 / SC-010 / SC-011 — verify the Logs
// window snapshot reader goes through `gwt_core::logging::read_log_file`
// and that the synthetic warning event is well-formed when malformed
// lines are skipped.

const PROD_LINE_INFO: &str = r#"{"timestamp":"2026-05-20T09:00:00.015355+09:00","level":"INFO","fields":{"message":"PTY resize completed","outcome":"ok"},"target":"gwt::resize::pty"}"#;
const MALFORMED_LINE: &str = r#"{"foo":"bar"}"#;

fn write_canonical_log_file(log_dir: &Path, lines: &[&str]) {
    fs::create_dir_all(log_dir).expect("create log dir");
    let log_path = current_log_file(log_dir);
    let mut file = fs::File::create(&log_path).expect("create log file");
    for line in lines {
        file.write_all(line.as_bytes()).expect("write line");
        file.write_all(b"\n").expect("write newline");
    }
}

#[test]
fn load_log_entries_from_dir_returns_outcome_with_no_skipped_lines() {
    let dir = tempdir().expect("tempdir");
    write_canonical_log_file(dir.path(), &[PROD_LINE_INFO]);

    let outcome = super::load_log_entries_from_dir(dir.path()).expect("read ok");

    assert_eq!(outcome.entries.len(), 1);
    assert_eq!(outcome.entries[0].message, "PTY resize completed");
    assert_eq!(outcome.diagnostics.skipped, 0);
}

#[test]
fn load_log_entries_from_dir_counts_skipped_lines() {
    let dir = tempdir().expect("tempdir");
    write_canonical_log_file(
        dir.path(),
        &[PROD_LINE_INFO, MALFORMED_LINE, PROD_LINE_INFO],
    );

    let outcome = super::load_log_entries_from_dir(dir.path()).expect("read ok");

    assert_eq!(outcome.entries.len(), 2);
    assert_eq!(outcome.diagnostics.skipped, 1);
}

#[test]
fn load_log_entries_from_dir_returns_empty_outcome_when_file_missing() {
    let dir = tempdir().expect("tempdir");

    let outcome = super::load_log_entries_from_dir(dir.path()).expect("read ok");

    assert!(outcome.entries.is_empty());
    assert_eq!(outcome.diagnostics.skipped, 0);
}

#[test]
fn skipped_lines_warning_is_warn_severity_and_includes_count_and_path() {
    let diagnostics = gwt_core::logging::ReadDiagnostics {
        path: PathBuf::from("/tmp/gwt.log.2026-05-20"),
        skipped: 3,
    };

    let event = super::skipped_lines_warning(&diagnostics);

    assert_eq!(event.severity, LogLevel::Warn);
    assert_eq!(event.source, "gwt_core::logging::reader");
    assert!(event.message.contains("Skipped 3 malformed lines"));
    assert!(event.message.contains("/tmp/gwt.log.2026-05-20"));
}

#[test]
fn skipped_lines_warning_singular_for_one_line() {
    let diagnostics = gwt_core::logging::ReadDiagnostics {
        path: PathBuf::from("/tmp/x.log"),
        skipped: 1,
    };

    let event = super::skipped_lines_warning(&diagnostics);

    assert!(event.message.contains("Skipped 1 malformed line "));
}

#[test]
fn os_url_open_command_keeps_oauth_query_intact_and_avoids_cmd() {
    // Regression: `cmd /C start "" <url>` truncated OAuth authorize URLs at
    // the first `&`, dropping redirect_uri/scope/state and breaking Slack
    // sign-in. The opener must pass the whole URL as one argument and must
    // not route through cmd.exe (whose shell parsing splits on `&`).
    let url = "https://slack.com/oauth/v2/authorize?client_id=A&redirect_uri=http%3A%2F%2F127.0.0.1%3A8765%2Foauth%2Fcallback&scope=chat%3Awrite%2Cchannels%3Aread&state=xyz";
    let (program, args) = super::os_url_open_command(url);

    assert!(
        args.iter().any(|arg| arg == url),
        "the full URL must be passed as a single intact argument, got {args:?}"
    );
    assert_ne!(program, "cmd", "must not open URLs through cmd.exe");
    assert!(
        !args.iter().any(|arg| arg.contains("start")),
        "must not use the cmd `start` builtin which splits on &: {args:?}"
    );
}

/// SPEC-2359 Phase W-15 (FR-380): the Backfill kind reaches the frontend as
/// "backfill" on the wire.
#[test]
fn workspace_work_event_kind_wire_maps_backfill() {
    assert_eq!(
        super::workspace_work_event_kind_wire(
            gwt_core::workspace_projection::WorkEventKind::Backfill
        ),
        "backfill"
    );
}

/// SPEC-2359 Phase W-15 (FR-379/FR-382, T-547): a real linked worktree with no
/// Work record is backfilled by `reconcile_workspace_worktrees` and surfaces
/// on the Workspace list as a Paused row. Repeated reconciliation is
/// idempotent (SC-255).
#[test]
fn app_runtime_reconcile_workspace_worktrees_backfills_existing_worktree() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    for args in [
        ["config", "user.email", "test@example.com"].as_slice(),
        ["config", "user.name", "Test User"].as_slice(),
        ["commit", "--allow-empty", "-m", "init"].as_slice(),
    ] {
        let output = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(&repo)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let worktree = temp.path().join("repo-foo");
    let output = gwt_core::process::hidden_command("git")
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            "work/foo",
            worktree.to_str().unwrap(),
        ])
        .current_dir(&repo)
        .output()
        .expect("git worktree add");
    assert!(
        output.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // SPEC-2359 Phase W-15 (FR-382): deliberately NO saved WorkspaceProjection
    // (current.json) and NO live agent session — a fresh home must still
    // surface backfilled records (the list must not depend on live agents or
    // on a previously launched project).
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.reconcile_workspace_worktrees(&repo);
    runtime.reconcile_workspace_worktrees(&repo);

    let work_items_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
    let projection =
        gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
            .expect("load works")
            .expect("works projection exists");
    let expected_main = gwt_core::workspace_projection::canonical_work_id(
        &repo,
        repo_head_branch(&repo).as_deref(),
        None,
    );
    let expected_foo =
        gwt_core::workspace_projection::canonical_work_id(&repo, Some("work/foo"), None)
            .expect("canonical id");
    assert!(
        projection
            .work_items
            .iter()
            .any(|item| item.id == expected_foo),
        "work/foo worktree must be backfilled: {:?}",
        projection
            .work_items
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>()
    );
    let foo_count = projection
        .work_items
        .iter()
        .filter(|item| item.id == expected_foo)
        .count();
    assert_eq!(foo_count, 1, "repeated reconcile must not duplicate items");
    let _ = expected_main;

    let events_path = gwt_core::paths::gwt_repo_local_work_events_path(&worktree);
    let events_text = fs::read_to_string(&events_path).expect("worktree events log");
    assert_eq!(
        events_text.lines().count(),
        1,
        "exactly one backfill event line after two reconciles"
    );

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let row = view
        .active_works
        .iter()
        .find(|work| work.id == expected_foo)
        .expect("backfilled Workspace row on the surface");
    assert_eq!(row.lifecycle_state, "paused");
    assert_eq!(row.branch.as_deref(), Some("work/foo"));
}

fn repo_head_branch(repo: &Path) -> Option<String> {
    let output = gwt_core::process::hidden_command("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!branch.is_empty()).then_some(branch)
}

/// SPEC-2359 Phase W-16 (FR-402, T-571): a Workspace row whose record has no
/// agents gains them from the machine-local session ledger — sessions whose
/// TOML carries the same repo hash and branch attach to the row with their
/// conversation history, and `session_agent_total` reports the count.
#[test]
fn app_runtime_active_work_projection_attaches_registry_sessions() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    for args in [
        ["config", "user.email", "test@example.com"].as_slice(),
        ["config", "user.name", "Test User"].as_slice(),
        ["commit", "--allow-empty", "-m", "init"].as_slice(),
    ] {
        let output = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(&repo)
            .output()
            .expect("run git");
        assert!(output.status.success());
    }
    let worktree = temp.path().join("repo-foo");
    let output = gwt_core::process::hidden_command("git")
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            "work/foo",
            worktree.to_str().unwrap(),
        ])
        .current_dir(&repo)
        .output()
        .expect("git worktree add");
    assert!(output.status.success());

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    // Machine-local ledger entry for the branch (no record agents exist).
    let mut session =
        gwt_agent::Session::new(&worktree, "work/foo", gwt_agent::AgentId::ClaudeCode);
    session.agent_session_id = Some("conv-1".to_string());
    session.session_history = vec![gwt_agent::AgentSessionHistoryEntry {
        agent_session_id: "conv-1".to_string(),
        started_at: chrono::Utc::now(),
    }];
    assert!(
        session.repo_hash.is_some(),
        "fixture session must derive the repo hash from its worktree"
    );
    session.save(&runtime.sessions_dir).expect("save session");

    runtime.reconcile_workspace_worktrees(&repo);

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let expected_id =
        gwt_core::workspace_projection::canonical_work_id(&repo, Some("work/foo"), None).unwrap();
    let row = view
        .active_works
        .iter()
        .find(|work| work.id == expected_id)
        .expect("backfilled Workspace row");
    assert_eq!(
        row.agents.len(),
        1,
        "ledger session must attach to the branch Workspace"
    );
    assert_eq!(row.agents[0].session_id, session.id);
    assert_eq!(row.agents[0].display_name, "Claude Code");
    assert_eq!(row.agents[0].sessions.len(), 1);
    assert_eq!(row.agents[0].sessions[0].agent_session_id, "conv-1");
    assert_eq!(
        row.works.len(),
        1,
        "backfilled Workspace has one child Work"
    );
    assert_eq!(
        row.works[0].agents.len(),
        1,
        "ledger session must also attach to the child Work"
    );
    assert_eq!(row.works[0].agents[0].session_id, session.id);
    assert_eq!(row.works[0].agents[0].sessions.len(), 1);
    assert_eq!(
        row.works[0].agents[0].sessions[0].agent_session_id,
        "conv-1"
    );
    assert_eq!(row.session_agent_total, 1);
}

/// SPEC-2359 Phase W-16 (FR-402): the wire cap applies to the row's TOTAL
/// agents (record agents included), not just registry additions — a
/// decomposed legacy row can carry hundreds of record agents and must not
/// flood the workspace payload. The uncapped count rides
/// `session_agent_total`, newest agents win.
#[test]
fn attach_registry_sessions_caps_total_agents_on_the_wire() {
    let agents: Vec<gwt::ActiveWorkAgentView> = (0..12)
        .map(|index| gwt::ActiveWorkAgentView {
            session_id: format!("rec-{index:02}"),
            window_id: None,
            agent_id: format!("custom-agent-{index:02}"),
            display_name: format!("Claude {index:02}"),
            affiliation_status: "assigned".to_string(),
            workspace_id: None,
            status_category: "idle".to_string(),
            current_focus: None,
            title_summary: None,
            branch: None,
            worktree_path: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: format!("2026-06-10T12:{index:02}:00Z"),
            sessions: Vec::new(),
        })
        .collect();
    let mut works = vec![gwt::ActiveWorkItemView {
        id: "work-develop-7ea5aa57".to_string(),
        title: "develop".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some("develop".to_string()),
        worktree_path: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents,
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: String::new(),
    }];

    super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        Path::new("/"),
    );

    assert_eq!(works[0].session_agent_total, 12, "uncapped count reported");
    assert_eq!(
        works[0].agents.len(),
        crate::workspace_session_registry::REGISTRY_SESSION_CAP,
        "wire payload capped"
    );
    assert_eq!(
        works[0].agents[0].session_id, "rec-11",
        "newest agents win the cap"
    );
}

/// User verification 2026-06-17 (follow-up): Workspace detail is a session
/// summary, not a live process inventory. Per agent identity only the latest
/// entry stays; live (active/running/blocked) duplicates are collapsed too.
#[test]
fn attach_registry_sessions_keeps_latest_entry_per_agent_identity() {
    fn agent_with_conv(
        session_id: &str,
        display_name: &str,
        status_category: &str,
        updated_at: &str,
        conversation: &str,
    ) -> gwt::ActiveWorkAgentView {
        gwt::ActiveWorkAgentView {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: String::new(),
            display_name: display_name.to_string(),
            affiliation_status: "assigned".to_string(),
            workspace_id: None,
            status_category: status_category.to_string(),
            current_focus: None,
            title_summary: None,
            branch: None,
            worktree_path: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: updated_at.to_string(),
            sessions: vec![gwt::WorkspaceHistorySessionView {
                agent_session_id: conversation.to_string(),
                started_at: updated_at.to_string(),
                is_active: true,
                resumable: true,
            }],
        }
    }

    let mut works = vec![gwt::ActiveWorkItemView {
        id: "work-develop-7ea5aa57".to_string(),
        title: "develop".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some("develop".to_string()),
        worktree_path: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents: vec![
            agent_with_conv(
                "c1",
                "Claude Code",
                "idle",
                "2026-06-11T10:00:00Z",
                "conv-c1",
            ),
            agent_with_conv(
                "c2",
                "Claude Code",
                "idle",
                "2026-06-12T02:00:00Z",
                "conv-c2",
            ),
            agent_with_conv(
                "c3",
                "Claude Code",
                "idle",
                "2026-06-10T08:00:00Z",
                "conv-c3",
            ),
            agent_with_conv("x1", "Codex", "idle", "2026-06-09T00:00:00Z", "conv-x1"),
            // A live agent of the same identity is still a duplicate in the
            // Workspace session summary.
            agent_with_conv(
                "c-live",
                "Claude Code",
                "active",
                "2026-06-08T00:00:00Z",
                "conv-l",
            ),
        ],
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: String::new(),
    }];

    super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        Path::new("/"),
    );

    let agents = &works[0].agents;
    let ids: Vec<&str> = agents
        .iter()
        .map(|agent| agent.session_id.as_str())
        .collect();
    assert!(
        ids.contains(&"c2"),
        "newest Claude Code history entry stays"
    );
    assert!(ids.contains(&"x1"), "the other agent identity stays");
    assert!(
        !ids.contains(&"c-live"),
        "older live duplicate collapses under the latest Claude Code row"
    );
    assert!(!ids.contains(&"c1"), "older duplicates collapse");
    assert!(!ids.contains(&"c3"), "older duplicates collapse");
    assert_eq!(agents.len(), 2);
    assert_eq!(
        works[0].session_agent_total, 5,
        "hidden same-agent candidates stay counted for the '+N more sessions' summary"
    );
}

fn workspace_test_agent_with_conversation(
    session_id: &str,
    updated_at: &str,
    conversation: &str,
) -> gwt::ActiveWorkAgentView {
    gwt::ActiveWorkAgentView {
        session_id: session_id.to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        affiliation_status: "assigned".to_string(),
        workspace_id: None,
        status_category: "idle".to_string(),
        current_focus: None,
        title_summary: None,
        branch: Some("work/shared".to_string()),
        worktree_path: None,
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        updated_at: updated_at.to_string(),
        sessions: vec![gwt::WorkspaceHistorySessionView {
            agent_session_id: conversation.to_string(),
            started_at: updated_at.to_string(),
            is_active: true,
            resumable: true,
        }],
    }
}

fn workspace_test_child(
    id: &str,
    agents: Vec<gwt::ActiveWorkAgentView>,
) -> gwt::ActiveWorkspaceWorkView {
    gwt::ActiveWorkspaceWorkView {
        id: id.to_string(),
        title: id.to_string(),
        work_summary: None,
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        owner: None,
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        manual_close_allowed: true,
        close_blocked_reason: None,
        agents,
        updated_at: String::new(),
    }
}

fn workspace_test_work(
    agents: Vec<gwt::ActiveWorkAgentView>,
    works: Vec<gwt::ActiveWorkspaceWorkView>,
) -> gwt::ActiveWorkItemView {
    gwt::ActiveWorkItemView {
        id: "work-shared".to_string(),
        title: "work/shared".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some("work/shared".to_string()),
        worktree_path: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        session_agent_total: agents.len() as u32,
        agents,
        works,
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: String::new(),
    }
}

#[test]
fn attach_registry_sessions_preserves_same_identity_agent_per_child_work() {
    let older = workspace_test_agent_with_conversation(
        "older-session",
        "2026-07-12T01:00:00Z",
        "older-conv",
    );
    let newer = workspace_test_agent_with_conversation(
        "newer-session",
        "2026-07-13T01:00:00Z",
        "newer-conv",
    );
    let mut works = vec![workspace_test_work(
        vec![older.clone(), newer.clone()],
        vec![
            workspace_test_child("work-session-older-session", vec![older]),
            workspace_test_child("work-session-newer-session", vec![newer]),
        ],
    )];

    super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        Path::new("/"),
    );

    assert_eq!(
        works[0]
            .agents
            .iter()
            .map(|agent| agent.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["newer-session"],
        "the Workspace summary still keeps only the latest agent identity"
    );
    for (work_id, session_id) in [
        ("work-session-older-session", "older-session"),
        ("work-session-newer-session", "newer-session"),
    ] {
        let child = works[0]
            .works
            .iter()
            .find(|child| child.id == work_id)
            .expect("child Work");
        assert_eq!(
            child
                .agents
                .iter()
                .map(|agent| agent.session_id.as_str())
                .collect::<Vec<_>>(),
            vec![session_id],
            "each child Work must preserve its own Agent and Session"
        );
    }
}

#[test]
fn attach_registry_sessions_preserves_distinct_conversations_within_one_child_work() {
    let older = workspace_test_agent_with_conversation(
        "older-session",
        "2026-07-12T01:00:00Z",
        "older-conv",
    );
    let newer = workspace_test_agent_with_conversation(
        "newer-session",
        "2026-07-13T01:00:00Z",
        "newer-conv",
    );
    let mut works = vec![workspace_test_work(
        vec![older.clone(), newer.clone()],
        vec![workspace_test_child("work-combined", vec![older, newer])],
    )];

    super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        Path::new("/"),
    );

    assert_eq!(
        works[0].works[0]
            .agents
            .iter()
            .map(|agent| agent.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["newer-session", "older-session"],
        "a child Work owns every distinct conversation selected for the payload"
    );
    assert_eq!(
        works[0]
            .agents
            .iter()
            .map(|agent| agent.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["newer-session"],
        "identity collapse remains limited to the Workspace summary"
    );
}

#[test]
fn attach_registry_sessions_collapses_same_conversation_across_child_works() {
    let older = workspace_test_agent_with_conversation(
        "older-session",
        "2026-07-12T01:00:00Z",
        "shared-conv",
    );
    let newer = workspace_test_agent_with_conversation(
        "newer-session",
        "2026-07-13T01:00:00Z",
        "shared-conv",
    );
    let mut works = vec![workspace_test_work(
        vec![older.clone(), newer.clone()],
        vec![
            workspace_test_child("work-session-older-session", vec![older]),
            workspace_test_child("work-session-newer-session", vec![newer]),
        ],
    )];

    super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        Path::new("/"),
    );

    assert!(
        works[0].works[0].agents.is_empty(),
        "the older sibling must not retain a duplicate conversation"
    );
    assert_eq!(
        works[0].works[1].agents[0].session_id, "newer-session",
        "the newest sibling owns the shared conversation"
    );
    assert_eq!(works[0].session_agent_total, 1);
}

#[test]
fn attach_registry_sessions_caps_agents_across_all_child_works() {
    let agents = (0..12)
        .map(|index| {
            workspace_test_agent_with_conversation(
                &format!("session-{index:02}"),
                &format!("2026-07-12T{index:02}:00:00Z"),
                &format!("conv-{index:02}"),
            )
        })
        .collect::<Vec<_>>();
    let child_works = agents
        .iter()
        .cloned()
        .map(|agent| {
            workspace_test_child(&format!("work-session-{}", agent.session_id), vec![agent])
        })
        .collect();
    let mut works = vec![workspace_test_work(agents, child_works)];

    super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        Path::new("/"),
    );

    assert_eq!(
        works[0]
            .works
            .iter()
            .map(|child| child.agents.len())
            .sum::<usize>(),
        crate::workspace_session_registry::REGISTRY_SESSION_CAP,
        "the wire cap applies to the union of child Work agents"
    );
    assert!(
        works[0]
            .works
            .iter()
            .take(4)
            .all(|child| child.agents.is_empty()),
        "the oldest sessions fall outside the cap"
    );
}

#[test]
fn attach_registry_sessions_assigns_shared_session_to_one_canonical_child_work() {
    let agent = workspace_test_agent_with_conversation(
        "shared-session",
        "2026-07-13T01:00:00Z",
        "shared-conv",
    );
    let mut child_works = (0..11)
        .map(|index| workspace_test_child(&format!("legacy-work-{index:02}"), vec![agent.clone()]))
        .collect::<Vec<_>>();
    child_works.insert(
        5,
        workspace_test_child("work-session-shared-session", vec![agent.clone()]),
    );
    let mut works = vec![workspace_test_work(vec![agent], child_works)];

    super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        Path::new("/"),
    );

    assert_eq!(
        works[0]
            .works
            .iter()
            .map(|child| child.agents.len())
            .sum::<usize>(),
        1,
        "one payload Agent must not be duplicated across child Works"
    );
    assert_eq!(
        works[0]
            .works
            .iter()
            .find(|child| child.id == "work-session-shared-session")
            .expect("canonical child Work")
            .agents[0]
            .session_id,
        "shared-session",
        "the canonical session-derived child owns the shared Agent"
    );
    assert_eq!(works[0].session_agent_total, 1);
}

#[test]
fn attach_registry_sessions_assigns_shared_session_to_latest_legacy_child_work() {
    let agent = workspace_test_agent_with_conversation(
        "shared-session",
        "2026-07-13T01:00:00Z",
        "shared-conv",
    );
    let mut older = workspace_test_child("legacy-work-older", vec![agent.clone()]);
    older.updated_at = "2026-07-12T01:00:00Z".to_string();
    let mut newer = workspace_test_child("legacy-work-newer", vec![agent.clone()]);
    newer.updated_at = "2026-07-13T01:00:00Z".to_string();
    let mut works = vec![workspace_test_work(vec![agent], vec![older, newer])];

    super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        Path::new("/"),
    );

    assert!(works[0].works[0].agents.is_empty());
    assert_eq!(
        works[0].works[1].agents[0].session_id, "shared-session",
        "the latest compatible legacy child owns the shared Agent"
    );
}

#[test]
fn attach_registry_sessions_recomputes_agent_counters_after_identity_collapse() {
    fn agent_view(
        session_id: &str,
        display_name: &str,
        status_category: &str,
        updated_at: &str,
    ) -> gwt::ActiveWorkAgentView {
        gwt::ActiveWorkAgentView {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: String::new(),
            display_name: display_name.to_string(),
            affiliation_status: "assigned".to_string(),
            workspace_id: None,
            status_category: status_category.to_string(),
            current_focus: None,
            title_summary: None,
            branch: None,
            worktree_path: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: updated_at.to_string(),
            sessions: vec![gwt::WorkspaceHistorySessionView {
                agent_session_id: format!("conv-{session_id}"),
                started_at: updated_at.to_string(),
                is_active: true,
                resumable: true,
            }],
        }
    }

    let mut works = vec![gwt::ActiveWorkItemView {
        id: "work-develop-7ea5aa57".to_string(),
        title: "develop".to_string(),
        status_category: "active".to_string(),
        status_text: "Active".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 99,
        blocked_agents: 99,
        branch: Some("develop".to_string()),
        worktree_path: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents: vec![
            agent_view(
                "claude-old",
                "Claude Code",
                "active",
                "2026-06-10T00:00:00Z",
            ),
            agent_view(
                "claude-new",
                "Claude Code",
                "running",
                "2026-06-11T00:00:00Z",
            ),
            agent_view("codex-blocked", "Codex", "blocked", "2026-06-09T00:00:00Z"),
        ],
        works: Vec::new(),
        lifecycle_state: "active".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: String::new(),
    }];

    super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        Path::new("/"),
    );

    let ids: Vec<&str> = works[0]
        .agents
        .iter()
        .map(|agent| agent.session_id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["claude-new", "codex-blocked"],
        "latest visible agent per identity determines counters"
    );
    assert_eq!(works[0].active_agents, 1);
    assert_eq!(works[0].blocked_agents, 1);
}

/// User verification 2026-06-12 (follow-up): a record agent whose ledger TOML
/// is gone and that recorded no identity and no conversation renders as a
/// dead "Agent / No session yet" group whose Resume cannot work. Such ghosts
/// are dropped from the view; identifiable or conversation-bearing agents stay.
#[test]
fn attach_registry_sessions_drops_ghost_agents_without_identity_or_sessions() {
    fn bare_agent(session_id: &str, display_name: &str) -> gwt::ActiveWorkAgentView {
        gwt::ActiveWorkAgentView {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: String::new(),
            display_name: display_name.to_string(),
            affiliation_status: "assigned".to_string(),
            workspace_id: None,
            status_category: "idle".to_string(),
            current_focus: None,
            title_summary: None,
            branch: None,
            worktree_path: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: "2026-06-12T02:00:00Z".to_string(),
            sessions: Vec::new(),
        }
    }

    let mut works = vec![gwt::ActiveWorkItemView {
        id: "work-work-x-12345678".to_string(),
        title: "work/x".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some("work/x".to_string()),
        worktree_path: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents: vec![
            bare_agent("gwt-ghost", ""),
            bare_agent("gwt-named", "Claude Code"),
        ],
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: String::new(),
    }];

    super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        Path::new("/"),
    );

    let agents = &works[0].agents;
    assert_eq!(
        agents.len(),
        1,
        "the identity-less, session-less ghost is dropped"
    );
    assert_eq!(agents[0].session_id, "gwt-named");
    assert_eq!(works[0].session_agent_total, 1);
}

/// User verification 2026-06-12: Work records written without agent metadata
/// (older record paths) rendered as an anonymous "Agent" group. The view
/// borrows display_name / agent_id from the ledger TOML keyed by the gwt
/// session id, so the group is named whenever the ledger still knows it.
#[test]
fn agent_view_borrows_identity_from_ledger_when_record_has_none() {
    let mut session = gwt_agent::Session::new(
        std::path::PathBuf::from("/tmp/none"),
        "work/foo",
        gwt_agent::AgentId::ClaudeCode,
    );
    session.id = "gwt-session-anon".to_string();
    session.display_name = "Claude Code".to_string();
    let mut index = std::collections::HashMap::new();
    index.insert("gwt-session-anon", &session);

    let agent_ref = gwt_core::workspace_projection::WorkAgentRef {
        session_id: "gwt-session-anon".to_string(),
        agent_id: None,
        display_name: None,
        updated_at: chrono::Utc::now(),
        attached_by: None,
    };
    let view = super::workspace_work_agent_view_from_ref(&agent_ref, &index, Path::new("/"));
    assert_eq!(view.display_name.as_deref(), Some("Claude Code"));
    assert!(view.agent_id.is_some(), "agent_id borrowed from the ledger");
}

/// User verification 2026-06-12: a Resume creates a NEW gwt session for the
/// SAME agent conversation, so the row showed two Work groups with the same
/// conversation id ("Agent" 15m ago + "Claude Code" 1d ago). Agents whose
/// latest conversation matches collapse into one row — newest wins, and a
/// missing display_name is borrowed from the duplicate.
#[test]
fn attach_registry_sessions_dedupes_agents_sharing_a_conversation() {
    fn agent_view(
        session_id: &str,
        display_name: &str,
        updated_at: &str,
        conversation: &str,
    ) -> gwt::ActiveWorkAgentView {
        let agent_id = if display_name == "Codex" {
            "codex"
        } else {
            "claude"
        };
        gwt::ActiveWorkAgentView {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: agent_id.to_string(),
            display_name: display_name.to_string(),
            affiliation_status: "assigned".to_string(),
            workspace_id: None,
            status_category: "idle".to_string(),
            current_focus: None,
            title_summary: None,
            branch: None,
            worktree_path: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: updated_at.to_string(),
            sessions: vec![gwt::WorkspaceHistorySessionView {
                agent_session_id: conversation.to_string(),
                started_at: updated_at.to_string(),
                is_active: true,
                resumable: true,
            }],
        }
    }

    let mut works = vec![gwt::ActiveWorkItemView {
        id: "work-work-x-12345678".to_string(),
        title: "work/x".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some("work/x".to_string()),
        worktree_path: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents: vec![
            // Resume-created record agent: no display_name recorded yet.
            agent_view("gwt-new", "", "2026-06-12T02:05:00Z", "conv-shared"),
            // The original session for the same conversation, a day older.
            agent_view(
                "gwt-old",
                "Claude Code",
                "2026-06-10T04:15:00Z",
                "conv-shared",
            ),
            // A different conversation must survive untouched.
            agent_view("gwt-other", "Codex", "2026-06-11T00:00:00Z", "conv-other"),
        ],
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: String::new(),
    }];

    super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        Path::new("/"),
    );

    let agents = &works[0].agents;
    assert_eq!(agents.len(), 2, "shared conversation collapses to one row");
    let kept = agents
        .iter()
        .find(|agent| agent.sessions[0].agent_session_id == "conv-shared")
        .expect("shared conversation row");
    assert_eq!(kept.session_id, "gwt-new", "newest gwt session wins");
    assert_eq!(
        kept.display_name, "Claude Code",
        "missing display_name is borrowed from the duplicate"
    );
    assert!(agents
        .iter()
        .any(|agent| agent.sessions[0].agent_session_id == "conv-other"));
    assert_eq!(
        works[0].session_agent_total, 2,
        "the collapsed duplicate is not counted as a hidden extra session"
    );
}

/// User verification 2026-06-19: a legacy branchless Work record can carry
/// agent refs from multiple branches. Once such refs reach a branch-backed row,
/// the row must drop sessions whose ledger branch/worktree belongs to another
/// Workspace so the same Codex conversation is not shown under two Workspaces.
#[test]
fn attach_registry_sessions_filters_agents_from_other_workspace_rows() {
    fn agent_view(
        session_id: &str,
        display_name: &str,
        conversation: &str,
    ) -> gwt::ActiveWorkAgentView {
        gwt::ActiveWorkAgentView {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: display_name.to_ascii_lowercase().replace(' ', "-"),
            display_name: display_name.to_string(),
            affiliation_status: "assigned".to_string(),
            workspace_id: None,
            status_category: "idle".to_string(),
            current_focus: None,
            title_summary: None,
            branch: None,
            worktree_path: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: "2026-06-19T13:49:00Z".to_string(),
            sessions: vec![gwt::WorkspaceHistorySessionView {
                agent_session_id: conversation.to_string(),
                started_at: "2026-06-19T13:49:00Z".to_string(),
                is_active: true,
                resumable: true,
            }],
        }
    }

    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("unity-cli");
    let issue_worktree = temp.path().join("unity-cli/work/issue-206");
    let other_worktree = temp.path().join("unity-cli/work/20260616-1102");
    fs::create_dir_all(&issue_worktree).expect("issue worktree");
    fs::create_dir_all(&other_worktree).expect("other worktree");

    let mut issue_session = gwt_agent::Session::new(
        &issue_worktree,
        "work/issue-206",
        gwt_agent::AgentId::ClaudeCode,
    );
    issue_session.id = "78992500-1502-4ab2-8e67-04f79803e013".to_string();
    issue_session.agent_session_id = Some("33939943-240d-461f-bf90-e7b5497e4ee8".to_string());
    issue_session.display_name = "Claude Code".to_string();
    let mut other_session = gwt_agent::Session::new(
        &other_worktree,
        "work/20260616-1102",
        gwt_agent::AgentId::Codex,
    );
    other_session.id = "5b907840-31ee-48d5-a7e3-277c93fda63b".to_string();
    other_session.agent_session_id = Some("019ed018-c208-7183-bb6e-b08ba2ef4981".to_string());
    other_session.display_name = "Codex".to_string();
    let mut session_index = std::collections::HashMap::new();
    session_index.insert(issue_session.id.as_str(), &issue_session);
    session_index.insert(other_session.id.as_str(), &other_session);

    let mut works = vec![gwt::ActiveWorkItemView {
        id: "work-work-issue-206-a0668517".to_string(),
        title: "contribution docs PR".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: Some("Issue #206".to_string()),
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some("work/issue-206".to_string()),
        worktree_path: Some(issue_worktree.display().to_string()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents: vec![
            agent_view(
                &issue_session.id,
                "Claude Code",
                "33939943-240d-461f-bf90-e7b5497e4ee8",
            ),
            agent_view(
                &other_session.id,
                "Codex",
                "019ed018-c208-7183-bb6e-b08ba2ef4981",
            ),
        ],
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: "2026-06-19T13:49:00Z".to_string(),
    }];

    super::attach_registry_sessions_to_active_works(&mut works, &[], None, &session_index, &repo);

    let agents = &works[0].agents;
    assert_eq!(agents.len(), 1, "only sessions owned by this row stay");
    assert_eq!(agents[0].display_name, "Claude Code");
    assert_eq!(
        agents[0].sessions[0].agent_session_id,
        "33939943-240d-461f-bf90-e7b5497e4ee8"
    );
    assert!(
        agents
            .iter()
            .flat_map(|agent| agent.sessions.iter())
            .all(|session| session.agent_session_id != "019ed018-c208-7183-bb6e-b08ba2ef4981"),
        "Codex conversation from work/20260616-1102 must not appear on work/issue-206"
    );
}

/// SPEC-2359 Phase W-16 (FR-402 follow-up, user verification 2026-06-10): on
/// this machine none of the ledger TOMLs carry `session_history` (the field
/// is newer than the sessions), but almost all carry `agent_session_id` (the
/// latest conversation). The view must synthesize that latest conversation as
/// a single Session row instead of rendering "No session yet" everywhere.
#[test]
fn agent_view_synthesizes_latest_conversation_when_history_is_empty() {
    let temp = tempdir().expect("tempdir");
    let worktree = temp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("create worktree");
    let mut session =
        gwt_agent::Session::new(&worktree, "work/foo", gwt_agent::AgentId::ClaudeCode);
    session.id = "gwt-session-1".to_string();
    session.agent_session_id = Some("conv-latest".to_string());
    session.session_history = Vec::new();
    let mut index = std::collections::HashMap::new();
    index.insert(session.id.as_str(), &session);

    let agent_ref = gwt_core::workspace_projection::WorkAgentRef {
        session_id: "gwt-session-1".to_string(),
        agent_id: Some("claude".to_string()),
        display_name: Some("Claude Code".to_string()),
        updated_at: chrono::Utc::now(),
        attached_by: None,
    };

    let view = super::workspace_work_agent_view_from_ref(&agent_ref, &index, temp.path());

    assert_eq!(
        view.sessions.len(),
        1,
        "latest conversation must be synthesized when history is empty"
    );
    assert_eq!(view.sessions[0].agent_session_id, "conv-latest");
    assert!(view.sessions[0].is_active);
    assert!(view.sessions[0].resumable);
}

#[test]
fn agent_view_marks_session_history_only_when_worktree_and_branch_are_missing() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let mut session = gwt_agent::Session::new(
        temp.path().join("deleted-worktree"),
        "feature/deleted-session-row",
        gwt_agent::AgentId::ClaudeCode,
    );
    session.id = "gwt-session-missing-branch".to_string();
    session.agent_session_id = Some("conv-latest".to_string());
    let mut index = std::collections::HashMap::new();
    index.insert(session.id.as_str(), &session);

    let agent_ref = gwt_core::workspace_projection::WorkAgentRef {
        session_id: session.id.clone(),
        agent_id: Some("claude".to_string()),
        display_name: Some("Claude Code".to_string()),
        updated_at: chrono::Utc::now(),
        attached_by: None,
    };

    let view = super::workspace_work_agent_view_from_ref(&agent_ref, &index, &repo);

    assert_eq!(view.sessions.len(), 1);
    assert!(
        !view.sessions[0].resumable,
        "missing worktree plus missing local/origin branch is history-only"
    );
}

#[test]
fn agent_view_keeps_session_resumable_when_missing_worktree_branch_exists() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let branch = "feature/session-row-resume";
    run_git(&repo, &["branch", branch]);
    let mut session = gwt_agent::Session::new(
        temp.path().join("deleted-worktree"),
        branch,
        gwt_agent::AgentId::ClaudeCode,
    );
    session.id = "gwt-session-existing-branch".to_string();
    session.agent_session_id = Some("conv-latest".to_string());
    let mut index = std::collections::HashMap::new();
    index.insert(session.id.as_str(), &session);

    let agent_ref = gwt_core::workspace_projection::WorkAgentRef {
        session_id: session.id.clone(),
        agent_id: Some("claude".to_string()),
        display_name: Some("Claude Code".to_string()),
        updated_at: chrono::Utc::now(),
        attached_by: None,
    };

    let view = super::workspace_work_agent_view_from_ref(&agent_ref, &index, &repo);

    assert_eq!(view.sessions.len(), 1);
    assert!(
        view.sessions[0].resumable,
        "available branch lets exact Session Resume re-materialize the worktree"
    );
}

/// SPEC-2359 Phase W-16 (FR-403): the Workspace list is ordered by last
/// update, newest first — rows with fresher records or fresher ledger
/// sessions float to the top, stale backfill rows sink.
#[test]
fn active_works_are_sorted_by_latest_update_descending() {
    let row = |id: &str, branch: &str, updated_at: &str| gwt::ActiveWorkItemView {
        id: id.to_string(),
        title: branch.to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some(branch.to_string()),
        worktree_path: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents: Vec::new(),
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: updated_at.to_string(),
    };
    let mut works = vec![
        row("work-old", "work/old", "2026-05-01T00:00:00Z"),
        row("work-new", "work/new", "2026-06-10T09:00:00Z"),
        row("work-mid", "work/mid", "2026-06-01T00:00:00Z"),
    ];
    // The mid row carries a fresher ledger session than its record stamp, so
    // it must outrank the 06-10 09:00 row.
    works[2].agents.push(gwt::ActiveWorkAgentView {
        session_id: "sess-fresh".to_string(),
        window_id: None,
        agent_id: "claude".to_string(),
        display_name: "Claude".to_string(),
        affiliation_status: "assigned".to_string(),
        workspace_id: None,
        status_category: "idle".to_string(),
        current_focus: None,
        title_summary: None,
        branch: None,
        worktree_path: None,
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        updated_at: "2026-06-10T12:00:00Z".to_string(),
        sessions: Vec::new(),
    });

    super::attach_registry_sessions_to_active_works(
        &mut works,
        &[],
        None,
        &std::collections::HashMap::new(),
        Path::new("/"),
    );

    let order: Vec<&str> = works.iter().map(|work| work.id.as_str()).collect();
    assert_eq!(
        order,
        vec!["work-mid", "work-new", "work-old"],
        "rows sort by max(record updated_at, agents updated_at) descending"
    );
}

/// SPEC-2359 W-15 (FR-386): rows flagged merged ("safe to delete") via the
/// background scan cache (canonical branch match) or the recorded PR state.
#[test]
fn mark_merged_active_works_flags_cache_and_pr_state() {
    let row = |branch: Option<&str>, pr_state: Option<&str>| gwt::ActiveWorkItemView {
        id: "w".to_string(),
        title: "t".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: branch.map(str::to_string),
        worktree_path: None,
        pr_number: None,
        pr_url: None,
        pr_state: pr_state.map(str::to_string),
        board_refs: Vec::new(),
        agents: Vec::new(),
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: String::new(),
    };
    let mut works = vec![
        row(Some("origin/work/merged"), None),
        row(Some("work/open"), None),
        row(None, Some("MERGED")),
    ];
    let merged: HashMap<String, chrono::DateTime<chrono::Utc>> =
        [("work/merged".to_string(), chrono::Utc::now())]
            .into_iter()
            .collect();

    super::mark_merged_active_works(&mut works, Some(&merged));

    assert!(
        works[0].merged_into_base,
        "cache match (origin/ normalized)"
    );
    assert!(
        !works[1].merged_into_base,
        "unmerged branch stays unflagged"
    );
    assert!(works[2].merged_into_base, "PR state merged flags the row");
}

#[test]
fn dirty_worktree_pr_state_merged_does_not_flag_or_cleanup() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    run_git(&repo, &["init", "-q", "-b", "develop"]);
    run_git(&repo, &["config", "user.name", "Codex"]);
    run_git(&repo, &["config", "user.email", "codex@example.com"]);
    fs::write(repo.join("README.md"), "repo\n").expect("write readme");
    run_git(&repo, &["add", "README.md"]);
    run_git(&repo, &["commit", "-qm", "init"]);
    run_git(&repo, &["checkout", "-q", "-b", "work/dirty"]);
    fs::write(repo.join("local-change.txt"), "current edits\n").expect("write dirty file");

    let mut works = vec![gwt::ActiveWorkItemView {
        id: "w-dirty".to_string(),
        title: "Dirty work".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some("work/dirty".to_string()),
        worktree_path: Some(repo.display().to_string()),
        pr_number: Some(123),
        pr_url: None,
        pr_state: Some("MERGED".to_string()),
        board_refs: Vec::new(),
        agents: Vec::new(),
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: "2026-06-10T12:00:00Z".to_string(),
    }];

    super::mark_merged_active_works(&mut works, None);
    super::mark_workspace_cleanup_candidates(&mut works, None, &[], &HashSet::new());

    assert!(
        !works[0].merged_into_base,
        "dirty current worktree must not inherit an old merged PR badge"
    );
    assert_eq!(
        works[0].cleanup_candidate, None,
        "dirty current worktree must not become cleanup-ready from old PR state"
    );
}

/// SPEC-2359 W-15 (FR-386): apply_work_merge_status stores the scan result
/// and the next projection build flags the matching rows.
#[test]
fn apply_work_merge_status_caches_and_flags_rows() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let now = chrono::Utc::now();
    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            "work-merged-row",
            now,
        );
        event.title = Some("merged work".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/merged".to_string()),
                worktree_path: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        event
    })
    .expect("record work");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let merged: HashMap<String, chrono::DateTime<chrono::Utc>> =
        [("work/merged".to_string(), chrono::Utc::now())]
            .into_iter()
            .collect();
    let _ = runtime.apply_work_merge_status(&repo, merged, HashMap::new());

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let row = view
        .active_works
        .iter()
        .find(|work| work.id == "work-merged-row")
        .expect("row");
    assert!(row.merged_into_base, "cached merge scan flags the row");
}

#[test]
fn spawn_work_merge_status_scan_skips_dirty_worktree_branch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    run_git(&repo, &["init", "-q", "-b", "develop"]);
    run_git(&repo, &["config", "user.name", "Codex"]);
    run_git(&repo, &["config", "user.email", "codex@example.com"]);
    fs::write(repo.join("README.md"), "seed\n").expect("seed");
    run_git(&repo, &["add", "README.md"]);
    run_git(&repo, &["commit", "-qm", "seed"]);
    run_git(&repo, &["branch", "work/dirty"]);
    run_git(
        &repo,
        &["update-ref", "refs/remotes/origin/develop", "develop"],
    );
    fs::write(repo.join("local-change.txt"), "uncommitted\n").expect("dirty file");

    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            "work-dirty-row",
            chrono::Utc::now(),
        );
        event.title = Some("dirty work".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/dirty".to_string()),
                worktree_path: Some(repo.clone()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        event
    })
    .expect("record work");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.spawn_work_merge_status_scan(repo.clone());

    wait_for_recorded_event("dirty work merge status", &events, |events| {
        events.iter().any(|event| {
            matches!(
                event,
                UserEvent::WorkMergeStatus {
                    project_root,
                    ..
                } if project_root == &repo
            )
        })
    });

    let snapshot = events.lock().expect("event log").clone();
    let (_, merged_branches, cleanup_ready_branches) = snapshot
        .iter()
        .find_map(|event| match event {
            UserEvent::WorkMergeStatus {
                project_root,
                merged_branches,
                cleanup_ready_branches,
            } if project_root == &repo => {
                Some((project_root, merged_branches, cleanup_ready_branches))
            }
            _ => None,
        })
        .expect("work merge status event");

    assert!(
        merged_branches.is_empty(),
        "dirty worktree branch must not render as merged: {merged_branches:?}"
    );
    assert!(
        cleanup_ready_branches.is_empty(),
        "dirty worktree branch must not become cleanup-ready: {cleanup_ready_branches:?}"
    );
}

#[test]
fn spawn_work_merge_status_scan_clears_stale_cache_when_no_targets_remain() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Done,
            "work-terminal-row",
            chrono::Utc::now(),
        );
        event.title = Some("terminal work".to_string());
        event.status_category = Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Done);
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/merged".to_string()),
                worktree_path: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        event
    })
    .expect("record terminal work");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.spawn_work_merge_status_scan(repo.clone());

    wait_for_recorded_event("empty work merge status", &events, |events| {
        events.iter().any(|event| {
            matches!(
                event,
                UserEvent::WorkMergeStatus {
                    project_root,
                    merged_branches,
                    cleanup_ready_branches,
                } if project_root == &repo
                    && merged_branches.is_empty()
                    && cleanup_ready_branches.is_empty()
            )
        })
    });
}

#[test]
fn apply_work_merge_status_caches_no_changes_cleanup_readiness() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let worktree = repo.join("work/no-changes");
    fs::create_dir_all(&worktree).expect("create worktree");
    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            "work-no-changes-row",
            chrono::Utc::now(),
        );
        event.title = Some("no changes work".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/no-changes".to_string()),
                worktree_path: Some(worktree.clone()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        event
    })
    .expect("record work");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let cleanup_ready: HashMap<String, String> =
        [("work/no-changes".to_string(), "no_changes".to_string())]
            .into_iter()
            .collect();
    let _ = runtime.apply_work_merge_status(&repo, HashMap::new(), cleanup_ready);

    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let row = view
        .active_works
        .iter()
        .find(|work| work.id == "work-no-changes-row")
        .expect("row");
    let candidate = row
        .cleanup_candidate
        .as_ref()
        .expect("no-changes readiness produces cleanup candidate");

    assert_eq!(candidate.reason, "no_changes");
    assert!(!row.merged_into_base, "no-changes is not a merged badge");

    let _ = runtime.apply_work_merge_status(&repo, HashMap::new(), HashMap::new());
    let view = runtime
        .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
        .expect("projection view after cache clear");
    let row = view
        .active_works
        .iter()
        .find(|work| work.id == "work-no-changes-row")
        .expect("row after cache clear");
    assert_eq!(
        row.cleanup_candidate, None,
        "empty readiness result clears stale no-changes cleanup candidate"
    );
}

// SPEC-2359 W-17 (FR-396): when a client's queue dropped streamed output,
// the repair path re-sends a fresh full snapshot — client-scoped, and only
// for panes that still have a live runtime.
#[test]
fn client_pane_snapshot_repair_replies_with_snapshots_for_known_panes_only() {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[WindowPreset::Shell],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let shell_id = combined_window_id("tab-1", "shell-1");
    insert_test_pane_runtime(&mut runtime, &shell_id);
    runtime
        .runtimes
        .get(&shell_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane lock")
        .process_bytes(b"hello-repair");

    let events = runtime.client_pane_snapshot_repair_events(
        "client-9",
        &[shell_id.clone(), "tab-1::missing-pane".to_string()],
    );

    assert_eq!(events.len(), 1, "unknown panes produce no repair events");
    assert!(
        matches!(&events[0].target, DispatchTarget::Client(id) if id == "client-9"),
        "repair snapshot is scoped to the requesting client"
    );
    match &events[0].event {
        BackendEvent::TerminalSnapshot { id, data_base64 } => {
            assert_eq!(id, &shell_id);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data_base64)
                .expect("snapshot base64");
            assert!(
                String::from_utf8_lossy(&bytes).contains("hello-repair"),
                "snapshot carries the pane's current screen content"
            );
        }
        other => panic!("expected TerminalSnapshot, got {other:?}"),
    }
}

// SPEC-2359 W-17 (FR-398, Issue #3034): a second spawn for the same Work
// while the first launch is still materializing (window registered, agent
// session not yet live) must focus the pending window, not spawn a duplicate.
#[test]
fn app_runtime_spawn_agent_window_dedupes_inflight_launch_for_same_work() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let build_config = || {
        gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .branch("work/20260610-inflight")
            .build()
    };

    runtime
        .spawn_agent_window("tab-1", build_config(), canvas_bounds(), None)
        .expect("first spawn");
    runtime
        .spawn_agent_window("tab-1", build_config(), canvas_bounds(), None)
        .expect("second spawn");

    let tab = runtime.tab("tab-1").expect("tab");
    let agent_windows = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
        agent_windows, 1,
        "in-flight re-click must not spawn a duplicate agent window"
    );
}

// SPEC-2359 W-17 (FR-398): a successful Resume replies a client-scoped
// `workspace_resume_agent_started` ack so pending UI settles deterministically.
#[test]
fn resume_workspace_agent_replies_started_ack_to_requesting_client() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let session = gwt_agent::Session::new(&repo, "feature/resume-ack", gwt_agent::AgentId::Codex);
    session.save(&sessions_dir).expect("save session");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.resume_workspace_agent_events(
        "client-7",
        session.id.clone(),
        None,
        canvas_bounds(),
    );

    let ack = events
        .iter()
        .find(|event| {
            matches!(
                &event.event,
                BackendEvent::WorkspaceResumeAgentStarted { session_id, .. }
                    if session_id == &session.id
            )
        })
        .expect("started ack present");
    assert!(
        matches!(&ack.target, DispatchTarget::Client(id) if id == "client-7"),
        "ack is scoped to the requesting client"
    );
    match &ack.event {
        BackendEvent::WorkspaceResumeAgentStarted { branch, .. } => {
            assert_eq!(branch.as_deref(), Some("feature/resume-ack"));
        }
        other => panic!("expected started ack, got {other:?}"),
    }
}

/// Close-latency root fix (2026-06-12): stopping an agent window records the
/// Paused Work marker (FR-350) on a background thread — the works.json
/// load+save must not run on the UI event loop (it reaches megabytes and the
/// synchronous write made × clicks stall for seconds). The record itself must
/// still land; the test polls for it.
#[test]
fn stop_window_runtime_records_paused_work_off_the_event_loop() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Agent],
    );
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-paused-offloop".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/paused-offloop".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let started = Instant::now();
    runtime.stop_window_runtime(&window_id);
    let stop_call = started.elapsed();
    // The stop call itself returns promptly (no synchronous multi-MB IO).
    assert!(
        stop_call < Duration::from_secs(2),
        "stop_window_runtime blocked for {stop_call:?}"
    );

    // The Paused Work record still lands (background write).
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut recorded = false;
    while Instant::now() < deadline {
        let works = gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(&repo)
            .unwrap_or_else(|_| gwt_core::workspace_projection::WorkItemsProjection {
                updated_at: chrono::Utc::now(),
                work_items: Vec::new(),
            });
        recorded = works.work_items.iter().any(|item| {
            item.id == "work-session-session-paused-offloop"
                && item.status_category
                    == gwt_core::workspace_projection::WorkspaceStatusCategory::Idle
        });
        if recorded {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(recorded, "Paused Work record must land in works.json");
}

/// SPEC-2359 W-16 (FR-387): the tab-change ingest trigger is throttled to
/// once per 30s per project; bootstrap / project-open callers bypass it.
#[test]
fn work_events_ingest_attempt_is_throttled_per_project() {
    let temp = tempdir().expect("tempdir");
    let runtime = sample_runtime(temp.path(), Vec::new(), None);
    let root = temp.path().join("repo");

    assert!(runtime.note_work_events_ingest_attempt(&root, false));
    assert!(
        !runtime.note_work_events_ingest_attempt(&root, false),
        "second attempt within 30s is throttled"
    );
    assert!(
        runtime.note_work_events_ingest_attempt(&root, true),
        "force bypasses the throttle"
    );
    let other = temp.path().join("other");
    assert!(
        runtime.note_work_events_ingest_attempt(&other, false),
        "throttle is per project root"
    );
}

/// SPEC-2359 W-16 (FR-387): the ingest completion handler runs the worktree
/// reconcile AFTER the intake (intake → reconcile order) and rebroadcasts
/// the projection only when the intake applied events.
#[test]
fn handle_work_events_ingested_broadcasts_only_on_change() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Shell],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    // Seed one Work record so the projection broadcast has content.
    let mut seed = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        "work-session-ingest-seed",
        chrono::Utc::now(),
    );
    seed.status_category = Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
    seed.title = Some("seed".to_string());
    gwt_core::workspace_projection::record_workspace_work_event(&repo, seed)
        .expect("seed work record");

    let unchanged = runtime.handle_work_events_ingested(repo.clone(), false);
    assert!(
        unchanged.is_empty(),
        "no-op ingest must not rebroadcast the projection"
    );

    let changed = runtime.handle_work_events_ingested(repo, true);
    assert!(
        changed
            .iter()
            .any(|outbound| matches!(&outbound.event, BackendEvent::ActiveWorkProjection { .. })),
        "changed ingest rebroadcasts the projection"
    );
}

/// SPEC-2359 W16-2 (FR-389 / SC-259): two Works on the same canonical branch
/// (any spelling) merge into ONE Workspace row — newest representative,
/// agents concatenated, counts summed — while branchless legacy rows keep
/// their own identity.
#[test]
fn assign_and_merge_workspace_groups_unifies_same_branch_rows() {
    fn row(
        id: &str,
        branch: Option<&str>,
        updated_at: &str,
        agents: usize,
        lifecycle: &str,
    ) -> gwt::ActiveWorkItemView {
        gwt::ActiveWorkItemView {
            id: id.to_string(),
            title: id.to_string(),
            status_category: "idle".to_string(),
            status_text: "Paused".to_string(),
            summary: None,
            progress_summary: None,
            work_summary: None,
            owner: None,
            next_action: None,
            active_agents: agents,
            blocked_agents: 0,
            branch: branch.map(str::to_string),
            worktree_path: None,
            pr_number: None,
            pr_url: None,
            pr_state: None,
            board_refs: Vec::new(),
            agents: Vec::new(),
            works: Vec::new(),
            lifecycle_state: lifecycle.to_string(),
            closed_at: None,
            session_agent_total: 1,
            merged_into_base: false,
            workspace_key: None,
            remote_only: false,
            done_equivalent: false,
            cleanup_candidate: None,
            cleanup_blocked_reason: None,
            updated_at: updated_at.to_string(),
        }
    }

    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("repo");
    let mut works = vec![
        row(
            "work-session-aaaa",
            Some("work/x"),
            "2026-06-11T10:00:00Z",
            0,
            "paused",
        ),
        row(
            "work-session-bbbb",
            Some("origin/work/x"),
            "2026-06-12T10:00:00Z",
            2,
            "active",
        ),
        row(
            "workspace-1748822400000",
            None,
            "2026-06-10T10:00:00Z",
            0,
            "paused",
        ),
    ];

    super::assign_and_merge_workspace_groups(&mut works, &root);

    assert_eq!(
        works.len(),
        2,
        "same-branch rows merge; legacy row survives"
    );
    let group = works
        .iter()
        .find(|work| {
            work.branch.as_deref() == Some("origin/work/x")
                || work.branch.as_deref() == Some("work/x")
        })
        .expect("grouped row");
    assert_eq!(
        group.id, "work-session-bbbb",
        "newest row is the representative"
    );
    assert_eq!(group.active_agents, 2, "agent counts sum");
    assert_eq!(group.session_agent_total, 2, "session totals sum");
    assert!(group.workspace_key.is_some());
    assert_eq!(
        group.works.len(),
        2,
        "Workspace grouping must preserve both child Works"
    );
    assert_eq!(
        group
            .works
            .iter()
            .map(|work| work.id.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["work-session-aaaa", "work-session-bbbb"]),
        "each child Work keeps its stable identity"
    );
    let paused = group
        .works
        .iter()
        .find(|work| work.id == "work-session-aaaa")
        .expect("paused child Work");
    assert_eq!(paused.lifecycle_state, "paused");
    assert!(paused.manual_close_allowed);
    assert_eq!(paused.close_blocked_reason, None);
    let active = group
        .works
        .iter()
        .find(|work| work.id == "work-session-bbbb")
        .expect("active child Work");
    assert!(!active.manual_close_allowed);
    assert_eq!(active.close_blocked_reason.as_deref(), Some("live_agent"));
    let legacy = works
        .iter()
        .find(|work| work.id == "workspace-1748822400000")
        .expect("legacy row");
    assert_eq!(
        legacy.workspace_key.as_deref(),
        Some("workspace-1748822400000")
    );
}

#[test]
fn legacy_workspace_lifecycle_does_not_create_an_implicit_close_target() {
    let legacy = serde_json::json!({
        "id": "work-legacy-parent",
        "title": "Legacy Workspace",
        "status_category": "idle",
        "status_text": "Paused",
        "summary": null,
        "owner": null,
        "next_action": null,
        "active_agents": 0,
        "blocked_agents": 0,
        "branch": "work/legacy",
        "worktree_path": null,
        "pr_number": null,
        "pr_url": null,
        "pr_state": null,
        "board_refs": [],
        "agents": [],
        "lifecycle_state": "paused"
    });

    let workspace: gwt::ActiveWorkItemView =
        serde_json::from_value(legacy).expect("deserialize legacy Workspace row");

    assert!(
        workspace.works.is_empty(),
        "legacy parent lifecycle is display compatibility only and must not invent a child Work"
    );
}

/// SPEC-2359 W16-3 (FR-390): a row whose branch is known only from fetched
/// refs (no recorded worktree, not in the local-worktree set) is flagged
/// `remote_only`; rows with a worktree or a locally checked-out branch and
/// branchless rows are not.
#[test]
fn mark_remote_only_flags_fetched_branches_without_local_worktree() {
    fn row(id: &str, branch: Option<&str>, worktree: Option<&str>) -> gwt::ActiveWorkItemView {
        gwt::ActiveWorkItemView {
            id: id.to_string(),
            title: id.to_string(),
            status_category: "idle".to_string(),
            status_text: "Paused".to_string(),
            summary: None,
            progress_summary: None,
            work_summary: None,
            owner: None,
            next_action: None,
            active_agents: 0,
            blocked_agents: 0,
            branch: branch.map(str::to_string),
            worktree_path: worktree.map(str::to_string),
            pr_number: None,
            pr_url: None,
            pr_state: None,
            board_refs: Vec::new(),
            agents: Vec::new(),
            works: Vec::new(),
            lifecycle_state: "paused".to_string(),
            closed_at: None,
            session_agent_total: 0,
            merged_into_base: false,
            workspace_key: None,
            remote_only: false,
            done_equivalent: false,
            cleanup_candidate: None,
            cleanup_blocked_reason: None,
            updated_at: String::new(),
        }
    }

    let mut local = std::collections::HashSet::new();
    local.insert("work/local".to_string());
    let mut works = vec![
        row("w-remote", Some("origin/work/fetched"), None),
        row("w-local-branch", Some("work/local"), None),
        row("w-with-worktree", Some("work/other"), Some("/tmp/x")),
        row("w-branchless", None, None),
    ];

    super::assign_and_merge_workspace_groups(&mut works, Path::new("/repo"));
    super::mark_remote_only_active_works(&mut works, Some(&local));

    assert!(works[0].remote_only, "fetched-only branch is Remote");
    assert!(!works[1].remote_only, "locally checked-out branch is not");
    assert!(!works[2].remote_only, "rows with a worktree are not");
    assert!(!works[3].remote_only, "branchless rows are not");
    assert_eq!(works[0].works.len(), 1);
    assert!(!works[0].works[0].manual_close_allowed);
    assert_eq!(
        works[0].works[0].close_blocked_reason.as_deref(),
        Some("remote_environment_unknown")
    );

    let mut unknown = vec![row("w-unknown", Some("work/unknown"), None)];
    super::assign_and_merge_workspace_groups(&mut unknown, Path::new("/repo"));
    super::mark_remote_only_active_works(&mut unknown, None);

    assert!(
        !unknown[0].remote_only,
        "missing local branch scan data must remain undetermined"
    );
    assert!(unknown[0].works[0].manual_close_allowed);
    assert_eq!(unknown[0].works[0].close_blocked_reason, None);
}

/// SPEC-2359 W16-4 (FR-391): merged ∧ stale rows classify as derived Done;
/// activity after the merge reference clears it; explicit terminal closes
/// and pr_state-only merges never enter the derived classification; and the
/// marking writes nothing (US-61).
#[test]
fn mark_merged_classifies_done_equivalent_for_stale_merged_rows() {
    fn row(
        id: &str,
        branch: Option<&str>,
        pr_state: Option<&str>,
        lifecycle: &str,
        updated_at: &str,
    ) -> gwt::ActiveWorkItemView {
        gwt::ActiveWorkItemView {
            id: id.to_string(),
            title: id.to_string(),
            status_category: "idle".to_string(),
            status_text: "Paused".to_string(),
            summary: None,
            progress_summary: None,
            work_summary: None,
            owner: None,
            next_action: None,
            active_agents: 0,
            blocked_agents: 0,
            branch: branch.map(str::to_string),
            worktree_path: None,
            pr_number: None,
            pr_url: None,
            pr_state: pr_state.map(str::to_string),
            board_refs: Vec::new(),
            agents: Vec::new(),
            works: Vec::new(),
            lifecycle_state: lifecycle.to_string(),
            closed_at: None,
            session_agent_total: 0,
            merged_into_base: false,
            workspace_key: None,
            remote_only: false,
            done_equivalent: false,
            cleanup_candidate: None,
            cleanup_blocked_reason: None,
            updated_at: updated_at.to_string(),
        }
    }

    let merge_at = chrono::Utc::now();
    let stale =
        (merge_at - chrono::Duration::hours(2)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let fresh =
        (merge_at + chrono::Duration::hours(2)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let merged: HashMap<String, chrono::DateTime<chrono::Utc>> =
        [("work/merged".to_string(), merge_at)]
            .into_iter()
            .collect();

    let mut works = vec![
        row("w-stale", Some("work/merged"), None, "paused", &stale),
        row("w-fresh", Some("work/merged"), None, "paused", &fresh),
        row("w-closed", Some("work/merged"), None, "done", &stale),
        row(
            "w-pr-only",
            Some("work/other"),
            Some("MERGED"),
            "paused",
            &stale,
        ),
    ];

    super::mark_merged_active_works(&mut works, Some(&merged));

    assert!(works[0].done_equivalent, "merged ∧ stale → derived Done");
    assert!(
        !works[1].done_equivalent,
        "updated after the merge → back to Active/Paused (FR-391)"
    );
    assert!(
        !works[2].done_equivalent,
        "explicit terminal close keeps its own lifecycle"
    );
    assert!(
        !works[3].done_equivalent,
        "pr_state stays badge-only — membership rides the scan verdict"
    );
    assert!(works[3].merged_into_base, "pr_state still drives the badge");
}

#[test]
fn mark_cleanup_candidates_exposes_no_changes_reason_without_merged_badge() {
    let mut works = vec![gwt::ActiveWorkItemView {
        id: "w-no-changes".to_string(),
        title: "No changes".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some("work/no-changes".to_string()),
        worktree_path: Some("/tmp/gwt-no-changes".to_string()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents: Vec::new(),
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: String::new(),
    }];
    let cleanup_ready: HashMap<String, String> =
        [("work/no-changes".to_string(), "no_changes".to_string())]
            .into_iter()
            .collect();

    super::mark_workspace_cleanup_candidates(
        &mut works,
        Some(&cleanup_ready),
        &[],
        &HashSet::new(),
    );

    let candidate = works[0]
        .cleanup_candidate
        .as_ref()
        .expect("no-changes branch is cleanup-ready");
    assert_eq!(candidate.branch, "work/no-changes");
    assert_eq!(candidate.reason, "no_changes");
    assert_eq!(works[0].cleanup_blocked_reason, None);
    assert!(
        !works[0].merged_into_base,
        "no-changes cleanup does not claim a merged badge"
    );
}

#[test]
fn mark_cleanup_candidates_sets_blocked_reason_for_live_agent_and_process() {
    let temp = tempdir().expect("tempdir");
    let live_process_worktree = temp.path().join("gwt-live-process");
    fs::create_dir_all(&live_process_worktree).expect("create live process worktree");
    let mut works = vec![
        gwt::ActiveWorkItemView {
            id: "w-live-agent".to_string(),
            title: "Live agent".to_string(),
            status_category: "active".to_string(),
            status_text: "Active".to_string(),
            summary: None,
            progress_summary: None,
            work_summary: None,
            owner: None,
            next_action: None,
            active_agents: 1,
            blocked_agents: 0,
            branch: Some("work/live-agent".to_string()),
            worktree_path: Some("/tmp/gwt-live-agent".to_string()),
            pr_number: None,
            pr_url: None,
            pr_state: Some("MERGED".to_string()),
            board_refs: Vec::new(),
            agents: Vec::new(),
            works: Vec::new(),
            lifecycle_state: "active".to_string(),
            closed_at: None,
            session_agent_total: 0,
            merged_into_base: true,
            workspace_key: None,
            remote_only: false,
            done_equivalent: false,
            cleanup_candidate: None,
            cleanup_blocked_reason: None,
            updated_at: String::new(),
        },
        gwt::ActiveWorkItemView {
            id: "w-live-process".to_string(),
            title: "Live process".to_string(),
            status_category: "idle".to_string(),
            status_text: "Paused".to_string(),
            summary: None,
            progress_summary: None,
            work_summary: None,
            owner: None,
            next_action: None,
            active_agents: 0,
            blocked_agents: 0,
            branch: Some("work/live-process".to_string()),
            worktree_path: Some(live_process_worktree.display().to_string()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
            board_refs: Vec::new(),
            agents: Vec::new(),
            works: Vec::new(),
            lifecycle_state: "paused".to_string(),
            closed_at: None,
            session_agent_total: 0,
            merged_into_base: false,
            workspace_key: None,
            remote_only: false,
            done_equivalent: false,
            cleanup_candidate: None,
            cleanup_blocked_reason: None,
            updated_at: String::new(),
        },
    ];
    let cleanup_ready: HashMap<String, String> = [
        ("work/live-agent".to_string(), "pr_merged".to_string()),
        ("work/live-process".to_string(), "no_changes".to_string()),
    ]
    .into_iter()
    .collect();
    let session = ActiveAgentSession {
        window_id: "window-live-agent".to_string(),
        session_id: "session-live-agent".to_string(),
        agent_id: "codex".to_string(),
        branch_name: "work/live-agent".to_string(),
        display_name: "Codex".to_string(),
        worktree_path: PathBuf::from("/tmp/gwt-live-agent"),
        agent_project_root: "/tmp/gwt-live-agent".to_string(),
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        tab_id: "tab-1".to_string(),
    };
    let live_process_paths: HashSet<PathBuf> =
        [fs::canonicalize(&live_process_worktree).expect("canonical live process worktree")]
            .into_iter()
            .collect();

    super::mark_workspace_cleanup_candidates(
        &mut works,
        Some(&cleanup_ready),
        &[&session],
        &live_process_paths,
    );

    assert_eq!(works[0].cleanup_candidate, None);
    assert_eq!(
        works[0].cleanup_blocked_reason.as_deref(),
        Some("live_agent")
    );
    assert_eq!(works[1].cleanup_candidate, None);
    assert_eq!(
        works[1].cleanup_blocked_reason.as_deref(),
        Some("live_process")
    );
}

// SPEC-3075: the rail "what work was running" summary derivation. Surfaces the
// agent-declared title-summary purpose, with a fallback chain that skips
// identifier-shaped titles (skill names / work ids / UUIDs) and the branch.
#[test]
fn derive_work_summary_prefers_live_agent_title_summary() {
    let summary = super::derive_work_summary(
        Some("Work 要約を目的第一に再構成"),
        Some("journal purpose"),
        Some("gwt-manage-pr"),
        Some("work/20260612-1405"),
    );
    assert_eq!(summary.as_deref(), Some("Work 要約を目的第一に再構成"));
}

#[test]
fn derive_work_summary_falls_back_to_journal_then_recorded_title() {
    // No live agent purpose -> recorded journal purpose wins.
    assert_eq!(
        super::derive_work_summary(
            None,
            Some("journal recorded purpose"),
            Some("gwt-build-spec"),
            Some("work/x"),
        )
        .as_deref(),
        Some("journal recorded purpose"),
    );
    // No agent / journal -> a real (non-identifier) recorded title wins.
    assert_eq!(
        super::derive_work_summary(None, None, Some("Release Notes cleanup"), None).as_deref(),
        Some("Release Notes cleanup"),
    );
}

#[test]
fn derive_work_summary_is_none_when_no_declared_purpose() {
    // A skill-name title is not a declared purpose -> None (the caller then
    // fills from the branch tip commit subject). The owner is NOT folded in.
    assert_eq!(
        super::derive_work_summary(None, None, Some("gwt-manage-pr"), None),
        None,
    );
    // Backfill Work: title == branch -> None (UI labels by branch).
    assert_eq!(
        super::derive_work_summary(
            None,
            None,
            Some("work/20260614-0444"),
            Some("work/20260614-0444"),
        ),
        None,
    );
    // A raw work-item id title is not a purpose.
    assert_eq!(
        super::derive_work_summary(
            None,
            None,
            Some("work-work-20260601-0908-9ffe416f"),
            Some("work/20260601-0908"),
        ),
        None,
    );
}

#[test]
fn apply_work_summary_external_sources_prefers_pr_then_ai_then_commit_subject() {
    use std::collections::HashMap;
    let mut tip_subjects: HashMap<String, String> = HashMap::new();
    tip_subjects.insert(
        "work/20260614-0444".to_string(),
        "feat(workspace): purpose-first rail".to_string(),
    );
    tip_subjects.insert(
        "work/20260610-0907".to_string(),
        "work/20260610-0907".to_string(), // subject == branch -> not a purpose
    );
    // SPEC-3075 FR-006: an AI-polished summary wins over the raw commit subject
    // for a gap row (no PR, no title-summary).
    tip_subjects.insert(
        "work/20260609-1130".to_string(),
        "Merge pull request #42 from x".to_string(), // noisy raw subject
    );
    tip_subjects.insert(
        "work/20260617-0417".to_string(),
        "Merge pull request #3102 from akiojin/work/20260616-1443".to_string(),
    );
    tip_subjects.insert(
        "work/20260617-0250".to_string(),
        "chore(release): v9.61.0".to_string(),
    );
    let mut ai_summaries: HashMap<String, String> = HashMap::new();
    ai_summaries.insert(
        "work/20260609-1130".to_string(),
        "tray の Copy URL のちらつきを修正".to_string(),
    );
    let mut pr_titles: HashMap<String, String> = HashMap::new();
    // A PR title overrides even a declared title-summary already in work_summary.
    pr_titles.insert(
        "work/20260612-1405".to_string(),
        "Surface work purpose in the Workspace rail".to_string(),
    );
    pr_titles.insert(
        "work/20260617-0422".to_string(),
        "Merge pull request #3102 from akiojin/work/20260616-1443".to_string(),
    );

    let base = |branch: &str, work_summary: Option<&str>| gwt::ActiveWorkItemView {
        id: branch.to_string(),
        title: branch.to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        work_summary: work_summary.map(str::to_string),
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: Some(branch.to_string()),
        worktree_path: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: vec![],
        agents: vec![],
        works: Vec::new(),
        lifecycle_state: "paused".to_string(),
        closed_at: None,
        session_agent_total: 0,
        updated_at: String::new(),
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
    };
    let mut works = vec![
        base("work/20260614-0444", None), // no PR, gap -> filled by commit subject
        base("work/20260612-1405", Some("Keep my purpose")), // PR title overrides title-summary
        base("work/20260610-0907", None), // no PR, subject == branch -> stays None
        base("work/20260609-1130", None), // no PR, AI summary beats noisy commit subject
        base("work/20260617-0417", None), // no AI, noisy merge subject -> no purpose
        base("work/20260617-0250", None), // no AI, release bump subject -> no purpose
        base("work/20260617-0422", Some("Declared purpose")), // noisy PR title must not override
    ];
    super::apply_work_summary_external_sources(
        &mut works,
        Some(&pr_titles),
        Some(&ai_summaries),
        Some(&tip_subjects),
    );
    assert_eq!(
        works[0].work_summary.as_deref(),
        Some("feat(workspace): purpose-first rail"),
    );
    assert_eq!(
        works[1].work_summary.as_deref(),
        Some("Surface work purpose in the Workspace rail"),
        "PR title overrides the declared title-summary",
    );
    assert_eq!(works[2].work_summary, None);
    assert_eq!(
        works[3].work_summary.as_deref(),
        Some("tray の Copy URL のちらつきを修正"),
        "AI-polished summary wins over the raw commit subject",
    );
    assert_eq!(
        works[4].work_summary, None,
        "raw merge commit subjects are git mechanics, not Work purpose",
    );
    assert_eq!(
        works[5].work_summary, None,
        "raw release bump subjects are git mechanics, not Work purpose",
    );
    assert_eq!(
        works[6].work_summary.as_deref(),
        Some("Declared purpose"),
        "noisy external titles must not override a declared purpose",
    );
}

#[test]
fn is_summary_noise_flags_merge_and_release_commits() {
    assert!(super::is_summary_noise(""));
    assert!(super::is_summary_noise(
        "Merge pull request #3078 from akiojin/work/x"
    ));
    assert!(super::is_summary_noise("Merge branch 'develop'"));
    assert!(super::is_summary_noise(
        "Merge remote-tracking branch 'origin/develop'"
    ));
    assert!(super::is_summary_noise("chore(release): v9.58.0"));
    assert!(super::is_summary_noise("chore: merge origin/develop"));
    // Real work is not noise.
    assert!(!super::is_summary_noise(
        "feat(workspace): purpose-first rail"
    ));
    assert!(!super::is_summary_noise("fix: reveal reused surface tabs"));
}

#[test]
fn is_identifier_like_title_classifies_shapes() {
    assert!(super::is_identifier_like_title("gwt-manage-pr"));
    assert!(super::is_identifier_like_title("gwt-build-spec"));
    assert!(super::is_identifier_like_title(
        "work-work-20260601-0908-9ffe416f"
    ));
    assert!(super::is_identifier_like_title(
        "550e8400-e29b-41d4-a716-446655440000"
    ));
    // Real purposes are not identifiers.
    assert!(!super::is_identifier_like_title("Release Notes cleanup"));
    assert!(!super::is_identifier_like_title(
        "Work 要約を目的第一に再構成"
    ));
    assert!(!super::is_identifier_like_title("SPEC-3075"));
    assert!(!super::is_identifier_like_title("develop"));
}

#[test]
fn issue_monitor_launch_succeeded_ack_is_non_scanning_and_persists() {
    // Issue #3222: the launch-success ACK used to re-enter the full
    // scan+claim flow on a fresh disk snapshot that could not see other
    // in-flight claims, re-claiming them (same-owner renewal) and spawning
    // duplicate windows past max_active. The ACK must only bind the window and
    // persist; scanning for a fresh snapshot is allowed, claiming is not.
    let temp = tempfile::TempDir::new().expect("tempdir");
    // Thread-local override: never mutate process-global HOME in parallel tests.
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    // Seed an in-flight claim (Launching, no window bound yet) on disk.
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let prefs = gwt::IssueMonitorPrefs {
        enabled: true,
        launching_issues: vec![gwt::IssueMonitorLaunchingIssue {
            issue_number: 42,
            claimed_at: None,
        }],
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, _recorded) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.issue_monitor_launch_succeeded_events(42, "tab-1::agent-1");

    // The ACK may scan for a fresh snapshot, but must NOT claim/launch: no
    // settings-required wizard, no "launch requested" toast.
    for event in &events {
        assert!(
            !matches!(event.event, BackendEvent::LaunchWizardState { .. }),
            "ACK must not open the launch wizard (settings-required prompt)"
        );
        if let BackendEvent::IssueMonitorToast { message, .. } = &event.event {
            assert!(
                !message.contains("launch requested"),
                "ACK must not trigger launches: {message}"
            );
        }
    }
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload");
    assert!(
        persisted
            .launched_issues
            .iter()
            .any(|entry| entry.issue_number == 42 && entry.window_id == "tab-1::agent-1"),
        "the ACK binds and persists the window: {:?}",
        persisted.launched_issues
    );
    assert!(
        persisted.launching_issues.is_empty(),
        "the in-flight marker is consumed by the bind"
    );
}

#[test]
fn issue_monitor_windows_closed_requeue_is_non_scanning() {
    // Issue #3222 (same re-entrancy class): closing a monitor window requeues
    // + persists and may rescan for the snapshot, but must not claim/launch.
    let temp = tempfile::TempDir::new().expect("tempdir");
    // Thread-local override: never mutate process-global HOME in parallel tests.
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let prefs = gwt::IssueMonitorPrefs {
        enabled: true,
        launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id: "tab-1::agent-1".to_string(),
        }],
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, _recorded) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.issue_monitor_windows_closed_events(&["tab-1::agent-1".to_string()]);

    // Close may scan for a fresh snapshot, but must NOT claim/launch — a
    // re-claim here could instantly respawn the just-closed window or
    // duplicate other in-flight launches.
    for event in &events {
        assert!(
            !matches!(event.event, BackendEvent::LaunchWizardState { .. }),
            "window close must not open the launch wizard"
        );
        if let BackendEvent::IssueMonitorToast { message, .. } = &event.event {
            assert!(
                !message.contains("launch requested"),
                "window close must not trigger launches: {message}"
            );
        }
    }
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload");
    assert!(
        persisted.launched_issues.is_empty(),
        "closed window is released from the launched set"
    );
}
