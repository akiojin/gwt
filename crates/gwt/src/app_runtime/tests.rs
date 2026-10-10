use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex, RwLock},
    thread,
    time::{Duration, Instant},
};

use fs2::FileExt;
use futures_util::{SinkExt, Stream, StreamExt};
use tempfile::tempdir;
use tokio::runtime::Runtime as TokioRuntime;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest, Error as WebSocketError, Message as WebSocketMessage,
    },
};

use base64::Engine;
use chrono::{TimeZone, Utc};
use gwt::{
    empty_workspace_state, load_restored_workspace_state, load_session_state, load_workspace_state,
    refresh_managed_gwt_assets_for_worktree, save_workspace_state, workspace_state_path,
    ArrangeMode, BackendEvent, BranchCleanupInfo, BranchListEntry, BranchScope, ContentLimits,
    FocusCycleDirection, FrontendEvent, LaunchWizardAction, LaunchWizardContext, LaunchWizardState,
    LinkedIssueKind, LogScopeSelection, ProfileEnvEntryView, ProjectKind, UiTracePayload,
    WindowCanvasState, WindowGeometry, WindowPlacement, WindowPreset, WindowProcessStatus,
};
use gwt_config::{Profile, Settings};
use gwt_core::{
    coordination::{
        coordination_events_path, load_snapshot, post_entry, AuthorKind, BoardAudienceScope,
        BoardEntry, BoardEntryKind, BoardMention, BoardMentionTargetKind, BoardWorktreeForm,
        CoordinationEvent,
    },
    logging::{current_log_file, init as init_logging, LogLevel, LoggingConfig},
    paths::gwt_cache_dir,
    recovery::{
        RecoveryAcknowledgement, RecoveryConflictKind, RecoveryIntent, RecoveryProvider,
        RecoveryProviderReceipt, RecoveryState, RecoveryStore,
    },
    repo_hash::detect_repo_hash,
    test_support::ScopedGwtHome,
};
use gwt_github::{
    ApiError, Cache, CommentId, CommentSnapshot, FakeIssueClient, FetchResult, IssueClient,
    IssueNumber, IssueSnapshot, IssueState, SpecListFilter, SpecSummary, UpdatedAt,
};
use gwt_terminal::{Pane, PaneStatus, SNAPSHOT_SCROLLBACK_REPLAY_LIMIT};
use tracing::{field::Visit, Event, Level, Subscriber};
use tracing_subscriber::{layer::Context, prelude::*, Layer};

#[cfg(unix)]
use super::continuation::set_durable_launch_recovery_directory_sync_test_hook;
use super::continuation::{
    clear_durable_launch_recovery, compensate_terminalized_genesis_workspace_projection,
    continuation_launch_config, durable_launch_recovery_exists,
    durable_launch_recovery_session_identity, nonlocal_runtime_index_scan_metrics,
    persist_durable_launch_recovery, reset_nonlocal_runtime_index_scan_metrics,
    resolve_split_workspace_state_external_commit,
    set_fresh_execution_pre_work_commit_hook_for_test, set_missing_session_cleanup_hook_for_test,
    ActiveOwnerLiveness, ContinueWorkLaunchSeed, DurableLaunchRecoveryKind,
};
use super::{
    active_work_projection_from_saved, continue_work_readiness_decision,
    dispatch_agent_launch_success, drive_local_issue_monitor_claim_effects_with,
    local_issue_monitor_fallback_commit_count, local_issue_monitor_remote_scan_count,
    prepare_local_issue_monitor_claim_proposals, rebase_mutate_and_persist_issue_monitor_state,
    record_issue_monitor_scan_failures, reset_local_issue_monitor_fallback_commit_count,
    reset_local_issue_monitor_remote_scan_count, save_resumed_workspace_projection,
    save_start_work_workspace_projection, save_workspace_launch_projection,
    set_scheduled_scan_after_lease_before_commit_test_hook, ActiveAgentSession,
    AgentKanbanLaunchTarget, AgentLaunchCompletion, AgentLaunchResult, AgentLaunchRuntimeContext,
    AppEventProxy, AppRuntime, ApprovalPromptLatch, AttachmentProgressPhase, BlockingTaskSpawner,
    BlockingTestTaskQueue, CachedContinueWorkOutcome, ContinueWorkReadinessWatch, DispatchTarget,
    IssueMonitorProfileSaveContext, KnowledgeLoadRequest, KnowledgeRefreshTask,
    KnowledgeSearchRequest, LaunchFeedbackContext, LaunchPaneDisposition, LaunchWizardMemoryCache,
    LaunchWizardSession, LocalIssueMonitorEffectOutcome, OutboundEvent, PendingContinueWork,
    PendingContinueWorkExecution, PendingFreshExecutionLaunch, PreparedProjectSwitch,
    ProcessLaunch, ProjectNavigationPayload, ProjectNavigationPrepared, ProjectOpenControlFailure,
    ProjectOpenReply, ProjectTabRuntime, ReadinessDeadlineDecision, ReadinessPaneEvidence,
    RecentProjectKeysResolved, ScheduledIssueMonitorScanOutcome, UserEvent, WindowAddress,
    WindowRuntime, WorkspaceLaunchProjectionKind, WorkspaceResumeContext,
};
use crate::app_runtime::initial_project_tab_incarnations;
use crate::embedded_server::{
    prepare_outbound_event, AgentPmSendResponder, ClientQueue, DrainStep,
};
use crate::{
    combined_window_id, geometry_to_pty_size, same_worktree_path, AgentFrontendRequest,
    AgentSelfCloseResponder, AgentSessionPrincipal, AttachmentUploadStore, PtyWriterRegistry,
    UploadedAttachment,
};

#[test]
fn pty_start_gate_helper() {
    if std::env::var_os("GWT_INTERNAL_PTY_GATE_ENDPOINT").is_none() {
        return;
    }
    let outcome = gwt_terminal::pty::run_start_gate_from_env();
    // Issue #4628: a released helper execs its target, so it only returns
    // here on an aborted or failed gate — exactly when the owner SIGKILLs the
    // group without waiting (#3705). Exiting through libtest would run the
    // coverage runtime's exit handler, and a kill in the middle of that write
    // leaves a truncated raw profile that fails the whole coverage merge.
    #[cfg(unix)]
    {
        if let Err(error) = &outcome {
            eprintln!("test PTY start gate failed: {error}");
        }
        // SAFETY: `_exit` only ends this helper process; skipping exit
        // handlers is the point.
        unsafe { libc::_exit(if matches!(outcome, Ok(0)) { 0 } else { 1 }) }
    }
    #[cfg(not(unix))]
    {
        let exit_code = outcome.expect("run test PTY start gate");
        assert_eq!(exit_code, 0, "released PTY target failed");
    }
}

fn recovery_center_test_intent(
    session_id: &str,
    recovery_id: &str,
    entry_id: &str,
    worktree_form: BoardWorktreeForm,
    body: &str,
) -> RecoveryIntent {
    let mut entry = BoardEntry::new(
        AuthorKind::Agent,
        "codex",
        BoardEntryKind::Status,
        body,
        Some("Recovery delivery".to_string()),
        Some("Public recovery summary".to_string()),
        Vec::new(),
        Vec::new(),
    );
    entry.id = entry_id.to_string();
    entry.origin_branch = Some("work/issue-1974".to_string());
    entry.origin_session_id = Some(session_id.to_string());
    entry.origin_agent_id = Some("codex".to_string());
    entry.origin_worktree_form = Some(worktree_form);
    entry.origin_recovery_id = Some(recovery_id.to_string());
    RecoveryIntent::new(recovery_id, RecoveryProvider::Local, worktree_form, entry)
        .expect("valid recovery intent")
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
    static CAPTURE_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    let _capture_lock = CAPTURE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CaptureTracingLayer {
        events: Arc::clone(&events),
    });
    super::launch_errors::with_launch_wizard_error_log_capture(|| {
        tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            run();
        });
    });
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

    fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
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

fn env_test_lock() -> &'static gwt_core::test_support::EnvLock {
    crate::env_test_lock()
}

fn write_executable_test_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("test executable parent"))
        .expect("create test executable parent");
    fs::write(path, contents).expect("write test executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path)
            .expect("test executable metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("make test executable runnable");
    }
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

/// Issue #3609: the issue-link cache handed to `AppRuntime`, derived from the
/// caller's temp root the way `sessions_dir` / `log_dir` / `session_state_path`
/// already are.
///
/// Resolving `gwt_cache_dir()` here instead made the shared runtime fixture
/// read the process-global `HOME` at construction time, so the 200+ tests that
/// build a runtime without owning the home inherited whatever tempdir a
/// parallel test had installed — the mechanism behind #3411 / #3414 / #3601.
///
/// The layout mirrors an isolated gwt home (`<home>/.gwt/cache`) on purpose:
/// tests that seed the store through [`write_issue_link_store`] reach it via
/// `gwt_cache_dir()`, because `knowledge_bridge::load_linked_branches` still
/// resolves that path itself rather than taking the runtime's field. Callers
/// that pin their home to the same root therefore see one cache, not two.
fn issue_link_cache_dir_for(temp_root: &Path) -> PathBuf {
    temp_root.join(".gwt").join("cache")
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
  *"--action search-multi"*"--scopes issues"*)
    printf '%s\n' '{"ok":true,"scope_results":{"issues":{"issueResults":[{"number":42,"distance":0.25}]}}}'
    exit 0
    ;;
  *"--action search-multi"*)
    printf '%s\n' '{"ok":true,"scope_results":{"specs":{"specResults":[{"spec_id":1930,"distance":0.4}]}}}'
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
  *"--action index-"*)
    printf '%s\n' '{"ok":true}'
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
set \"gwt_arg1=%~1\"\r\n\
set \"gwt_arg2=%~2\"\r\n\
if /I \"%~3\"==\"--include\" (echo HTTP/1.1 200 OK& echo.)\r\n\
if /I \"%GWT_FAKE_GH_MODE%\"==\"cache_merge_empty\" (\r\n\
  if /I \"%gwt_arg2:~0,26%\"==\"repos/{owner}/{repo}/pulls\" (\r\n\
    echo []\r\n\
    exit /b 0\r\n\
  )\r\n\
  if /I \"%gwt_arg1% %gwt_arg2%\"==\"pr list\" (\r\n\
    echo []\r\n\
    exit /b 0\r\n\
  )\r\n\
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
if [ "$3" = "--include" ]; then
  printf 'HTTP/1.1 200 OK\r\n\r\n'
fi
# SPEC #4093 FR-003: the merged-PR readback is the REST closed-pulls sync.
case "$1 $2" in
  "api repos/{owner}/{repo}/pulls?state=closed"*) merged_pulls_query=1 ;;
  *) merged_pulls_query=0 ;;
esac
if [ "$GWT_FAKE_GH_MODE" = "cache_merge_empty" ] && { [ "$merged_pulls_query" = "1" ] || { [ "$1" = "pr" ] && [ "$2" = "list" ]; }; }; then
  printf '%s\n' '[]'
  exit 0
fi
if [ "$GWT_FAKE_GH_MODE" = "cache_merge_empty" ]; then
  printf '%s\n' 'gh refresh failed' >&2
  exit 1
fi
if [ "$GWT_FAKE_GH_MODE" = "merge_fail" ] && { [ "$merged_pulls_query" = "1" ] || { [ "$1" = "pr" ] && [ "$2" = "list" ]; }; }; then
  printf '%s\n' 'gh merged query failed' >&2
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

fn write_fake_agent_command(temp_root: &Path, command: &str) -> PathBuf {
    #[cfg(windows)]
    {
        let fake_agent = temp_root.join(format!("{command}.cmd"));
        fs::write(&fake_agent, "@echo off\r\nexit /b 0\r\n").expect("write fake agent");
        fake_agent
    }
    #[cfg(not(windows))]
    {
        let fake_agent = temp_root.join(command);
        fs::write(&fake_agent, "#!/bin/sh\nexit 0\n").expect("write fake agent");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_agent, fs::Permissions::from_mode(0o755))
            .expect("chmod fake agent");
        fake_agent
    }
}

fn write_fake_codex(temp_root: &Path) -> PathBuf {
    write_fake_agent_command(temp_root, "codex")
}

/// Issue #3611: a scanned-but-empty branch snapshot. Fixtures that used to
/// pass a non-repository path (where every `git show-ref` failed) get the same
/// verdict — "no branch survives" — without any Git process.
fn scanned_without_branches() -> super::ResumeBranchIndex<'static> {
    static EMPTY: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
    super::ResumeBranchIndex::scanned(Some(EMPTY.get_or_init(HashSet::new)))
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

/// Installed-agent fixtures shared by runtime tests. Each answers version
/// probes locally, so launch health checks never depend on host installations.
fn shared_fixture_agent_bin() -> &'static Path {
    static SHARED: std::sync::OnceLock<(tempfile::TempDir, PathBuf)> = std::sync::OnceLock::new();
    let (_dir, bin) = SHARED.get_or_init(|| {
        let dir = tempdir().expect("shared fixture agent tempdir");
        let commands = gwt_agent::builtin_agent_descriptors()
            .iter()
            .map(|descriptor| descriptor.command)
            .collect::<Vec<_>>();
        let bin = write_fixture_runners(dir.path(), &commands);
        (dir, bin)
    });
    bin
}

/// Fixture runners named `names`, each answering `--version` with a semver.
fn write_fixture_runners(temp_root: &Path, names: &[&str]) -> PathBuf {
    let bin = temp_root.join("fixture-runner-bin");
    fs::create_dir_all(&bin).expect("create fixture runner bin");
    for name in names {
        #[cfg(windows)]
        {
            let runner = bin.join(format!("{name}.cmd"));
            fs::write(&runner, "@echo off\r\necho 1.2.3\r\nexit /b 0\r\n")
                .expect("write fixture runner");
        }
        #[cfg(not(windows))]
        {
            let runner = bin.join(name);
            fs::write(&runner, "#!/bin/sh\nprintf '1.2.3\\n'\nexit 0\n")
                .expect("write fixture runner");
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&runner, fs::Permissions::from_mode(0o755))
                .expect("chmod fixture runner");
        }
    }
    bin
}

/// Prepend fixture executables to the launch environment so version probes
/// answer from the fixture instead of the host.
fn pin_config_fixture_runners(
    config: &mut gwt_agent::LaunchConfig,
    temp_root: &Path,
    names: &[&str],
) {
    let bin = write_fixture_runners(temp_root, names);
    let mut paths = vec![bin];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    let joined = std::env::join_paths(paths).expect("join launch PATH");
    config.env_vars.insert(
        "PATH".to_string(),
        joined.to_str().expect("UTF-8 launch PATH").to_string(),
    );
}

/// Pin a launch profile to fixture executables without mutating process PATH.
fn pin_launch_agents(settings: &mut Settings, runner_bin: &Path) {
    let mut paths = vec![runner_bin.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    let joined = std::env::join_paths(paths).expect("join launch PATH");
    settings
        .profiles
        .set_env_var(
            "default",
            "PATH",
            joined.to_str().expect("UTF-8 launch PATH"),
        )
        .expect("pin launch PATH to fixture agents");
}

/// Write the profile config a monitor fixture reads, with installed-agent
/// fixtures pinned plus its per-test environment.
fn pin_monitor_fixture_agents(
    fixture: &MonitorRelaunchFixture,
    temp_root: &Path,
    extra_env: &[(&str, &str)],
) {
    pin_runtime_agents(&fixture.runtime, temp_root, extra_env);
}

/// Pin runtime launches to installed-agent fixtures and per-test environment.
fn pin_runtime_agents(runtime: &AppRuntime, temp_root: &Path, extra_env: &[(&str, &str)]) {
    let mut settings = Settings::default();
    let commands = gwt_agent::builtin_agent_descriptors()
        .iter()
        .map(|descriptor| descriptor.command)
        .collect::<Vec<_>>();
    let runner_bin = write_fixture_runners(temp_root, &commands);
    pin_launch_agents(&mut settings, &runner_bin);
    for (key, value) in extra_env {
        settings
            .profiles
            .set_env_var("default", key, value)
            .expect("set fixture profile env var");
    }
    write_profile_config(
        runtime
            .profile_config_path
            .as_deref()
            .expect("fixture profile config path"),
        &settings,
    );
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
        worktree_form: gwt::WindowWorktreeForm::Unknown,
        tab_group_id: None,
        tab_group_active: false,
        session_id: None,
        linked_issue_number: None,
        runtime_started_at_ms: None,
        is_pm: false,
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
    let mut session = session.clone();
    if !session.worktree_path.is_absolute() || !session.worktree_path.exists() {
        session.worktree_path = repo.to_path_buf();
    }
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
        &session,
        Some("develop"),
        None,
        None,
        Some(&context),
        WorkspaceLaunchProjectionKind::StartWork,
        Some(&live),
    )
}

fn materialized_project_state_sots(name: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(gwt_core::paths::gwt_projects_dir()) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path().join("project-state").join(name))
        .filter(|path| path.is_file())
        .collect();
    paths.sort();
    paths
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
        continuation_readiness_nonce: None,
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
        continuation_readiness_nonce: None,
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

/// Drive the same background projection continuation that the tao event loop
/// handles in production, then return the committed projection for the active
/// tab. Tests that mutate Work/Workspace state must not assume the initiating
/// handler still performs the potentially large disk decode synchronously.
fn wait_for_active_work_projection(runtime: &mut AppRuntime) -> gwt::ActiveWorkProjectionView {
    let tab_id = runtime
        .active_tab_id
        .clone()
        .expect("active tab for projection completion");
    let recorded_events = match &runtime.proxy {
        AppEventProxy::Stub(events) => events.clone(),
        AppEventProxy::Real(_) | AppEventProxy::Project { .. } => {
            panic!("test runtime must use a stub event proxy")
        }
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let completion = {
            let mut events = recorded_events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            events
                .iter()
                .position(|event| {
                    matches!(
                        recorded_project_payload(event),
                        UserEvent::ActiveWorkProjectionPrepared(_)
                    )
                })
                .map(|index| events.remove(index))
        };
        if let Some(UserEvent::ActiveWorkProjectionPrepared(completion)) =
            completion.and_then(|event| runtime.accept_project_completion(event))
        {
            let commit = runtime.handle_active_work_projection_prepared(*completion);
            if commit.prepared_dispatch.is_some() {
                return runtime
                    .project_state_for_tab(&tab_id)
                    .unwrap()
                    .active_work_projection_cache
                    .borrow()
                    .get(&tab_id)
                    .cloned()
                    .expect("committed active Work projection");
            }
        }
        assert!(
            Instant::now() < deadline,
            "background Active Work projection did not commit before the deadline"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn run_scheduled_scan_to_completion(
    tasks: &BlockingTestTaskQueue,
    events: &Arc<Mutex<Vec<UserEvent>>>,
) -> (
    PathBuf,
    PathBuf,
    String,
    Result<ScheduledIssueMonitorScanOutcome, String>,
) {
    let task = {
        let mut tasks = tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(tasks.len(), 1, "the tick enqueues one scheduled scan");
        tasks.remove(0)
    };
    task();
    let event = {
        let mut events = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let index = events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::IssueMonitorScheduledScanComplete { .. }
                )
            })
            .expect("the completed scan worker emitted its completion");
        events.remove(index)
    };
    assert!(
        matches!(&event, UserEvent::ProjectCompletion { .. }),
        "scan worker completion must carry its project generation"
    );
    let UserEvent::IssueMonitorScheduledScanComplete {
        project_root,
        prefs_path,
        now,
        outcome,
        vanished_window_failures: _,
    } = into_recorded_project_payload(event)
    else {
        unreachable!("matched scheduled completion")
    };
    (project_root, prefs_path, now, outcome)
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

/// Install a runtime whose child has already exited with `exit_code`, so the
/// pane carries the real `PaneExit` receipt Issue #3341 persists.
///
/// The PTY output is drained the way the production reader thread does. That
/// is required, not cosmetic: a macOS session leader stays in "trying to exit"
/// until its tty output is consumed, so an undrained pane never becomes
/// reapable and the receipt never appears.
fn insert_exited_test_pane_runtime(runtime: &mut AppRuntime, window_id: &str, exit_code: u8) {
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                format!("exit /b {exit_code}"),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-c".to_string(), format!("exit {exit_code}")],
        )
    };
    let mut pane = Pane::new(
        window_id.to_string(),
        command,
        args,
        80,
        24,
        HashMap::new(),
        test_pane_cwd(),
    )
    .expect("exited test pane");
    // Windows ConPTY waits for the terminal to answer its startup cursor
    // position query before the attached command can finish. Production does
    // this from the frontend; this fixture has no frontend, so complete the
    // handshake explicitly before waiting for the natural exit.
    #[cfg(windows)]
    let _ = pane.pty().write_input(b"\x1b[1;1R");
    if let Ok(mut reader) = pane.reader() {
        std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            while std::io::Read::read(&mut reader, &mut buffer).is_ok_and(|read| read > 0) {}
        });
    }
    for _ in 0..100 {
        if pane
            .check_status()
            .is_ok_and(|status| *status != PaneStatus::Running)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        pane.last_exit().is_some(),
        "test fixture must observe the child exit before installing the runtime"
    );
    runtime.runtimes.insert(
        window_id.to_string(),
        WindowRuntime::new(
            super::next_window_runtime_incarnation(),
            Arc::new(Mutex::new(pane)),
        ),
    );
}

/// Persist a durable Session under the id `sample_active_agent_session` uses,
/// so writes through `gwt_agent::update_session` have a file to update.
fn save_sample_agent_session_toml(runtime: &AppRuntime, worktree: &Path) {
    let mut session = gwt_agent::Session::new(worktree, "feature/test", gwt_agent::AgentId::Codex);
    session.id = "session-1".to_string();
    session
        .save(&runtime.sessions_dir)
        .expect("save durable test Session");
}

/// Issue #3705 made pane teardown asynchronous: `PtyHandle::kill` no longer
/// waits for the reap, so `process_has_exited()` can still be false when the
/// stop action runs. In that case the terminal proof is finished on a
/// background thread instead of under the caller's lease, which makes any
/// test that reads the proof synchronously racy under CI parallelism.
///
/// These tests assert the synchronous proof, so settle the child first and
/// keep them deterministic on the already-exited path they were written for.
/// Coverage for the still-running path belongs to Issue #3744, which restores
/// a bounded guarantee instead of dropping the proof.
///
/// Twenty seconds preserves the established fixture budget while allowing
/// shared CI runners to reap a PTY process tree under saturation. Both waits
/// poll positive settlement signals, so this is an upper bound rather than a
/// fixed delay (Issue #3751).
const TEST_PTY_STOP_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(20);
const TEST_PTY_STOP_SETTLEMENT_POLL_INTERVAL: Duration = Duration::from_millis(10);

fn settle_test_pane_child(runtime: &AppRuntime, window_id: &str) {
    let pane = runtime
        .runtimes
        .get(window_id)
        .expect("window runtime for settle")
        .pane
        .clone();
    pane.lock()
        .expect("test pane")
        .kill()
        .expect("kill test child");
    let deadline = Instant::now() + TEST_PTY_STOP_SETTLEMENT_TIMEOUT;
    loop {
        if pane
            .lock()
            .expect("test pane")
            .process_has_exited()
            .expect("test child exit probe")
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "test child for {window_id} did not exit before the deadline"
        );
        std::thread::sleep(TEST_PTY_STOP_SETTLEMENT_POLL_INTERVAL);
    }
}

fn wait_for_test_pty_stop_settlement(
    sessions_dir: &Path,
    session_id: &str,
    identity: &gwt_agent::SessionExecutionIdentity,
    incarnation: u64,
    child_pid: u32,
    child_started_at: u64,
) {
    let session_path = sessions_dir.join(format!("{session_id}.toml"));
    let runtime_path = gwt_agent::runtime_state_path(sessions_dir, session_id);
    let deadline = Instant::now() + TEST_PTY_STOP_SETTLEMENT_TIMEOUT;
    loop {
        let process_tree_exited =
            !gwt::process::exact_pty_process_tree_is_alive(child_pid, child_started_at);
        let session_stopped = gwt_agent::Session::load(&session_path)
            .is_ok_and(|session| session.status == gwt_agent::AgentStatus::Stopped);
        let terminal_proof_persisted = gwt_agent::SessionRuntimeState::load(&runtime_path)
            .is_ok_and(|proof| {
                proof.status == gwt_agent::AgentStatus::Stopped
                    && proof.execution_identity.as_ref() == Some(identity)
                    && proof.runtime_incarnation == Some(incarnation)
                    && proof.child_pid == Some(child_pid)
                    && proof.child_started_at == Some(child_started_at)
            });
        if process_tree_exited && session_stopped && terminal_proof_persisted {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "test PTY stop did not settle before the deadline: session={session_id}, process_tree_exited={process_tree_exited}, session_stopped={session_stopped}, terminal_proof_persisted={terminal_proof_persisted}",
        );
        thread::sleep(TEST_PTY_STOP_SETTLEMENT_POLL_INTERVAL);
    }
}

fn insert_test_pane_runtime(runtime: &mut AppRuntime, window_id: &str) {
    insert_test_pane_runtime_with_pane(runtime, window_id, long_running_test_pane(window_id));
}

fn insert_test_pane_runtime_with_pane(runtime: &mut AppRuntime, window_id: &str, pane: Pane) {
    let incarnation = super::next_window_runtime_incarnation();
    let pane = Arc::new(Mutex::new(pane));
    let child_pid = pane
        .lock()
        .expect("test pane")
        .pty()
        .process_id()
        .expect("test child pid");
    let child_started_at =
        gwt::process::host_process_start_time(child_pid).expect("test child process start time");
    runtime
        .runtimes
        .insert(window_id.to_string(), WindowRuntime::new(incarnation, pane));
    if let Some(session_id) = runtime
        .active_agent_sessions
        .get(window_id)
        .map(|active| active.session_id.clone())
    {
        let path = runtime.sessions_dir.join(format!("{session_id}.toml"));
        if let Ok(session) = gwt_agent::Session::load(&path) {
            if let Ok(Some(identity)) = gwt_agent::SessionExecutionIdentity::from_session(&session)
            {
                let _ = gwt_agent::persist_session_running_state_if_execution_identity_matches(
                    &runtime.sessions_dir,
                    &identity,
                    incarnation,
                    gwt::process::host_process_start_time(std::process::id())
                        .expect("test Host process start time"),
                    child_pid,
                    child_started_at,
                );
            }
        }
    }
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
    // Keep launch health checks independent of host provider installations.
    // Tests that pre-write their own profile config keep it.
    let profile_config_path = temp_root.join("profile-config.toml");
    if !profile_config_path.exists() {
        let mut settings = Settings::default();
        pin_launch_agents(&mut settings, shared_fixture_agent_bin());
        write_profile_config(&profile_config_path, &settings);
    }
    let launch_wizard_cache =
        LaunchWizardMemoryCache::load_with_agent_options(&sessions_dir, sample_agent_options());
    let pty_writers: PtyWriterRegistry = Arc::new(RwLock::new(HashMap::new()));
    let blocking_tasks = BlockingTaskSpawner::thread();
    let persist_dispatcher = Arc::new(super::persist_dispatcher::PersistDispatcher::new(
        &blocking_tasks,
    ));
    let (project_tab_incarnations, next_project_incarnation) =
        initial_project_tab_incarnations(&tabs);
    let mut runtime = AppRuntime {
        tabs,
        active_tab_id: active_tab_id.map(str::to_owned),
        project_states: super::initial_project_states(&project_tab_incarnations),
        project_aggregates: HashMap::new(),
        next_project_aggregate_revision: 0,
        project_tab_incarnations,
        next_project_incarnation,
        project_navigation_request: 0,
        pending_project_navigation: None,
        project_picker: Default::default(),
        project_route: Default::default(),
        recent_projects: Vec::new(),
        profile_selections: HashMap::new(),
        profile_config_path: Some(profile_config_path),
        runtimes: HashMap::new(),
        window_details: HashMap::new(),
        launch_error_terminal_details: HashMap::new(),
        window_lookup: HashMap::new(),
        window_lifecycle_generations: Arc::new(Mutex::new(HashMap::new())),
        board_all_view_windows: std::collections::HashSet::new(),

        session_state_path: temp_root.join("session-state.json"),
        log_dir,
        project_log_router: None,
        project_log_scopes: HashMap::new(),
        proxy,
        blocking_tasks,
        sessions_dir,
        launch_wizard_cache,

        pending_workspace_resume_contexts: HashMap::new(),
        inflight_launches: HashMap::new(),
        project_open_started: None,
        pending_startup_pm_tabs: Vec::new(),
        deferred_issue_monitor_launches: None,
        startup_worktree_inventories: HashMap::new(),
        pending_launch_feedback_contexts: HashMap::new(),
        pending_launch_delivery_acks: HashMap::new(),
        pending_launch_completions: HashMap::new(),
        issue_monitor_launch_deliveries: HashMap::new(),
        issue_monitor_launch_preparations: HashSet::new(),
        issue_monitor_materializer_id: "app-runtime-test-materializer".to_string(),
        // Issue #3878: own the fallback commit budget instead of inheriting
        // the GUI-thread one; tests that assert that budget set it explicitly.
        issue_monitor_fallback_commit_timeout: super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT,
        runtime_hook_agent_failures_in_flight: HashMap::new(),
        // Issue #3676 AC-2: tests default to fail-open so ambient developer /
        // CI credential state never decides a launch; auth-preflight tests
        // install a real or explicit probe themselves.
        issue_monitor_provider_auth_probe: |_| gwt::issue_monitor::ProviderAuthState::Unknown,
        // Issue #3633: tests must never leave real daemons behind on the
        // developer's machine, but the ensure pass still has to be observable
        // so the wiring cannot silently disappear again.
        daemon_supervisor: Arc::new(gwt::daemon_supervisor::DaemonSupervisor::disabled()),
        pending_continue_work: HashMap::new(),
        pending_fresh_execution_launches: HashMap::new(),
        pending_fresh_execution_finalizations: HashMap::new(),

        pending_auto_resume_sources: HashMap::new(),
        pending_startup_restore_log: None,
        pending_restore_summaries: Vec::new(),
        restore_launch_windows: HashMap::new(),
        pending_startup_auto_resume_sessions: Vec::new(),
        update_resume_tab_ids: HashSet::new(),
        update_drain_released_projects: Vec::new(),
        pending_update_resume_notice: None,
        active_agent_sessions: HashMap::<String, ActiveAgentSession>::new(),
        issue_monitor_review_dispatch_windows: HashSet::new(),
        terminal_close_candidates: HashMap::new(),
        terminal_convergence_scan_in_flight: false,
        update_drain_scan_in_flight: false,
        terminal_close_grace: Duration::from_secs(60),
        work_known_branch_refs: HashMap::new(),
        work_dirty_branches: HashMap::new(),
        work_live_process_branches: HashMap::new(),
        work_merge_status_cache: Default::default(),
        work_cleanup_ready_branches: HashMap::new(),
        work_tip_subjects: HashMap::new(),
        work_pr_titles: HashMap::new(),
        work_ai_summaries: HashMap::new(),
        session_ledger_cache: Arc::new(Mutex::new(
            crate::session_ledger_cache::SessionLedgerCache::new(),
        )),
        active_work_projection_refresh: std::cell::RefCell::new(
            super::ActiveWorkProjectionRefreshBroker::default(),
        ),
        active_work_session_ledger_cache: Arc::new(Mutex::new(
            crate::session_ledger_cache::SessionLedgerCache::new(),
        )),
        last_work_events_ingest: std::cell::RefCell::new(HashMap::new()),
        last_work_pr_titles_scan: std::cell::RefCell::new(HashMap::new()),
        local_worktree_branches: std::cell::RefCell::new(HashMap::new()),
        window_pty_statuses: HashMap::new(),
        window_output_bytes: HashMap::new(),
        remote_terminal_previews: HashMap::new(),
        window_last_output_at: HashMap::new(),
        window_hook_states: HashMap::new(),
        window_approval_waiting: HashMap::new(),
        approval_settle_epoch: 0,
        recoverable_agent_error_windows: HashSet::new(),
        provider_quota_holds: HashMap::new(),
        provider_quota_candidates: HashMap::new(),
        provider_api_error_holds: HashMap::new(),
        released_provider_quota_notices: HashMap::new(),
        provider_usage_accounts: Vec::new(),
        last_agent_activity: HashMap::new(),
        last_issue_monitor_heartbeat: HashMap::new(),
        agent_capability_issuer: None,
        agent_capability_tokens: HashMap::new(),
        pending_agent_self_closes: HashMap::new(),
        issue_link_cache_dir: issue_link_cache_dir_for(temp_root),
        knowledge_related_snapshot: Default::default(),
        knowledge_monitor_snapshot: Default::default(),
        issue_client_factory: super::default_issue_client_factory(),
        pending_update: None,
        update_download_in_flight: None,
        deferred_update_discovery: None,
        pty_writers,
        attachment_uploads: AttachmentUploadStore::new(temp_root.join("attachment-uploads")),
        persist_dispatcher,
        file_tree_worktree_roots: HashMap::new(),

        server_url: None,
        usage_refresh: None,
        image_paste_sequence: std::sync::atomic::AtomicU64::new(0),
        agent_launch_stage_counter: std::sync::atomic::AtomicU64::new(1),
        agent_backend_probe_generation: 0,
        agent_backend_latest_probe_generations: HashMap::new(),
    };
    runtime.rebuild_window_lookup();
    runtime.seed_window_pty_statuses();
    (runtime, _events)
}

/// Seed the window-keyed maps that an ordinary close used to leave behind.
/// Only containers whose value type is trivially constructible are seeded; the
/// release path itself is generated from one shared list, so these stand in for
/// every entry on it.
fn seed_window_scoped_state(runtime: &mut AppRuntime, window_id: &str) {
    let address = runtime.window_lookup[window_id].clone();
    let project_root = runtime
        .tab(&address.tab_id)
        .expect("seeded window tab")
        .project_root
        .clone();
    let identity = runtime
        .runtime_hook_agent_failure_identity(&project_root, window_id)
        .expect("seeded failure identity");
    runtime
        .runtime_hook_agent_failures_in_flight
        .insert(window_id.to_string(), identity);
    runtime
        .launch_error_terminal_details
        .insert(window_id.to_string(), "x".repeat(64 * 1024));
    runtime.board_all_view_windows.insert(window_id.to_string());
    runtime
        .pending_auto_resume_sources
        .insert(window_id.to_string(), "session-resume-source".to_string());
    runtime
        .window_pty_statuses
        .insert(window_id.to_string(), WindowProcessStatus::Running);
    runtime
        .window_hook_states
        .insert(window_id.to_string(), WindowProcessStatus::Running);
    runtime
        .window_output_bytes
        .insert(window_id.to_string(), 42);
    runtime
        .recoverable_agent_error_windows
        .insert(window_id.to_string());
    runtime
        .last_agent_activity
        .insert(window_id.to_string(), chrono::Utc::now());
    runtime
        .last_issue_monitor_heartbeat
        .insert(window_id.to_string(), chrono::Utc::now());
    runtime
        .window_last_output_at
        .insert(window_id.to_string(), chrono::Utc::now());
}

async fn next_test_agent_websocket_event<S>(
    socket: &mut S,
    expected_kind: &str,
    deadline: Duration,
) -> serde_json::Value
where
    S: Stream<Item = Result<WebSocketMessage, WebSocketError>> + Unpin,
{
    tokio::time::timeout(deadline, async {
        loop {
            match socket.next().await {
                Some(Ok(WebSocketMessage::Text(payload))) => {
                    let value: serde_json::Value =
                        serde_json::from_str(payload.as_ref()).expect("agent WebSocket JSON");
                    if value.get("kind").and_then(serde_json::Value::as_str) == Some(expected_kind)
                    {
                        return value;
                    }
                }
                Some(Ok(
                    WebSocketMessage::Ping(_)
                    | WebSocketMessage::Pong(_)
                    | WebSocketMessage::Binary(_)
                    | WebSocketMessage::Frame(_),
                )) => {}
                Some(Ok(WebSocketMessage::Close(frame))) => {
                    panic!("agent WebSocket closed before {expected_kind}: {frame:?}")
                }
                Some(Err(error)) => {
                    panic!("agent WebSocket failed before {expected_kind}: {error}")
                }
                None => panic!("agent WebSocket ended before {expected_kind}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!("agent WebSocket did not return {expected_kind} within {deadline:?}")
    })
}

fn materialize_active_agent_pane_binding(
    project: &Path,
    session_id: &str,
) -> gwt_agent::SessionExecutionBinding {
    fs::create_dir_all(project).expect("create repository fixture");
    for args in [
        vec!["init"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test User"],
        vec![
            "remote",
            "add",
            "origin",
            "https://example.invalid/acme/pane-lease.git",
        ],
        vec!["commit", "--allow-empty", "-m", "initial"],
    ] {
        let output =
            gwt_core::process::run_git_logged(&args, Some(project)).expect("run fixture git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let mut session =
        gwt_agent::Session::new(project, "work/pane-lease", gwt_agent::AgentId::Codex);
    session.id = session_id.to_string();
    session.project_state_root = Some(project.to_path_buf());
    session.linked_issue_number = Some(owner.number);
    session
        .save(&gwt_core::paths::gwt_sessions_dir())
        .expect("save durable Session");
    gwt::cli::execution_state::materialize_at_launch(
        project,
        owner.kind,
        owner.number,
        session_id,
        "gwt-execute",
        false,
    )
    .expect("materialize active execution");
    gwt::cli::execution_state::ensure_generation_ledger(
        project,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize owner generation ledger");
    let identity = gwt::cli::execution_state::current_execution_binding(project, owner)
        .expect("read active execution identity")
        .expect("active generation identity");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: session.repo_hash.clone().expect("Session repository hash"),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    session
        .set_execution_binding(Some(binding.clone()))
        .expect("bind durable Session");
    session
        .save(&gwt_core::paths::gwt_sessions_dir())
        .expect("persist active binding");
    binding
}

fn active_bound_agent_pane_runtime(
    temp_root: &Path,
    project: &Path,
    session_id: &str,
    binding: &gwt_agent::SessionExecutionBinding,
) -> (
    AppRuntime,
    crate::embedded_server::AgentCapabilityIssuer,
    crate::embedded_server::AgentCapabilityGrant,
) {
    let mut tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-project",
        project.to_path_buf(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-project", Some(session_id.to_string())));
    let (mut runtime, _) = sample_runtime_with_events(temp_root, vec![tab], Some("tab-project"));
    insert_test_pane_runtime(&mut runtime, "tab-project::agent-project");
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    let target = issuer
        .issue_bound(project, session_id, binding.clone())
        .expect("issue active pane capability");
    let grant = issuer
        .grant_for_test(&target.token)
        .expect("active pane grant");
    runtime.agent_capability_issuer = Some(issuer.clone());
    (runtime, issuer, grant)
}

fn self_close_runtime(
    temp_root: &Path,
) -> (
    AppRuntime,
    Arc<Mutex<Vec<UserEvent>>>,
    crate::embedded_server::AgentCapabilityIssuer,
    crate::embedded_server::AgentCapabilityGrant,
    String,
) {
    let project = temp_root.join("project");
    fs::create_dir_all(&project).expect("project");
    let mut tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-project",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-project", Some("session-project".to_string())));
    let (mut runtime, events) =
        sample_runtime_with_events(temp_root, vec![tab], Some("tab-project"));
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    let target = issuer
        .issue(&project, "session-project")
        .expect("self-close capability");
    let grant = issuer
        .grant_for_test(&target.token)
        .expect("authenticated self-close grant");
    let window_id = "tab-project::agent-project".to_string();
    runtime.agent_capability_issuer = Some(issuer.clone());
    runtime
        .agent_capability_tokens
        .insert(window_id.clone(), target.token);
    (runtime, events, issuer, grant, window_id)
}

fn take_self_close_commit(
    events: &Arc<Mutex<Vec<UserEvent>>>,
) -> super::AgentSelfCloseCapabilityTicket {
    let mut events = events.lock().expect("event log");
    let position = events
        .iter()
        .position(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::CommitAgentSelfClose { .. }
            )
        })
        .expect("self-close acceptance drop must schedule a commit");
    match events.remove(position) {
        UserEvent::CommitAgentSelfClose { ticket } => ticket,
        _ => unreachable!("matched self-close commit"),
    }
}

fn apply_recorded_window_close_finalized(
    runtime: &mut AppRuntime,
    recorded: &Arc<Mutex<Vec<UserEvent>>>,
) -> Vec<OutboundEvent> {
    let event = {
        let mut recorded = recorded.lock().expect("event log");
        let position = recorded
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::WindowCloseFinalized { .. }
                )
            })
            .expect("window close finalizer completion");
        recorded.remove(position)
    };
    let Some(event) = runtime.accept_project_completion(event) else {
        return Vec::new();
    };
    let UserEvent::WindowCloseFinalized {
        window_id,
        project_root,
        closing_session_id,
        pm_close,
        pm_deregistered,
        pm_status,
        monitor_result,
    } = event
    else {
        unreachable!("selected window close finalizer event")
    };
    runtime.handle_window_close_finalized(
        &window_id,
        project_root.as_deref(),
        closing_session_id.as_deref(),
        pm_close,
        pm_deregistered,
        pm_status,
        monitor_result,
    )
}

fn wait_for_recorded_event(
    label: &str,
    events: &Arc<Mutex<Vec<UserEvent>>>,
    predicate: impl Fn(&[UserEvent]) -> bool,
) {
    wait_for_recorded_event_with_timeout(label, events, Duration::from_secs(20), predicate);
}

fn wait_for_recorded_event_with_timeout(
    label: &str,
    events: &Arc<Mutex<Vec<UserEvent>>>,
    timeout: Duration,
    predicate: impl Fn(&[UserEvent]) -> bool,
) {
    let deadline = Instant::now() + timeout;
    loop {
        {
            let events = events.lock().expect("event log");
            if predicate(&events) {
                return;
            }
        }
        if Instant::now() >= deadline {
            break;
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
                if let UserEvent::Dispatch(dispatched)
                | UserEvent::ProjectDispatch {
                    events: dispatched, ..
                } = event
                {
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
                recorded_project_payload(event),
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
                    recorded_project_payload(event),
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
    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        None,
    );
    assert_eq!(events.len(), 1);
    let pending_view = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .view();
    assert!(pending_view.runtime_resolution_pending);
    assert!(!pending_view.runtime_context_resolved);

    wait_for_recorded_event(label, recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::LaunchWizardRuntimeResolved { .. }
            )
        })
    });
    let resolved_event = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardRuntimeResolved { .. }
                )
            })
            .map(|index| events.remove(index))
            .expect("runtime resolved event")
    };
    let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
        unreachable!("matched above")
    };
    let resolved_events = runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
    assert_eq!(resolved_events.len(), 1);
}

fn sample_launch_wizard_session(tab_id: &str, project_root: &Path) -> LaunchWizardSession {
    LaunchWizardSession {
        project_context: super::ProjectContext {
            tab_id: tab_id.to_string(),
            project_key: gwt_core::paths::resolve_project_scope(project_root).hash,
            generation: 1,
            project_root: project_root.to_path_buf(),
        },
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
            },
            Vec::new(),
        ),
        workspace_resume_context: None,
        agent_kanban_target: None,
        auto_submit_after_runtime_resolution: None,
        issue_monitor_profile_save: None,
        issue_monitor_launch_issue_number: None,
        origin: super::LaunchWizardOrigin::ManualLaunchAgent,
        manual_holder_intent: None,
    }
}

fn sample_agent_options() -> Vec<gwt::AgentOption> {
    vec![gwt::AgentOption {
        id: "codex".to_string(),
        name: "Codex".to_string(),
        available: true,
        installed_version: Some("latest".to_string()),
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
        fast_mode: true,
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        docker_service: None,
        docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
        windows_shell: None,
        prefer_for: Vec::new(),
    }
}

fn legacy_issue_monitor_git_failure(project_root: &Path) -> String {
    format!(
        "Current branch is unavailable: Git error: Not a git repository: {}",
        project_root.display()
    )
}

// #4499: cache membership alone does not authorize monitor admission.
fn queued_issue_monitor_prefs(issue_numbers: &[u64]) -> gwt::IssueMonitorPrefs {
    let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig::default());
    monitor.terminal_queue_push(issue_numbers, "operator", "2026-07-28T00:00:00Z");
    monitor.prefs()
}

fn legacy_issue_monitor_failed_prefs(
    project_root: &Path,
    issue_number: u64,
) -> gwt::IssueMonitorPrefs {
    gwt::IssueMonitorPrefs {
        enabled: false,
        legacy_git_launch_failure_migration_version: 0,
        failed_issues: vec![gwt::IssueMonitorFailedIssue {
            issue_number,
            message: legacy_issue_monitor_git_failure(project_root),
            window_id: None,
        }],
        ..queued_issue_monitor_prefs(&[issue_number])
    }
}

fn issue_monitor_autonomous_record(
    issue_number: u64,
    phase: gwt::AutonomousPhase,
    attempts: u32,
) -> gwt::AutonomousIssueRecord {
    gwt::AutonomousIssueRecord {
        issue_number,
        phase,
        active_launch_id: None,
        attempts,
        non_agent_attempts: 0,
        acceptance_snapshot: None,
        retry_not_before: None,
        retry_hold_reason: None,
        retry_hold_provider: None,
        last_heartbeat: None,
        pr_number: None,
        reviewed_sha: None,
        review_passed: None,
        wait: None,
        needs_human_kind: None,
        steering: None,
        review_dispatch_hold: None,
        last_failure_message: None,
        delivering_since: None,
        review_attempts: None,
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
    init_git_clone_with_default_branch(repo, "develop")
}

fn init_git_clone_with_default_branch(repo: &Path, default_branch: &str) -> PathBuf {
    let root = repo.parent().expect("repo parent");
    let seed = root.join("seed");
    let origin = root.join("origin.git");
    fs::create_dir_all(&seed).expect("create seed");
    run_git(&seed, &["init", "-q", "-b", default_branch]);
    run_git(&seed, &["config", "user.name", "Codex"]);
    run_git(&seed, &["config", "user.email", "codex@example.com"]);
    fs::write(seed.join("README.md"), "repo\n").expect("seed readme");
    run_git(&seed, &["add", "README.md"]);
    run_git(&seed, &["commit", "-qm", "init"]);
    run_git_with_paths(&["clone", "--bare"], &[&seed, &origin]);
    // Keep checkout bytes independent of the host's Git line-ending settings.
    run_git_with_paths(
        &["clone", "--config", "core.autocrlf=false"],
        &[&origin, repo],
    );
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
        project_context: super::ProjectContext {
            tab_id: tab_id.to_string(),
            project_key: gwt_core::paths::resolve_project_scope(project_root).hash,
            generation: 1,
            project_root: project_root.to_path_buf(),
        },
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
            },
            Vec::new(),
            Vec::new(),
        ),
        workspace_resume_context: None,
        agent_kanban_target: None,
        auto_submit_after_runtime_resolution: None,
        issue_monitor_profile_save: None,
        issue_monitor_launch_issue_number: None,
        origin: super::LaunchWizardOrigin::ManualLaunchAgent,
        manual_holder_intent: None,
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
        project_context: super::ProjectContext {
            tab_id: tab_id.to_string(),
            project_key: gwt_core::paths::resolve_project_scope(project_root).hash,
            generation: 1,
            project_root: project_root.to_path_buf(),
        },
        tab_id: tab_id.to_string(),
        wizard_id: "wizard-start-work-confirm".to_string(),
        wizard,
        workspace_resume_context: None,
        agent_kanban_target: None,
        auto_submit_after_runtime_resolution: None,
        issue_monitor_profile_save: None,
        issue_monitor_launch_issue_number: None,
        origin: super::LaunchWizardOrigin::StartWork,
        manual_holder_intent: None,
    }
}

fn sample_ready_agent_launch_wizard_session(
    tab_id: &str,
    project_root: &Path,
) -> LaunchWizardSession {
    LaunchWizardSession {
        project_context: super::ProjectContext {
            tab_id: tab_id.to_string(),
            project_key: gwt_core::paths::resolve_project_scope(project_root).hash,
            generation: 1,
            project_root: project_root.to_path_buf(),
        },
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
            },
            sample_agent_options(),
            Vec::new(),
        ),
        workspace_resume_context: None,
        agent_kanban_target: None,
        auto_submit_after_runtime_resolution: None,
        issue_monitor_profile_save: None,
        issue_monitor_launch_issue_number: None,
        origin: super::LaunchWizardOrigin::ManualLaunchAgent,
        manual_holder_intent: None,
    }
}

fn install_manual_launch_holder(
    runtime: &mut AppRuntime,
    repo: &Path,
    session_id: &str,
    status: gwt_agent::AgentStatus,
    local_window_id: Option<&str>,
) -> gwt_agent::SessionExecutionIdentity {
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 42,
    };
    let mut session = gwt_agent::Session::new(repo, "feature/demo", gwt_agent::AgentId::Codex);
    session.id = session_id.to_string();
    session.project_state_root = Some(repo.to_path_buf());
    session.linked_issue_number = Some(owner.number);
    session.status = status;
    session
        .save(&runtime.sessions_dir)
        .expect("save manual launch holder Session");
    gwt::cli::execution_state::materialize_at_launch(
        repo,
        owner.kind,
        owner.number,
        session_id,
        "gwt-execute",
        false,
    )
    .expect("materialize manual holder execution");
    gwt::cli::execution_state::ensure_generation_ledger(
        repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize manual holder ledger");
    let identity = gwt::cli::execution_state::current_execution_binding(repo, owner)
        .expect("read manual holder binding")
        .expect("manual holder binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: session.repo_hash.clone().expect("holder repository hash"),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    session
        .set_execution_binding(Some(binding))
        .expect("bind manual holder Session");
    session
        .save(&runtime.sessions_dir)
        .expect("persist bound manual holder Session");
    runtime.launch_wizard_cache = LaunchWizardMemoryCache::load(&runtime.sessions_dir);
    if let Some(window_id) = local_window_id {
        runtime.active_agent_sessions.insert(
            window_id.to_string(),
            ActiveAgentSession {
                window_id: window_id.to_string(),
                session_id: session_id.to_string(),
                agent_id: "codex".to_string(),
                branch_name: "feature/demo".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: repo.to_path_buf(),
                agent_project_root: repo.display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );
        runtime
            .window_pty_statuses
            .insert(window_id.to_string(), WindowProcessStatus::Running);
    }
    let identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
        .expect("validate holder Session identity")
        .expect("holder Session identity");
    if matches!(
        status,
        gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
    ) {
        gwt_agent::SessionRuntimeState::for_execution_process(
            status,
            &identity,
            1,
            gwt::process::host_process_start_time(std::process::id())
                .expect("test Host process start time"),
            i32::MAX as u32,
            1,
        )
        .save(&gwt_agent::runtime_state_path(
            &runtime.sessions_dir,
            session_id,
        ))
        .expect("persist terminal holder runtime proof");
    }
    identity
}

fn install_manual_holder_capability(
    runtime: &mut AppRuntime,
    repo: &Path,
    window_id: &str,
    holder: &gwt_agent::SessionExecutionIdentity,
) -> crate::embedded_server::AgentCapabilityIssuer {
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:45155/internal/hook-live",
        "ws://127.0.0.1:46255/ws",
        "ws://127.0.0.1:45155/internal/pane-ws",
    );
    let capability = issuer
        .issue_bound(repo, &holder.session_id, holder.execution_binding.clone())
        .expect("issue exact holder capability");
    runtime.agent_capability_issuer = Some(issuer.clone());
    runtime
        .agent_capability_tokens
        .insert(window_id.to_string(), capability.token);
    issuer
}

#[derive(Clone, Copy)]
enum StaleManualHolderAction {
    Stop,
    Move,
}

fn assert_manual_launch_action_rejects_replaced_runtime(action: StaleManualHolderAction) {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let session_id = match action {
        StaleManualHolderAction::Stop => "manual-stale-stop-holder",
        StaleManualHolderAction::Move => "manual-stale-move-holder",
    };
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-holder",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-holder", Some(session_id.to_string())));
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let holder_window_id = combined_window_id("tab-1", "agent-holder");
    let holder = install_manual_launch_holder(
        &mut runtime,
        &repo,
        session_id,
        gwt_agent::AgentStatus::Running,
        Some(&holder_window_id),
    );
    insert_test_pane_runtime(&mut runtime, &holder_window_id);
    let holder_incarnation = runtime
        .runtimes
        .get(&holder_window_id)
        .expect("holder runtime")
        .incarnation;
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:45155/internal/hook-live",
        "ws://127.0.0.1:46255/ws",
        "ws://127.0.0.1:45155/internal/pane-ws",
    );
    let capability = issuer
        .issue_bound(&repo, session_id, holder.execution_binding.clone())
        .expect("issue exact holder capability");
    runtime.agent_capability_issuer = Some(issuer.clone());
    runtime
        .agent_capability_tokens
        .insert(holder_window_id.clone(), capability.token.clone());
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));
    settle_test_pane_child(&runtime, &holder_window_id);
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );
    let decision = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("holder decision wizard")
        .wizard
        .view()
        .holder_decision
        .expect("local holder decision");

    let replaced = runtime
        .runtimes
        .remove(&holder_window_id)
        .expect("replace decided holder runtime");
    replaced
        .pane
        .lock()
        .expect("lock replaced holder pane")
        .kill()
        .expect("stop replaced holder pane");
    insert_test_pane_runtime(&mut runtime, &holder_window_id);
    let successor_runtime = runtime
        .runtimes
        .get(&holder_window_id)
        .expect("successor runtime");
    let successor_incarnation = successor_runtime.incarnation;
    let successor_pane = successor_runtime.pane.clone();
    assert_ne!(successor_incarnation, holder_incarnation);

    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 42,
    };
    let session_path = runtime.sessions_dir.join(format!("{session_id}.toml"));
    let runtime_state_path = gwt_agent::runtime_state_path(&runtime.sessions_dir, session_id);
    let session_before = fs::read(&session_path).expect("read successor Session before action");
    let runtime_state_before = fs::read(&runtime_state_path).ok();
    let ledger_before = serde_json::to_vec(
        &gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("read successor ledger")
            .expect("successor ledger"),
    )
    .expect("serialize successor ledger");
    let projection_before =
        fs::read(gwt::cli::execution_state::state_path(&repo)).expect("read successor projection");
    let workspace_before = runtime.tabs[0].workspace.persisted().clone();
    let active_before = runtime
        .active_agent_sessions
        .get(&holder_window_id)
        .expect("successor active Session")
        .clone();
    let capabilities_before = runtime.agent_capability_tokens.clone();
    assert!(issuer.authenticates_token(&capability.token));
    assert!(recorded_events.lock().expect("event log").is_empty());

    let action = match action {
        StaleManualHolderAction::Stop => LaunchWizardAction::StopAndStartSuccessor {
            fingerprint: decision.fingerprint,
            window_id: holder_window_id.clone(),
        },
        StaleManualHolderAction::Move => LaunchWizardAction::MoveExistingPane {
            fingerprint: decision.fingerprint,
            window_id: holder_window_id.clone(),
        },
    };
    let events =
        runtime.handle_launch_wizard_action(&runtime.test_context(), action, Some(canvas_bounds()));

    let current_runtime = runtime
        .runtimes
        .get(&holder_window_id)
        .expect("stale action must preserve successor runtime");
    assert_eq!(current_runtime.incarnation, successor_incarnation);
    assert!(Arc::ptr_eq(&current_runtime.pane, &successor_pane));
    assert!(matches!(
        successor_pane
            .lock()
            .expect("lock successor pane")
            .check_status()
            .expect("read successor pane status"),
        gwt_terminal::PaneStatus::Running
    ));
    assert_eq!(
        runtime.tabs[0].workspace.persisted(),
        &workspace_before,
        "stale action must not focus, stop, or rewrite the successor workspace",
    );
    let active_after = runtime
        .active_agent_sessions
        .get(&holder_window_id)
        .expect("stale action must preserve successor active Session");
    assert_eq!(format!("{active_after:?}"), format!("{active_before:?}"));
    assert_eq!(
        fs::read(&session_path).expect("read successor Session after refusal"),
        session_before,
        "stale action must not rewrite the durable Session",
    );
    assert_eq!(
        fs::read(&runtime_state_path).ok(),
        runtime_state_before,
        "stale action must not write runtime terminal proof",
    );
    assert_eq!(
        serde_json::to_vec(
            &gwt::cli::execution_state::load_generation_ledger(&repo, owner)
                .expect("read successor ledger after refusal")
                .expect("successor ledger after refusal"),
        )
        .expect("serialize successor ledger after refusal"),
        ledger_before,
        "stale action must not change generation authority",
    );
    assert_eq!(
        fs::read(gwt::cli::execution_state::state_path(&repo))
            .expect("read successor projection after refusal"),
        projection_before,
        "stale action must not rewrite execution projection",
    );
    assert_eq!(runtime.agent_capability_tokens, capabilities_before);
    assert!(issuer.authenticates_token(&capability.token));
    assert!(recorded_events.lock().expect("event log").is_empty());
    assert!(runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.wizard.error.as_deref())
        .is_some_and(|error| error.contains("holder changed")));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::LaunchWizardState { wizard: Some(view) }
            if view.error.as_deref().is_some_and(|error| error.contains("holder changed"))
    )));

    runtime.stop_window_runtime_without_session_projection(&holder_window_id);
}

fn issue_monitor_feedback(issue_number: u64) -> LaunchFeedbackContext {
    LaunchFeedbackContext {
        client_id: "__issue_monitor__".to_string(),
        title: "Issue Monitor".to_string(),
        issue_monitor_issue_number: Some(issue_number),
        issue_monitor_delivery_id: None,
        issue_monitor_project_root: None,
        issue_monitor_session_mode: None,
        issue_monitor_autonomous_handoff: None,
        issue_monitor_autonomous_submit_started: false,
        issue_monitor_review_dispatch: false,
    }
}

#[test]
fn issue_monitor_final_spawn_reads_fresh_shared_auto_capacity() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_repo_with_initial_commit(&repo);
    let repo = dunce::canonicalize(repo).expect("canonical repo");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.blocking_tasks = BlockingTaskSpawner::queued().0;
    // The helper pins mock providers. Fail before provider/PTY preparation so
    // this test observes pane admission without starting an agent process.
    fs::write(
        runtime
            .profile_config_path
            .as_ref()
            .expect("fixture profile"),
        "invalid = [",
    )
    .expect("write malformed fixture profile");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Auto,
            max_active_agents: 9,
            ..Default::default()
        },
    )
    .expect("save Auto prefs");
    let prefs_before = fs::read(&prefs_path).expect("prefs before admission");
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode)
        .working_dir(&repo)
        .build();
    let mut feedback = issue_monitor_feedback(43);
    feedback.issue_monitor_project_root = Some(repo.clone());
    feedback.issue_monitor_session_mode = Some(gwt_agent::SessionMode::Normal);

    let rejected = runtime.spawn_agent_window_with_feedback(
        "tab-1",
        config.clone(),
        canvas_bounds(),
        None,
        feedback.clone(),
    );
    assert!(rejected.is_err_and(|reason| reason.contains("max_active")));
    assert!(runtime.tabs[0].workspace.persisted().windows.is_empty());
    assert_eq!(fs::read(&prefs_path).unwrap(), prefs_before);

    let target = repo.join("target");
    fs::create_dir_all(&target).expect("measured target dir");
    let artifact = target.join("fixture-artifact");
    fs::write(&artifact, b"measured target fixture").expect("target artifact");
    let target_bytes = fs::metadata(&artifact).expect("measured artifact").len();
    let now = u64::try_from(Utc::now().timestamp()).expect("positive epoch");
    let snapshot = serde_json::json!({
        "observed_at": now, "expires_at": now + 30,
        "performance_cores": 2, "gui_cpu_millicores": 0,
        "available_ram_bytes": 2_u64 * 1024 * 1024 * 1024,
        "per_agent_ram_bytes": 512_u64 * 1024 * 1024,
        "free_disk_bytes": 4096,
        "targets": BTreeMap::from([(target, serde_json::json!({
            "bytes": target_bytes, "observed_at": now
        }))]),
        "disk_observations": BTreeMap::from([(repo.clone(), serde_json::json!({
            "available_bytes": 4096, "observed_at": now
        }))]),
        "inventory": {"sessions": [], "uncertainties": []}
    });
    let machine_state = gwt_core::paths::gwt_home().join("machine-state");
    fs::create_dir_all(&machine_state).expect("machine state dir");
    fs::write(
        machine_state.join("agent-capacity.json"),
        serde_json::to_vec(&snapshot).expect("serialize measured snapshot"),
    )
    .expect("save shared measured snapshot");
    let measured = gwt::agent_capacity::project_capacity(&repo, &Default::default(), 0);
    assert!(measured.measurement_complete && measured.is_fresh());
    assert_eq!(measured.machine_budget, Some(2));
    assert_eq!(measured.recommended_worker_limit, 1, "reserve one own PM");

    runtime
        .spawn_agent_window_with_feedback("tab-1", config, canvas_bounds(), None, feedback)
        .expect("fresh shared capacity admits the proposed Monitor pane");
    assert_eq!(runtime.tabs[0].workspace.persisted().windows.len(), 1);
    let completion = take_monitor_launch_complete("post-admission fixture failure", &events);
    assert!(completion.is_err_and(|reason| reason.detail.contains("config parse error")));
    assert!(runtime.runtimes.is_empty(), "no provider PTY was created");
    assert!(fs::read_dir(&runtime.sessions_dir)
        .unwrap()
        .next()
        .is_none());
    assert_eq!(fs::read(&prefs_path).unwrap(), prefs_before);
}

#[test]
fn issue_monitor_final_spawn_rejects_a_saturated_cap_without_delivery() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo_with_initial_commit(&repo);
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Agent],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.blocking_tasks = BlockingTaskSpawner::queued().0;
    let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig {
        enabled: true,
        max_active: 1,
        ..Default::default()
    });
    monitor.complete_active_launch(42, "tab-1::agent-1");
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &monitor.prefs(),
    )
    .unwrap();
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/issue-43")
        .build();
    let mut feedback = issue_monitor_feedback(43);
    feedback.issue_monitor_project_root = Some(repo.clone());
    feedback.issue_monitor_session_mode = Some(gwt_agent::SessionMode::Normal);
    let result =
        runtime.spawn_agent_window_with_feedback("tab-1", config, canvas_bounds(), None, feedback);
    assert!(
        result
            .as_ref()
            .is_err_and(|reason| reason.contains("max_active")),
        "Monitor legacy/manual dispatch must honor capacity before creating a pane: {result:?}"
    );
    assert_eq!(runtime.tabs[0].workspace.persisted().windows.len(), 1);

    // A retained Monitor pane in a sibling local tab is still a physical slot.
    let empty_tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut sibling = sample_project_tab(
        "tab-2",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Agent],
    );
    let raw_id = sibling.workspace.persisted().windows[0].id.clone();
    sibling
        .workspace
        .set_status(&raw_id, WindowProcessStatus::Error);
    let mut runtime = sample_runtime(temp.path(), vec![empty_tab, sibling], Some("tab-1"));
    runtime.blocking_tasks = BlockingTaskSpawner::queued().0;
    let mut retained = issue_monitor_feedback(42);
    retained.issue_monitor_project_root = Some(repo.clone());
    runtime
        .pending_launch_feedback_contexts
        .insert(combined_window_id("tab-2", &raw_id), retained);
    let snapshot = runtime
        .issue_monitor_window_snapshot_for_tab("tab-1", "2026-10-06T00:00:00Z")
        .unwrap();
    assert_eq!(
        snapshot.windows.len(),
        1,
        "the physical snapshot covers every local project tab"
    );
    assert_eq!(
        snapshot.windows[0].window_id,
        combined_window_id("tab-2", &raw_id)
    );
    let monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig {
        enabled: true,
        max_active: 1,
        ..Default::default()
    });
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &monitor.prefs(),
    )
    .unwrap();
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/issue-43")
        .build();
    let mut feedback = issue_monitor_feedback(43);
    feedback.issue_monitor_project_root = Some(repo.clone());
    feedback.issue_monitor_session_mode = Some(gwt_agent::SessionMode::Normal);
    let result =
        runtime.spawn_agent_window_with_feedback("tab-1", config, canvas_bounds(), None, feedback);
    assert!(
        result
            .as_ref()
            .is_err_and(|reason| reason.contains("max_active")),
        "a sibling retained pane exhausts capacity: {result:?}"
    );
    assert!(runtime.tabs[0].workspace.persisted().windows.is_empty());

    // Binding-free same-Issue panes also prevent duplication while the cap has room.
    runtime.tabs[1]
        .workspace
        .set_status(&raw_id, WindowProcessStatus::Running);
    let monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig {
        max_active: 3,
        ..Default::default()
    });
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &monitor.prefs(),
    )
    .unwrap();
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/issue-42")
        .build();
    let mut feedback = issue_monitor_feedback(42);
    feedback.issue_monitor_project_root = Some(repo);
    feedback.issue_monitor_session_mode = Some(gwt_agent::SessionMode::Normal);
    let result =
        runtime.spawn_agent_window_with_feedback("tab-1", config, canvas_bounds(), None, feedback);
    assert!(
        result
            .as_ref()
            .is_err_and(|reason| reason.contains("already")),
        "a sibling implementation blocks the same Issue before adding a pane: {result:?}"
    );
    assert!(runtime.tabs[0].workspace.persisted().windows.is_empty());
    assert_eq!(runtime.tabs[1].workspace.persisted().windows.len(), 1);
}

fn spawned_agent_placement(runtime: &AppRuntime, tab_id: &str) -> WindowPlacement {
    runtime
        .tab(tab_id)
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window")
        .placement
        .clone()
}

fn take_project_navigation_completion(
    recorded_events: &Arc<Mutex<Vec<UserEvent>>>,
) -> ProjectNavigationPrepared {
    wait_for_recorded_event("project navigation completion", recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::ProjectNavigationPrepared(_)
            )
        })
    });
    let mut events = recorded_events.lock().expect("event log");
    let index = events
        .iter()
        .position(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::ProjectNavigationPrepared(_)
            )
        })
        .expect("project navigation completion");
    match events.remove(index) {
        UserEvent::ProjectNavigationPrepared(prepared) => *prepared,
        _ => unreachable!("matched project navigation completion"),
    }
}

pub(super) fn drain_queued_blocking_tasks(tasks: &BlockingTestTaskQueue) {
    loop {
        let task = tasks.lock().expect("blocking task queue").pop();
        let Some(task) = task else {
            break;
        };
        task();
    }
}

/// SPEC #3170: project open / switch is prepared off the tao thread, so a
/// focused test drives the queued worker and the generation-checked commit
/// explicitly instead of reading a tab straight out of the dispatch call.
fn commit_pending_project_navigation(
    runtime: &mut AppRuntime,
    queued_tasks: &BlockingTestTaskQueue,
    recorded_events: &Arc<Mutex<Vec<UserEvent>>>,
) -> Vec<OutboundEvent> {
    drain_queued_blocking_tasks(queued_tasks);
    let prepared = take_project_navigation_completion(recorded_events);
    runtime.handle_project_navigation_prepared(prepared)
}

fn issue_monitor_review_launch_completion(
    repo: &Path,
    session_id: &str,
) -> super::launch::AgentLaunchCompletion {
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "ping -n 31 127.0.0.1 >NUL".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "sleep 30".to_string()],
        )
    };
    (
        ProcessLaunch {
            initial_prompt_file: None,
            command,
            args,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: Some(repo.to_path_buf()),
            resource_policy: None,
        },
        session_id.to_string(),
        "work/issue-42".to_string(),
        "Codex".to_string(),
        repo.to_path_buf(),
        gwt_agent::AgentId::Codex,
        Some(42),
        Some("origin/develop".to_string()),
        gwt_agent::LaunchRuntimeTarget::Host,
        gwt_agent::SessionMode::Normal,
        false,
        repo.display().to_string().into(),
    )
}

fn assert_genesis_receipt_cleanup_failure(close_before_enqueue: bool, close_before_apply: bool) {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-genesis-receipt-cleanup");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "genesis-receipt-cleanup-failure";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize genesis execution");
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize genesis ledger");
    let identity = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read genesis binding")
        .expect("genesis binding");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, recorded) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let mut session = gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    session.id = session_id.to_string();
    session.project_state_root = Some(repo.clone());
    session.linked_issue_number = Some(owner.number);
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    session
        .set_execution_binding(Some(binding.clone()))
        .expect("bind genesis Session");
    session
        .save(&runtime.sessions_dir)
        .expect("save genesis Session");
    // Resolve a previously visible Session before its launch gains a recovery receipt.
    runtime.launch_wizard_cache.record_session(session.clone());
    persist_durable_launch_recovery(
        &runtime.sessions_dir,
        DurableLaunchRecoveryKind::Genesis,
        session_id,
        &repo,
        &repo,
        owner,
        Some(&binding),
        Some(&gwt_agent::AgentId::Codex),
    )
    .expect("persist exact genesis recovery receipt");
    let receipt_path = runtime
        .sessions_dir
        .join("execution-launch-recovery")
        .join(format!("{session_id}.json"));
    fs::remove_file(&receipt_path).expect("remove receipt file");
    fs::create_dir(&receipt_path).expect("create receipt cleanup blocker");
    assert!(
        runtime
            .launch_wizard_cache
            .quick_start_entries(&repo, "work/issue-2359")
            .iter()
            .any(|entry| entry.session_id == session_id),
        "the failed genesis starts visible in the warm cache",
    );
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.pending_workspace_resume_contexts.insert(
        window_id.clone(),
        WorkspaceResumeContext {
            title: Some("Genesis receipt rollback".to_string()),
            owner: Some("Issue #2359".to_string()),
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
                "ping -n 31 127.0.0.1 >NUL".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "sleep 30".to_string()],
        )
    };

    if close_before_enqueue {
        assert!(runtime.close_window_outcome(&window_id).closed);
    }
    assert!(runtime
        .handle_launch_complete(
            window_id.clone(),
            Ok((
                ProcessLaunch {
                    initial_prompt_file: None,
                    command,
                    args,
                    env: HashMap::new(),
                    remove_env: Vec::new(),
                    cwd: Some(repo.clone()),
                    resource_policy: None,
                },
                session_id.to_string(),
                "work/issue-2359".to_string(),
                "Codex".to_string(),
                repo.clone(),
                gwt_agent::AgentId::Codex,
                Some(owner.number),
                Some("origin/develop".to_string()),
                gwt_agent::LaunchRuntimeTarget::Host,
                gwt_agent::SessionMode::Normal,
                false,
                repo.display().to_string().into(),
            )),
        )
        .is_empty());
    drain_queued_blocking_tasks(&tasks);
    let prepared = take_prepared_agent_launch(&recorded);
    if close_before_apply {
        assert!(runtime.close_window_outcome(&window_id).closed);
        assert!(!runtime.pending_launch_completions.contains_key(&window_id));
    }
    let events = runtime.handle_agent_launch_prepared(prepared);
    if close_before_apply {
        assert!(
            events.is_empty(),
            "stale failure must not update a closed window"
        );
    } else if !close_before_enqueue {
        assert!(events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::TerminalStatus {
                status: WindowProcessStatus::Error,
                ..
            }
        )));
    }
    assert!(!runtime.active_agent_sessions.contains_key(&window_id));
    if !close_before_enqueue {
        let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
            .expect("read compensated Workspace")
            .expect("compensated Workspace projection");
        assert!(projection.latest_agent_for_session(session_id).is_none());
        let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
            .expect("read compensated WorkItems")
            .expect("compensated WorkItems projection");
        assert_eq!(work_items.work_items.len(), 1);
        assert!(
            work_items.work_items[0].is_terminal() && work_items.work_items[0].discarded,
            "the published Work must be discarded when readiness cannot commit",
        );
    }
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("read terminal genesis ledger")
            .expect("terminal genesis ledger")
            .current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
    );
    assert!(
        receipt_path.is_dir(),
        "the injected cleanup blocker must remain"
    );
    assert!(
        !runtime
            .sessions_dir
            .join(format!("{session_id}.toml"))
            .exists(),
        "exact Session removal must precede the pending receipt cleanup",
    );
    assert!(
        runtime
            .launch_wizard_cache
            .quick_start_entries(&repo, "work/issue-2359")
            .iter()
            .all(|entry| entry.session_id != session_id),
        "an exact failed genesis Session must disappear from Quick Start even when durable cleanup remains pending",
    );
}

struct ContinueWorkLaunchFailureFixture {
    runtime: AppRuntime,
    repo: PathBuf,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    window_id: String,
    selected_work_id: &'static str,
    candidate_session_id: &'static str,
    candidate_session: gwt_agent::Session,
    candidate_runtime_path: PathBuf,
}

fn continue_work_launch_failure_fixture(temp_root: &Path) -> ContinueWorkLaunchFailureFixture {
    let repo = temp_root.join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let selected_work_id = "work-selected";
    let candidate_session_id = "candidate-session";
    let now = chrono::Utc::now();
    let mut start = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        selected_work_id,
        now,
    );
    start.title = Some("Continue selected Work".to_string());
    start.owner = Some("Issue #2359".to_string());
    start.agent_session_id = Some("predecessor-session".to_string());
    start.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-2359".to_string()),
            worktree_path: Some(repo.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, start)
        .expect("record selected Work");
    gwt_core::workspace_projection::record_workspace_work_event(
        &repo,
        gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Done,
            selected_work_id,
            now + chrono::Duration::seconds(1),
        ),
    )
    .expect("complete selected Work");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp_root, vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut active = sample_active_agent_session("tab-1", &window_id);
    active.session_id = candidate_session_id.to_string();
    active.branch_name = "work/issue-2359".to_string();
    active.worktree_path = repo.clone();
    active.agent_project_root = repo.display().to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), active);

    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        "predecessor-session",
        "gwt-execute",
        false,
    )
    .expect("materialize predecessor");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            "predecessor-session",
            gwt::cli::execution_state::ExecutionSettlement::Completed,
        )
        .expect("settle predecessor"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
    )
    .expect("import predecessor");
    let predecessor_binding = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read predecessor binding")
        .expect("predecessor binding");
    let request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: "continue-op-1".to_string(),
        principal_id: "gwt-host-continuation".to_string(),
        work_id: Some(selected_work_id.to_string()),
        source: "continue-work:resume".to_string(),
        session_binding_id: "binding-candidate".to_string(),
        initial_session_id: candidate_session_id.to_string(),
        entrypoint: "gwt-execute".to_string(),
        requested_at: now,
    };
    gwt::cli::execution_state::prepare_successor(&repo, owner, &request)
        .expect("prepare successor");
    let planned =
        gwt::cli::execution_state::prepared_successor_execution_binding(&repo, owner, &request)
            .expect("derive successor binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: candidate_session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity: planned,
        capability_generation: 1,
    };
    let mut candidate_session =
        gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    candidate_session.id = candidate_session_id.to_string();
    candidate_session.project_state_root = Some(repo.clone());
    candidate_session.linked_issue_number = Some(owner.number);
    candidate_session
        .set_execution_binding(Some(binding.clone()))
        .expect("bind candidate Session");
    candidate_session
        .save(&runtime.sessions_dir)
        .expect("save candidate Session");
    let candidate_runtime_path =
        gwt_agent::runtime_state_path(&runtime.sessions_dir, candidate_session_id);
    gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running)
        .save(&candidate_runtime_path)
        .expect("save candidate runtime");
    runtime.pending_continue_work.insert(
        window_id.clone(),
        PendingContinueWork {
            client_id: "client-1".to_string(),
            operation_id: "continue-op-1".to_string(),
            work_id: selected_work_id.to_string(),
            project_root: repo.clone(),
            worktree_path: repo.clone(),
            owner,
            work_branch: "work/issue-2359".to_string(),
            work_agent_id: gwt_agent::AgentId::Codex,
            work_agent_session_id: Some("predecessor-session".to_string()),
            execution: PendingContinueWorkExecution::Successor(request),
            binding,
            readiness_nonce: "continue-ready-1".to_string(),
            outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
            resume_context: WorkspaceResumeContext {
                title: Some("Continue selected Work".to_string()),
                owner: Some("Issue #2359".to_string()),
                summary: None,
                next_action: None,
            },
            predecessor_session_id: "predecessor-session".to_string(),
            predecessor_binding,
        },
    );

    ContinueWorkLaunchFailureFixture {
        runtime,
        repo,
        owner,
        window_id,
        selected_work_id,
        candidate_session_id,
        candidate_session,
        candidate_runtime_path,
    }
}

/// Issue #3475: walk the pure readiness policy forward until it is one deadline
/// away from giving up. Runtime-level tests use this to reach the terminal
/// deadline without hard-coding the extension budget and without sleeping on a
/// wall clock (Issue #3339).
fn readiness_watch_at_last_extension(
    operation_id: &str,
    output_bytes: u64,
) -> ContinueWorkReadinessWatch {
    let mut watch = ContinueWorkReadinessWatch::new(operation_id.to_string());
    loop {
        match continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Live, output_bytes) {
            ReadinessDeadlineDecision::Extend(next) => watch = next,
            ReadinessDeadlineDecision::HandOff { .. } | ReadinessDeadlineDecision::Abort { .. } => {
                return watch
            }
        }
    }
}

#[derive(Clone, Copy)]
struct ProjectionOnlyContinueFixture {
    work_owner: Option<&'static str>,
    owner_number: u64,
    owner_kind: gwt::cli::execution_state::ExecutionOwnerKind,
    agent_id: Option<&'static str>,
    cross_worktree: bool,
    missing_container: bool,
    conflicting_owner: bool,
    conflicting_container: bool,
    conflicting_branch_only: bool,
    conflicting_agent: bool,
    legacy_flat_only: bool,
}

fn projection_only_continue_runtime(
    temp_root: &Path,
    case_name: &str,
    fixture: ProjectionOnlyContinueFixture,
) -> (
    AppRuntime,
    PathBuf,
    gwt::cli::execution_state::ExecutionOwnerKey,
    String,
) {
    let project_root = temp_root.join(case_name);
    fs::create_dir_all(&project_root).expect("create projection-only repo");
    init_repo_with_initial_commit(&project_root);
    let repo = if fixture.cross_worktree {
        let worktree = temp_root.join(format!("{case_name}-worktree"));
        let output = gwt_core::process::hidden_command("git")
            .args(["worktree", "add", "-q", "-b", "work/issue-2359"])
            .arg(&worktree)
            .current_dir(&project_root)
            .output()
            .expect("add projection-only target worktree");
        assert!(
            output.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        worktree
    } else {
        project_root.clone()
    };
    if !fixture.cross_worktree {
        run_git(&repo, &["branch", "-M", "work/issue-2359"]);
    }
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: fixture.owner_kind,
        number: fixture.owner_number,
    };
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        "historical-session",
        "gwt-execute",
        false,
    )
    .expect("materialize projection-only predecessor");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            "historical-session",
            gwt::cli::execution_state::ExecutionSettlement::Completed,
        )
        .expect("settle projection-only predecessor"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    if !fixture.legacy_flat_only {
        gwt::cli::execution_state::ensure_generation_ledger(
            &repo,
            owner,
            gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
        )
        .expect("import projection-only predecessor");
    }

    let work_id = format!("work-{case_name}");
    let now = chrono::Utc::now();
    let mut start = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        &work_id,
        now,
    );
    start.title = Some("Historical Work without a local Session".to_string());
    start.owner = fixture.work_owner.map(str::to_string);
    start.agent_session_id = Some("historical-session".to_string());
    start.agent_id = fixture.agent_id.map(str::to_string);
    start.display_name = fixture.agent_id.map(str::to_string);
    start.execution_container = (!fixture.missing_container).then(|| {
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-2359".to_string()),
            worktree_path: Some(repo.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        }
    });
    gwt_core::workspace_projection::record_workspace_work_event(&project_root, start)
        .expect("record projection-only Work");
    if fixture.conflicting_owner {
        let mut update = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            &work_id,
            now + chrono::Duration::microseconds(1),
        );
        update.owner = Some("SPEC #3248".to_string());
        gwt_core::workspace_projection::record_workspace_work_event(&project_root, update)
            .expect("record conflicting owner");
    }
    if fixture.conflicting_agent {
        let mut update = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            &work_id,
            now,
        );
        update.agent_session_id = Some("conflicting-session".to_string());
        update.agent_id = Some("Claude Code".to_string());
        update.display_name = Some("Claude Code".to_string());
        gwt_core::workspace_projection::record_workspace_work_event(&project_root, update)
            .expect("record conflicting agent");
    }
    if fixture.conflicting_container {
        let foreign = temp_root.join(format!("{case_name}-foreign"));
        fs::create_dir_all(&foreign).expect("create conflicting container");
        init_repo(&foreign);
        let mut update = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            &work_id,
            now + chrono::Duration::milliseconds(1),
        );
        update.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                // The folded projection treats a matching branch as the same
                // container. The continuation resolver must still inspect the
                // append-only event history and reject the conflicting path.
                branch: Some("work/issue-2359".to_string()),
                worktree_path: Some(foreign),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        gwt_core::workspace_projection::record_workspace_work_event(&project_root, update)
            .expect("record conflicting container");
    }
    if fixture.conflicting_branch_only {
        let mut update = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            &work_id,
            now + chrono::Duration::milliseconds(1),
        );
        update.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/other-owner".to_string()),
                worktree_path: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        gwt_core::workspace_projection::record_workspace_work_event(&project_root, update)
            .expect("record branch-only conflict");
    }
    gwt_core::workspace_projection::record_workspace_work_event(
        &project_root,
        gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Done,
            &work_id,
            now + chrono::Duration::milliseconds(2),
        ),
    )
    .expect("settle projection-only Work");

    let tab = sample_project_tab("tab-1", "Repo", project_root, ProjectKind::Git, &[]);
    let runtime = sample_runtime(
        &temp_root.join(format!("{case_name}-runtime")),
        vec![tab],
        Some("tab-1"),
    );
    (runtime, repo, owner, work_id)
}

fn projection_only_continue_prepared_spec_recovery_case(divergent_candidate_branch: bool) {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_codex = write_fake_codex(temp.path());
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let case_name = "projection-only-prepared-spec-recovery";
    let operation_id = "projection-prepared-spec-recovery-operation";
    let runtime_root = temp.path().join(format!("{case_name}-runtime"));
    let (mut runtime, repo, owner, work_id) = projection_only_continue_runtime(
        temp.path(),
        case_name,
        ProjectionOnlyContinueFixture {
            work_owner: Some("SPEC #3248"),
            owner_number: 3248,
            owner_kind: gwt::cli::execution_state::ExecutionOwnerKind::Spec,
            agent_id: Some("Codex"),
            cross_worktree: false,
            missing_container: false,
            conflicting_owner: false,
            conflicting_container: false,
            conflicting_branch_only: false,
            conflicting_agent: false,
            legacy_flat_only: false,
        },
    );
    let project_root = runtime.tab("tab-1").expect("tab").project_root.clone();

    runtime.continue_work_events(
        &runtime.test_context(),
        "client-projection-prepared-spec",
        operation_id.to_string(),
        work_id.clone(),
        canvas_bounds(),
    );
    let pending = runtime
        .pending_continue_work
        .values()
        .find(|pending| pending.operation_id == operation_id)
        .expect("projection fallback must prepare candidate");
    let candidate_branch = if divergent_candidate_branch {
        "work/divergent-candidate"
    } else {
        "work/issue-2359"
    };
    let mut candidate = gwt_agent::Session::new(&repo, candidate_branch, gwt_agent::AgentId::Codex);
    candidate.id = pending.binding.session_id.clone();
    candidate.project_state_root = Some(project_root.clone());
    candidate.linked_issue_number = Some(owner.number);
    candidate
        .set_execution_binding(Some(pending.binding.clone()))
        .expect("bind Prepared candidate Session");
    candidate
        .save(&runtime.sessions_dir)
        .expect("save Prepared candidate Session");
    let mut predecessor =
        gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    predecessor.id = pending.predecessor_session_id.clone();
    predecessor.project_state_root = Some(project_root.clone());
    predecessor.repo_hash = detect_repo_hash(&repo).map(|value| value.to_string());
    predecessor.linked_issue_number = Some(owner.number);
    predecessor
        .save(&runtime.sessions_dir)
        .expect("save independently authenticated predecessor Session");
    assert!(runtime
        .sessions_dir
        .join(format!("{}.toml", pending.binding.session_id))
        .exists());
    assert_eq!(pending.owner, owner);
    drop(runtime);

    fs::write(
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root),
        b"{not-json",
    )
    .expect("corrupt Work projection for recovery path");
    fs::write(
        gwt_core::paths::gwt_repo_local_work_events_path(&project_root),
        b"{not-json\n",
    )
    .expect("corrupt Work events for recovery path");
    let tab = sample_project_tab("tab-retry", "Repo", project_root, ProjectKind::Git, &[]);
    let mut restarted = sample_runtime(&runtime_root, vec![tab], Some("tab-retry"));
    let work_items_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
    let work_events_path = gwt_core::paths::gwt_repo_local_work_events_path(&repo);
    let work_items_before = fs::read(&work_items_path).expect("read corrupt Work projection");
    let work_events_before = fs::read(&work_events_path).expect("read corrupt Work events");
    let authority_before =
        snapshot_optional_files(&exact_continue_authority_artifacts(&repo, owner));
    let session_paths = fs::read_dir(&restarted.sessions_dir)
        .expect("read recovery Sessions")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("toml"))
        .collect::<Vec<_>>();
    let sessions_before = snapshot_optional_files(&session_paths);
    let workspace_before = serde_json::to_vec(
        restarted
            .tab("tab-retry")
            .expect("retry tab")
            .workspace
            .persisted(),
    )
    .expect("serialize recovery Workspace before");

    let events = restarted.continue_work_events(
        &restarted.test_context(),
        "client-projection-prepared-spec-retry",
        operation_id.to_string(),
        work_id,
        canvas_bounds(),
    );

    if divergent_candidate_branch {
        assert!(
            events.iter().any(|event| matches!(
                &event.event,
                BackendEvent::ContinueWorkOutcome {
                    outcome: gwt::ContinueWorkOutcomeKind::ConflictUnknown,
                    retryable: true,
                    ..
                }
            )),
            "divergent correlated Session must conflict: {events:#?}"
        );
        assert!(restarted.pending_continue_work.is_empty());
        assert_eq!(
            serde_json::to_vec(
                restarted
                    .tab("tab-retry")
                    .expect("retry tab")
                    .workspace
                    .persisted(),
            )
            .expect("serialize recovery Workspace after"),
            workspace_before,
            "divergent branch must not materialize a pane",
        );
        assert_eq!(
            fs::read(&work_items_path).expect("read Work projection after"),
            work_items_before
        );
        assert_eq!(
            fs::read(&work_events_path).expect("read Work events after"),
            work_events_before
        );
        assert_optional_files_unchanged(&authority_before);
        assert_optional_files_unchanged(&sessions_before);
        return;
    }

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                error_code: Some(code),
                retryable: true,
                ..
            } if code == "continuation_reconciliation_required"
        )),
        "retry must find the exact Prepared Spec attempt: {events:?}"
    );
    assert!(events.iter().all(|event| !matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            error_code: Some(code),
            ..
        } if code == "work_state_unavailable"
    )));
    assert!(
        gwt::cli::execution_state::continuation_attempt_for_operation(&repo, owner, operation_id,)
            .expect("read Prepared attempt")
            .is_some()
    );
}

fn assert_projection_only_continue_rejection(
    case_name: &str,
    fixture: ProjectionOnlyContinueFixture,
    expected_code: &str,
) {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (mut runtime, repo, owner, work_id) =
        projection_only_continue_runtime(temp.path(), case_name, fixture);
    let project_root = runtime.tab("tab-1").expect("tab").project_root.clone();
    let work_items_path =
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root);
    let before_work_items = fs::read(&work_items_path).expect("read Work projection before");
    let before_work_events = tracked_work_event_store_snapshot(&project_root);
    let before_workspace =
        serde_json::to_vec(runtime.tab("tab-1").expect("tab").workspace.persisted())
            .expect("serialize Workspace before");
    let before_sessions = fs::read_dir(&runtime.sessions_dir)
        .expect("read Sessions before")
        .map(|entry| entry.expect("Session entry").file_name())
        .collect::<Vec<_>>();
    let before_ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read ledger before")
        .expect("ledger before");

    let events = runtime.continue_work_events(
        &runtime.test_context(),
        "client-invalid-projection",
        format!("operation-{case_name}"),
        work_id,
        canvas_bounds(),
    );

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                outcome: gwt::ContinueWorkOutcomeKind::Failed,
                error_code: Some(code),
                ..
            } if code == expected_code
        )),
        "{case_name} must fail with {expected_code}"
    );
    assert!(runtime.pending_continue_work.is_empty(), "{case_name}");
    assert!(
        runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .persisted()
            .windows
            .is_empty(),
        "{case_name} must not create a pane"
    );
    let after_ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read unchanged ledger")
        .expect("unchanged ledger");
    assert_eq!(after_ledger, before_ledger, "{case_name} ledger changed");
    assert_eq!(
        fs::read(&work_items_path).expect("read Work projection after"),
        before_work_items,
        "{case_name} Work projection changed"
    );
    assert_eq!(
        tracked_work_event_store_snapshot(&project_root),
        before_work_events,
        "{case_name} Work event log changed"
    );
    assert_eq!(
        serde_json::to_vec(runtime.tab("tab-1").expect("tab").workspace.persisted(),)
            .expect("serialize Workspace after"),
        before_workspace,
        "{case_name} Workspace changed"
    );
    assert_eq!(
        fs::read_dir(&runtime.sessions_dir)
            .expect("read Sessions after")
            .map(|entry| entry.expect("Session entry").file_name())
            .collect::<Vec<_>>(),
        before_sessions,
        "{case_name} Session files changed"
    );
}

fn continue_work_activated_successor_recovery_case(
    capability_generation: u64,
    mutate_candidate_after_repair: bool,
    mutate_candidate_before_work_commit: bool,
    same_generation_takeover: bool,
    substitute_candidate_agent: bool,
    substitute_live_agent: bool,
    candidate_only_durable_fallback: bool,
) {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    run_git(
        &repo,
        &["symbolic-ref", "HEAD", "refs/heads/work/issue-2359"],
    );

    let predecessor_session_id = "response-loss-predecessor";
    let candidate_session_id = "response-loss-candidate";
    let work_id = if same_generation_takeover {
        "work-takeover-response-loss"
    } else {
        "work-response-loss"
    };
    let operation_id = if same_generation_takeover {
        "continue-takeover-response-loss-operation"
    } else {
        "continue-response-loss-operation"
    };
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        predecessor_session_id,
        "gwt-execute",
        false,
    )
    .expect("materialize predecessor");
    if !same_generation_takeover {
        assert!(matches!(
            gwt::cli::execution_state::settle(
                &repo,
                predecessor_session_id,
                gwt::cli::execution_state::ExecutionSettlement::Completed,
            )
            .expect("settle predecessor"),
            gwt::cli::execution_state::SettleResult::Settled(_)
        ));
    }
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        if same_generation_takeover {
            gwt::cli::execution_state::LegacyActiveDisposition::Live
        } else {
            gwt::cli::execution_state::LegacyActiveDisposition::Unknown
        },
    )
    .expect("import predecessor");
    let predecessor_binding = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read predecessor binding")
        .expect("predecessor binding");
    let now = chrono::Utc::now();
    let (execution, planned) = if same_generation_takeover {
        let request = gwt::cli::execution_state::GenerationTakeoverRequest {
            operation_id: operation_id.to_string(),
            principal_id: "gwt-host-continuation".to_string(),
            work_id: Some(work_id.to_string()),
            source: Some("continue-work:resume".to_string()),
            from_session_id: predecessor_session_id.to_string(),
            to_session_id: candidate_session_id.to_string(),
            reason: "continue-work-stale-takeover: test owner is stale".to_string(),
            requested_at: now,
        };
        gwt::cli::execution_state::prepare_generation_takeover(&repo, owner, &request)
            .expect("prepare takeover");
        let planned = gwt::cli::execution_state::prepared_generation_takeover_execution_binding(
            &repo, owner, &request,
        )
        .expect("derive takeover binding");
        (PendingContinueWorkExecution::Takeover(request), planned)
    } else {
        let request = gwt::cli::execution_state::SuccessorRequest {
            operation_id: operation_id.to_string(),
            principal_id: "gwt-host-continuation".to_string(),
            work_id: Some(work_id.to_string()),
            source: "continue-work:resume".to_string(),
            session_binding_id: "response-loss-binding".to_string(),
            initial_session_id: candidate_session_id.to_string(),
            entrypoint: "gwt-execute".to_string(),
            requested_at: now,
        };
        gwt::cli::execution_state::prepare_successor(&repo, owner, &request)
            .expect("prepare successor");
        let planned =
            gwt::cli::execution_state::prepared_successor_execution_binding(&repo, owner, &request)
                .expect("derive successor binding");
        (PendingContinueWorkExecution::Successor(request), planned)
    };

    let mut current =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(repo.clone());
    current.id = work_id.to_string();
    current.title = "Response-loss Work".to_string();
    gwt_core::workspace_projection::save_workspace_projection(&repo, &current)
        .expect("save current Work projection");
    let mut start = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id,
        now,
    );
    start.title = Some("Response-loss Work".to_string());
    start.owner = Some("Issue #2359".to_string());
    start.agent_session_id = Some(predecessor_session_id.to_string());
    start.agent_id = Some("Codex".to_string());
    start.display_name = Some("Codex".to_string());
    start.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-2359".to_string()),
            worktree_path: Some(repo.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, start).expect("record Work");
    if !same_generation_takeover {
        gwt_core::workspace_projection::record_workspace_work_event(
            &repo,
            gwt_core::workspace_projection::WorkEvent::new(
                gwt_core::workspace_projection::WorkEventKind::Done,
                work_id,
                now + chrono::Duration::seconds(1),
            ),
        )
        .expect("settle Work");
    }

    let runtime_root = temp.path().join(".gwt");
    let runtime = sample_runtime(&runtime_root, Vec::new(), None);
    let mut predecessor =
        gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    predecessor.id = predecessor_session_id.to_string();
    predecessor.project_state_root = Some(repo.clone());
    predecessor.repo_hash = detect_repo_hash(&repo).map(|value| value.to_string());
    predecessor.linked_issue_number = Some(owner.number);
    predecessor.agent_session_id = Some("provider-conversation".to_string());
    predecessor
        .save(&runtime.sessions_dir)
        .expect("save predecessor Session");
    let candidate_binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: candidate_session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity: planned.clone(),
        capability_generation,
    };
    let candidate_agent_id = if substitute_candidate_agent {
        gwt_agent::AgentId::Custom("review-bot".to_string())
    } else {
        gwt_agent::AgentId::Codex
    };
    let mut candidate = gwt_agent::Session::new(&repo, "work/issue-2359", candidate_agent_id);
    candidate.id = candidate_session_id.to_string();
    candidate.project_state_root = Some(repo.clone());
    candidate.repo_hash = Some(candidate_binding.repo_hash.clone());
    candidate.linked_issue_number = Some(owner.number);
    candidate
        .set_execution_binding(Some(candidate_binding))
        .expect("bind candidate");
    candidate
        .save(&runtime.sessions_dir)
        .expect("save candidate Session");
    let mut active = sample_active_agent_session("tab-1", "tab-1::candidate");
    active.session_id = candidate_session_id.to_string();
    active.branch_name = "work/issue-2359".to_string();
    active.worktree_path = repo.clone();
    active.agent_project_root = repo.display().to_string();
    let resume_context = WorkspaceResumeContext {
        title: Some("Response-loss Work".to_string()),
        owner: Some("Issue #2359".to_string()),
        summary: None,
        next_action: None,
    };
    let live_session_ids = HashSet::from([candidate_session_id.to_string()]);
    let transaction = gwt_core::workspace_projection::transact_workspace_state_with_commit(
        &repo,
        operation_id,
        |projection, work_items, _| {
            let matching = work_items
                .work_items
                .iter()
                .filter(|item| item.id == work_id)
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 1);
            let event = super::workspace::apply_workspace_launch_transition(
                projection,
                &active,
                super::workspace::WorkspaceLaunchTransition {
                    work_id: Some(work_id.to_string()),
                    base_branch: None,
                    linked_issue_number: Some(owner.number),
                    canonical_owner: Some(owner),
                    resume_context: Some(&resume_context),
                    kind: WorkspaceLaunchProjectionKind::Resume {
                        created_by_start_work: true,
                    },
                    live_session_ids: &live_session_ids,
                    now: now + chrono::Duration::seconds(2),
                },
            );
            Ok(((), vec![event]))
        },
        || {
            match &execution {
                PendingContinueWorkExecution::Successor(request) => {
                    gwt::cli::execution_state::activate_successor(&repo, owner, request)
                        .expect("activate durable successor");
                }
                PendingContinueWorkExecution::Takeover(request) => {
                    gwt::cli::execution_state::activate_generation_takeover(&repo, owner, request)
                        .expect("activate durable takeover");
                }
            }
            Err(gwt_core::error::GwtError::Other(
                "simulated response loss after generation activation".to_string(),
            ))
        },
    );
    assert!(transaction.is_err());
    if same_generation_takeover {
        assert_eq!(
            gwt::cli::execution_state::generation_takeover_attempt_for_operation(
                &repo,
                owner,
                operation_id,
            )
            .expect("read takeover attempt")
            .expect("takeover attempt")
            .status,
            gwt::cli::execution_state::GenerationTakeoverAttemptStatus::Activated
        );
    } else {
        assert_eq!(
            gwt::cli::execution_state::continuation_attempt_for_operation(
                &repo,
                owner,
                operation_id,
            )
            .expect("read attempt")
            .expect("attempt")
            .status,
            gwt::cli::execution_state::ContinuationAttemptStatus::Activated
        );
    }

    if candidate_only_durable_fallback {
        fs::remove_file(
            runtime
                .sessions_dir
                .join(format!("{predecessor_session_id}.toml")),
        )
        .expect("remove predecessor Session before candidate-only recovery");
        fs::write(
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo),
            b"{not-json",
        )
        .expect("corrupt Work projection before candidate-only recovery");
        fs::write(
            gwt_core::paths::gwt_repo_local_work_events_path(&repo),
            b"{not-json\n",
        )
        .expect("corrupt Work events before candidate-only recovery");
    }

    let candidate_path = runtime
        .sessions_dir
        .join(format!("{candidate_session_id}.toml"));
    let (retry_window_id, retry_preset, retry_status) = if substitute_live_agent {
        (
            "candidate",
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        )
    } else {
        (
            "shell-retry",
            WindowPreset::Shell,
            WindowProcessStatus::Ready,
        )
    };
    let tab = sample_project_tab_with_window_at(
        "tab-retry",
        retry_window_id,
        repo.clone(),
        retry_preset,
        retry_status,
    );
    let mut restarted_runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-retry"));
    if substitute_live_agent {
        let window_id = combined_window_id("tab-retry", retry_window_id);
        let mut substituted = sample_active_agent_session("tab-retry", &window_id);
        substituted.session_id = candidate_session_id.to_string();
        substituted.agent_id = "review-bot".to_string();
        substituted.display_name = "Review Bot".to_string();
        substituted.branch_name = "work/issue-2359".to_string();
        substituted.worktree_path = repo.clone();
        substituted.agent_project_root = repo.display().to_string();
        restarted_runtime
            .active_agent_sessions
            .insert(window_id, substituted);
    }
    let project_root = repo.clone();
    let work_items_path =
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root);
    let work_items_before = fs::read(&work_items_path).expect("read Work projection before retry");
    let work_events_before = tracked_work_event_store_snapshot(&project_root);
    let authority_paths = exact_continue_authority_artifacts(&repo, owner);
    let authority_before = snapshot_optional_files(&authority_paths);
    let session_before = fs::read(&candidate_path).expect("read candidate Session before retry");
    let workspace_before = serde_json::to_vec(
        restarted_runtime
            .tab("tab-retry")
            .expect("retry tab")
            .workspace
            .persisted(),
    )
    .expect("serialize retry Workspace before");
    let replacement_bytes = std::rc::Rc::new(std::cell::RefCell::new(None));
    if mutate_candidate_after_repair {
        let candidate_path = candidate_path.clone();
        let replacement_bytes_for_hook = std::rc::Rc::clone(&replacement_bytes);
        super::continuation::set_durable_continue_work_post_repair_hook_for_test(Box::new(
            move || {
                let mut replacement =
                    gwt_agent::Session::load(&candidate_path).expect("load race replacement");
                replacement
                    .execution_binding
                    .as_mut()
                    .expect("race replacement binding")
                    .capability_generation += 1;
                replacement
                    .save(
                        candidate_path
                            .parent()
                            .expect("candidate Session directory"),
                    )
                    .expect("save race replacement");
                *replacement_bytes_for_hook.borrow_mut() =
                    Some(fs::read(&candidate_path).expect("read race replacement bytes"));
            },
        ));
    }
    if mutate_candidate_before_work_commit {
        let candidate_path = candidate_path.clone();
        let replacement_bytes_for_hook = std::rc::Rc::clone(&replacement_bytes);
        super::continuation::set_durable_continue_work_pre_work_commit_hook_for_test(Box::new(
            move || {
                let mut replacement =
                    gwt_agent::Session::load(&candidate_path).expect("load late race replacement");
                replacement
                    .execution_binding
                    .as_mut()
                    .expect("late race replacement binding")
                    .capability_generation += 1;
                replacement
                    .save(
                        candidate_path
                            .parent()
                            .expect("candidate Session directory"),
                    )
                    .expect("save late race replacement");
                *replacement_bytes_for_hook.borrow_mut() =
                    Some(fs::read(&candidate_path).expect("read late race replacement bytes"));
            },
        ));
    }
    if capability_generation == 1
        && !mutate_candidate_after_repair
        && !mutate_candidate_before_work_commit
        && !same_generation_takeover
        && !substitute_candidate_agent
        && !substitute_live_agent
        && !candidate_only_durable_fallback
    {
        let current_path =
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root);
        let lock_path = gwt_core::workspace_projection::external_workspace_operation_lock_path(
            &current_path,
            &work_items_path,
            operation_id,
        );
        let legacy_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .expect("open staged operation lock");
        legacy_lock
            .try_lock_exclusive()
            .expect("hold operation lock without holder metadata");
        let holder_path = lock_path.with_extension("lock.holder.json");
        match fs::remove_file(holder_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("remove operation holder metadata: {error}"),
        }
        let busy_events = restarted_runtime.continue_work_events(
            &restarted_runtime.test_context(),
            "client-busy-retry",
            operation_id.to_string(),
            work_id.to_string(),
            canvas_bounds(),
        );
        let message = busy_events
            .iter()
            .find_map(|event| match &event.event {
                BackendEvent::ContinueWorkOutcome {
                    error_code: Some(code),
                    message: Some(message),
                    retryable: true,
                    ..
                } if code == "continuation_reconciliation_required" => Some(message),
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!("operation contention must remain retryable: {busy_events:#?}")
            });
        assert!(message.contains(operation_id), "{message}");
        assert!(
            message.contains(&lock_path.display().to_string()),
            "{message}"
        );
        assert!(
            message.contains("holder_unknown_reason=file_absent"),
            "{message}"
        );
        assert!(
            message.contains("OS lock") && message.contains("release"),
            "{message}"
        );
        assert!(
            message.contains("same operation") && message.contains("retry"),
            "{message}"
        );
        FileExt::unlock(&legacy_lock).expect("release operation lock before retry");
    }
    // A parallel agent probe can inherit the operation flock until its exec.
    // Retry only that transient Busy response; retain all recovery assertions.
    let retry_deadline = Instant::now() + Duration::from_secs(30);
    let events = loop {
        let events = restarted_runtime.continue_work_events(
            &restarted_runtime.test_context(),
            "client-retry",
            operation_id.to_string(),
            work_id.to_string(),
            canvas_bounds(),
        );
        let busy = events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                outcome: gwt::ContinueWorkOutcomeKind::Failed,
                message: Some(message),
                error_code: Some(code),
                retryable: true,
                ..
            } if code == "continuation_reconciliation_required"
                && message.starts_with("The committed continuation Work transaction is still being reconciled.")
        ));
        if !busy {
            break events;
        }
        assert!(
            Instant::now() < retry_deadline,
            "operation lock remained Busy: {events:#?}"
        );
        thread::sleep(Duration::from_millis(100));
    };
    if capability_generation != 1
        || mutate_candidate_after_repair
        || mutate_candidate_before_work_commit
        || substitute_candidate_agent
        || substitute_live_agent
        || candidate_only_durable_fallback
    {
        assert!(
            events.iter().any(|event| matches!(
                &event.event,
                BackendEvent::ContinueWorkOutcome {
                    outcome: gwt::ContinueWorkOutcomeKind::ConflictUnknown,
                    retryable: true,
                    ..
                }
            )),
            "exact candidate mismatch must conflict: {events:#?}"
        );
        assert!(restarted_runtime.pending_continue_work.is_empty());
        assert_eq!(
            serde_json::to_vec(
                restarted_runtime
                    .tab("tab-retry")
                    .expect("retry tab")
                    .workspace
                    .persisted(),
            )
            .expect("serialize retry Workspace after"),
            workspace_before,
            "candidate mismatch must not materialize a pane",
        );
        assert_eq!(
            fs::read(&work_items_path).expect("read Work projection after retry"),
            work_items_before,
        );
        assert_eq!(
            tracked_work_event_store_snapshot(&project_root),
            work_events_before,
        );
        assert_optional_files_unchanged(&authority_before);
        let expected_session = replacement_bytes.borrow().clone().unwrap_or(session_before);
        assert_eq!(
            fs::read(&candidate_path).expect("read retained candidate Session"),
            expected_session,
            "the exact replacement Session must be retained unchanged",
        );
        return;
    }
    assert!(events.iter().all(|event| !matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
            ..
        }
    )));
    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                error_code: Some(code),
                retryable: true,
                ..
            } if code == "continuation_reconciliation_required"
        )),
        "unexpected retry outcome: {events:#?}"
    );
    let work = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load repaired Work")
        .expect("repaired Work")
        .work_items
        .into_iter()
        .find(|item| item.id == work_id)
        .expect("continued Work");
    assert_eq!(
        work.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Active
    );
    assert!(work
        .agents
        .iter()
        .any(|agent| agent.session_id == candidate_session_id));
    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read repaired ledger")
        .expect("repaired ledger");
    assert_eq!(ledger.generations.len(), 2);
    assert_eq!(
        ledger.current_generation_id, planned.generation_id,
        "repair must not append another generation"
    );
    assert!(
        !gwt::cli::execution_state::current_active_execution_binding_matches(
            &repo,
            owner,
            predecessor_session_id,
            &predecessor_binding,
        )
        .expect("verify predecessor fence")
    );
}

/// #3426: drive an owner-kind-healed Continue work all the way through
/// authenticated SessionStart. `work_owner_text` is what the Work projection
/// declares; the trusted execution authority is always `spec/2359`.
fn continue_work_heal_session_start_events(
    temp_root: &Path,
    case: &str,
    work_owner_text: &str,
) -> (AppRuntime, PathBuf, Vec<OutboundEvent>, String) {
    let repo = temp_root.join(format!("repo-{case}"));
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    run_git(
        &repo,
        &["symbolic-ref", "HEAD", "refs/heads/work/issue-2359"],
    );

    let predecessor_session_id = "heal-predecessor-session";
    let candidate_session_id = "heal-candidate-session";
    let selected_work_id = "work-healed";
    let operation_id = "continue-op-heal";
    let readiness_nonce = "continue-ready-heal";
    // The trusted authority is a SPEC owner (the gwt-spec labeled Issue).
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Spec,
        number: 2359,
    };
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        predecessor_session_id,
        "gwt-execute",
        false,
    )
    .expect("materialize predecessor execution");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            predecessor_session_id,
            gwt::cli::execution_state::ExecutionSettlement::Completed,
        )
        .expect("settle predecessor"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
    )
    .expect("import completed predecessor");
    let predecessor_binding = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read predecessor binding")
        .expect("predecessor binding");

    let now = chrono::Utc::now();
    let request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: operation_id.to_string(),
        principal_id: "gwt-host-continuation".to_string(),
        work_id: Some(selected_work_id.to_string()),
        source: "continue-work:resume".to_string(),
        session_binding_id: "binding-heal-candidate".to_string(),
        initial_session_id: candidate_session_id.to_string(),
        entrypoint: "gwt-execute".to_string(),
        requested_at: now,
    };
    gwt::cli::execution_state::prepare_successor(&repo, owner, &request)
        .expect("prepare successor");
    let planned_identity =
        gwt::cli::execution_state::prepared_successor_execution_binding(&repo, owner, &request)
            .expect("derive Prepared binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: candidate_session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity: planned_identity,
        capability_generation: 1,
    };

    // The Work projection still carries the presentation-derived owner that
    // diverged from the trusted authority — the #3426 (A) stuck state.
    let mut start = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        selected_work_id,
        now,
    );
    start.title = Some("Healed Work".to_string());
    start.owner = Some(work_owner_text.to_string());
    start.agent_session_id = Some(predecessor_session_id.to_string());
    start.agent_id = Some("Codex".to_string());
    start.display_name = Some("Codex".to_string());
    start.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-2359".to_string()),
            worktree_path: Some(repo.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&repo, start)
        .expect("record selected Work");
    gwt_core::workspace_projection::record_workspace_work_event(
        &repo,
        gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Done,
            selected_work_id,
            now + chrono::Duration::seconds(1),
        ),
    )
    .expect("complete selected Work");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    // The Prepared-binding probe reads the canonical sessions directory under
    // the scoped gwt home, so the runtime root must be that same `.gwt`.
    let mut runtime = sample_runtime(&temp_root.join(".gwt"), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut active = sample_active_agent_session("tab-1", &window_id);
    active.session_id = candidate_session_id.to_string();
    active.branch_name = "work/issue-2359".to_string();
    active.worktree_path = repo.clone();
    active.agent_project_root = repo.display().to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), active);

    let mut candidate_session =
        gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    candidate_session.id = candidate_session_id.to_string();
    candidate_session.project_state_root = Some(repo.clone());
    candidate_session.linked_issue_number = Some(owner.number);
    candidate_session
        .set_execution_binding(Some(binding.clone()))
        .expect("bind candidate Session");
    candidate_session
        .save(&runtime.sessions_dir)
        .expect("save candidate Session");

    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43323/internal/hook-live",
        "ws://127.0.0.1:43324/ws",
        "ws://127.0.0.1:43323/internal/pane-ws",
    );
    let capability = issuer
        .issue_prepared(&repo, candidate_session_id, binding.clone())
        .expect("issue Prepared capability");
    runtime.agent_capability_issuer = Some(issuer);
    runtime
        .agent_capability_tokens
        .insert(window_id.clone(), capability.token);
    runtime.pending_continue_work.insert(
        window_id.clone(),
        PendingContinueWork {
            client_id: "client-heal".to_string(),
            operation_id: operation_id.to_string(),
            work_id: selected_work_id.to_string(),
            project_root: repo.clone(),
            worktree_path: repo.clone(),
            owner,
            work_branch: "work/issue-2359".to_string(),
            work_agent_id: gwt_agent::AgentId::Codex,
            work_agent_session_id: Some(predecessor_session_id.to_string()),
            execution: PendingContinueWorkExecution::Successor(request),
            binding,
            readiness_nonce: readiness_nonce.to_string(),
            outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
            resume_context: WorkspaceResumeContext {
                title: Some("Healed Work".to_string()),
                owner: Some(work_owner_text.to_string()),
                summary: None,
                next_action: None,
            },
            predecessor_session_id: predecessor_session_id.to_string(),
            predecessor_binding,
        },
    );

    let events = runtime.finalize_continue_work_session_start(&window_id, Some(readiness_nonce));
    (runtime, repo, events, selected_work_id.to_string())
}

struct PendingFreshExecutionFixture {
    runtime: AppRuntime,
    repo: PathBuf,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    window_id: String,
    operation_id: String,
    candidate_session_id: String,
    predecessor_binding: gwt_agent::ExecutionBindingIdentity,
    binding: gwt_agent::SessionExecutionBinding,
    issuer: crate::embedded_server::AgentCapabilityIssuer,
    token: String,
}

fn replace_current_generation_authority_with_owner(
    worktree: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    session_id: &str,
) {
    let trusted_dir = gwt::cli::trusted_store::trusted_dir_for_worktree(worktree)
        .expect("trusted worktree directory");
    for path in [
        trusted_dir.join("execution-control.json"),
        trusted_dir.join("execution-generation-pointer.json"),
        worktree.join(gwt::cli::execution_state::EXECUTION_CONTROL_STATE_RELATIVE),
        worktree.join(gwt::cli::execution_state::EXECUTION_GENERATION_POINTER_STATE_RELATIVE),
    ] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("remove superseded authority artifact: {error}"),
        }
    }
    gwt::cli::execution_state::materialize_at_launch(
        worktree,
        owner.kind,
        owner.number,
        session_id,
        &format!("$gwt-execute #{}", owner.number),
        false,
    )
    .expect("materialize superseding owner");
    gwt::cli::execution_state::ensure_generation_ledger(
        worktree,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize superseding owner ledger");
}

fn current_generation_authority_artifacts(worktree: &Path) -> Vec<PathBuf> {
    let trusted_dir = gwt::cli::trusted_store::trusted_dir_for_worktree(worktree)
        .expect("trusted worktree directory");
    vec![
        trusted_dir.join("execution-control.json"),
        trusted_dir.join("execution-generation-pointer.json"),
        worktree.join(gwt::cli::execution_state::EXECUTION_CONTROL_STATE_RELATIVE),
        worktree.join(gwt::cli::execution_state::EXECUTION_GENERATION_POINTER_STATE_RELATIVE),
    ]
}

fn exact_continue_authority_artifacts(
    worktree: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
) -> Vec<PathBuf> {
    let trusted_dir = gwt::cli::trusted_store::trusted_dir_for_worktree(worktree)
        .expect("trusted worktree directory");
    let mut paths = current_generation_authority_artifacts(worktree);
    paths.push(
        trusted_dir
            .parent()
            .expect("trusted repository directory")
            .join("execution-owners")
            .join(format!("owner-{}", owner.number))
            .join("generation-ledger.json"),
    );
    paths
}

fn snapshot_optional_files(paths: &[PathBuf]) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    paths
        .iter()
        .map(|path| {
            let bytes = match fs::read(path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => panic!("read snapshot {}: {error}", path.display()),
            };
            (path.clone(), bytes)
        })
        .collect()
}

fn assert_optional_files_unchanged(snapshot: &[(PathBuf, Option<Vec<u8>>)]) {
    for (path, expected) in snapshot {
        let actual = match fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("read snapshot readback {}: {error}", path.display()),
        };
        assert_eq!(actual, *expected, "unexpected mutation: {}", path.display());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrackedWorkEventStoreSnapshot {
    legacy: Option<Vec<u8>>,
    shards: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrackedWorkspaceWorkStoreSnapshot {
    work_items: Option<Vec<u8>>,
    events: TrackedWorkEventStoreSnapshot,
}

fn tracked_work_event_store_snapshot(repo: &Path) -> TrackedWorkEventStoreSnapshot {
    let legacy = fs::read(gwt_core::paths::gwt_repo_local_work_events_path(repo)).ok();
    let events_dir = gwt_core::paths::gwt_repo_local_work_events_dir(repo);
    let mut shards = BTreeMap::new();
    match fs::read_dir(&events_dir) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.expect("read tracked Work event shard entry");
                let name = entry
                    .file_name()
                    .into_string()
                    .expect("UTF-8 tracked Work event shard name");
                let file_type = entry
                    .file_type()
                    .expect("read tracked Work event shard entry type");
                if file_type.is_file() {
                    shards.insert(
                        name,
                        fs::read(entry.path()).expect("read tracked Work event shard"),
                    );
                } else if file_type.is_dir() {
                    for bucket_entry in
                        fs::read_dir(entry.path()).expect("read Work event shard bucket")
                    {
                        let bucket_entry =
                            bucket_entry.expect("read bucketed Work event shard entry");
                        let bucket_name = bucket_entry
                            .file_name()
                            .into_string()
                            .expect("UTF-8 bucketed Work event shard name");
                        assert!(
                            bucket_entry
                                .file_type()
                                .expect("read bucketed Work event shard entry type")
                                .is_file(),
                            "tracked Work event bucket entries must be files: {}",
                            bucket_entry.path().display()
                        );
                        shards.insert(
                            format!("{name}/{bucket_name}"),
                            fs::read(bucket_entry.path()).expect("read bucketed Work event shard"),
                        );
                    }
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("read {}: {error}", events_dir.display()),
    }
    TrackedWorkEventStoreSnapshot { legacy, shards }
}

fn tracked_workspace_work_store_snapshot(repo: &Path) -> TrackedWorkspaceWorkStoreSnapshot {
    TrackedWorkspaceWorkStoreSnapshot {
        work_items: fs::read(gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(repo))
            .ok(),
        events: tracked_work_event_store_snapshot(repo),
    }
}

fn assert_tracked_workspace_work_store_unchanged(
    repo: &Path,
    expected: &TrackedWorkspaceWorkStoreSnapshot,
) {
    assert_eq!(
        tracked_workspace_work_store_snapshot(repo),
        *expected,
        "unexpected Work store mutation under {}",
        repo.display(),
    );
}

fn load_tracked_work_events(repo: &Path) -> Vec<gwt_core::workspace_projection::WorkEvent> {
    let snapshot = tracked_work_event_store_snapshot(repo);
    let mut by_id = BTreeMap::<String, gwt_core::workspace_projection::WorkEvent>::new();
    let sources = snapshot
        .legacy
        .iter()
        .chain(snapshot.shards.values())
        .flat_map(|bytes| bytes.split(|byte| *byte == b'\n'))
        .filter(|line| !line.iter().all(u8::is_ascii_whitespace));
    for line in sources {
        let event = serde_json::from_slice::<gwt_core::workspace_projection::WorkEvent>(line)
            .expect("tracked Work event JSON");
        if let Some(existing) = by_id.insert(event.id.clone(), event.clone()) {
            assert_eq!(
                existing, event,
                "duplicate tracked Work event identity must be semantic equivalent"
            );
        }
    }
    let mut events = by_id.into_values().collect::<Vec<_>>();
    events.sort_by(|left, right| {
        left.updated_at
            .cmp(&right.updated_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    events
}

fn pending_fresh_execution_fixture(
    temp_root: &Path,
    operation_id: &str,
) -> PendingFreshExecutionFixture {
    pending_fresh_execution_fixture_with_owner_kind(
        temp_root,
        operation_id,
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
    )
}

fn pending_fresh_execution_fixture_with_owner_kind(
    temp_root: &Path,
    operation_id: &str,
    owner_kind: gwt::cli::execution_state::ExecutionOwnerKind,
) -> PendingFreshExecutionFixture {
    pending_fresh_execution_fixture_with_predecessor_status(
        temp_root,
        operation_id,
        owner_kind,
        false,
    )
}

fn pending_fresh_execution_fixture_with_predecessor_status(
    temp_root: &Path,
    operation_id: &str,
    owner_kind: gwt::cli::execution_state::ExecutionOwnerKind,
    completed: bool,
) -> PendingFreshExecutionFixture {
    let repo = temp_root.join(format!("repo-{operation_id}"));
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: owner_kind,
        number: 2359,
    };
    let predecessor_session_id = format!("blocked-{operation_id}");
    let candidate_session_id = format!("candidate-{operation_id}");
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        &predecessor_session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize predecessor");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            &predecessor_session_id,
            if completed {
                gwt::cli::execution_state::ExecutionSettlement::Completed
            } else {
                gwt::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "legacy terminal blocker".to_string(),
                    missing_verification: Some("legacy evidence gap".to_string()),
                }
            },
        )
        .expect("settle Blocked predecessor"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
    )
    .expect("import Blocked predecessor");
    let predecessor_binding = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read predecessor binding")
        .expect("predecessor binding");
    let request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: operation_id.to_string(),
        principal_id: "gwt-host-launch".to_string(),
        work_id: None,
        source: if completed {
            gwt::cli::execution_state::MANUAL_COMPLETED_OWNER_LAUNCH_SOURCE
        } else {
            gwt::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE
        }
        .to_string(),
        session_binding_id: format!("binding-{operation_id}"),
        initial_session_id: candidate_session_id.clone(),
        entrypoint: "$gwt-execute #2359".to_string(),
        requested_at: Utc::now(),
    };
    if completed {
        gwt::cli::execution_state::prepare_successor(&repo, owner, &request)
    } else {
        gwt::cli::execution_state::prepare_fresh_linked_owner_launch_successor(
            &repo, owner, &request,
        )
    }
    .expect("prepare fresh successor");
    let planned =
        gwt::cli::execution_state::prepared_successor_execution_binding(&repo, owner, &request)
            .expect("derive prepared binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: candidate_session_id.clone(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity: planned,
        capability_generation: 1,
    };
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let runtime_root = temp_root.join(".gwt");
    let mut runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-1"));
    let (spawner, _) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut candidate =
        gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    candidate.id = candidate_session_id.clone();
    candidate.project_state_root = Some(repo.clone());
    candidate.linked_issue_number = Some(owner.number);
    candidate
        .set_execution_binding(Some(binding.clone()))
        .expect("bind candidate Session");
    candidate
        .save(&runtime.sessions_dir)
        .expect("save candidate Session");
    let session_identity = gwt_agent::SessionExecutionIdentity::from_session(&candidate)
        .unwrap()
        .unwrap();
    persist_durable_launch_recovery(
        &runtime.sessions_dir,
        DurableLaunchRecoveryKind::FreshSuccessor {
            operation_id: operation_id.to_string(),
        },
        &candidate_session_id,
        &repo,
        &repo,
        owner,
        Some(&binding),
        Some(&gwt_agent::AgentId::Codex),
    )
    .expect("persist fresh-launch recovery receipt");
    runtime.launch_wizard_cache = LaunchWizardMemoryCache::load(&runtime.sessions_dir);
    let mut active = sample_active_agent_session("tab-1", &window_id);
    active.session_id = candidate_session_id.clone();
    active.branch_name = "work/issue-2359".to_string();
    active.worktree_path = repo.clone();
    active.agent_project_root = repo.display().to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), active);
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:45132/internal/hook-live",
        "ws://127.0.0.1:46242/ws",
        "ws://127.0.0.1:45132/internal/pane-ws",
    );
    let capability = issuer
        .issue_prepared(&repo, &candidate_session_id, binding.clone())
        .expect("issue Prepared capability");
    let token = capability.token.clone();
    runtime.agent_capability_issuer = Some(issuer.clone());
    runtime
        .agent_capability_tokens
        .insert(window_id.clone(), capability.token);
    runtime.pending_fresh_execution_launches.insert(
        window_id.clone(),
        PendingFreshExecutionLaunch {
            operation_id: operation_id.to_string(),
            project_root: repo.clone(),
            worktree_path: repo.clone(),
            owner,
            request,
            binding: binding.clone(),
            session_identity,
            readiness_nonce: format!("readiness-{operation_id}"),
            predecessor_binding: predecessor_binding.clone(),
            base_branch: Some("origin/develop".to_string()),
            linked_issue_number: Some(owner.number),
            resume_context: None,
            launch_feedback_context: None,
        },
    );
    PendingFreshExecutionFixture {
        runtime,
        repo,
        owner,
        window_id,
        operation_id: operation_id.to_string(),
        candidate_session_id,
        predecessor_binding,
        binding,
        issuer,
        token,
    }
}

fn take_fresh_execution_finalization(
    runtime: &AppRuntime,
) -> super::continuation::FreshExecutionFinalization {
    let BlockingTaskSpawner::Queued(tasks) = &runtime.blocking_tasks else {
        panic!("fresh execution test needs a queued worker");
    };
    drain_queued_blocking_tasks(tasks);
    let AppEventProxy::Stub(events) = &runtime.proxy else {
        panic!("fresh execution test needs a stub proxy");
    };
    let mut events = events.lock().expect("fresh execution completions");
    let index = events
        .iter()
        .position(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::FreshExecutionFinalized(_)
            )
        })
        .expect("fresh execution worker completion");
    let event = events.remove(index);
    drop(events);
    match into_recorded_project_payload(event) {
        UserEvent::FreshExecutionFinalized(completion) => *completion,
        _ => unreachable!("matched fresh execution finalization"),
    }
}

fn commit_pending_fresh_execution(runtime: &mut AppRuntime) -> Vec<OutboundEvent> {
    let completion = take_fresh_execution_finalization(runtime);
    runtime.handle_fresh_execution_finalized(completion)
}

fn replace_fresh_candidate_session_incarnation(sessions_dir: &Path, session_id: &str) {
    gwt_agent::update_session(sessions_dir, session_id, |session| {
        let mut replacement = session
            .execution_binding
            .clone()
            .expect("replacement Session binding");
        replacement.capability_generation += 1;
        session
            .set_execution_binding(Some(replacement))
            .map_err(std::io::Error::other)
    })
    .expect("replace candidate Session incarnation before Work commit");
}

fn assert_pending_fresh_execution_was_rolled_back(fixture: &PendingFreshExecutionFixture) {
    assert!(
        !durable_launch_recovery_exists(
            &fixture.runtime.sessions_dir,
            &fixture.candidate_session_id,
        ),
        "an exactly rolled-back launch must remove its durable recovery receipt",
    );
    assert!(!fixture
        .runtime
        .pending_fresh_execution_launches
        .contains_key(&fixture.window_id));
    assert!(!fixture
        .runtime
        .active_agent_sessions
        .contains_key(&fixture.window_id));
    assert!(!fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id))
        .exists());
    assert!(
        fixture
            .runtime
            .launch_wizard_cache
            .session_by_id(&fixture.candidate_session_id)
            .is_none(),
        "an exactly rolled-back launch must evict its candidate Session from the in-memory cache",
    );
    assert!(!fixture
        .issuer
        .prepared_token_is_current(&fixture.token, &fixture.binding));
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
            .expect("read current binding"),
        Some(fixture.predecessor_binding.clone()),
    );
    let ledger = gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.owner)
        .expect("read rollback ledger")
        .expect("rollback ledger");
    assert_eq!(ledger.generations.len(), 1);
    assert_eq!(
        gwt::cli::execution_state::continuation_attempt_for_operation(
            &fixture.repo,
            fixture.owner,
            &fixture.operation_id,
        )
        .expect("read rollback attempt")
        .expect("rollback attempt")
        .status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Aborted,
    );
}

fn leave_fresh_execution_activated_before_projection_commit(
    fixture: &mut PendingFreshExecutionFixture,
) {
    fixture
        .issuer
        .promote_prepared(&fixture.token, &fixture.binding)
        .expect("promote Prepared capability before simulated commit response loss");
    let transaction = gwt_core::workspace_projection::transact_workspace_state_with_commit(
        &fixture.repo,
        &fixture.operation_id,
        |projection, work_items, _| {
            let pending = fixture
                .runtime
                .pending_fresh_execution_launches
                .get(&fixture.window_id)
                .expect("pending fresh launch");
            let active = fixture
                .runtime
                .active_agent_sessions
                .get(&fixture.window_id)
                .expect("active fresh candidate");
            let live_session_ids = HashSet::from([fixture.candidate_session_id.clone()]);
            let event = super::workspace::apply_workspace_launch_for_current_work(
                &fixture.repo,
                projection,
                work_items,
                active,
                super::workspace::WorkspaceLaunchTransition {
                    work_id: None,
                    base_branch: pending.base_branch.as_deref(),
                    linked_issue_number: pending.linked_issue_number,
                    canonical_owner: Some(pending.owner),
                    resume_context: pending.resume_context.as_ref(),
                    kind: WorkspaceLaunchProjectionKind::StartWork,
                    live_session_ids: &live_session_ids,
                    now: Utc::now(),
                },
            )?;
            Ok(((), vec![event]))
        },
        || {
            gwt::cli::execution_state::activate_successor(
                &fixture.repo,
                fixture.owner,
                &fixture
                    .runtime
                    .pending_fresh_execution_launches
                    .get(&fixture.window_id)
                    .expect("pending fresh launch")
                    .request,
            )
            .map_err(gwt_core::error::GwtError::Io)?;
            Err(gwt_core::error::GwtError::Other(
                "simulated response loss after ledger activation".to_string(),
            ))
        },
    );
    assert!(
        transaction.is_err(),
        "response loss must leave reconciliation work"
    );
    let trusted_dir = gwt::cli::trusted_store::trusted_dir_for_worktree(&fixture.repo)
        .expect("trusted worktree directory");
    for path in [
        trusted_dir.join("execution-control.json"),
        trusted_dir.join("execution-generation-pointer.json"),
        fixture
            .repo
            .join(gwt::cli::execution_state::EXECUTION_CONTROL_STATE_RELATIVE),
        fixture
            .repo
            .join(gwt::cli::execution_state::EXECUTION_GENERATION_POINTER_STATE_RELATIVE),
    ] {
        if path.exists() {
            fs::remove_file(&path).expect("remove partial activation projection artifact");
        }
    }
    assert!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner).is_err(),
        "the fixture must expose an Activated ledger with an incomplete projection/pointer"
    );
    assert!(durable_launch_recovery_exists(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
    ));
}

fn queued_fresh_execution_completion_fixture(
    temp_root: &Path,
    operation_id: &str,
) -> (
    PendingFreshExecutionFixture,
    Arc<Mutex<Vec<UserEvent>>>,
    BlockingTestTaskQueue,
    AgentLaunchCompletion,
    PendingFreshExecutionLaunch,
) {
    let mut fixture = pending_fresh_execution_fixture(temp_root, operation_id);
    let pending = fixture
        .runtime
        .pending_fresh_execution_launches
        .remove(&fixture.window_id)
        .expect("fresh candidate is only durable before worker handoff");
    fixture
        .runtime
        .active_agent_sessions
        .remove(&fixture.window_id);
    fixture
        .runtime
        .agent_capability_tokens
        .remove(&fixture.window_id);
    fixture
        .runtime
        .launch_wizard_cache
        .forget_session(&fixture.candidate_session_id);
    let (proxy, recorded) = AppEventProxy::stub();
    fixture.runtime.proxy = proxy;
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec!["/d".into(), "/s".into(), "/c".into(), "exit /b 0".into()],
        )
    } else {
        ("/bin/sh".to_string(), vec!["-c".into(), "exit 0".into()])
    };
    let mut completion = bound_runtime_launch_completion(
        &fixture.repo,
        &fixture.candidate_session_id,
        pending.session_identity.clone(),
        command,
        args,
    );
    completion.0.env.insert(
        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV.into(),
        fixture.token.clone(),
    );
    completion.0.env.insert(
        gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV.into(),
        pending.readiness_nonce.clone(),
    );
    completion.2 = "work/issue-2359".into();
    completion.6 = Some(fixture.owner.number);
    completion.7 = Some("origin/develop".into());
    completion.9 = gwt_agent::SessionMode::Normal;
    (fixture, recorded, tasks, completion, pending)
}

impl AppRuntime {
    fn finish_queued_delivery_acks(&mut self, tasks: &BlockingTestTaskQueue) -> Vec<OutboundEvent> {
        drain_queued_blocking_tasks(tasks);
        let AppEventProxy::Stub(recorded) = &self.proxy else {
            panic!("recording proxy required")
        };
        let recorded = recorded.clone();
        let mut outbound = Vec::new();
        loop {
            let acknowledged = {
                let mut events = recorded.lock().unwrap();
                events
                    .iter()
                    .position(|event| {
                        matches!(
                            recorded_project_payload(event),
                            UserEvent::IssueMonitorLaunchDeliveryAcknowledged(_)
                        )
                    })
                    .map(|index| into_recorded_project_payload(events.remove(index)))
            };
            let Some(UserEvent::IssueMonitorLaunchDeliveryAcknowledged(ack)) = acknowledged else {
                break;
            };
            outbound.extend(self.handle_issue_monitor_launch_delivery_ack(*ack));
        }
        outbound
    }

    fn issue_monitor_launch_succeeded_and_drain(
        &mut self,
        root: &Path,
        issue_number: u64,
        window_id: &str,
    ) -> Vec<OutboundEvent> {
        let (spawner, tasks) = BlockingTaskSpawner::queued();
        let previous = std::mem::replace(&mut self.blocking_tasks, spawner);
        let mut events = self.issue_monitor_launch_succeeded_events(root, issue_number, window_id);
        events.extend(self.finish_queued_delivery_acks(&tasks));
        self.blocking_tasks = previous;
        events
    }

    pub(crate) fn update_drain_tick_events_and_drain_at(
        &mut self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Vec<OutboundEvent> {
        let (spawner, tasks) = BlockingTaskSpawner::queued();
        let previous = std::mem::replace(&mut self.blocking_tasks, spawner);
        fn recording(proxy: &AppEventProxy) -> Arc<Mutex<Vec<UserEvent>>> {
            match proxy {
                AppEventProxy::Stub(events) => events.clone(),
                AppEventProxy::Project { inner, .. } => recording(inner),
                AppEventProxy::Real(_) => panic!("test drain must use a recording proxy"),
            }
        }
        let recorded = recording(&self.proxy);
        let mut outbound = self.update_drain_tick_events_at(now);
        drain_queued_blocking_tasks(&tasks);
        let completion = {
            let mut events = recorded.lock().expect("event log");
            events
                .iter()
                .position(|event| {
                    matches!(
                        recorded_project_payload(event),
                        UserEvent::UpdateDrainObserved { .. }
                    )
                })
                .map(|index| into_recorded_project_payload(events.remove(index)))
        };
        if let Some(UserEvent::UpdateDrainObserved { now, observations }) = completion {
            outbound.extend(self.update_drain_observed_events(now, observations));
        }
        drain_queued_blocking_tasks(&tasks);
        self.blocking_tasks = previous;
        outbound
    }

    /// Drive the same queued preparation and GUI completion used in production.
    pub(crate) fn handle_launch_complete_and_drain(
        &mut self,
        window_id: String,
        result: AgentLaunchResult,
    ) -> Vec<OutboundEvent> {
        let (spawner, tasks) = BlockingTaskSpawner::queued();
        let previous_spawner = std::mem::replace(&mut self.blocking_tasks, spawner);
        fn recording(proxy: &AppEventProxy) -> Arc<Mutex<Vec<UserEvent>>> {
            match proxy {
                AppEventProxy::Stub(events) => events.clone(),
                AppEventProxy::Project { inner, .. } => recording(inner),
                AppEventProxy::Real(_) => panic!("test launch must use a recording proxy"),
            }
        }
        let recorded = recording(&self.proxy);
        let mut outbound = self.handle_launch_complete(window_id, result);
        loop {
            drain_queued_blocking_tasks(&tasks);
            let completion = {
                let mut events = recorded.lock().expect("event log");
                events
                    .iter()
                    .position(|event| {
                        matches!(
                            recorded_project_payload(event),
                            UserEvent::AgentLaunchPrepared(_)
                                | UserEvent::IssueMonitorLaunchDeliveryAcknowledged(_)
                                | UserEvent::RuntimeHook(_)
                                | UserEvent::DaemonRuntimeHook(_)
                        )
                    })
                    .map(|index| into_recorded_project_payload(events.remove(index)))
            };
            match completion {
                Some(UserEvent::AgentLaunchPrepared(prepared)) => {
                    outbound.extend(self.handle_agent_launch_prepared(*prepared))
                }
                Some(UserEvent::IssueMonitorLaunchDeliveryAcknowledged(prepared)) => {
                    outbound.extend(self.handle_issue_monitor_launch_delivery_ack(*prepared))
                }
                Some(UserEvent::RuntimeHook(event)) => {
                    outbound.extend(self.handle_runtime_hook_event(event))
                }
                Some(UserEvent::DaemonRuntimeHook(event)) => {
                    outbound.extend(self.handle_daemon_runtime_hook_event(event))
                }
                _ => break,
            }
        }
        self.blocking_tasks = previous_spawner;
        outbound
    }
}

pub(super) fn queued_agent_completion_fixture(
    temp_root: &Path,
) -> (
    AppRuntime,
    Arc<Mutex<Vec<UserEvent>>>,
    BlockingTestTaskQueue,
    String,
    AgentLaunchResult,
) {
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp_root.into(),
        ProjectKind::Git,
        &[WindowPreset::Agent],
    );
    let window_id = combined_window_id("tab-1", &tab.workspace.persisted().windows[0].id);
    let (mut runtime, recorded) = sample_runtime_with_events(temp_root, vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let session = gwt_agent::Session::new(temp_root, "feature/test", gwt_agent::AgentId::Codex);
    session
        .save(&runtime.sessions_dir)
        .expect("persist launch Session");
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
            vec!["-c".to_string(), "exit 0".to_string()],
        )
    };
    let result = Ok((
        ProcessLaunch {
            initial_prompt_file: None,
            command,
            args,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: Some(temp_root.into()),
            resource_policy: None,
        },
        session.id,
        "feature/test".into(),
        "Agent".into(),
        temp_root.into(),
        gwt_agent::AgentId::Codex,
        None,
        None,
        gwt_agent::LaunchRuntimeTarget::Host,
        gwt_agent::SessionMode::Normal,
        false,
        temp_root.display().to_string().into(),
    ));
    (runtime, recorded, tasks, window_id, result)
}

fn take_prepared_agent_launch(recorded: &Arc<Mutex<Vec<UserEvent>>>) -> super::PreparedAgentLaunch {
    let mut events = recorded.lock().expect("event log");
    let index = events
        .iter()
        .position(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::AgentLaunchPrepared(_)
            )
        })
        .expect("agent launch preparation completion");
    match into_recorded_project_payload(events.remove(index)) {
        UserEvent::AgentLaunchPrepared(prepared) => *prepared,
        _ => unreachable!("matched agent launch preparation"),
    }
}

fn queued_continue_work_completion_fixture(
    temp_root: &Path,
) -> (
    ContinueWorkLaunchFailureFixture,
    Arc<Mutex<Vec<UserEvent>>>,
    BlockingTestTaskQueue,
    AgentLaunchResult,
) {
    let mut fixture = continue_work_launch_failure_fixture(temp_root);
    // The prepared candidate has not yet installed a GUI runtime/session entry.
    fixture.runtime.active_agent_sessions.clear();
    let (proxy, recorded) = AppEventProxy::stub();
    fixture.runtime.proxy = proxy;
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec!["/d".into(), "/s".into(), "/c".into(), "exit /b 0".into()],
        )
    } else {
        ("/bin/sh".to_string(), vec!["-c".into(), "exit 0".into()])
    };
    let identity = gwt_agent::SessionExecutionIdentity::from_session(&fixture.candidate_session)
        .expect("candidate identity")
        .expect("bound candidate");
    let mut result = bound_runtime_launch_completion(
        &fixture.repo,
        fixture.candidate_session_id,
        identity,
        command,
        args,
    );
    result.2 = "work/issue-2359".into();
    result.6 = Some(fixture.owner.number);
    (fixture, recorded, tasks, Ok(result))
}

fn assert_aborted_continue_work_launch(
    fixture: &ContinueWorkLaunchFailureFixture,
    pending: &PendingContinueWork,
) {
    assert!(
        !fixture
            .runtime
            .sessions_dir
            .join(format!("{}.toml", fixture.candidate_session_id))
            .exists(),
        "a canceled prepared continuation must not remain restorable",
    );
    assert!(!fixture.candidate_runtime_path.exists());
    assert_eq!(
        gwt::cli::execution_state::continuation_attempt_for_operation(
            &fixture.repo,
            fixture.owner,
            &pending.operation_id,
        )
        .expect("read canceled attempt")
        .expect("canceled attempt")
        .status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Aborted,
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
            .expect("read predecessor generation"),
        Some(pending.predecessor_binding.clone()),
    );
    let work = gwt_core::workspace_projection::load_workspace_work_items(&fixture.repo)
        .expect("read unchanged Work")
        .expect("selected Work");
    assert!(work
        .work_items
        .iter()
        .all(|item| { item.id != format!("work-session-{}", fixture.candidate_session_id) }));
    assert_eq!(
        work.work_items
            .iter()
            .find(|item| item.id == fixture.selected_work_id)
            .expect("selected Work remains")
            .status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Done,
    );
}

fn managed_hook_inspection_text(rendered: &str) -> String {
    let mut inspected = rendered.to_string();
    if let Ok(root) = serde_json::from_str::<serde_json::Value>(rendered) {
        for command in root["hooks"]
            .as_object()
            .into_iter()
            .flat_map(|hooks| hooks.values())
            .filter_map(|groups| groups.as_array())
            .flatten()
            .filter_map(|group| group["hooks"].as_array())
            .flatten()
            .filter_map(|hook| hook["command"].as_str())
        {
            if let Some(decoded) = gwt_skills::decode_powershell_encoded_command(command) {
                inspected.push('\n');
                inspected.push_str(&decoded);
            }
        }
    }
    inspected
}

// SPEC-3214 T-005/T-007: an ephemeral intake session leaves NO Work identity
// and its throwaway `.intake-*` worktree is removed when it ends (clean), while
// a dirty intake worktree is kept so no in-progress work is lost.
fn seed_codex_project_trust_for_cleanup(worktree: &Path, home: &Path) -> (PathBuf, String) {
    let config_path = home.join(".codex/config.toml");
    let report = gwt_skills::register_codex_managed_project_trust(worktree, &config_path)
        .expect("seed Codex project trust");
    (
        config_path,
        report
            .project_path
            .to_str()
            .expect("project path UTF-8")
            .to_string(),
    )
}

fn codex_project_trust_level(config_path: &Path, project_key: &str) -> Option<String> {
    let root = fs::read_to_string(config_path)
        .ok()
        .and_then(|content| toml::from_str::<toml::Value>(&content).ok())?;
    root.get("projects")?
        .as_table()?
        .get(project_key)?
        .as_table()?
        .get("trust_level")?
        .as_str()
        .map(str::to_string)
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
            diagnosis: None,
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

fn bound_runtime_launch_completion(
    repo: &Path,
    session_id: &str,
    expected_identity: gwt_agent::SessionExecutionIdentity,
    command: String,
    args: Vec<String>,
) -> AgentLaunchCompletion {
    (
        ProcessLaunch {
            initial_prompt_file: None,
            command,
            args,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: Some(repo.to_path_buf()),
            resource_policy: None,
        },
        session_id.to_string(),
        "feature/demo".to_string(),
        "Codex".to_string(),
        repo.to_path_buf(),
        gwt_agent::AgentId::Codex,
        Some(42),
        None,
        gwt_agent::LaunchRuntimeTarget::Host,
        gwt_agent::SessionMode::Resume,
        false,
        AgentLaunchRuntimeContext {
            agent_project_root: repo.display().to_string(),
            expected_execution_identity: Some(expected_identity),
            active_launch_handshake: None,
        },
    )
}

/// The exact Codex notice observed on 2026-08-16, soft-wrapped as the TUI
/// prints it.
const CODEX_USAGE_LIMIT_SCREEN: &str = "\
■ You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage
  to purchase more credits or try again at Aug 22nd, 2026 12:46 PM.";

/// The exact Claude wording observed on 2026-08-17 (Issue #3616 comment). It
/// names no provider, so the account can only be attributed from the pane.
const CLAUDE_USAGE_LIMIT_SCREEN: &str = "\
You've hit your weekly limit · resets Aug 20 at 6am (Asia/Tokyo)
/usage-credits to finish what you're working on.";

/// The screen a Claude pane shows after a provider API error ended its turn:
/// the error, the CLI's own "done" line, then the prompt it went back to.
const CLAUDE_API_ERROR_SCREEN: &str = concat!(
    "⏺ Running the verification matrix now.\n",
    "API Error: 529 Overloaded. This is a server-side issue, usually temporary\n",
    "✻ Sautéed for 1h 40m 43s · done 9:56 AM\n",
    "───────────────────────────────────────────\n",
    "❯\n",
    "───────────────────────────────────────────\n",
    "⏵⏵ bypass permissions on\n",
);

/// A pane mid-turn, exactly as the two windows in Issue #4584's report were:
/// hooks last said the agent was working, and nothing has contradicted that.
fn api_error_live_runtime(temp: &Path) -> (AppRuntime, String) {
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp, vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.agent_id = "claude".to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);
    let _ = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Running",
        "PreToolUse",
        "session-1",
    ));
    (runtime, window_id)
}

fn quota_live_runtime(temp: &Path, agent_id: &str) -> (AppRuntime, String) {
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp, vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.agent_id = agent_id.to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);
    let _ = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Idle",
        "Stop",
        "session-1",
    ));
    (runtime, window_id)
}

fn instant(text: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(text)
        .expect("fixture instant")
        .with_timezone(&chrono::Utc)
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

/// Issue #3934: seed one Active owner whose durable holder is in `status` and
/// has no runtime sidecar anywhere, and return its worktree inventory.
#[cfg(test)]
fn seed_defunct_active_owner(
    sessions_dir: &std::path::Path,
    repo: &std::path::Path,
    worktree: &std::path::Path,
    branch: &str,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    session_id: &str,
    status: gwt_agent::AgentStatus,
) -> Vec<std::path::PathBuf> {
    gwt::cli::execution_state::materialize_at_launch(
        worktree,
        owner.kind,
        owner.number,
        session_id,
        "gwt-execute",
        false,
    )
    .expect("materialize Active execution");
    gwt::cli::execution_state::ensure_generation_ledger(
        worktree,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("ensure generation ledger");
    let binding = gwt::cli::execution_state::current_execution_binding(worktree, owner)
        .expect("load current binding")
        .expect("current binding");
    let mut session = gwt_agent::Session::new(worktree, branch, gwt_agent::AgentId::Codex);
    session.id = session_id.to_string();
    session.agent_session_id = Some(format!("native-{session_id}"));
    session.linked_issue_number = Some(owner.number);
    session.restore_window_on_startup = false;
    session.execution_binding = Some(gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session.id.clone(),
        repo_hash: session.repo_hash.clone().expect("repo hash"),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity: binding,
        capability_generation: 1,
    });
    session.update_status(status);
    session
        .save(sessions_dir)
        .expect("save defunct holder Session");
    gwt::worktree_inventory::enumerate_worktrees(repo, None)
        .expect("worktree inventory")
        .into_iter()
        .map(|entry| entry.path)
        .collect()
}

thread_local! {
    static WORKTREE_LISTINGS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn count_worktree_listing(_started: Instant) {
    WORKTREE_LISTINGS.with(|count| count.set(count.get() + 1));
}

/// `git worktree list` runs made on the calling thread (Issue #4378 AC-1).
/// The observer is per process; counting per thread keeps parallel tests
/// from seeing each other's listings.
fn worktree_listings_on_this_thread() -> u64 {
    gwt_git::worktree::set_worktree_list_observer(count_worktree_listing);
    WORKTREE_LISTINGS.with(std::cell::Cell::get)
}

fn approval_settle_runtime() -> (tempfile::TempDir, AppRuntime, String, Vec<u8>) {
    let temp = tempdir().expect("tempdir");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    runtime
        .window_hook_states
        .insert(window_id.clone(), WindowProcessStatus::Running);
    let prompt = b"Would you like to run the following command?\r\n command\r\n\
        > 1. Yes, proceed\r\n 2. No, and tell Codex what to do differently\r\n\
        Press enter to confirm or esc to cancel\r\n"
        .to_vec();
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(&prompt);
    runtime.handle_runtime_output(window_id.clone(), prompt.clone());
    runtime.terminal_input_events(&window_id, "1\r");
    (temp, runtime, window_id, prompt)
}

/// Fake index python for the transient-semantic-failure runtime contract
/// (SPEC #3170 T-944): canonical `search-multi` classifies the issues scope
/// as missing, the legacy per-kind semantic actions fail hard, and repair
/// (`index-*`) succeeds instantly.
#[cfg(unix)]
fn write_fake_project_index_runtime_with_missing_issues_scope(home: &Path) {
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
  *"--action search-multi"*)
    printf '%s\n' '{"ok":true,"scopes":{"issues":{"state":"missing"}}}'
    exit 0
    ;;
  *"--action search-issues"*|*"--action search-specs"*)
    printf '%s\n' '{"ok":false,"error":"legacy semantic action used"}'
    exit 1
    ;;
  *"--action index-"*)
    printf '%s\n' '{"ok":true}'
    exit 0
    ;;
esac
printf '%s\n' '{"ok":false,"error":"unexpected fake python invocation"}'
exit 1
"#;
    let legacy_python = home
        .join(".gwt")
        .join("runtime")
        .join("chroma-venv")
        .join("bin")
        .join("python3");
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
        _reason: Option<gwt_github::client::IssueCloseReason>,
    ) -> Result<IssueSnapshot, ApiError> {
        unreachable!("quick register only creates issues")
    }

    fn list_spec_issues(&self, _filter: &SpecListFilter) -> Result<Vec<SpecSummary>, ApiError> {
        unreachable!("quick register only creates issues")
    }
}

fn assert_ack_preserves_failed_window_replacement(test_root: &Path, restart_existing_id: bool) {
    let repo = test_root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo_without_origin(&repo);
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Agent, WindowPreset::Agent],
    );
    let failed_raw_id = tab.workspace.persisted().windows[1].id.clone();
    let launched_id = combined_window_id("tab-1", &tab.workspace.persisted().windows[0].id);
    let failed_id = combined_window_id("tab-1", &failed_raw_id);
    let (mut runtime, recorded) = sample_runtime_with_events(test_root, vec![tab], Some("tab-1"));
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            launching_issues: vec![gwt::IssueMonitorLaunchingIssue {
                issue_number: 42,
                claimed_at: None,
            }],
            failed_issues: vec![gwt::IssueMonitorFailedIssue {
                issue_number: 42,
                message: "previous pane failed".into(),
                window_id: Some(failed_id.clone()),
            }],
            ..Default::default()
        },
    )
    .unwrap();
    // A prepared worker may mint its runtime before ACK enqueue, then install
    // it later. Numeric runtime age alone cannot authorize stale-pane cleanup.
    let replacement_runtime = restart_existing_id.then(|| {
        WindowRuntime::new(
            super::next_window_runtime_incarnation(),
            Arc::new(Mutex::new(long_running_test_pane(&failed_id))),
        )
    });
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.issue_monitor_launch_succeeded_delivery_events(&repo, 42, &launched_id, None);
    tasks.lock().unwrap().pop().expect("ACK worker")();
    let completion = {
        let mut events = recorded.lock().unwrap();
        let index = events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::IssueMonitorLaunchDeliveryAcknowledged(_)
                )
            })
            .expect("ACK completion");
        into_recorded_project_payload(events.remove(index))
    };
    if let Some(replacement_runtime) = replacement_runtime {
        runtime
            .runtimes
            .insert(failed_id.clone(), replacement_runtime);
    } else {
        runtime.close_window_events(&failed_id);
        let replacement = runtime
            .tab_mut("tab-1")
            .unwrap()
            .workspace
            .add_window(WindowPreset::Agent, canvas_bounds());
        assert_eq!(replacement.id, failed_raw_id, "closed highest ID is reused");
        runtime.register_window("tab-1", &replacement.id);
    }
    let UserEvent::IssueMonitorLaunchDeliveryAcknowledged(ack) = completion else {
        unreachable!()
    };
    runtime.handle_issue_monitor_launch_delivery_ack(*ack);
    let replacement_survived = runtime.tracked_window_exists(&failed_id);
    runtime.stop_window_runtime_without_session_projection(&failed_id);
    drain_queued_blocking_tasks(&tasks);
    assert!(
        replacement_survived,
        "ACK cleanup must preserve a replacement failed-window ID"
    );
    assert!(runtime.tracked_window_exists(&launched_id));
}

/// Every place a launch can carry its command text, in one searchable list.
///
/// Issue #4014: on Windows the command and its arguments do not stay in
/// `args`. The resolver moves them into `GWT_WINDOWS_CMD_WRAPPER_EXPRESSION`
/// so `cmd.exe` never has to quote them, and `args` is left holding a
/// PowerShell script that only references the placeholder. A prompt assertion
/// that reads `args` alone therefore finds the wrapper instead of the prompt,
/// and fails on Windows for a reason that has nothing to do with the prompt.
/// This was invisible until #4014 let the `--bin gwt` target run there at all.
fn launch_payload_fragments(process: &ProcessLaunch) -> Vec<String> {
    let mut fragments = process.args.clone();
    for key in [
        gwt_core::process::WINDOWS_CMD_WRAPPER_EXPRESSION_ENV,
        crate::launch_runtime::WINDOWS_HOST_SHELL_EXPRESSION_ENV,
    ] {
        if let Some(expression) = process.env.get(key) {
            fragments.push(expression.clone());
        }
    }
    fragments
}

#[derive(Debug, Clone, Copy)]
enum MonitorProviderConversationFixture {
    Present,
    Missing,
    Foreign,
    Unknown,
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MonitorNativeHolderFixture {
    None,
    ActiveSameConversation,
    ActiveSameConversationMissingWindow,
    ActiveSameConversationStopped,
    ActiveSameConversationError,
    ActiveOtherConversation,
    MaterializingSameConversation,
    StaleMaterializingSameConversation,
}

struct MonitorRelaunchFixture {
    runtime: AppRuntime,
    recorded_events: Arc<Mutex<Vec<UserEvent>>>,
    sessions_dir: PathBuf,
    project_root: PathBuf,
    worktree: PathBuf,
    repo_hash: String,
    source_session_id: String,
    native_conversation_id: String,
    execution_owner: gwt::cli::execution_state::ExecutionOwnerKey,
    predecessor_execution_binding: gwt_agent::ExecutionBindingIdentity,
    holder_window_id: Option<String>,
    delivery_id: Option<String>,
}

fn prepare_monitor_relaunch(
    fixture: &mut MonitorRelaunchFixture,
    strategy: gwt::IssueMonitorLaunchSessionStrategy,
) -> Vec<OutboundEvent> {
    let previous_spawner = fixture.runtime.blocking_tasks.clone();
    let (spawner, queued) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events_for_project(
            &fixture.project_root,
            3165,
            LinkedIssueKind::Spec,
            None,
            strategy,
        );
    drain_queued_blocking_tasks(&queued);
    fixture.runtime.blocking_tasks = previous_spawner;
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &fixture.recorded_events,
        ))
}

fn codex_issue_monitor_launch_profile() -> gwt::IssueMonitorLaunchProfile {
    gwt::IssueMonitorLaunchProfile {
        agent_id: "codex".to_string(),
        model: Some("gpt-5.5".to_string()),
        reasoning: Some("high".to_string()),
        version: Some("latest".to_string()),
        session_mode: gwt_agent::SessionMode::Normal,
        skip_permissions: true,
        fast_mode: false,
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        docker_service: None,
        docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
        windows_shell: None,
        prefer_for: Vec::new(),
    }
}

fn claude_issue_monitor_launch_profile() -> gwt::IssueMonitorLaunchProfile {
    gwt::IssueMonitorLaunchProfile {
        agent_id: "claude".to_string(),
        model: Some("sonnet".to_string()),
        reasoning: Some("low".to_string()),
        version: Some("latest".to_string()),
        session_mode: gwt_agent::SessionMode::Normal,
        skip_permissions: true,
        fast_mode: false,
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        docker_service: None,
        docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
        windows_shell: None,
        prefer_for: Vec::new(),
    }
}

fn monitor_relaunch_fixture(
    root: &Path,
    case_name: &str,
    conversation: MonitorProviderConversationFixture,
    holder: MonitorNativeHolderFixture,
    durable_delivery: bool,
) -> MonitorRelaunchFixture {
    monitor_relaunch_fixture_with_settlement(
        root,
        case_name,
        conversation,
        holder,
        durable_delivery,
        gwt::cli::execution_state::ExecutionSettlement::Completed,
    )
}

fn monitor_relaunch_fixture_with_settlement(
    root: &Path,
    case_name: &str,
    conversation: MonitorProviderConversationFixture,
    holder: MonitorNativeHolderFixture,
    durable_delivery: bool,
    settlement: gwt::cli::execution_state::ExecutionSettlement,
) -> MonitorRelaunchFixture {
    let case_root = root.join(case_name);
    fs::create_dir_all(&case_root).expect("create monitor relaunch case root");
    let repo = case_root.join("repo");
    init_git_clone_with_origin(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            3165,
            "SPEC: monitor relaunch safety",
            &["gwt-spec"],
            "Monitor relaunch fixture",
            "2026-08-13T00:00:00Z",
        ))
        .expect("seed monitored SPEC authority");
    let worktree = case_root.join("issue-worktree");
    let target_branch = "work/issue-3165";
    let status = gwt_core::process::hidden_command("git")
        .args(["worktree", "add", "-b", target_branch])
        .arg(&worktree)
        .arg("develop")
        .current_dir(&repo)
        .status()
        .expect("materialize monitored issue worktree");
    assert!(status.success(), "git worktree add failed with {status}");

    // Resume authority and the launch handshake must read the same Session store.
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let native_conversation_id = format!("native-monitor-{case_name}");
    let source_session_id = format!("session-monitor-{case_name}");
    let now = Utc::now();
    let mut source = gwt_agent::Session::new(&worktree, target_branch, gwt_agent::AgentId::Codex);
    source.id = source_session_id.clone();
    source.agent_session_id = Some(native_conversation_id.clone());
    source.project_state_root = Some(repo.clone());
    source.linked_issue_number = Some(3165);
    source.model = Some("gpt-5.5".to_string());
    source.reasoning_level = Some("low".to_string());
    source.tool_version = Some("latest".to_string());
    source.skip_permissions = true;
    source.status = gwt_agent::AgentStatus::Stopped;
    source.created_at = now;
    source.updated_at = now;
    source.last_activity_at = now;
    if matches!(conversation, MonitorProviderConversationFixture::Unknown) {
        source.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        source.docker_service = Some("app".to_string());
    }
    let execution_owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Spec,
        number: 3165,
    };
    gwt::cli::execution_state::materialize_at_launch(
        &worktree,
        execution_owner.kind,
        execution_owner.number,
        &source_session_id,
        "$gwt-execute #3165",
        false,
    )
    .expect("materialize monitored predecessor execution");
    assert!(matches!(
        gwt::cli::execution_state::settle(&worktree, &source_session_id, settlement,)
            .expect("settle monitored predecessor"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    gwt::cli::execution_state::ensure_generation_ledger(
        &worktree,
        execution_owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
    )
    .expect("import monitored predecessor generation");
    let predecessor_execution_binding =
        gwt::cli::execution_state::current_execution_binding(&worktree, execution_owner)
            .expect("read monitored predecessor binding")
            .expect("monitored predecessor binding");
    let repo_hash = source
        .repo_hash
        .clone()
        .expect("monitored predecessor repository hash");
    source
        .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: source_session_id.clone(),
            repo_hash: repo_hash.clone(),
            owner_kind: execution_owner.kind.as_str().to_string(),
            owner_number: execution_owner.number,
            identity: predecessor_execution_binding.clone(),
            capability_generation: 1,
        }))
        .expect("bind monitored predecessor Session");
    source
        .save(&sessions_dir)
        .expect("save resume source Session");
    let holder_window_id = (holder != MonitorNativeHolderFixture::None)
        .then(|| combined_window_id("tab-1", "agent-holder"));
    let holder_session_id = holder_window_id.as_ref().map(|_| {
        let holder_session_id = format!("session-holder-{case_name}");
        let holder_conversation_id = match holder {
            MonitorNativeHolderFixture::ActiveOtherConversation => {
                format!("native-other-{case_name}")
            }
            MonitorNativeHolderFixture::ActiveSameConversation
            | MonitorNativeHolderFixture::ActiveSameConversationMissingWindow
            | MonitorNativeHolderFixture::ActiveSameConversationStopped
            | MonitorNativeHolderFixture::ActiveSameConversationError
            | MonitorNativeHolderFixture::MaterializingSameConversation
            | MonitorNativeHolderFixture::StaleMaterializingSameConversation => {
                native_conversation_id.clone()
            }
            MonitorNativeHolderFixture::None => unreachable!("holder window implies holder"),
        };
        let holder_at = now - chrono::Duration::hours(1);
        let mut holder_session =
            gwt_agent::Session::new(&worktree, target_branch, gwt_agent::AgentId::Codex);
        holder_session.id = holder_session_id.clone();
        holder_session.agent_session_id = Some(holder_conversation_id);
        holder_session.project_state_root = Some(repo.clone());
        holder_session.linked_issue_number = Some(3165);
        holder_session.status = if matches!(
            holder,
            MonitorNativeHolderFixture::MaterializingSameConversation
                | MonitorNativeHolderFixture::StaleMaterializingSameConversation
        ) {
            gwt_agent::AgentStatus::Stopped
        } else {
            gwt_agent::AgentStatus::Running
        };
        holder_session.created_at = holder_at;
        holder_session.updated_at = holder_at;
        holder_session.last_activity_at = holder_at;
        holder_session
            .save(&sessions_dir)
            .expect("save native conversation holder Session");
        holder_session_id
    });

    let codex_home = PathBuf::from(
        std::env::var_os("CODEX_HOME").expect("CODEX_HOME is isolated for this test"),
    );
    let rollout_dir = codex_home.join("sessions/2026/08/13");
    fs::create_dir_all(&rollout_dir).expect("create Codex rollout directory");
    let rollout_path = rollout_dir.join(format!(
        "rollout-2026-08-13T00-00-00-{native_conversation_id}.jsonl"
    ));
    match conversation {
        MonitorProviderConversationFixture::Present
        | MonitorProviderConversationFixture::Unknown => fs::write(
            &rollout_path,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{native_conversation_id}\",\"cwd\":{}}}}}\n",
                serde_json::to_string(&worktree.display().to_string())
                    .expect("serialize monitored worktree")
            ),
        )
        .expect("write present Codex rollout"),
        MonitorProviderConversationFixture::Foreign => {
            let foreign = case_root.join("foreign-worktree");
            fs::create_dir_all(&foreign).expect("create foreign worktree fixture");
            fs::write(
                &rollout_path,
                format!(
                    "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{native_conversation_id}\",\"cwd\":{}}}}}\n",
                    serde_json::to_string(&foreign.display().to_string())
                        .expect("serialize foreign worktree")
                ),
            )
            .expect("write foreign Codex rollout");
        }
        MonitorProviderConversationFixture::Corrupt => {
            fs::write(&rollout_path, b"{corrupt rollout\n").expect("write corrupt Codex rollout");
        }
        MonitorProviderConversationFixture::Missing => {}
    }

    let delivery_id = durable_delivery.then(|| format!("launch:effect-{case_name}"));
    let mut prefs = if let Some(delivery_id) = delivery_id.as_deref() {
        let mut monitor = gwt::IssueMonitorState::with_prefs(
            gwt::IssueMonitorConfig {
                enabled: true,
                ..gwt::IssueMonitorConfig::default()
            },
            gwt::IssueMonitorPrefs {
                max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
                queued_launch_session_strategies: std::collections::BTreeMap::from([(
                    3165,
                    gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
                )]),
                ..gwt::IssueMonitorPrefs::default()
            },
        );
        monitor.terminal_queue_push(&[3165], "operator", "2026-07-28T00:00:00Z");
        monitor.record_candidate(gwt::IssueMonitorIssue {
            number: 3165,
            title: "SPEC: monitor relaunch safety".to_string(),
            labels: vec!["gwt-spec".to_string()],
            state: gwt::IssueMonitorIssueState::Open,
            body: None,
            url: None,
            readiness: gwt::IssueMonitorReadiness::Ready,
            updated_at: None,
        });
        assert!(monitor.apply_confirmed_claim(
            3165,
            format!("claim-{case_name}"),
            "host/session",
            delivery_id.trim_start_matches("launch:"),
            &now.to_rfc3339(),
        ));
        monitor.prefs()
    } else {
        gwt::IssueMonitorPrefs {
            max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
            max_active_agents: 1,
            ..gwt::IssueMonitorPrefs::default()
        }
    };
    prefs.launch_profile = Some(codex_issue_monitor_launch_profile());
    gwt::save_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo), &prefs)
        .expect("save monitor relaunch prefs");

    let holder_window_status = match holder {
        MonitorNativeHolderFixture::ActiveSameConversationStopped => {
            Some(WindowProcessStatus::Stopped)
        }
        MonitorNativeHolderFixture::ActiveSameConversationError => Some(WindowProcessStatus::Error),
        MonitorNativeHolderFixture::ActiveSameConversation
        | MonitorNativeHolderFixture::ActiveOtherConversation
        | MonitorNativeHolderFixture::MaterializingSameConversation => {
            Some(WindowProcessStatus::Running)
        }
        MonitorNativeHolderFixture::None
        | MonitorNativeHolderFixture::ActiveSameConversationMissingWindow
        | MonitorNativeHolderFixture::StaleMaterializingSameConversation => None,
    };
    let tab = match holder_window_status {
        Some(status) => sample_project_tab_with_window_at(
            "tab-1",
            "agent-holder",
            repo.clone(),
            WindowPreset::Agent,
            status,
        ),
        None => sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]),
    };
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(&case_root, vec![tab], Some("tab-1"));
    runtime.sessions_dir = sessions_dir.clone();
    let mut agent_options = sample_agent_options();
    agent_options.push(gwt::AgentOption {
        id: "claude".to_string(),
        name: "Claude Code".to_string(),
        available: true,
        installed_version: Some("latest".to_string()),
        custom_agent: None,
    });
    runtime.launch_wizard_cache =
        LaunchWizardMemoryCache::load_with_agent_options(&sessions_dir, agent_options);
    runtime.agent_capability_issuer =
        Some(crate::embedded_server::AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:43123/internal/hook-live",
            "ws://127.0.0.1:43124/ws",
            "ws://127.0.0.1:43123/internal/pane-ws",
        ));
    assert_eq!(
        runtime
            .latest_resumable_branch_session(&repo, target_branch)
            .expect("latest resumable monitored Session")
            .id,
        source_session_id,
        "holder fixtures must not replace the intended resume candidate",
    );
    if let (Some(holder_window_id), Some(holder_session_id)) =
        (holder_window_id.as_ref(), holder_session_id)
    {
        match holder {
            MonitorNativeHolderFixture::ActiveSameConversation
            | MonitorNativeHolderFixture::ActiveSameConversationMissingWindow
            | MonitorNativeHolderFixture::ActiveSameConversationStopped
            | MonitorNativeHolderFixture::ActiveSameConversationError
            | MonitorNativeHolderFixture::ActiveOtherConversation => {
                runtime.active_agent_sessions.insert(
                    holder_window_id.clone(),
                    ActiveAgentSession {
                        window_id: holder_window_id.clone(),
                        session_id: holder_session_id,
                        agent_id: "codex".to_string(),
                        branch_name: target_branch.to_string(),
                        display_name: "Codex".to_string(),
                        worktree_path: worktree.clone(),
                        agent_project_root: worktree.display().to_string(),
                        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                        tab_id: "tab-1".to_string(),
                    },
                );
                if holder == MonitorNativeHolderFixture::ActiveSameConversation {
                    runtime
                        .window_hook_states
                        .insert(holder_window_id.clone(), WindowProcessStatus::Running);
                }
            }
            MonitorNativeHolderFixture::MaterializingSameConversation
            | MonitorNativeHolderFixture::StaleMaterializingSameConversation => {
                runtime
                    .pending_auto_resume_sources
                    .insert(holder_window_id.clone(), holder_session_id);
                if holder == MonitorNativeHolderFixture::MaterializingSameConversation {
                    runtime.inflight_launches.insert(
                        format!("monitor-holder-{case_name}"),
                        (holder_window_id.clone(), Instant::now()),
                    );
                }
            }
            MonitorNativeHolderFixture::None => unreachable!("holder tuple excludes none"),
        }
    }

    let fixture = MonitorRelaunchFixture {
        runtime,
        recorded_events,
        sessions_dir,
        project_root: repo,
        worktree,
        repo_hash,
        source_session_id,
        native_conversation_id,
        execution_owner,
        predecessor_execution_binding,
        holder_window_id,
        delivery_id,
    };
    // Keep installed-agent probes local to the fixture. Tests that rewrite
    // this profile must retain the pin through `pin_monitor_fixture_agents`.
    pin_monitor_fixture_agents(&fixture, &case_root, &[]);
    fixture
}

fn convert_monitor_relaunch_fixture_to_grok(
    fixture: &mut MonitorRelaunchFixture,
    grok_home: &Path,
) {
    let mut sessions = Vec::new();
    for entry in fs::read_dir(&fixture.sessions_dir)
        .expect("read fixture sessions")
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
            continue;
        }
        let mut session = gwt_agent::Session::load(&path).expect("load fixture Session");
        session.agent_id = gwt_agent::AgentId::GrokBuild;
        session.launch_command = "grok".to_string();
        session.display_name = "Grok Build".to_string();
        session
            .save(&fixture.sessions_dir)
            .expect("save Grok Session");
        if session.id == fixture.source_session_id {
            session
                .save(&gwt_core::paths::gwt_sessions_dir())
                .expect("save globally discoverable Grok Session");
        }
        sessions.push(session);
    }
    for active in fixture.runtime.active_agent_sessions.values_mut() {
        active.agent_id = "grok".to_string();
        active.display_name = "Grok Build".to_string();
    }
    let mut agent_options = sample_agent_options();
    agent_options.extend([
        gwt::AgentOption {
            id: "claude".to_string(),
            name: "Claude Code".to_string(),
            available: true,
            installed_version: Some("latest".to_string()),
            custom_agent: None,
        },
        gwt::AgentOption {
            id: "grok".to_string(),
            name: "Grok Build".to_string(),
            available: true,
            installed_version: Some("latest".to_string()),
            custom_agent: None,
        },
    ]);
    fixture.runtime.launch_wizard_cache =
        LaunchWizardMemoryCache::load_with_agent_options(&fixture.sessions_dir, agent_options);

    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load Monitor prefs");
    prefs.launch_profile = Some(gwt::IssueMonitorLaunchProfile {
        agent_id: "grok".to_string(),
        model: Some("grok-4.20-beta".to_string()),
        reasoning: Some("high".to_string()),
        version: Some("latest".to_string()),
        session_mode: gwt_agent::SessionMode::Normal,
        skip_permissions: true,
        fast_mode: false,
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        docker_service: None,
        docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
        windows_shell: None,
        prefer_for: Vec::new(),
    });
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("save Grok Monitor profile");

    let summary = grok_home
        .join("sessions/%2Ffixture%2Fworktree")
        .join(&fixture.native_conversation_id)
        .join("summary.json");
    fs::create_dir_all(summary.parent().expect("Grok summary parent"))
        .expect("create Grok summary parent");
    fs::write(
        &summary,
        serde_json::json!({
            "info": {
                "id": fixture.native_conversation_id,
                "cwd": fixture.worktree,
            }
        })
        .to_string(),
    )
    .expect("write Grok summary");
    fs::write(
        summary
            .parent()
            .expect("Grok summary parent")
            .join("updates.jsonl"),
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": fixture.native_conversation_id,
                "update": {"sessionUpdate": "agent_message_chunk"}
            }
        })
        .to_string()
            + "\n",
    )
    .expect("write Grok updates");
}

fn take_monitor_launch_complete(
    label: &str,
    events: &Arc<Mutex<Vec<UserEvent>>>,
) -> AgentLaunchResult {
    take_monitor_launch_complete_event(label, events).1
}

fn take_monitor_launch_complete_event(
    label: &str,
    events: &Arc<Mutex<Vec<UserEvent>>>,
) -> (String, AgentLaunchResult) {
    // This integration-style fixture performs real Git setup plus managed
    // asset/trust materialization in a debug build. Under parallel CI load it
    // can legitimately exceed the generic 20-second unit-test poll budget.
    wait_for_recorded_event_with_timeout(label, events, Duration::from_secs(60), |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::LaunchComplete { .. }
            )
        })
    });
    let mut events = events.lock().expect("event log");
    let index = events
        .iter()
        .position(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::LaunchComplete { .. }
            )
        })
        .expect("launch complete event");
    let UserEvent::LaunchComplete { window_id, result } =
        into_recorded_project_payload(events.remove(index))
    else {
        unreachable!("matched launch complete above")
    };
    (window_id, *result)
}

fn assert_monitor_exact_resume(result: AgentLaunchResult, fixture: &MonitorRelaunchFixture) {
    let Ok((process, session_id, _, _, _, _, _, _, _, session_mode, _, _)) = result else {
        panic!("Issue Monitor exact Resume failed: {result:?}");
    };
    assert_eq!(session_mode, gwt_agent::SessionMode::Resume);
    assert!(
        process
            .args
            .iter()
            .any(|argument| argument.contains(&fixture.native_conversation_id)),
        "exact Resume must pass the selected native conversation id: {:?}",
        process.args,
    );
    let resumed =
        gwt_agent::Session::load(&fixture.sessions_dir.join(format!("{session_id}.toml")))
            .expect("load prepared exact Resume Session");
    assert_eq!(resumed.session_mode, gwt_agent::SessionMode::Resume);
    let binding = resumed
        .execution_binding
        .as_ref()
        .expect("exact Monitor Resume must retain producing authority");
    assert_eq!(binding.session_id, session_id);
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(
            &fixture.worktree,
            fixture.execution_owner,
        )
        .unwrap(),
        Some(binding.identity.clone())
    );
    assert_eq!(
        resumed.agent_session_id.as_deref(),
        Some(fixture.native_conversation_id.as_str()),
        "exact Resume must retain the provider conversation identity",
    );
    assert_eq!(resumed.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(resumed.reasoning_level.as_deref(), Some("low"));
    let monitor_prefs = gwt::load_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root),
    )
    .expect("load monitor prefs after exact Resume");
    if monitor_prefs.autonomous_mode {
        assert_eq!(
            process
                .env
                .get(gwt::autonomous_handoff::GWT_AUTONOMOUS_EXECUTION_ENV)
                .map(String::as_str),
            Some("1"),
            "an autonomous exact Resume must retain the question guard marker",
        );
        assert_eq!(
            process
                .env
                .get(gwt::autonomous_handoff::GWT_AUTONOMOUS_ISSUE_ENV)
                .map(String::as_str),
            Some("3165"),
            "an autonomous exact Resume must retain its owner identity",
        );
    }
    for handoff in &monitor_prefs.autonomous_handoffs {
        if handoff.issue_number == 3165 {
            if let Some(answer) = handoff.answer.as_deref() {
                assert!(
                    process
                        .args
                        .iter()
                        .any(|argument| argument.contains(answer)),
                    "the exact Resume must receive the one-shot human answer: {:?}",
                    process.args,
                );
            }
        }
    }
}

fn seed_resumed_autonomous_handoff(fixture: &MonitorRelaunchFixture, answer: &str) {
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load monitor prefs");
    prefs.autonomous_mode = true;
    let source = gwt_agent::Session::load(
        &fixture
            .sessions_dir
            .join(format!("{}.toml", fixture.source_session_id)),
    )
    .expect("load handoff source Session");
    let mut handoff = gwt::autonomous_handoff::AutonomousQuestionHandoff::new(
        "handoff-3716".to_string(),
        &gwt::autonomous_handoff::AutonomousExecutionContext {
            issue_number: 3165,
            session_id: fixture.source_session_id.clone(),
        },
        &source.agent_id.to_string(),
        "ask_user_question",
        gwt::autonomous_handoff::ExtractedQuestion {
            question: "Approve User Verification?".to_string(),
            options: vec![gwt::autonomous_handoff::AutonomousHandoffOption {
                label: "Approve".to_string(),
                description: "Continue to the PR gate".to_string(),
            }],
        },
        "2026-08-20T00:00:00Z",
    );
    handoff.state = gwt::autonomous_handoff::AutonomousHandoffState::Resumed;
    handoff.answer = Some(answer.to_string());
    handoff.answered_at = Some("2026-08-20T00:01:00Z".to_string());
    prefs.autonomous_handoffs.push(handoff);
    prefs
        .queued_launch_session_strategies
        .insert(3165, gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe);
    for delivery in &mut prefs.pending_launch_deliveries {
        if delivery.issue_number == 3165 {
            delivery.launch_session_strategy = gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe;
        }
    }
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed resumed handoff");
}

fn assert_monitor_fresh_successor(result: AgentLaunchResult, fixture: &MonitorRelaunchFixture) {
    let Ok((
        process,
        session_id,
        branch,
        _,
        worktree,
        agent_id,
        linked_issue_number,
        _,
        runtime_target,
        session_mode,
        _,
        _,
    )) = result
    else {
        panic!("Issue Monitor fresh successor failed: {result:?}");
    };
    assert_eq!(session_mode, gwt_agent::SessionMode::Normal);
    assert_eq!(runtime_target, gwt_agent::LaunchRuntimeTarget::Host);
    assert_eq!(agent_id, gwt_agent::AgentId::Codex);
    assert_eq!(linked_issue_number, Some(fixture.execution_owner.number));
    assert_eq!(branch, "work/issue-3165");
    assert!(same_worktree_path(&worktree, &fixture.worktree));
    assert_ne!(session_id, fixture.source_session_id);
    assert!(
        process
            .args
            .iter()
            .all(|argument| !argument.contains(&fixture.native_conversation_id)),
        "fresh successor must not receive the old native conversation id: {:?}",
        process.args,
    );
    assert!(
        process
            .env
            .contains_key(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV),
        "fresh fallback must prepare successor authority instead of minting genesis",
    );
    let successor =
        gwt_agent::Session::load(&fixture.sessions_dir.join(format!("{session_id}.toml")))
            .expect("load prepared fresh successor Session");
    assert_eq!(successor.session_mode, gwt_agent::SessionMode::Normal);
    assert!(
        successor.agent_session_id.is_none(),
        "fresh successor launch config must clear the old resume_session_id",
    );
    assert_eq!(successor.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(successor.reasoning_level.as_deref(), Some("high"));
    assert_eq!(successor.agent_id, gwt_agent::AgentId::Codex);
    assert_eq!(successor.tool_version.as_deref(), Some("1.2.3"));
    assert!(successor.tool_version_selector.is_none());
    assert!(successor.skip_permissions);
    assert!(!successor.fast_mode);
    assert!(!successor.codex_fast_mode);
    assert_eq!(
        successor.runtime_target,
        gwt_agent::LaunchRuntimeTarget::Host
    );
    assert!(same_worktree_path(
        &successor.worktree_path,
        &fixture.worktree
    ));
    assert!(successor
        .project_state_root
        .as_deref()
        .is_some_and(|root| same_worktree_path(root, &fixture.project_root)));
    assert_eq!(
        successor.repo_hash.as_deref(),
        Some(fixture.repo_hash.as_str())
    );
    assert_eq!(successor.branch, "work/issue-3165");
    assert_eq!(
        successor.linked_issue_number,
        Some(fixture.execution_owner.number)
    );
    let successor_binding = successor
        .execution_binding
        .as_ref()
        .expect("fresh successor must carry a Prepared execution binding");
    assert_eq!(
        successor_binding.owner_kind,
        fixture.execution_owner.kind.as_str()
    );
    assert_eq!(
        successor_binding.owner_number,
        fixture.execution_owner.number
    );
    assert_eq!(successor_binding.repo_hash, fixture.repo_hash);
    assert_eq!(successor_binding.session_id, session_id);
    assert_ne!(
        successor_binding.identity.generation_id,
        fixture.predecessor_execution_binding.generation_id,
        "fresh successor must prepare a new execution generation",
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(
            &worktree,
            fixture.execution_owner,
        )
        .expect("read predecessor fence after successor preparation")
        .as_ref(),
        Some(&fixture.predecessor_execution_binding),
        "predecessor failure evidence and generation fence must remain current until spawn succeeds",
    );
}

fn pool_profile(agent_id: &str) -> gwt::IssueMonitorLaunchProfile {
    gwt::IssueMonitorLaunchProfile {
        agent_id: agent_id.to_string(),
        model: None,
        reasoning: None,
        version: None,
        session_mode: Default::default(),
        skip_permissions: false,
        fast_mode: false,
        runtime_target: Default::default(),
        docker_service: None,
        docker_lifecycle_intent: Default::default(),
        windows_shell: None,
        prefer_for: Vec::new(),
    }
}

fn agent_settings_option(id: &str, name: &str) -> gwt::AgentOption {
    gwt::AgentOption {
        id: id.to_string(),
        name: name.to_string(),
        available: true,
        installed_version: Some("0.159.2".to_string()),
        custom_agent: None,
    }
}

/// Issue #4911: a runtime whose Launch Wizard offers three agents, with the
/// Agent Settings form opened on the seeded candidate pool.
fn open_agent_settings_sets(
    temp: &Path,
    repo: &Path,
    pool: Vec<gwt::IssueMonitorLaunchProfile>,
) -> (AppRuntime, Arc<Mutex<Vec<UserEvent>>>, Vec<OutboundEvent>) {
    let mut seeded = gwt::IssueMonitorPrefs::default();
    seeded.set_launch_profile_pool(pool);
    gwt::save_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(repo), &seeded)
        .expect("seed pool");
    let tab = sample_project_tab("tab-1", "Repo", repo.to_path_buf(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) = sample_runtime_with_events(temp, vec![tab], Some("tab-1"));
    runtime.launch_wizard_cache = LaunchWizardMemoryCache::load_with_agent_options(
        &temp.join("sessions"),
        vec![
            agent_settings_option("codex", "Codex"),
            agent_settings_option("claude", "Claude Code"),
            agent_settings_option("grok", "Grok Build"),
        ],
    );
    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorConfigureProfile,
    );
    (runtime, recorded_events, events)
}

fn agent_settings_view(events: &[OutboundEvent]) -> &gwt::LaunchWizardView {
    events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::LaunchWizardState {
                wizard: Some(wizard),
            } => Some(wizard.as_ref()),
            _ => None,
        })
        .expect("launch wizard view")
}

fn agent_settings_set_agents(view: &gwt::LaunchWizardView) -> Vec<String> {
    view.issue_monitor_pool
        .as_ref()
        .expect("Agent Settings must list its sets")
        .sets
        .iter()
        .map(|set| set.agent_id.clone())
        .collect()
}

/// Settings → Runtime → Confirm → Save, as the operator drives it.
fn save_agent_settings_sets(
    runtime: &mut AppRuntime,
    recorded_events: &Arc<Mutex<Vec<UserEvent>>>,
) -> Vec<OutboundEvent> {
    save_agent_settings_sets_in(runtime, recorded_events, |_| {})
}

/// [`save_agent_settings_sets`] in a repository whose runtime context the
/// test describes, e.g. one that has a Compose service.
fn save_agent_settings_sets_in(
    runtime: &mut AppRuntime,
    recorded_events: &Arc<Mutex<Vec<UserEvent>>>,
    describe_repository: impl FnOnce(&mut gwt::LaunchWizardHydration),
) -> Vec<OutboundEvent> {
    runtime.handle_launch_wizard_action(&runtime.test_context(), LaunchWizardAction::Submit, None);
    wait_for_recorded_event(
        "agent settings runtime resolution",
        recorded_events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardRuntimeResolved { .. }
                )
            })
        },
    );
    let resolved_event = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardRuntimeResolved { .. }
                )
            })
            .map(|index| events.remove(index))
            .expect("runtime resolved event")
    };
    let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
        unreachable!("matched above")
    };
    let mut hydration = result.expect("runtime context resolves");
    describe_repository(&mut hydration);
    runtime.handle_launch_wizard_runtime_resolved(wizard_id, Ok(hydration));
    runtime.handle_launch_wizard_action(&runtime.test_context(), LaunchWizardAction::Submit, None);
    runtime.handle_launch_wizard_action(&runtime.test_context(), LaunchWizardAction::Submit, None)
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

fn commit_workspace_watcher_update(
    runtime: &mut AppRuntime,
    repo: &Path,
    projection: &gwt_core::workspace_projection::WorkspaceProjection,
) -> Vec<OutboundEvent> {
    let (proxy, recorded) = AppEventProxy::stub();
    let old_proxy = std::mem::replace(&mut runtime.proxy, proxy);
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    let old_spawner = std::mem::replace(&mut runtime.blocking_tasks, spawner);
    let mut events = runtime.handle_workspace_projection_changed_events(repo, projection);
    std::thread::spawn(move || drain_queued_blocking_tasks(&tasks))
        .join()
        .unwrap();
    loop {
        let next = {
            let mut recorded = recorded.lock().unwrap();
            if recorded.is_empty() {
                None
            } else {
                Some(recorded.remove(0))
            }
        };
        let Some(next) = next else {
            break;
        };
        match into_recorded_project_payload(next) {
            UserEvent::WorkspaceProjectionPatchPrepared(prepared) => {
                if let Some(dispatch) = runtime.apply_workspace_projection_patch(*prepared) {
                    let payload: serde_json::Value =
                        serde_json::from_str(&dispatch.payload).unwrap();
                    events.push(OutboundEvent::project(
                        dispatch.context.project_key,
                        BackendEvent::ActiveWorkProjectionPatch {
                            projection: Box::new(
                                serde_json::from_value(payload["projection"].clone()).unwrap(),
                            ),
                        },
                    ));
                }
            }
            UserEvent::ProjectDispatch {
                events: dispatched, ..
            } => events.extend(dispatched),
            other => panic!("unexpected watcher event: {other:?}"),
        }
    }
    runtime.proxy = old_proxy;
    runtime.blocking_tasks = old_spawner;
    events
}

/// Issue #4406 AC-3/AC-4: a project whose Workspace rail has one recorded Work
/// row, plus the event sink the off-loop refresh requests travel through.
fn active_work_off_loop_setup(
    temp_root: &Path,
    repo: &Path,
) -> (AppRuntime, Arc<Mutex<Vec<UserEvent>>>, String) {
    fs::create_dir_all(repo).expect("create repo");
    init_repo(repo);
    gwt_core::workspace_projection::record_workspace_work_event(repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            // A Work-item id shaped title is not a purpose, so the row has no
            // recorded summary and the tip-subject fallback — the AC-3 path —
            // is the only thing that can fill it.
            "work-offloop-a1b2c3",
            chrono::Utc::now(),
        );
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/off-loop".to_string()),
                worktree_path: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        event
    })
    .expect("record work");

    let mut tab_workspace = empty_workspace_state();
    let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent.title = "Codex".to_string();
    tab_workspace.windows.push(agent);
    tab_workspace.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.to_path_buf(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let (mut runtime, events) = sample_runtime_with_events(temp_root, vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/off-loop".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.to_path_buf(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    (runtime, events, window_id)
}

/// The off-loop refresh requests recorded for `project_root` so far.
/// Issue #3777 AC-2: an off-loop refresh is asked for through the profiled
/// broker, so a request becomes a background projection prepare completion.
/// Waits until `expected` of them have landed (or the deadline) and reports how
/// many did, so a caller can assert the exact count.
fn wait_for_active_work_prepare_completions(
    events: &Arc<Mutex<Vec<UserEvent>>>,
    expected: usize,
) -> usize {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let count = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::ActiveWorkProjectionPrepared(_)
                )
            })
            .count();
        if count >= expected || Instant::now() >= deadline {
            return count;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn active_work_refresh_requests(events: &Arc<Mutex<Vec<UserEvent>>>, project_root: &Path) -> usize {
    events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .filter(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::ActiveWorkProjectionChanged { project_root: root }
                    if root == project_root
            )
        })
        .count()
}

/// Issue #4406 AC-6: run the refresh the runtime just asked for and apply it,
/// the way the GUI event loop does. Tests that used to read the rail straight
/// out of a handler's return value drain it through here instead of weakening
/// what they assert.
fn drain_active_work_projection_refresh(
    runtime: &mut AppRuntime,
    project_root: &Path,
) -> Vec<OutboundEvent> {
    let Some(job) = runtime.active_work_projection_refresh_job(project_root) else {
        return Vec::new();
    };
    let refreshed = super::run_active_work_projection_refresh(job);
    runtime.apply_active_work_projection_refresh(refreshed)
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

/// Issue #3967: materialize a real linked git worktree so the launch-path
/// trust registration is exercised against the same `.codex/hooks.json` layout
/// Codex actually discovers (worktree-local plus workspace-home).
fn codex_hook_trust_linked_worktree_fixture(root: &Path) -> (PathBuf, PathBuf) {
    let repo = root.join("repo");
    fs::create_dir_all(&repo).expect("create repo dir");
    let git = |args: &[&str], cwd: &Path| {
        let status = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .unwrap_or_else(|error| panic!("git {args:?} failed to start: {error}"));
        assert!(status.success(), "git {args:?} failed with {status}");
    };
    git(&["init", "--initial-branch=develop"], &repo);
    git(&["config", "user.email", "test@example.com"], &repo);
    git(&["config", "user.name", "Test User"], &repo);
    // The repo-owned Stop hook is tracked content in this repository, so a
    // fresh worktree always carries it alongside the five managed hooks.
    fs::create_dir_all(repo.join(".codex")).expect("create .codex dir");
    fs::write(
        repo.join(".codex/hooks.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "hooks": {
                "Stop": [
                    {
                        "matcher": "*",
                        "hooks": [
                            {
                                "command": "gwt_bin=\"${GWT_BIN_PATH:-gwtd}\"; \"$gwt_bin\" hook gwt-self-improvement-stop 2>/dev/null || true",
                                "type": "command"
                            }
                        ]
                    }
                ]
            }
        }))
        .unwrap(),
    )
    .expect("seed tracked codex hooks");
    git(&["add", "-A"], &repo);
    git(&["commit", "-m", "seed"], &repo);

    let worktree = root.join("issue-worktree");
    let status = gwt_core::process::hidden_command("git")
        .args(["worktree", "add", "-b", "work/issue-3967"])
        .arg(&worktree)
        .arg("develop")
        .current_dir(&repo)
        .status()
        .expect("materialize linked worktree");
    assert!(status.success(), "git worktree add failed with {status}");
    (repo, worktree)
}

/// Every hook handler present in `hooks_path` must have a `[hooks.state]`
/// entry in `config`, otherwise Codex reports it as "new or changed" and
/// blocks the pane on `Hooks need review`.
fn assert_every_codex_hook_is_trusted(config: &toml::Value, hooks_path: &Path) {
    let events = [
        ("SessionStart", "session_start"),
        ("UserPromptSubmit", "user_prompt_submit"),
        ("PreToolUse", "pre_tool_use"),
        ("PostToolUse", "post_tool_use"),
        ("Stop", "stop"),
    ];
    // Issue #4071: Codex keys hooks by the plain absolute path, never the
    // Windows `\\?\` verbatim form `std::fs::canonicalize` returns.
    //
    // Issue #4879: ask the registration side for that form instead of
    // restating it. `dunce::canonicalize` stood here, which on macOS resolves a
    // `/var/...` launch path to `/private/var/...` — a key the product never
    // writes and Codex never reads.
    //
    // The caller must hand over a path that came from
    // `codex_hooks_paths_for_codex_discovery` rather than one joined by hand.
    // The two registered copies do not share a canonical form: the
    // worktree-local copy keeps the launch path as given, while the
    // workspace-home copy is reached through git's `gitdir`, which git wrote
    // canonically. No single rule applied to a hand-built path can match both,
    // which is why this helper derives nothing of its own.
    let canonical =
        gwt_skills::codex_hook_trust_key_path(hooks_path).expect("derive Codex hook trust key");
    let hooks: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(hooks_path).expect("read hooks.json"))
            .expect("parse hooks.json");
    let state = config
        .get("hooks")
        .and_then(|hooks| hooks.get("state"))
        .and_then(toml::Value::as_table)
        .unwrap_or_else(|| panic!("Codex config has no [hooks.state]: {config:?}"));
    let mut checked = 0usize;
    for (event_json, event_snake) in events {
        let Some(groups) = hooks["hooks"].get(event_json).and_then(|g| g.as_array()) else {
            continue;
        };
        for (group_index, group) in groups.iter().enumerate() {
            let handlers = group["hooks"].as_array().expect("hook handlers");
            for handler_index in 0..handlers.len() {
                let key = format!(
                    "{}:{event_snake}:{group_index}:{handler_index}",
                    canonical.display()
                );
                assert!(
                    state.contains_key(&key),
                    "hook {key} is not trusted; Codex would prompt. state keys: {:?}",
                    state.keys().collect::<Vec<_>>()
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "fixture must contain at least one hook");
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
        execution_diagnosis: None,
        updated_at: String::new(),
    }
}

fn workspace_test_work(
    agents: Vec<gwt::ActiveWorkAgentView>,
    works: Vec<gwt::ActiveWorkspaceWorkView>,
) -> gwt::ActiveWorkItemView {
    gwt::ActiveWorkItemView {
        linked_issue_numbers: Vec::new(),
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
        managed_hook_health: None,
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

/// Issue #4095 AC-1 fixture: an Ink-style renderer (Claude Code) redraws its
/// two dynamic lines with cursor-up + erase-line sequences while multi-line
/// tool output streams, cut into PTY-sized reads that ignore frame and escape
/// boundaries (only UTF-8 boundaries are honored; a read never splits a glyph
/// in this fixture).
fn spinner_redraw_pty_chunks() -> Vec<Vec<u8>> {
    const ERASE_DYNAMIC_LINES: &str = "\x1b[2K\x1b[1A\x1b[2K\x1b[G";
    const SPINNERS: [&str; 4] = ["✻", "✢", "✶", "✽"];
    const READ_SIZE: usize = 48;
    let dynamic = |tick: usize| {
        format!(
            "{} Frosting… (1h 57m {:02}s · ↓85.5k tokens) · esc to interrupt\r\n  Tip: Use /clear to start fresh when switching topics",
            SPINNERS[tick % SPINNERS.len()],
            tick % 60
        )
    };
    let mut stream = dynamic(0);
    for tick in 1..=24 {
        stream.push_str(ERASE_DYNAMIC_LINES);
        if tick % 4 == 0 {
            stream.push_str(&format!(
                "● Bash(sleep 570; gh run view 3408017659{tick} --repo akiojin/gwt --json status)\r\n  ⎿  Running… ({tick}m 43s · timeout 10m)\r\n     (ctrl+b to run in background)\r\n"
            ));
        }
        stream.push_str(&dynamic(tick));
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < stream.len() {
        let mut end = (start + READ_SIZE).min(stream.len());
        while !stream.is_char_boundary(end) {
            end -= 1;
        }
        chunks.push(stream.as_bytes()[start..end].to_vec());
        start = end;
    }
    chunks
}

/// Replay one client's drained queue into a fresh vt100 model the way
/// xterm.js applies it: a snapshot resets the terminal, output appends.
fn replay_client_terminal(queue: &ClientQueue) -> vt100::Parser {
    let mut client = vt100::Parser::new(24, 80, SNAPSHOT_SCROLLBACK_REPLAY_LIMIT);
    while let Some(step) = queue.try_next() {
        let DrainStep::Message { payload, .. } = step else {
            break;
        };
        let value: serde_json::Value = serde_json::from_str(&payload).expect("client payload");
        let Some(data) = value.get("data_base64").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .expect("terminal payload base64");
        match value.get("kind").and_then(serde_json::Value::as_str) {
            Some("terminal_snapshot") => {
                client = vt100::Parser::new(24, 80, SNAPSHOT_SCROLLBACK_REPLAY_LIMIT);
                client.process(&bytes);
            }
            Some("terminal_output") => client.process(&bytes),
            _ => {}
        }
    }
    client
}

// ---- SPEC-3431: resident PM pane lifecycle (T-010/T-011) ----

/// Opt this test's project out of the SPEC-3431 PM auto-start: the test
/// exercises startup/restore behavior that predates the resident PM pane and
/// its window/session counts intentionally exclude it.
fn disable_pm_auto_start(project_root: &Path) {
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(project_root);
    gwt::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
        prefs.settings.auto_start = false;
    })
    .expect("disable PM auto-start");
}

fn pm_registration_fixture(session_id: &str, worktree: &Path) -> gwt::pm_registry::PmRegistration {
    gwt::pm_registry::PmRegistration {
        session_id: session_id.to_string(),
        agent_id: "claude".to_string(),
        worktree_path: worktree.to_string_lossy().into_owned(),
        created_at: Some("2026-08-03T00:00:00Z".to_string()),
        consecutive_crashes: 0,
        next_not_before: None,
    }
}

fn create_detached_pm_worktree_fixture(repo: &Path) -> PathBuf {
    let worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(repo);
    fs::create_dir_all(worktree.parent().expect("PM worktree parent"))
        .expect("create PM directory");
    run_git(
        repo,
        &[
            "worktree",
            "add",
            "--detach",
            worktree.to_str().expect("PM worktree path"),
        ],
    );
    worktree
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = gwt_core::process::hidden_command("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git for stdout");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git stdout must be UTF-8")
        .trim()
        .to_string()
}

fn advance_origin_develop_by_one_commit(repo: &Path, origin: &Path) -> String {
    let seed = repo.parent().expect("repo parent").join("seed");
    fs::write(seed.join("UPSTREAM.md"), "origin/develop commit B\n")
        .expect("write upstream change");
    run_git(&seed, &["add", "UPSTREAM.md"]);
    run_git(&seed, &["commit", "-qm", "advance develop"]);
    run_git(
        &seed,
        &["push", origin.to_str().expect("origin path"), "develop"],
    );
    git_stdout(&seed, &["rev-parse", "HEAD"])
}

fn close_registered_pm_worktree_fixture(
    runtime_root: &Path,
    repo: &Path,
    pm_worktree: &Path,
    suffix: &str,
) {
    let tab_id = format!("tab-{suffix}");
    let raw_id = "agent-1";
    let tab = sample_project_tab_with_window_at(
        &tab_id,
        raw_id,
        repo.to_path_buf(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(runtime_root, vec![tab], Some(&tab_id));
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let window_id = format!("{tab_id}::{raw_id}");
    let session_id = format!("pm-session-{suffix}");
    let mut session = sample_active_agent_session(&tab_id, &window_id);
    session.session_id = session_id.clone();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(repo);
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture(&session_id, pm_worktree),
        |_| false,
    )
    .expect("seed PM registration");

    runtime.close_window_events(&window_id);
    loop {
        let next = finalizers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop();
        match next {
            Some(finalizer) => finalizer(),
            None => break,
        }
    }
}

/// Issue #3607 fixture: one repository whose stores split.
///
/// A repository with no `origin` falls back to a path hash, so its main
/// worktree and a linked worktree land in *different* project stores while
/// sharing one git common dir — the exact `b19aac…` / `99a866…` shape from the
/// incident, reproduced without depending on a stale on-disk store.
struct SplitStoreRepo {
    main: PathBuf,
    linked: PathBuf,
}

fn split_store_repo(root: &Path) -> SplitStoreRepo {
    let main = root.join("repo");
    fs::create_dir_all(&main).expect("create repo");
    init_repo_without_origin(&main);
    run_git(&main, &["config", "user.name", "Test User"]);
    run_git(&main, &["config", "user.email", "test@example.com"]);
    run_git(&main, &["commit", "--allow-empty", "-m", "init"]);
    let linked = root.join("linked");
    run_git(
        &main,
        &[
            "worktree",
            "add",
            "-b",
            "work/split",
            linked.to_str().expect("linked path"),
        ],
    );
    assert_ne!(
        gwt_core::paths::project_scope_hash(&main).as_str(),
        gwt_core::paths::project_scope_hash(&linked).as_str(),
        "fixture must reproduce a split: the two roots need different stores"
    );
    SplitStoreRepo { main, linked }
}

/// Materialize `<gwt projects dir>/<hash>/pm/worktree` for a store that is not
/// the one under test, so restore has a real foreign PM worktree to refuse.
fn foreign_pm_worktree(store_hash: &str) -> PathBuf {
    let worktree = gwt_core::paths::gwt_projects_dir()
        .join(store_hash)
        .join("pm/worktree");
    fs::create_dir_all(&worktree).expect("create foreign PM worktree");
    worktree
}

fn assert_pm_ensure_spawns_from_default_branch(default_branch: &str, cached_default: bool) {
    let _pm_gate = super::pm::test_gate::PmEnsureTestGuard::enable();
    // FR-001/FR-002: no registration + auto_start default ON => silent spawn
    // with a pending PM marker so launch completion can register the session.
    // Each test caller owns the process environment lock for this fixture.
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_default_branch(&repo, default_branch);
    if !cached_default {
        run_git(
            &repo,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/obsolete",
            ],
        );
    }
    // #4484: a tracked project plugin link must survive PM regeneration.
    #[cfg(unix)]
    {
        fs::create_dir_all(repo.join(".claude/agents")).unwrap();
        std::os::unix::fs::symlink("../../README.md", repo.join(".claude/agents/project.md"))
            .unwrap();
        run_git(&repo, &["add", ".claude/agents/project.md"]);
        fs::create_dir_all(repo.join(".claude/commands")).unwrap();
        std::os::unix::fs::symlink("../../README.md", repo.join(".claude/commands/release.md"))
            .unwrap();
        run_git(&repo, &["add", ".claude/commands/release.md"]);
        run_git(&repo, &["commit", "-qm", "track project agent symlink"]);
        run_git(&repo, &["push", "origin", default_branch]);
    }
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.ensure_pm_agent_for_tab("tab-1", super::pm::PmEnsureTrigger::Automatic);

    let events = drain_pm_worktree_preparation(&mut runtime, &recorded_events);

    assert!(!events.is_empty(), "fresh spawn emits workspace events");
    let windows = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .clone();
    assert_eq!(windows.len(), 1, "exactly one PM pane spawned");
    assert_eq!(windows[0].preset, WindowPreset::Agent);
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_launches
            .len(),
        1,
        "pending PM marker tracks the launch for registration at completion"
    );
    assert!(runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .pending_pm_launches
        .values()
        .all(|project_root| project_root == &repo));
    // T-052: the spawn targets the canonical PM worktree, which is what makes
    // the $gwt-pm bootstrap prompt resolvable — materialization keys on that
    // path. The materialization contract itself is owned by
    // crates/gwt/tests/managed_assets_test.rs
    // (pm_worktree_keeps_gwt_pm_guidance_after_asset_distribution); asserting
    // the skill file here would only observe the state before the launch
    // thread's asset refresh runs, which is exactly how the prune regression
    // stayed invisible.
    let pm_worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo);
    assert!(
        pm_worktree.join(".git").exists(),
        "the PM must spawn in its canonical worktree at {}",
        pm_worktree.display()
    );
    let scratch = gwt::pm_registry::pm_scratch_dir_for_repo_path(&repo);
    assert!(
        !scratch.starts_with(&pm_worktree),
        "PM spawn preparation must keep scratch outside the disposable worktree"
    );
    assert!(
        scratch.is_dir(),
        "PM spawn preparation must create the project-state scratch directory at {}",
        scratch.display()
    );
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    let prefs = gwt::pm_registry::load_pm_prefs(&prefs_path).unwrap();
    assert!(prefs.settings.auto_start);
    assert_eq!(
        prefs
            .worktree_freshness
            .as_ref()
            .map(|state| state.base_ref.as_str()),
        Some(format!("origin/{default_branch}").as_str())
    );
    assert_eq!(
        prefs.worktree_freshness.as_ref().map(|state| state.state),
        Some(gwt::pm_registry::PmWorktreeFreshnessState::Fresh)
    );
    #[cfg(unix)]
    assert_eq!(
        fs::read_link(pm_worktree.join(".claude/agents/project.md")).unwrap(),
        PathBuf::from("../../README.md")
    );
    // Exercise the launch-completion registration after the automatic preparation.
    #[cfg(unix)]
    assert_eq!(
        fs::read_link(pm_worktree.join(".claude/commands/release.md")).unwrap(),
        PathBuf::from("../../README.md")
    );
    runtime.register_pm_after_launch(&repo, "pm-project-symlink", "claude", &pm_worktree);
    assert_eq!(
        gwt::pm_registry::load_pm_prefs(&prefs_path)
            .unwrap()
            .registration
            .map(|registration| registration.session_id),
        Some("pm-project-symlink".to_string())
    );
}

/// Issue #4375: run the completion of the PM worktree preparation the runtime
/// just handed to a blocking worker, so a test can observe the spawn it gates.
fn drain_pm_worktree_preparation(
    runtime: &mut AppRuntime,
    recorded_events: &Arc<Mutex<Vec<UserEvent>>>,
) -> Vec<OutboundEvent> {
    wait_for_recorded_event("PM worktree preparation", recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::PmWorktreePrepared { .. }
            )
        })
    });
    let prepared = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::PmWorktreePrepared { .. }
                )
            })
            .map(|index| events.remove(index))
            .expect("PM worktree preparation event")
    };
    assert!(
        matches!(&prepared, UserEvent::ProjectCompletion { .. }),
        "PM worktree completion must carry its project generation"
    );
    let UserEvent::PmWorktreePrepared {
        continuation,
        result,
    } = into_recorded_project_payload(prepared)
    else {
        unreachable!("matched above")
    };
    runtime.handle_pm_worktree_prepared(*continuation, result)
}

fn assert_pm_refresh_failure_restores_old_checkout_and_assets(
    temp: &Path,
    fail_tree_transition: bool,
) {
    let repo = temp.join("repo");
    let origin = init_git_clone_with_origin(&repo);
    let seed = temp.join("seed");
    let pm_worktree = create_detached_pm_worktree_fixture(&repo);
    let old_head = git_stdout(&pm_worktree, &["rev-parse", "HEAD"]);
    let relative = ".claude/skills/gwt-agent/SKILL.md";
    for root in [&pm_worktree, &seed] {
        fs::create_dir_all(root.join(relative).parent().unwrap()).unwrap();
    }
    fs::write(pm_worktree.join(relative), "original generated bytes\n").unwrap();
    fs::write(seed.join(relative), "incoming tracked bytes\n").unwrap();
    if fail_tree_transition {
        // Not a managed asset, so the refresh transaction does not displace it:
        // the fast-forward aborts on the collision with HEAD still in place.
        fs::write(seed.join("UPSTREAM.md"), "incoming upstream bytes\n").unwrap();
        fs::write(pm_worktree.join("UPSTREAM.md"), "untracked PM bytes\n").unwrap();
    } else {
        // A directory where the merged hook config must be read and replaced
        // fails on every platform. It is not a pruned `gwt-*` entry, so the
        // distribution plan cannot clear it before the writer reaches it.
        fs::create_dir_all(
            pm_worktree
                .parent()
                .unwrap()
                .join("runtime/.claude/settings.local.json"),
        )
        .unwrap();
    }
    run_git(&seed, &["add", "--force", "--", ".claude"]);
    if fail_tree_transition {
        run_git(&seed, &["add", "--", "UPSTREAM.md"]);
    }
    run_git(
        &seed,
        &["commit", "-qm", "track managed asset and failure fixture"],
    );
    run_git(&seed, &["push", origin.to_str().unwrap(), "develop"]);

    let refresh = gwt::pm_registry::refresh_pm_worktree_for_repo_path(&repo);

    assert_eq!(
        git_stdout(&pm_worktree, &["rev-parse", "HEAD"]),
        old_head,
        "failed refresh must restore its previous commit: {refresh:?}"
    );
    assert_eq!(
        fs::read_to_string(pm_worktree.join(relative)).unwrap(),
        "original generated bytes\n"
    );
    let freshness =
        gwt::pm_registry::load_pm_prefs(&gwt::pm_registry::pm_prefs_path_for_repo_path(&repo))
            .unwrap()
            .worktree_freshness
            .unwrap();
    assert_eq!(
        freshness.failure_stage,
        Some(if fail_tree_transition {
            gwt::pm_registry::PmWorktreeRefreshFailureStage::Repoint
        } else {
            gwt::pm_registry::PmWorktreeRefreshFailureStage::ManagedAssets
        })
    );
}

/// Commit one PM change in `worktree` and return its SHA.
fn commit_pm_worktree_change(worktree: &Path, file: &str, contents: &str, message: &str) -> String {
    fs::write(worktree.join(file), contents).expect("write PM change");
    run_git(worktree, &["add", file]);
    run_git(worktree, &["commit", "-qm", message]);
    git_stdout(worktree, &["rev-parse", "HEAD"])
}

// ---- SPEC-3431 T-093: daemon wake path for the resident PM loop ----

fn pm_wake_inbox_item(number: u64, state: gwt::MonitorInboxState) -> gwt::IssueMonitorInboxItem {
    gwt::IssueMonitorInboxItem {
        issue: gwt::IssueMonitorIssue {
            number,
            title: format!("Issue {number}"),
            labels: vec!["auto-merge".to_string()],
            state: gwt::IssueMonitorIssueState::Open,
            body: None,
            url: None,
            readiness: gwt::IssueMonitorReadiness::NotApplicable,
            updated_at: None,
        },
        state,
        claim_id: None,
        blocked_by_owner: None,
        claim_expires_at: None,
        blocked_by_claim_id: None,
        claim_block_issue_updated_at: None,
        launched_window_id: None,
        launch_plan: None,
        error_message: None,
        exclusion_reason: None,
    }
}

/// Repo with an enabled Issue Monitor, a registered PM whose pane is live,
/// and a second non-PM pane that the wake must never reach.
pub(super) fn pm_wake_fixture(temp: &tempfile::TempDir) -> (PathBuf, AppRuntime, String) {
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let monitor_prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &monitor_prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed monitor prefs");

    let mut persisted = empty_workspace_state();
    persisted.windows.push(sample_window(
        "pm-window",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    ));
    persisted.windows.push(sample_window(
        "other-window",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    ));
    persisted.next_z_index = 3;
    let mut tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    assert!(tab
        .workspace
        .set_session_id("pm-window", Some("pm-session-live".to_string())));
    assert!(tab
        .workspace
        .set_session_id("other-window", Some("other-session".to_string())));
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let pm_window_id = "tab-1::pm-window".to_string();
    let mut pm_session = sample_active_agent_session("tab-1", &pm_window_id);
    pm_session.session_id = "pm-session-live".to_string();
    runtime
        .active_agent_sessions
        .insert(pm_window_id.clone(), pm_session);
    let other_window_id = "tab-1::other-window".to_string();
    let mut other_session = sample_active_agent_session("tab-1", &other_window_id);
    other_session.session_id = "other-session".to_string();
    runtime
        .active_agent_sessions
        .insert(other_window_id, other_session);

    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&repo);
    gwt::pm_registry::try_register_pm(
        &prefs_path,
        pm_registration_fixture("pm-session-live", &repo),
        |_| false,
    )
    .expect("seed registration");

    (repo, runtime, pm_window_id)
}

fn drain_pm_wake_delivery_tasks(runtime: &mut AppRuntime) {
    let BlockingTaskSpawner::Queued(tasks) = &runtime.blocking_tasks else {
        panic!("PM wake fixture must use queued workers");
    };
    let tasks = Arc::clone(tasks);
    let AppEventProxy::Stub(completions) = &runtime.proxy else {
        panic!("PM wake fixture must record completions");
    };
    let completions = Arc::clone(completions);
    loop {
        let queued = std::mem::take(&mut *tasks.lock().unwrap());
        if queued.is_empty() {
            break;
        }
        for task in queued {
            task();
        }
        let completed = std::mem::take(&mut *completions.lock().unwrap());
        for event in completed {
            if let Some(UserEvent::PmWakeDeliveryComplete(delivery)) =
                runtime.accept_project_completion(event)
            {
                runtime.pm_wake_delivery_complete(delivery);
            }
        }
    }
}

/// Issue #3632 AC-3/AC-6 (user ruling 2026-08-17): neither wake prompt may
/// order a report unconditionally.
///
/// The gwt-pm contract has always been milestone-only, but both wake prompts
/// closed with "report the milestone digest" — injected text outranks the skill
/// body, so every scheduled tick produced a digest whether or not anything had
/// changed. The wording is pinned here so a reinstated unconditional order
/// fails the suite instead of shipping.
fn assert_wake_prompt_reports_only_on_change(prompt: &str, label: &str) {
    assert!(
        prompt.contains(gwt::pm_registry::PM_CYCLE_REPORTING_CLAUSE),
        "{label} must carry the shared conditional-reporting clause; got: {prompt}"
    );
    assert!(
        !prompt.contains("report the milestone digest"),
        "{label} must not order a digest unconditionally; got: {prompt}"
    );
    assert!(
        prompt.contains("issue.monitor.status"),
        "{label} must still drive one full reconcile cycle (FR-3); got: {prompt}"
    );
    assert!(
        prompt.contains(gwt::pm_registry::PM_GWTD_EXECUTION_WAKE_CLAUSE),
        "{label} must carry the canonical gwtd execution-isolation clause verbatim; got: {prompt}"
    );
    assert!(
        !prompt.contains("contract's 10-second outer deadline"),
        "{label} must not retain the superseded ten-second ceiling; got: {prompt}"
    );
    // Issue #3825 AC-1 / AC-4: the supervision tick must not hand the PM a
    // subscribe that blocks for the whole loop interval. Five seconds is the
    // ceiling for one call, and the tick never waits on it at all.
    for phrase in [
        "contract's 5-second outer deadline",
        "`params.timeout_seconds:5`",
        "background task",
        "do not wait for it",
    ] {
        assert!(
            prompt.contains(phrase),
            "{label} must carry the nonblocking subscribe contract `{phrase}`; got: {prompt}"
        );
    }
    assert!(
        !prompt.contains("`params.timeout_seconds:60`"),
        "{label} must not restore the 60-second blocking subscribe (#3825); got: {prompt}"
    );
    assert!(
        prompt.contains("`pr.list`"),
        "{label} must inventory open PRs each cycle (Issue #3781); got: {prompt}"
    );
    assert!(
        prompt.contains("never auto-close"),
        "{label} must keep close proposals in the digest (Issue #3781); got: {prompt}"
    );
    // Issue #3767 AC-2 / AC-3: both prompts steer the running launches through
    // the ruling channels before the no-change judgment.
    assert!(
        prompt.contains(gwt::pm_registry::PM_STEERING_WAKE_CLAUSE),
        "{label} must carry the steering clause verbatim (Issue #3767); got: {prompt}"
    );
    // Issue #3868 / #3825: both prompts are written into the PM pane's PTY,
    // whose canonical queue is 1024 bytes on macOS. A longer prompt does not
    // fail — the writer blocks forever and the whole suite hangs with it.
    // Keep a margin for the composer line the wake can be submitted behind.
    const PTY_CANONICAL_QUEUE_BYTES: usize = 1024;
    const COMPOSER_LINE_MARGIN_BYTES: usize = 64;
    assert!(
        prompt.len() + COMPOSER_LINE_MARGIN_BYTES <= PTY_CANONICAL_QUEUE_BYTES,
        "{label} is {} bytes; it must stay at or under {} bytes so the PTY write cannot block \
         (#3825); got: {prompt}",
        prompt.len(),
        PTY_CANONICAL_QUEUE_BYTES - COMPOSER_LINE_MARGIN_BYTES
    );
}

/// Issue #3655 AC-5 / AC-9: the blocker text has to travel with the wake.
///
/// The production failure: the Board showed the routine "ready for the next
/// instruction" line and nothing else, so the PM had no way to tell a finished
/// agent from one that had concluded it could not proceed, and resorted to
/// reading panes one at a time — the channel that fails under GUI event-loop
/// saturation. This reads the escalation index instead, so no pane is involved.
fn seed_open_escalation(repo: &std::path::Path, owner: &str, body: &str) -> String {
    let entry = gwt_core::coordination::BoardEntry::new(
        gwt_core::coordination::AuthorKind::Agent,
        "Claude Code",
        gwt_core::coordination::BoardEntryKind::Blocked,
        body,
        None,
        None,
        vec![],
        vec![owner.to_string()],
    );
    let id = entry.id.clone();
    gwt_core::coordination::post_entry(repo, entry).expect("post escalation");
    id
}

fn seed_quiet_standing_supervision(repo: &Path) {
    let monitor_prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(repo);
    let mut monitor = gwt::IssueMonitorState::with_prefs(
        gwt::IssueMonitorConfig {
            enabled: true,
            max_active: 2,
            ..gwt::IssueMonitorConfig::default()
        },
        gwt::load_issue_monitor_prefs(&monitor_prefs_path).expect("prefs"),
    );
    gwt::scan_issue_monitor_candidates(
        &mut monitor,
        &[pm_wake_inbox_item(42, gwt::MonitorInboxState::Queued).issue],
        "2026-08-10T00:00:00Z",
    );
    monitor.complete_active_launch(42, "tab-1::other-window");
    gwt::save_issue_monitor_prefs(&monitor_prefs_path, &monitor.prefs()).expect("save prefs");

    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(repo);
    gwt::pm_registry::save_pm_loop_state(
        &loop_path,
        &gwt::pm_registry::PmLoopState {
            consecutive_continuations: 12,
            last_continued_at: Some("2026-08-10T00:00:00Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("seed quiet loop");
}

/// Own the fixture's child independently of runtime/finalizer Arc clones.
/// Drain output as well as input: an unread macOS PTY can stall child exit.
pub(super) struct TestPaneGuard {
    pty: Arc<gwt_terminal::PtyHandle>,
    reader: Option<thread::JoinHandle<()>>,
}

impl TestPaneGuard {
    fn new(pane: &Pane) -> Self {
        let pty = pane.shared_pty();
        let mut reader = pane.reader().expect("test PTY reader");
        let reader = thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            while std::io::Read::read(&mut reader, &mut buffer).is_ok_and(|n| n > 0) {}
        });
        #[cfg(windows)]
        pty.write_input(b"\x1b[1;1R").expect("ConPTY cursor reply");
        Self {
            pty,
            reader: Some(reader),
        }
    }
}

impl Drop for TestPaneGuard {
    fn drop(&mut self) {
        let _ = self.pty.kill();
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if self.pty.try_wait().is_ok_and(|status| status.is_some()) {
                // ConPTY holds its output pipe open until the master is closed.
                self.pty.release_descriptors();
                if self
                    .reader
                    .as_ref()
                    .is_none_or(|reader| reader.is_finished())
                {
                    if let Some(reader) = self.reader.take() {
                        reader.join().expect("test PTY reader");
                    }
                    return;
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
        if !thread::panicking() {
            panic!(
                "test PTY {:?} was not reaped and drained",
                self.pty.process_id()
            );
        }
    }
}

pub(super) fn attach_live_pm_pane(runtime: &mut AppRuntime, window_id: &str) -> TestPaneGuard {
    runtime.blocking_tasks = BlockingTaskSpawner::queued().0;
    let (command, args) = if cfg!(windows) {
        (
            "powershell",
            vec![
                "-NoProfile",
                "-Command",
                "while ($null -ne [Console]::ReadLine()) {}",
            ],
        )
    } else {
        // Raw input avoids the canonical queue boundary even before a submit.
        (
            "/bin/sh",
            vec!["-c", "stty -echo -icanon; exec cat >/dev/null"],
        )
    };
    let pane = Pane::new(
        window_id.to_string(),
        command.to_string(),
        args.into_iter().map(str::to_string).collect(),
        80,
        24,
        HashMap::new(),
        test_pane_cwd(),
    )
    .expect("draining PM test pane");
    let guard = TestPaneGuard::new(&pane);
    insert_test_pane_runtime_with_pane(runtime, window_id, pane);
    guard
}

fn assert_pm_pane_is_not_in_protected_inject(runtime: &AppRuntime, window_id: &str) {
    let pty = runtime
        .runtimes
        .get(window_id)
        .expect("live PM pane")
        .pane
        .lock()
        .expect("pane lock")
        .shared_pty();
    let reservation = pty
        .reserve_input_transaction()
        .expect("a composing PM pane must not already have a protected wake inject in flight");
    drop(reservation);
}

/// Issue #3528 driver fixture: an enabled autonomous monitor whose live issue
/// list (fake `gh`) carries one claimable candidate, #43.
fn seed_scheduled_scan_claim_fixture(
    temp: &tempfile::TempDir,
    launch_profile: Option<gwt::IssueMonitorLaunchProfile>,
) -> (PathBuf, PathBuf) {
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            launch_profile,
            ..queued_issue_monitor_prefs(&[43])
        },
    )
    .expect("seed enabled prefs");
    (repo, prefs_path)
}

fn pending_claim_issue_numbers(prefs: &gwt::IssueMonitorPrefs) -> Vec<u64> {
    prefs
        .pending_effects
        .iter()
        .filter_map(|effect| match effect.payload {
            gwt::IssueMonitorEffectPayload::AcquireClaim { issue_number, .. } => Some(issue_number),
            _ => None,
        })
        .collect()
}

// Issue #3863 AC-6: every wizard-open path must inject the Hermes launch
// choices enumerated from the user's global Hermes home. Missing one path
// would leave that entry point with empty candidates only.
fn seed_hermes_home_with_profile(root: &Path) -> PathBuf {
    let hermes_home = root.join("hermes-home");
    fs::create_dir_all(&hermes_home).expect("hermes home");
    fs::write(
        hermes_home.join("config.yaml"),
        "model:\n  provider: zai\n  default: glm-5.2\nagent:\n  personalities:\n    concise: Be brief.\n",
    )
    .expect("hermes config");
    hermes_home
}

fn assert_hermes_choices_injected(view: &gwt::LaunchWizardView, path: &str) {
    assert_eq!(
        view.hermes_profile_options,
        vec!["concise".to_string()],
        "{path}: Hermes profile choices must come from HERMES_HOME config"
    );
    assert_eq!(
        view.hermes_model_options,
        vec!["glm-5.2".to_string()],
        "{path}: Hermes model choices must follow the config default provider"
    );
}

fn codex_usage_account(
    weekly_used_percent: f32,
    limit_reached: bool,
) -> gwt_core::usage::ProviderUsage {
    gwt_core::usage::ProviderUsage {
        provider: gwt_core::usage::UsageProvider::Codex,
        account_id: None,
        account_label: None,
        plan: None,
        windows: vec![gwt_core::usage::UsageWindow::new(
            gwt_core::usage::WindowKind::Weekly,
            weekly_used_percent,
            Some(instant("2026-09-07T03:58:00Z")),
        )],
        limit_reached,
        state: gwt_core::usage::UsageState::Ok,
        fetched_at: None,
    }
}

/// Issue #3489: build the durable Session seed the Continue work fixtures use
/// for owner-linkage regression coverage.
fn issue_3489_durable_seed(linked_issue_number: Option<u64>) -> ContinueWorkLaunchSeed {
    let mut session = gwt_agent::Session::new(
        Path::new("/tmp/gwt-issue-3489"),
        "work/issue-3489",
        gwt_agent::AgentId::Codex,
    );
    session.id = "durable-3489".to_string();
    session.repo_hash = Some("repo-3489".to_string());
    session.linked_issue_number = linked_issue_number;
    ContinueWorkLaunchSeed::DurableSession(Box::new(session))
}

/// Issue #3489: mint the binding Continue work installs for the Work owner and
/// prove the launch Session accepts it. `set_execution_binding` runs before the
/// PTY starts, so a rejection here is exactly the pre-PTY launch failure.
fn issue_3489_binding_installs(config: &gwt_agent::LaunchConfig, owner_number: u64) -> bool {
    let mut session = gwt_agent::Session::new(
        Path::new("/tmp/gwt-issue-3489"),
        "work/issue-3489",
        gwt_agent::AgentId::Codex,
    );
    session.id = "continuation-3489".to_string();
    session.repo_hash = Some("repo-3489".to_string());
    session.linked_issue_number = config.linked_issue_number;
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session.id.clone(),
        repo_hash: "repo-3489".to_string(),
        owner_kind: "issue".to_string(),
        owner_number,
        identity: gwt_agent::ExecutionBindingIdentity {
            generation_id: "generation-3489".to_string(),
            binding_id: "binding-3489".to_string(),
            ledger_head_hash: "head-3489".to_string(),
        },
        capability_generation: 1,
    };
    session.set_execution_binding(Some(binding)).is_ok()
}

// Issue #4038 (AC-4 / AC-7): a resume marker whose `to_version` matches the
// running build settles at bootstrap — the apply result is recorded, sessions
// of the marker's projects bypass the 24h freshness gate, the frontend gets a
// notification-center record once the canvas is ready, and the marker is
// gone so a second bootstrap is a no-op.
fn update_resume_fixture(temp: &Path, branch: &str) -> (PathBuf, AppRuntime) {
    let repo = temp.join("repo");
    init_git_clone_with_origin(&repo);
    let worktree = temp.join("worktrees").join(branch.replace('/', "-"));
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            branch,
            worktree.to_str().expect("worktree path"),
        ],
    );
    let tab = sample_project_tab(
        "tab-update",
        "Update Resume",
        worktree.clone(),
        ProjectKind::Git,
        &[],
    );
    let runtime = sample_runtime(temp, vec![tab], Some("tab-update"));
    let mut session = gwt_agent::Session::new(&worktree, branch, gwt_agent::AgentId::Codex);
    session.id = "session-update-resume".to_string();
    session.agent_session_id = Some("native-update-resume".to_string());
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    // Older than the 24h startup auto-resume window: skipped on a normal
    // launch, resumed when the launch is the tail of an update apply.
    session.last_activity_at = chrono::Utc::now() - chrono::Duration::hours(30);
    session
        .save(&runtime.sessions_dir)
        .expect("save stale resumable session");
    // Issue #4037: the apply raised the update drain for this project; the
    // settling bootstrap must release it whether or not the update landed.
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&worktree);
    fs::create_dir_all(prefs_path.parent().expect("prefs dir")).expect("create prefs dir");
    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path)
        .unwrap_or_else(|_| gwt::IssueMonitorPrefs::recovery_default());
    prefs.update_drain = Some(gwt::IssueMonitorUpdateDrain {
        version: "9.99.0".to_string(),
        since: "2026-09-07T00:00:00Z".to_string(),
        reason: gwt::IssueMonitorUpdateDrainReason::Auto,
        blocking: Vec::new(),
    });
    // Issue #3906 AC-4 / #4076 AC-3: the monitor state the apply left behind
    // (enabled, parallelism, the launches being drained) must come back
    // exactly, never reset the way the 2026-09-02 restart did (#3883).
    prefs.enabled = true;
    prefs.autonomous_mode = true;
    prefs.max_active_agents = 3;
    prefs.launched_issues = vec![gwt::IssueMonitorLaunchedIssue {
        issue_number: 4076,
        window_id: "tab-update::agent-4076".to_string(),
    }];
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed update_drain hold");
    (worktree, runtime)
}

fn update_drain_is_raised(worktree: &Path) -> bool {
    gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(worktree))
        .map(|prefs| prefs.update_drain.is_some())
        .unwrap_or(false)
}

fn update_resume_marker_for(
    worktree: &Path,
    to_version: &str,
) -> gwt_core::update::UpdateResumeMarker {
    gwt_core::update::UpdateResumeMarker {
        from_version: "0.0.0".to_string(),
        to_version: to_version.to_string(),
        started_at: "2026-09-07T00:00:00Z".to_string(),
        restart_args: Vec::new(),
        projects: vec![gwt_core::update::UpdateResumeProject {
            hash: gwt_core::paths::project_scope_hash(worktree).to_string(),
            update_drain: true,
        }],
        attempt: 1,
    }
}

fn update_resume_toasts(events: &[OutboundEvent]) -> Vec<(String, String)> {
    events
        .iter()
        .filter_map(|event| match &event.event {
            BackendEvent::IssueMonitorToast { level, message, .. } if message.contains("Updat") => {
                Some((level.clone(), message.clone()))
            }
            _ => None,
        })
        .collect()
}

fn restore_fixture_tab(
    tab_id: &str,
    repo: &Path,
    placeholders: &[(String, String)],
) -> ProjectTabRuntime {
    let mut persisted = empty_workspace_state();
    for (index, (window_id, session_id)) in placeholders.iter().enumerate() {
        let mut window =
            sample_window(window_id, WindowPreset::Agent, WindowProcessStatus::Stopped);
        window.agent_id = Some("codex".to_string());
        window.session_id = Some(session_id.clone());
        window.z_index = index as u32 + 1;
        persisted.windows.push(window);
    }
    persisted.next_z_index = placeholders.len() as u32 + 1;
    ProjectTabRuntime {
        id: tab_id.to_string(),
        title: "Repo".to_string(),
        project_root: repo.to_path_buf(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    }
}

fn save_restore_fixture_session(
    sessions_dir: &Path,
    session_id: &str,
    worktree: &Path,
    native_session_id: Option<&str>,
    linked_issue: Option<u64>,
) {
    fs::create_dir_all(worktree).expect("create restore fixture worktree");
    let mut session =
        gwt_agent::Session::new(worktree, "work/restore-fixture", gwt_agent::AgentId::Codex);
    session.id = session_id.to_string();
    session.agent_session_id = native_session_id.map(str::to_string);
    session.linked_issue_number = linked_issue;
    session.restore_window_on_startup = true;
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session
        .save(sessions_dir)
        .expect("save restore fixture session");
}

fn restore_admission_refusals(
    events: &[CapturedTracingEvent],
) -> std::collections::BTreeMap<String, String> {
    events
        .iter()
        .filter(|event| {
            event.fields.get("message").map(String::as_str) == Some("session restore refused")
        })
        .filter_map(|event| {
            Some((
                event.fields.get("session_id")?.clone(),
                event.fields.get("reason")?.clone(),
            ))
        })
        .collect()
}

fn restore_admission_summary(events: &[CapturedTracingEvent]) -> &CapturedTracingEvent {
    let summaries = events
        .iter()
        .filter(|event| {
            event.fields.get("message").map(String::as_str)
                == Some("session restore admission summary")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        summaries.len(),
        1,
        "AC-4 requires exactly one summary line per startup, got {summaries:?}"
    );
    summaries[0]
}

/// Issue #4441: backdate a restore fixture Session so it reads as history
/// rather than as a window that was open at the last exit.
fn age_restore_fixture_session(sessions_dir: &Path, session_id: &str, age: chrono::Duration) {
    let path = sessions_dir.join(format!("{session_id}.toml"));
    let mut session = gwt_agent::Session::load(&path).expect("load restore fixture session");
    session.last_activity_at = chrono::Utc::now() - age;
    session
        .save(sessions_dir)
        .expect("save aged fixture session");
}

fn assert_pm_delivery_refused(
    temp: &tempfile::TempDir,
    other_session: bool,
    pending_refusal: bool,
) {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    {
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let (repo, mut runtime, pm_window_id) = pm_wake_fixture(temp);
        insert_test_pane_runtime(&mut runtime, &pm_window_id);
        runtime.register_pty_writer(&pm_window_id, None);
        let target = if other_session {
            let target = "tab-1::other-window".to_string();
            let worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo);
            fs::create_dir_all(&worktree).expect("PM worktree");
            runtime
                .active_agent_sessions
                .get_mut(&target)
                .unwrap()
                .worktree_path = worktree;
            insert_test_pane_runtime(&mut runtime, &target);
            runtime.register_pty_writer(&target, None);
            target
        } else {
            pm_window_id
        };
        let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:43123/internal/hook-live",
            "ws://127.0.0.1:43124/ws",
            "ws://127.0.0.1:43123/internal/pane-ws",
        );
        let capability = issuer.issue(&repo, "pm-session-live").expect("capability");
        let grant = issuer.grant_for_test(&capability.token).expect("grant");
        let operation_id = uuid::Uuid::new_v4().to_string();
        let pending = pending_refusal.then(|| {
            let path = gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&repo);
            let hash = gwt::pm_registry::pm_delivery_prompt_sha256("do not deliver to the PM");
            let receipt = gwt::pm_registry::PmDeliveryReceipt {
                operation_id: operation_id.clone(),
                recorded_at: Utc::now().to_rfc3339(),
                status: gwt::pm_registry::PmDeliveryReceiptStatus::Prepared,
                principal_session_id: "pm-session-live".to_string(),
                target_window_id: target.clone(),
                target_session_id: "pm-session-live".to_string(),
                body_sha256: hash.clone(),
                reason: None,
            };
            gwt::pm_registry::prepare_pm_delivery_receipt(&path, &receipt).unwrap();
            (path, receipt, hash)
        });
        let (responder, mut response, cancellation) =
            AgentPmSendResponder::channel_with_acceptance_window(Duration::from_secs(2));
        runtime.authenticated_pm_pane_send_input_events(
            &issuer,
            "client".to_string(),
            grant,
            &operation_id,
            &target,
            "do not deliver to the PM\r",
            Some(responder),
        );
        if let Some((path, receipt, hash)) = pending {
            assert!(
                matches!(
                    response.try_recv(),
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty)
                ),
                "replayed Prepared delivery must remain pending before its refusal receipt"
            );
            assert_eq!(
                gwt::pm_registry::pm_delivery_receipt_for_operation(&path, &receipt.operation_id)
                    .expect("pending receipt")
                    .expect("Prepared operation")
                    .status,
                gwt::pm_registry::PmDeliveryReceiptStatus::Prepared
            );
            gwt::pm_registry::finish_pm_delivery_receipt(
                &path,
                &receipt.operation_id,
                &receipt.target_session_id,
                &hash,
                gwt::pm_registry::PmDeliveryReceiptStatus::Refused,
                Some(&format!(
                    "self-delivery refused for {}",
                    receipt.target_window_id
                )),
            )
            .expect("finish the exact pending refusal");
        }
        let response = response.blocking_recv().expect("response");
        assert!(
            matches!(&response, BackendEvent::PmMessageSendResult {status, reason: Some(reason), ..}
            if status == "refused" && reason.contains("self") && reason.contains(&target)),
            "{response:?}"
        );
        assert!(
            !cancellation.cancel(),
            "refusal must precede physical input commit"
        );
        let receipts = gwt::pm_registry::load_pm_delivery_receipts(
            &gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&repo),
        )
        .expect("receipts");
        let receipt = receipts.last().expect("refused receipt");
        assert_eq!(
            receipt.status,
            gwt::pm_registry::PmDeliveryReceiptStatus::Refused
        );
        assert!(receipt
            .reason
            .as_ref()
            .is_some_and(|reason| reason.contains("self") && reason.contains(&target)));
    }
}

pub(super) fn seed_pm_session_escalation(repo: &Path, session: &gwt_agent::Session, body: &str) {
    session
        .save(&gwt_core::paths::gwt_sessions_dir())
        .expect("save subject session");
    let mut entry = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Blocked,
        body,
        None,
        None,
        vec![],
        vec!["4274".to_string()],
    );
    entry.origin_session_id = Some(session.id.clone());
    post_entry(repo, entry).expect("post escalation");
}

include!("wizard_project_tests.rs");

include!("outbound_scope_tests.rs");

include!("project_request_tests.rs");

include!("window_project_scope_tests.rs");

include!("async_project_tests.rs");

include!("project_route_tests.rs");

// Handler unit tests inspect the payload; ingress-generation rejection is covered
// separately through accept_project_completion in async_project_tests.rs.
fn recorded_project_payload(event: &UserEvent) -> &UserEvent {
    match event {
        UserEvent::ProjectCompletion { event, .. } => recorded_project_payload(event),
        event => event,
    }
}

fn into_recorded_project_payload(event: UserEvent) -> UserEvent {
    match event {
        UserEvent::ProjectCompletion { event, .. } => into_recorded_project_payload(*event),
        event => event,
    }
}

include!("pm_project_state_tests.rs");

include!("project_owned_state_tests.rs");

include!("project_aggregate_tests.rs");

fn take_issue4803_monitor_preparation(
    events: &Arc<Mutex<Vec<UserEvent>>>,
) -> super::IssueMonitorLaunchPrepared {
    let mut events = events.lock().expect("recorded events");
    let index = events
        .iter()
        .position(|event| matches!(event, UserEvent::IssueMonitorLaunchPrepared(_)))
        .expect("monitor preparation completion");
    match events.remove(index) {
        UserEvent::IssueMonitorLaunchPrepared(prepared) => *prepared,
        _ => unreachable!(),
    }
}

fn commit_issue4803_monitor_preparation(
    runtime: &mut AppRuntime,
    queued: &BlockingTestTaskQueue,
    events: &Arc<Mutex<Vec<UserEvent>>>,
) -> Vec<OutboundEvent> {
    drain_queued_blocking_tasks(queued);
    let started = Instant::now();
    let result =
        runtime.handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(events));
    eprintln!("monitor fresh completion: {:?}", started.elapsed());
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "fresh completion blocked the GUI: {:?}",
        started.elapsed()
    );
    result
}

/// Issue #4868: the Rust 1.99 rewrite replaced `fetch_update` + `checked_add`
/// with `fetch_add`, which wraps where the old form refused to. These pin the
/// two properties callers depend on: the value returned is the one claimed
/// (not its successor), and a counter that can no longer promise a successor
/// panics instead of wrapping around to a value already in use.
mod incarnation_claiming {
    use std::sync::atomic::AtomicU64;

    #[test]
    fn claiming_returns_the_current_value_and_advances_by_one() {
        let counter = AtomicU64::new(1);
        assert_eq!(super::super::claim_incarnation(&counter), 1);
        assert_eq!(super::super::claim_incarnation(&counter), 2);
        assert_eq!(super::super::claim_incarnation(&counter), 3);
    }

    #[test]
    fn claiming_panics_instead_of_wrapping_when_the_space_is_exhausted() {
        let counter = AtomicU64::new(u64::MAX);
        let exhausted =
            std::panic::catch_unwind(|| super::super::claim_incarnation(&counter)).is_err();
        assert!(
            exhausted,
            "a counter that cannot promise a successor must panic, not hand out a reused value"
        );
    }
}

// Runtime acceptance tests, grouped by behavior. Shared fixtures stay above.
#[cfg(test)]
mod board_tests;
#[cfg(test)]
mod continuation_authority_tests;
#[cfg(test)]
mod continuation_completion_tests;
#[cfg(test)]
mod execution_genesis_tests;
#[cfg(test)]
mod execution_readiness_tests;
#[cfg(test)]
mod execution_recovery_tests;
#[cfg(test)]
mod frontend_launch_tests;
#[cfg(test)]
mod issue_monitor_claim_tests;
#[cfg(test)]
mod issue_monitor_handoff_tests;
#[cfg(test)]
mod issue_monitor_scan_tests;
#[cfg(test)]
mod issue_monitor_settings_tests;
#[cfg(test)]
mod knowledge_tests;
#[cfg(test)]
mod managed_codex_config_tests;
#[cfg(test)]
mod managed_launch_tests;
#[cfg(test)]
mod monitor_feedback_tests;
#[cfg(test)]
mod pane_control_tests;
#[cfg(test)]
mod pm_context_tests;
#[cfg(test)]
mod pm_delivery_tests;
#[cfg(test)]
mod pm_registration_work_tests;
#[cfg(test)]
mod pm_wake_tests;
#[cfg(test)]
mod profiling_tests;
#[cfg(test)]
mod project_navigation_tests;
#[cfg(test)]
mod registry_sessions_tests;
#[cfg(test)]
mod restore_launch_tests;
#[cfg(test)]
mod runtime_contract_tests;
#[cfg(test)]
mod startup_status_tests;
#[cfg(test)]
mod terminal_approval_tests;
#[cfg(test)]
mod terminal_lifecycle_tests;
#[cfg(test)]
mod ui_trace_attachment_tests;
#[cfg(test)]
mod window_expiry_startup_tests;
#[cfg(test)]
mod work_projection_tests;
#[cfg(test)]
mod workspace_resume_tests;
#[cfg(test)]
mod workspace_watcher_tests;

fn drain_runtime_hook_agent_failure(
    runtime: &mut AppRuntime,
    queued_tasks: &BlockingTestTaskQueue,
    recorded_events: &Arc<Mutex<Vec<UserEvent>>>,
) -> Vec<OutboundEvent> {
    drain_queued_blocking_tasks(queued_tasks);
    let mut outbound = Vec::new();
    loop {
        let completion = {
            let mut events = recorded_events.lock().expect("recorded events");
            events
                .iter()
                .position(|event| {
                    matches!(
                        recorded_project_payload(event),
                        UserEvent::RuntimeHookAgentFailurePrepared(_)
                            | UserEvent::IssueMonitorDaemonStatus { .. }
                            | UserEvent::IssueMonitorDaemonInbox { .. }
                    )
                })
                .map(|index| events.remove(index))
        };
        let Some(completion) = completion else {
            break;
        };
        match runtime.accept_project_completion(completion) {
            Some(UserEvent::RuntimeHookAgentFailurePrepared(prepared)) => {
                outbound.extend(runtime.handle_runtime_hook_agent_failure_prepared(*prepared));
            }
            Some(UserEvent::IssueMonitorDaemonStatus {
                project_root,
                status,
            }) => {
                outbound.extend(runtime.issue_monitor_daemon_status_events(&project_root, status));
            }
            Some(UserEvent::IssueMonitorDaemonInbox {
                project_root,
                items,
            }) => {
                outbound.extend(runtime.issue_monitor_daemon_inbox_events(&project_root, items));
            }
            None => {}
            _ => unreachable!("matched RuntimeHook monitor completion"),
        }
    }
    outbound
}
