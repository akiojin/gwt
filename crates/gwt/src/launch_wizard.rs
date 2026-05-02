use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::BranchListEntry;

mod quick_start;

pub use quick_start::load_quick_start_entries;

const DEFAULT_NEW_BRANCH_BASE_BRANCH: &str = "develop";
const BRANCH_TYPE_PREFIXES: [&str; 4] = ["feature/", "bugfix/", "hotfix/", "release/"];

/// Source kind of a SPEC/Issue knowledge bridge that opened the Launch Wizard.
/// Used to seed branch names as `{prefix}{kind}-{number}` per SPEC-2014 FR-024/025.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkedIssueKind {
    Issue,
    Spec,
}

impl LinkedIssueKind {
    fn branch_kind_segment(self) -> &'static str {
        match self {
            LinkedIssueKind::Issue => "issue",
            LinkedIssueKind::Spec => "spec",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchWizardStep {
    QuickStart,
    FocusExistingSession,
    BranchAction,
    BranchTypeSelect,
    BranchNameInput,
    LaunchTarget,
    AgentSelect,
    ModelSelect,
    ReasoningLevel,
    RuntimeTarget,
    WindowsShell,
    DockerServiceSelect,
    DockerLifecycle,
    VersionSelect,
    ExecutionMode,
    SkipPermissions,
    CodexFastMode,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchWizardOptionView {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
    /// Agent-specific color hint used by the frontend for candidate rows.
    /// `agent_options` から派生した option のみが `Some` を持ち、branch
    /// type や model など agent 非関連の他選択肢は常に `None`。
    /// SPEC #2133 FR-009.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<gwt_agent::AgentColor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchTargetKind {
    Agent,
    Shell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickStartLaunchMode {
    Resume,
    StartNew,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchWizardQuickStartView {
    pub index: usize,
    pub tool_label: String,
    pub summary: String,
    pub resume_session_id: Option<String>,
    pub reuse_action_label: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchWizardLiveSessionView {
    pub index: usize,
    pub name: String,
    pub detail: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchWizardSummaryView {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchWizardView {
    pub title: String,
    pub branch_name: String,
    pub selected_branch_name: String,
    pub linked_issue_number: Option<u64>,
    pub is_hydrating: bool,
    pub hydration_error: Option<String>,
    pub quick_start_entries: Vec<LaunchWizardQuickStartView>,
    pub live_sessions: Vec<LaunchWizardLiveSessionView>,
    pub branch_mode: String,
    pub branch_type_options: Vec<LaunchWizardOptionView>,
    pub selected_branch_type: Option<String>,
    pub launch_target_options: Vec<LaunchWizardOptionView>,
    pub selected_launch_target: String,
    pub agent_options: Vec<LaunchWizardOptionView>,
    pub selected_agent_id: String,
    pub model_options: Vec<LaunchWizardOptionView>,
    pub selected_model: String,
    pub reasoning_options: Vec<LaunchWizardOptionView>,
    pub selected_reasoning: String,
    pub runtime_target_options: Vec<LaunchWizardOptionView>,
    pub selected_runtime_target: String,
    pub windows_shell_options: Vec<LaunchWizardOptionView>,
    pub selected_windows_shell: Option<String>,
    pub docker_service_options: Vec<LaunchWizardOptionView>,
    pub selected_docker_service: Option<String>,
    pub docker_lifecycle_options: Vec<LaunchWizardOptionView>,
    pub selected_docker_lifecycle: String,
    pub version_options: Vec<LaunchWizardOptionView>,
    pub selected_version: String,
    pub execution_mode_options: Vec<LaunchWizardOptionView>,
    pub selected_execution_mode: String,
    pub skip_permissions: bool,
    pub show_agent_settings: bool,
    pub show_reasoning: bool,
    pub show_runtime_target: bool,
    pub show_windows_shell: bool,
    pub show_docker_service: bool,
    pub show_docker_lifecycle: bool,
    pub show_version: bool,
    pub show_execution_mode: bool,
    pub show_skip_permissions: bool,
    pub show_codex_fast_mode: bool,
    pub codex_fast_mode: bool,
    pub launch_summary: Vec<LaunchWizardSummaryView>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AgentOption {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub installed_version: Option<String>,
    pub versions: Vec<String>,
    pub custom_agent: Option<gwt_agent::CustomCodingAgent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickStartEntry {
    pub session_id: String,
    pub agent_id: String,
    pub tool_label: String,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub version: Option<String>,
    pub resume_session_id: Option<String>,
    pub live_window_id: Option<String>,
    pub skip_permissions: bool,
    pub codex_fast_mode: bool,
    pub runtime_target: gwt_agent::LaunchRuntimeTarget,
    pub docker_service: Option<String>,
    pub docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchWizardPreviousProfile {
    pub agent_id: String,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub version: Option<String>,
    pub session_mode: gwt_agent::SessionMode,
    pub skip_permissions: bool,
    pub codex_fast_mode: bool,
    pub runtime_target: gwt_agent::LaunchRuntimeTarget,
    pub docker_service: Option<String>,
    pub docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent,
    pub windows_shell: Option<gwt_agent::WindowsShellKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellLaunchConfig {
    pub working_dir: Option<PathBuf>,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub display_name: String,
    pub runtime_target: gwt_agent::LaunchRuntimeTarget,
    pub docker_service: Option<String>,
    pub docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent,
    pub windows_shell: Option<gwt_agent::WindowsShellKind>,
    pub env_vars: HashMap<String, String>,
    pub remove_env: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSessionEntry {
    pub session_id: String,
    pub window_id: String,
    pub agent_id: String,
    pub kind: String,
    pub name: String,
    pub detail: Option<String>,
    pub active: bool,
}

impl QuickStartEntry {
    fn reuse_action_label(&self) -> Option<&'static str> {
        if self.live_window_id.is_some() {
            Some("Focus")
        } else if self.resume_session_id.is_some() {
            Some("Resume")
        } else {
            None
        }
    }

    fn can_reuse(&self) -> bool {
        self.reuse_action_label().is_some()
    }
}

pub fn load_previous_launch_profile(
    repo_path: &Path,
    sessions_dir: &Path,
) -> Option<LaunchWizardPreviousProfile> {
    let entries = std::fs::read_dir(sessions_dir).ok()?;
    let sessions = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension().and_then(|ext| ext.to_str()) == Some("toml")).then_some(path)
        })
        .filter_map(|path| gwt_agent::Session::load_and_migrate(&path).ok())
        .collect::<Vec<_>>();
    previous_launch_profile_from_sessions(repo_path, &sessions)
}

pub fn previous_launch_profile_from_sessions(
    repo_path: &Path,
    sessions: &[gwt_agent::Session],
) -> Option<LaunchWizardPreviousProfile> {
    sessions
        .iter()
        .filter(|session| same_launch_profile_repo(repo_path, session))
        .max_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| left.created_at.cmp(&right.created_at))
                .then_with(|| left.id.cmp(&right.id))
        })
        .cloned()
        .map(previous_profile_from_session)
}

pub fn quick_start_entries_from_sessions(
    repo_path: &Path,
    branch_name: &str,
    sessions: &[gwt_agent::Session],
) -> Vec<QuickStartEntry> {
    let sessions = sessions
        .iter()
        .filter(|session| session.branch == branch_name)
        .filter(|session| same_launch_profile_repo(repo_path, session))
        .cloned()
        .map(|mut session| {
            session.worktree_path = repo_path.to_path_buf();
            session
        })
        .collect::<Vec<_>>();
    quick_start::collect_quick_start_entries_from_sessions(repo_path, branch_name, sessions)
}

fn previous_profile_from_session(session: gwt_agent::Session) -> LaunchWizardPreviousProfile {
    LaunchWizardPreviousProfile {
        agent_id: session.agent_id.command().to_string(),
        model: session.model,
        reasoning: session.reasoning_level,
        version: session.tool_version.or_else(|| {
            session
                .agent_id
                .package_name()
                .map(|_| "installed".to_string())
        }),
        session_mode: session.session_mode,
        skip_permissions: session.skip_permissions,
        codex_fast_mode: session.codex_fast_mode,
        runtime_target: session.runtime_target,
        docker_service: session.docker_service,
        docker_lifecycle_intent: session.docker_lifecycle_intent,
        windows_shell: session.windows_shell,
    }
}

fn same_launch_profile_repo(repo_path: &Path, session: &gwt_agent::Session) -> bool {
    let session_worktree_path = &session.worktree_path;
    if same_path_or_exact(repo_path, session_worktree_path) {
        return true;
    }

    if let (Some(current_repo_hash), Some(session_repo_hash)) = (
        repo_hash_for_existing_path(repo_path),
        session.repo_hash.as_deref(),
    ) {
        if current_repo_hash == session_repo_hash {
            return true;
        }
    }

    let Ok(repo_root) = gwt_git::worktree::main_worktree_root(repo_path) else {
        return false;
    };
    let Ok(session_root) = gwt_git::worktree::main_worktree_root(session_worktree_path) else {
        return false;
    };
    same_path_or_exact(&repo_root, &session_root)
}

fn repo_hash_for_existing_path(path: &Path) -> Option<String> {
    gwt_core::repo_hash::detect_repo_hash(path)
        .or_else(|| {
            gwt_git::worktree::main_worktree_root(path)
                .ok()
                .and_then(|root| gwt_core::repo_hash::detect_repo_hash(&root))
        })
        .map(|hash| hash.as_str().to_string())
}

fn same_path_or_exact(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DockerWizardContext {
    pub services: Vec<String>,
    pub suggested_service: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LaunchWizardContext {
    pub selected_branch: BranchListEntry,
    pub normalized_branch_name: String,
    pub worktree_path: Option<PathBuf>,
    pub quick_start_root: PathBuf,
    pub live_sessions: Vec<LiveSessionEntry>,
    pub docker_context: Option<DockerWizardContext>,
    pub docker_service_status: gwt_docker::ComposeServiceStatus,
    pub linked_issue_number: Option<u64>,
    /// Source kind of the SPEC/Issue knowledge bridge that opened this wizard.
    /// `None` for Branches-window callers, preserving non-breaking behavior.
    pub linked_issue_kind: Option<LinkedIssueKind>,
}

impl LaunchWizardContext {
    /// Returns the auto-seeded suffix `"{kind}-{number}"` (e.g. `"issue-42"`,
    /// `"spec-2014"`) when both `linked_issue_kind` and `linked_issue_number`
    /// are present. Used during `BranchAction::CreateNew` and `BranchTypeSelect`
    /// to pre-fill `branch_name` per SPEC-2014 FR-025.
    pub fn linked_issue_branch_suffix(&self) -> Option<String> {
        let number = self.linked_issue_number?;
        let kind = self.linked_issue_kind?;
        Some(format!("{}-{}", kind.branch_kind_segment(), number))
    }
}

#[derive(Debug, Clone)]
pub struct LaunchWizardHydration {
    pub selected_branch: Option<BranchListEntry>,
    pub normalized_branch_name: String,
    pub worktree_path: Option<PathBuf>,
    pub quick_start_root: PathBuf,
    pub docker_context: Option<DockerWizardContext>,
    pub docker_service_status: gwt_docker::ComposeServiceStatus,
    pub agent_options: Vec<AgentOption>,
    pub quick_start_entries: Vec<QuickStartEntry>,
    pub previous_profile: Option<LaunchWizardPreviousProfile>,
}

#[derive(Debug, Clone)]
pub enum LaunchWizardLaunchRequest {
    Agent(Box<gwt_agent::LaunchConfig>),
    Shell(Box<ShellLaunchConfig>),
}

#[derive(Debug, Clone)]
pub enum LaunchWizardCompletion {
    Launch(Box<LaunchWizardLaunchRequest>),
    FocusWindow { window_id: String },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchWizardAction {
    Select {
        index: usize,
    },
    Back,
    Cancel,
    SubmitText {
        value: String,
    },
    ApplyQuickStart {
        index: usize,
        mode: QuickStartLaunchMode,
    },
    FocusExistingSession {
        index: usize,
    },
    SetBranchMode {
        create_new: bool,
    },
    SetBranchType {
        prefix: String,
    },
    SetBranchName {
        value: String,
    },
    SetLaunchTarget {
        target: LaunchTargetKind,
    },
    SetAgent {
        agent_id: String,
    },
    SetModel {
        model: String,
    },
    SetReasoning {
        reasoning: String,
    },
    SetRuntimeTarget {
        target: gwt_agent::LaunchRuntimeTarget,
    },
    SetWindowsShell {
        shell: gwt_agent::WindowsShellKind,
    },
    SetDockerService {
        service: String,
    },
    SetDockerLifecycle {
        intent: gwt_agent::DockerLifecycleIntent,
    },
    SetVersion {
        version: String,
    },
    SetExecutionMode {
        mode: String,
    },
    SetLinkedIssue {
        issue_number: u64,
    },
    ClearLinkedIssue,
    SetSkipPermissions {
        enabled: bool,
    },
    SetCodexFastMode {
        enabled: bool,
    },
    Submit,
}

#[derive(Debug, Clone)]
pub struct LaunchWizardState {
    pub context: LaunchWizardContext,
    pub step: LaunchWizardStep,
    pub selected: usize,
    pub detected_agents: Vec<AgentOption>,
    pub quick_start_entries: Vec<QuickStartEntry>,
    pub is_new_branch: bool,
    pub base_branch_name: Option<String>,
    pub launch_target: LaunchTargetKind,
    pub agent_id: String,
    pub model: String,
    pub reasoning: String,
    pub version: String,
    pub mode: String,
    pub resume_session_id: Option<String>,
    pub runtime_target: gwt_agent::LaunchRuntimeTarget,
    pub windows_shell: gwt_agent::WindowsShellKind,
    pub docker_service: Option<String>,
    pub docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent,
    pub skip_permissions: bool,
    pub codex_fast_mode: bool,
    pub branch_name: String,
    pub completion: Option<LaunchWizardCompletion>,
    pub error: Option<String>,
    pub is_hydrating: bool,
    pub hydration_error: Option<String>,
    pub linked_issue_number: Option<u64>,
}

impl LaunchWizardState {
    fn hydrate_live_window_ids(
        context: &LaunchWizardContext,
        quick_start_entries: &mut [QuickStartEntry],
    ) {
        for entry in quick_start_entries {
            entry.live_window_id = context
                .live_sessions
                .iter()
                .find(|session| session.session_id == entry.session_id)
                .or_else(|| {
                    context
                        .live_sessions
                        .iter()
                        .find(|session| session.agent_id == entry.agent_id)
                })
                .map(|session| session.window_id.clone());
        }
    }

    fn new_with(
        context: LaunchWizardContext,
        agent_options: Vec<AgentOption>,
        mut quick_start_entries: Vec<QuickStartEntry>,
        previous_profile: Option<LaunchWizardPreviousProfile>,
        is_hydrating: bool,
    ) -> Self {
        Self::hydrate_live_window_ids(&context, &mut quick_start_entries);
        let runtime_target = if context.docker_context.is_some() {
            gwt_agent::LaunchRuntimeTarget::Docker
        } else {
            gwt_agent::LaunchRuntimeTarget::Host
        };
        let docker_service = context
            .docker_context
            .as_ref()
            .and_then(|ctx| ctx.suggested_service.clone());
        let docker_lifecycle_intent =
            default_docker_lifecycle_intent(context.docker_service_status);
        let has_quick_start = !quick_start_entries.is_empty() || !context.live_sessions.is_empty();
        let step = if has_quick_start {
            LaunchWizardStep::QuickStart
        } else {
            LaunchWizardStep::BranchAction
        };

        let mut state = Self {
            context: context.clone(),
            step,
            selected: 0,
            detected_agents: agent_options,
            quick_start_entries,
            is_new_branch: false,
            base_branch_name: None,
            launch_target: LaunchTargetKind::Agent,
            agent_id: String::new(),
            model: String::new(),
            reasoning: String::new(),
            version: String::new(),
            mode: "normal".to_string(),
            resume_session_id: None,
            runtime_target,
            windows_shell: default_windows_shell_kind(),
            docker_service,
            docker_lifecycle_intent,
            skip_permissions: false,
            codex_fast_mode: false,
            branch_name: String::new(),
            completion: None,
            error: None,
            is_hydrating,
            hydration_error: None,
            linked_issue_number: context.linked_issue_number,
        };
        state.branch_name = state.context.normalized_branch_name.clone();
        state.sync_selected_agent_options();
        let previous_profile_applied = previous_profile
            .map(|profile| state.apply_previous_profile(profile))
            .unwrap_or(false);
        if !previous_profile_applied {
            state.sync_docker_lifecycle_default();
        }
        state.selected = step_default_selection(state.step, &state);
        state
    }

    pub fn open_with(
        context: LaunchWizardContext,
        agent_options: Vec<AgentOption>,
        quick_start_entries: Vec<QuickStartEntry>,
    ) -> Self {
        Self::new_with(context, agent_options, quick_start_entries, None, false)
    }

    pub fn open_with_previous_profile(
        context: LaunchWizardContext,
        agent_options: Vec<AgentOption>,
        quick_start_entries: Vec<QuickStartEntry>,
        previous_profile: Option<LaunchWizardPreviousProfile>,
    ) -> Self {
        Self::new_with(
            context,
            agent_options,
            quick_start_entries,
            previous_profile,
            false,
        )
    }

    pub fn open_loading(context: LaunchWizardContext, agent_options: Vec<AgentOption>) -> Self {
        Self::new_with(context, agent_options, Vec::new(), None, true)
    }

    pub fn open(context: LaunchWizardContext, sessions_dir: &Path, cache_path: &Path) -> Self {
        let agent_options = load_agent_options(&gwt_agent::VersionCache::load(cache_path));
        let quick_start_entries = load_quick_start_entries(
            &context.quick_start_root,
            sessions_dir,
            &context.normalized_branch_name,
        );
        let previous_profile =
            load_previous_launch_profile(&context.quick_start_root, sessions_dir);
        Self::open_with_previous_profile(
            context,
            agent_options,
            quick_start_entries,
            previous_profile,
        )
    }

    pub fn view(&self) -> LaunchWizardView {
        LaunchWizardView {
            title: "Launch Agent".to_string(),
            branch_name: self.branch_name.clone(),
            selected_branch_name: self.context.selected_branch.name.clone(),
            linked_issue_number: self.linked_issue_number,
            is_hydrating: self.is_hydrating,
            hydration_error: self.hydration_error.clone(),
            quick_start_entries: self.quick_start_entries_view(),
            live_sessions: self.live_sessions_view(),
            branch_mode: if self.is_new_branch {
                "create_new".to_string()
            } else {
                "use_selected".to_string()
            },
            branch_type_options: branch_type_options_view(),
            selected_branch_type: self.selected_branch_type_prefix().map(str::to_string),
            launch_target_options: launch_target_options_view(),
            selected_launch_target: launch_target_value(self.launch_target).to_string(),
            agent_options: self.agent_options_view(),
            selected_agent_id: self.effective_agent_id().to_string(),
            model_options: self.model_options_view(),
            selected_model: self.model.clone(),
            reasoning_options: self.reasoning_options_view(),
            selected_reasoning: self.reasoning.clone(),
            runtime_target_options: runtime_target_options_view(),
            selected_runtime_target: runtime_target_value(self.runtime_target).to_string(),
            windows_shell_options: windows_shell_options_view(),
            selected_windows_shell: self
                .windows_shell_for_launch()
                .map(|shell| windows_shell_option_value(shell).to_string()),
            docker_service_options: self.docker_service_options_view(),
            selected_docker_service: self.docker_service.clone(),
            docker_lifecycle_options: self.docker_lifecycle_options_view(),
            selected_docker_lifecycle: docker_lifecycle_value(self.docker_lifecycle_intent)
                .to_string(),
            version_options: self.version_options_view(),
            selected_version: self.version.clone(),
            execution_mode_options: execution_mode_options_view(),
            selected_execution_mode: self.mode.clone(),
            skip_permissions: self.skip_permissions,
            show_agent_settings: self.launch_target_is_agent(),
            show_reasoning: self.launch_target_is_agent() && self.agent_uses_reasoning_step(),
            show_runtime_target: self.has_docker_workflow(),
            show_windows_shell: self.show_windows_shell_selection(),
            show_docker_service: self.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker
                && self.docker_service_prompt_required(),
            show_docker_lifecycle: self.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker,
            show_version: self.launch_target_is_agent()
                && agent_has_npm_package(self.effective_agent_id()),
            show_execution_mode: self.launch_target_is_agent(),
            show_skip_permissions: self.launch_target_is_agent(),
            show_codex_fast_mode: self.launch_target_is_agent() && self.agent_is_codex(),
            codex_fast_mode: self.codex_fast_mode,
            launch_summary: self.launch_summary_view(),
            error: self.error.clone(),
        }
    }

    pub fn apply_hydration(&mut self, hydration: LaunchWizardHydration) {
        let LaunchWizardHydration {
            selected_branch,
            normalized_branch_name,
            worktree_path,
            quick_start_root,
            docker_context,
            docker_service_status,
            agent_options,
            mut quick_start_entries,
            previous_profile,
        } = hydration;
        if let Some(selected_branch) = selected_branch {
            self.context.selected_branch = selected_branch;
        }
        self.context.normalized_branch_name = normalized_branch_name;
        self.context.worktree_path = worktree_path;
        self.context.quick_start_root = quick_start_root;
        self.context.docker_context = docker_context;
        self.context.docker_service_status = docker_service_status;
        self.detected_agents = agent_options;
        Self::hydrate_live_window_ids(&self.context, &mut quick_start_entries);
        self.quick_start_entries = quick_start_entries;
        self.is_hydrating = false;
        self.hydration_error = None;
        self.branch_name = if self.is_new_branch {
            self.branch_name.clone()
        } else {
            self.context.normalized_branch_name.clone()
        };
        if self.has_docker_workflow() {
            self.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
            if self.docker_service.is_none() {
                self.docker_service = self.preferred_docker_service().map(str::to_string);
            }
        } else {
            self.runtime_target = gwt_agent::LaunchRuntimeTarget::Host;
            self.docker_service = None;
        }
        self.sync_selected_agent_options();
        let previous_profile_applied = previous_profile
            .map(|profile| self.apply_previous_profile(profile))
            .unwrap_or(false);
        if !previous_profile_applied {
            self.sync_docker_lifecycle_default();
        }
        self.selected = self
            .selected
            .min(self.current_options().len().saturating_sub(1));
    }

    pub fn set_hydration_error(&mut self, error: String) {
        self.is_hydrating = false;
        self.hydration_error = Some(error);
    }

    pub fn apply(&mut self, action: LaunchWizardAction) {
        self.error = None;

        match action {
            LaunchWizardAction::Cancel => {
                self.completion = Some(LaunchWizardCompletion::Cancelled);
            }
            LaunchWizardAction::Submit => {
                self.submit_panel();
            }
            LaunchWizardAction::ApplyQuickStart { index, mode } => {
                self.apply_quick_start_action(index, mode);
            }
            LaunchWizardAction::FocusExistingSession { index } => {
                self.focus_existing_session(index);
            }
            LaunchWizardAction::SetBranchMode { create_new } => {
                self.set_branch_mode(create_new);
            }
            LaunchWizardAction::SetBranchType { prefix } => {
                self.set_branch_type(&prefix);
            }
            LaunchWizardAction::SetBranchName { value } => {
                self.branch_name = value;
            }
            LaunchWizardAction::SetLaunchTarget { target } => {
                self.set_launch_target(target);
            }
            LaunchWizardAction::SetAgent { agent_id } => {
                self.set_agent_id(&agent_id);
            }
            LaunchWizardAction::SetModel { model } => {
                self.set_model(&model);
            }
            LaunchWizardAction::SetReasoning { reasoning } => {
                self.set_reasoning(&reasoning);
            }
            LaunchWizardAction::SetRuntimeTarget { target } => {
                self.set_runtime_target(target);
            }
            LaunchWizardAction::SetWindowsShell { shell } => {
                self.windows_shell = shell;
            }
            LaunchWizardAction::SetDockerService { service } => {
                self.set_docker_service(&service);
            }
            LaunchWizardAction::SetDockerLifecycle { intent } => {
                self.set_docker_lifecycle(intent);
            }
            LaunchWizardAction::SetVersion { version } => {
                self.set_version(&version);
            }
            LaunchWizardAction::SetExecutionMode { mode } => {
                self.set_execution_mode(&mode);
            }
            LaunchWizardAction::SetSkipPermissions { enabled } => {
                self.skip_permissions = enabled;
            }
            LaunchWizardAction::SetLinkedIssue { issue_number } => {
                self.linked_issue_number = Some(issue_number);
            }
            LaunchWizardAction::ClearLinkedIssue => {
                self.linked_issue_number = None;
            }
            LaunchWizardAction::SetCodexFastMode { enabled } => {
                self.codex_fast_mode = enabled && self.agent_is_codex();
            }
            LaunchWizardAction::Back => {
                if let Some(prev) = prev_step(self.step, self) {
                    self.step = prev;
                    self.selected = step_default_selection(prev, self);
                } else {
                    self.completion = Some(LaunchWizardCompletion::Cancelled);
                }
            }
            LaunchWizardAction::SubmitText { value } => {
                if self.step != LaunchWizardStep::BranchNameInput {
                    return;
                }
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    self.error = Some("Branch name is required".to_string());
                    return;
                }
                self.branch_name = trimmed.to_string();
                self.advance_after_current_step();
            }
            LaunchWizardAction::Select { index } => {
                let max_index = self.current_options().len().saturating_sub(1);
                self.selected = index.min(max_index);
                self.apply_selection();
                if self.completion.is_none() && self.error.is_none() {
                    self.advance_after_current_step();
                }
            }
        }
    }

    pub fn build_launch_config(&self) -> Result<gwt_agent::LaunchConfig, String> {
        if self.is_hydrating {
            return Err("Launch options are still loading".to_string());
        }
        if !self.launch_target_is_agent() {
            return Err("Agent launch target is not selected".to_string());
        }
        let selected_agent = self
            .selected_agent()
            .cloned()
            .ok_or_else(|| "Agent option is unavailable".to_string())?;

        let agent_id = agent_id_from_key(&selected_agent.id);
        let mut builder = gwt_agent::AgentLaunchBuilder::new(agent_id.clone());
        if let Some(custom_agent) = selected_agent.custom_agent.clone() {
            builder = builder.custom_agent(custom_agent);
        }

        if !self.is_new_branch {
            if let Some(worktree_path) = &self.context.worktree_path {
                builder = builder.working_dir(worktree_path.clone());
            }
        }

        if !self.branch_name.is_empty() {
            builder = builder.branch(self.branch_name.clone());
        }

        if self.is_new_branch {
            builder = builder.base_branch(
                self.base_branch_name
                    .clone()
                    .unwrap_or_else(|| DEFAULT_NEW_BRANCH_BASE_BRANCH.to_string()),
            );
        }

        if is_explicit_model_selection(&self.model) {
            builder = builder.model(self.model.clone());
        }

        if !self.version.is_empty() {
            builder = builder.version(self.version.clone());
        }

        if let Some(reasoning_level) = self.reasoning_level_for_launch() {
            builder = builder.reasoning_level(reasoning_level.to_string());
        }

        if self.skip_permissions {
            builder = builder.skip_permissions(true);
        }

        if self.agent_is_codex() && self.codex_fast_mode {
            builder = builder.fast_mode(true);
        }

        builder = builder.runtime_target(self.runtime_target);
        if let Some(windows_shell) = self.windows_shell_for_launch() {
            builder = builder.windows_shell(windows_shell);
        }
        if let Some(docker_service) = self.docker_service.as_deref() {
            builder = builder.docker_service(docker_service.to_string());
        }
        builder = builder.docker_lifecycle_intent(self.docker_lifecycle_intent);
        builder = match self.mode.as_str() {
            "continue" => builder.session_mode(gwt_agent::SessionMode::Continue),
            "resume" if self.resume_session_id.is_some() => builder
                .session_mode(gwt_agent::SessionMode::Resume)
                .resume_session_id(self.resume_session_id.clone().expect("resume session id")),
            "resume" => builder.session_mode(gwt_agent::SessionMode::Continue),
            _ => builder.session_mode(gwt_agent::SessionMode::Normal),
        };

        if let Some(n) = self.linked_issue_number {
            builder = builder.linked_issue_number(n);
        }

        let mut config = builder.build();
        if !self.version.is_empty() {
            config.tool_version = Some(self.version.clone());
        }
        if let Some(reasoning_level) = self.reasoning_level_for_launch() {
            config.reasoning_level = Some(reasoning_level.to_string());
        }
        Ok(config)
    }

    fn build_shell_launch_config(&self) -> Result<ShellLaunchConfig, String> {
        if self.is_hydrating {
            return Err("Launch options are still loading".to_string());
        }

        let working_dir = if !self.is_new_branch {
            self.context.worktree_path.clone()
        } else {
            None
        };
        let branch = (!self.branch_name.is_empty()).then(|| self.branch_name.clone());
        let base_branch = self.is_new_branch.then(|| {
            self.base_branch_name
                .clone()
                .unwrap_or_else(|| DEFAULT_NEW_BRANCH_BASE_BRANCH.to_string())
        });
        let mut env_vars = HashMap::new();
        if let Some(dir) = working_dir.as_ref() {
            env_vars.insert("GWT_PROJECT_ROOT".to_string(), dir.display().to_string());
        }

        Ok(ShellLaunchConfig {
            working_dir,
            branch: branch.clone(),
            base_branch,
            display_name: "Shell".to_string(),
            runtime_target: self.runtime_target,
            docker_service: self.docker_service.clone(),
            docker_lifecycle_intent: self.docker_lifecycle_intent,
            windows_shell: self.windows_shell_for_launch(),
            env_vars,
            remove_env: Vec::new(),
        })
    }

    fn build_launch_request(&self) -> Result<LaunchWizardLaunchRequest, String> {
        match self.launch_target {
            LaunchTargetKind::Agent => Ok(LaunchWizardLaunchRequest::Agent(Box::new(
                self.build_launch_config()?,
            ))),
            LaunchTargetKind::Shell => Ok(LaunchWizardLaunchRequest::Shell(Box::new(
                self.build_shell_launch_config()?,
            ))),
        }
    }

    fn advance_after_current_step(&mut self) {
        if self.completion.is_some() {
            return;
        }

        if let Some(next) = next_step(self.step, self) {
            self.step = next;
            self.selected = step_default_selection(next, self);
            return;
        }

        match self.build_launch_request() {
            Ok(config) => {
                self.completion = Some(LaunchWizardCompletion::Launch(Box::new(config)));
            }
            Err(error) => {
                self.error = Some(error);
            }
        }
    }

    fn apply_selection(&mut self) {
        match self.step {
            LaunchWizardStep::QuickStart => match self.selected_quick_start_action() {
                QuickStartAction::ReuseEntry { .. } | QuickStartAction::StartNewEntry { .. } => {
                    self.apply_quick_start_selection();
                    self.sync_docker_lifecycle_default();
                }
                QuickStartAction::FocusExistingSession | QuickStartAction::ChooseDifferent => {}
            },
            LaunchWizardStep::FocusExistingSession => {
                if let Some(entry) = self.context.live_sessions.get(self.selected) {
                    self.completion = Some(LaunchWizardCompletion::FocusWindow {
                        window_id: entry.window_id.clone(),
                    });
                } else {
                    self.error = Some("No running session is available".to_string());
                }
            }
            LaunchWizardStep::BranchAction => {
                if self.selected == 0 {
                    self.is_new_branch = false;
                    self.base_branch_name = None;
                    self.branch_name = self.context.normalized_branch_name.clone();
                } else {
                    self.is_new_branch = true;
                    self.base_branch_name = Some(self.context.normalized_branch_name.clone());
                    self.branch_name.clear();
                }
            }
            LaunchWizardStep::BranchTypeSelect => {
                if let Some(prefix) = BRANCH_TYPE_PREFIXES.get(self.selected) {
                    self.seed_branch_name_for_prefix(prefix);
                }
            }
            LaunchWizardStep::LaunchTarget => {
                self.set_launch_target(if self.selected == 0 {
                    LaunchTargetKind::Agent
                } else {
                    LaunchTargetKind::Shell
                });
            }
            LaunchWizardStep::AgentSelect => {
                if let Some(agent) = self.detected_agents.get(self.selected) {
                    self.agent_id = agent.id.clone();
                }
                self.sync_selected_agent_options();
            }
            LaunchWizardStep::ModelSelect => {
                if let Some(model) =
                    current_model_options(self.effective_agent_id()).get(self.selected)
                {
                    self.model = model.to_string();
                }
                self.sync_reasoning_state();
            }
            LaunchWizardStep::ReasoningLevel => {
                if let Some(option) = self.current_reasoning_options().get(self.selected) {
                    self.reasoning = option.stored_value.to_string();
                }
            }
            LaunchWizardStep::RuntimeTarget => {
                self.runtime_target = if self.selected == 0 {
                    gwt_agent::LaunchRuntimeTarget::Host
                } else {
                    gwt_agent::LaunchRuntimeTarget::Docker
                };
                if self.runtime_target == gwt_agent::LaunchRuntimeTarget::Host {
                    self.docker_service = None;
                } else if self.docker_service.is_none() {
                    self.docker_service = self.preferred_docker_service().map(str::to_string);
                }
                self.sync_docker_lifecycle_default();
            }
            LaunchWizardStep::WindowsShell => {
                if let Some(option) = WINDOWS_SHELL_OPTIONS.get(self.selected) {
                    self.windows_shell = *option;
                }
            }
            LaunchWizardStep::DockerServiceSelect => {
                if let Some(service) = self.docker_service_options().get(self.selected) {
                    self.docker_service = Some(service.clone());
                }
                self.sync_docker_lifecycle_default();
            }
            LaunchWizardStep::DockerLifecycle => {
                if let Some(option) = self.docker_lifecycle_options().get(self.selected) {
                    self.docker_lifecycle_intent = option.intent;
                }
            }
            LaunchWizardStep::VersionSelect => {
                if let Some(option) = self.current_version_options().get(self.selected) {
                    self.version = option.value.clone();
                }
            }
            LaunchWizardStep::ExecutionMode => {
                if let Some(option) = EXECUTION_MODE_OPTIONS.get(self.selected) {
                    self.mode = option.value.to_string();
                }
            }
            LaunchWizardStep::SkipPermissions => {
                self.skip_permissions = self.selected == 0;
            }
            LaunchWizardStep::CodexFastMode => {
                self.codex_fast_mode = self.selected == 0;
            }
            LaunchWizardStep::BranchNameInput => {}
        }
    }

    fn submit_panel(&mut self) {
        if self.is_new_branch {
            let trimmed = self.branch_name.trim();
            if trimmed.is_empty() {
                self.error = Some("Branch name is required".to_string());
                return;
            }
            self.branch_name = trimmed.to_string();
        }

        match self.build_launch_request() {
            Ok(config) => {
                self.completion = Some(LaunchWizardCompletion::Launch(Box::new(config)));
            }
            Err(error) => {
                self.error = Some(error);
            }
        }
    }

    fn apply_quick_start_action(&mut self, index: usize, mode: QuickStartLaunchMode) {
        let Some(entry) = self.quick_start_entries.get(index).cloned() else {
            self.error = Some("Quick start entry is unavailable".to_string());
            return;
        };

        self.launch_target = LaunchTargetKind::Agent;
        self.agent_id = entry.agent_id.clone();
        self.sync_selected_agent_options();
        self.apply_saved_model(entry.model.as_deref());
        if let Some(reasoning) = entry.reasoning {
            self.reasoning = reasoning;
        }
        if let Some(version) = entry.version {
            self.version = version;
        }
        self.skip_permissions = entry.skip_permissions;
        self.codex_fast_mode = entry.codex_fast_mode && self.agent_is_codex();
        self.runtime_target = entry.runtime_target;
        self.docker_service = entry.docker_service.clone();
        self.docker_lifecycle_intent = entry.docker_lifecycle_intent;
        self.sync_docker_lifecycle_default();
        match mode {
            QuickStartLaunchMode::Resume => {
                if let Some(window_id) = entry.live_window_id {
                    self.completion = Some(LaunchWizardCompletion::FocusWindow { window_id });
                } else if let Some(resume_session_id) = entry.resume_session_id {
                    self.mode = "resume".to_string();
                    self.resume_session_id = Some(resume_session_id);
                    match self.build_launch_request() {
                        Ok(config) => {
                            self.completion =
                                Some(LaunchWizardCompletion::Launch(Box::new(config)));
                        }
                        Err(error) => self.error = Some(error),
                    }
                } else {
                    self.error = Some("No saved session is available".to_string());
                }
            }
            QuickStartLaunchMode::StartNew => {
                self.mode = "normal".to_string();
                self.resume_session_id = None;
                match self.build_launch_request() {
                    Ok(config) => {
                        self.completion = Some(LaunchWizardCompletion::Launch(Box::new(config)));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
        }
    }

    fn apply_previous_profile(&mut self, profile: LaunchWizardPreviousProfile) -> bool {
        let saved_agent = self
            .detected_agents
            .iter()
            .find(|candidate| candidate.id == profile.agent_id);
        let Some(agent) = saved_agent.or_else(|| self.detected_agents.first()) else {
            return false;
        };

        self.launch_target = LaunchTargetKind::Agent;
        self.agent_id = agent.id.clone();
        self.sync_selected_agent_options();
        self.apply_saved_model(profile.model.as_deref());
        if let Some(reasoning) = profile.reasoning.as_deref() {
            if self
                .current_reasoning_options()
                .iter()
                .any(|option| option.stored_value == reasoning)
            {
                self.reasoning = reasoning.to_string();
            }
        }
        self.sync_reasoning_state();
        if let Some(version) = profile.version.as_deref() {
            if self
                .current_version_options()
                .iter()
                .any(|option| option.value == version)
            {
                self.version = version.to_string();
            }
        }
        self.mode = execution_mode_value_from_session_mode(profile.session_mode).to_string();
        self.resume_session_id = None;
        self.skip_permissions = profile.skip_permissions;
        self.codex_fast_mode = profile.codex_fast_mode && self.agent_is_codex();
        self.apply_previous_runtime_profile(
            profile.runtime_target,
            profile.docker_service.as_deref(),
            profile.docker_lifecycle_intent,
        );
        if let Some(windows_shell) = profile.windows_shell {
            self.windows_shell = windows_shell;
        }
        true
    }

    fn apply_previous_runtime_profile(
        &mut self,
        runtime_target: gwt_agent::LaunchRuntimeTarget,
        docker_service: Option<&str>,
        docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent,
    ) {
        if runtime_target != gwt_agent::LaunchRuntimeTarget::Docker || !self.has_docker_workflow() {
            self.runtime_target = gwt_agent::LaunchRuntimeTarget::Host;
            self.docker_service = None;
            self.sync_docker_lifecycle_default();
            return;
        }

        self.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        let services = self.docker_service_options();
        self.docker_service = docker_service
            .filter(|service| services.iter().any(|candidate| candidate == service))
            .map(str::to_string)
            .or_else(|| {
                self.context
                    .docker_context
                    .as_ref()
                    .and_then(|ctx| ctx.suggested_service.clone())
            })
            .or_else(|| services.first().cloned());
        self.docker_lifecycle_intent = docker_lifecycle_intent;
        self.sync_docker_lifecycle_default();
    }

    fn focus_existing_session(&mut self, index: usize) {
        if let Some(entry) = self.context.live_sessions.get(index) {
            self.completion = Some(LaunchWizardCompletion::FocusWindow {
                window_id: entry.window_id.clone(),
            });
        } else {
            self.error = Some("No running session is available".to_string());
        }
    }

    fn set_branch_mode(&mut self, create_new: bool) {
        self.is_new_branch = create_new;
        if create_new {
            self.base_branch_name = Some(self.context.normalized_branch_name.clone());
            if self.branch_name.is_empty()
                || self.branch_name == self.context.normalized_branch_name
            {
                self.branch_name.clear();
                self.seed_branch_name_for_prefix(BRANCH_TYPE_PREFIXES[0]);
            }
        } else {
            self.base_branch_name = None;
            self.branch_name = self.context.normalized_branch_name.clone();
        }
    }

    fn set_branch_type(&mut self, prefix: &str) {
        if !BRANCH_TYPE_PREFIXES
            .iter()
            .any(|candidate| candidate == &prefix)
        {
            self.error = Some("Branch type is unavailable".to_string());
            return;
        }
        self.seed_branch_name_for_prefix(prefix);
    }

    /// Apply `prefix` to `branch_name`. When the current name has no
    /// user-entered suffix (empty or just a known prefix), pre-fill from
    /// `LinkedIssueKind` + `linked_issue_number` per SPEC-2014 FR-024/025.
    /// User-entered suffixes are preserved (NFR-008).
    fn seed_branch_name_for_prefix(&mut self, prefix: &str) {
        let trimmed = self.branch_name.trim();
        let user_suffix = BRANCH_TYPE_PREFIXES
            .iter()
            .find_map(|known| trimmed.strip_prefix(known))
            .unwrap_or(trimmed)
            .trim_matches('/');
        if user_suffix.is_empty() {
            self.branch_name = match self.context.linked_issue_branch_suffix() {
                Some(seed) => format!("{prefix}{seed}"),
                None => prefix.to_string(),
            };
        } else {
            self.branch_name = format!("{prefix}{user_suffix}");
        }
    }

    fn set_launch_target(&mut self, target: LaunchTargetKind) {
        self.launch_target = target;
        if self.launch_target_is_shell() {
            self.mode = "normal".to_string();
            self.resume_session_id = None;
            self.skip_permissions = false;
            self.codex_fast_mode = false;
        } else {
            self.sync_selected_agent_options();
        }
    }

    fn set_agent_id(&mut self, agent_id: &str) {
        match self
            .detected_agents
            .iter()
            .position(|candidate| candidate.id == agent_id)
        {
            Some(index) => {
                self.agent_id = agent_id.to_string();
                if self.step == LaunchWizardStep::AgentSelect {
                    self.selected = index;
                }
                self.sync_selected_agent_options();
            }
            _ => {
                self.error = Some("Agent option is unavailable".to_string());
            }
        }
    }

    fn set_model(&mut self, model: &str) {
        if current_model_options(self.effective_agent_id())
            .iter()
            .any(|candidate| candidate == &model)
        {
            self.model = model.to_string();
            self.sync_reasoning_state();
        } else if model.is_empty() && !self.agent_has_models() {
            self.model.clear();
        } else {
            self.error = Some("Model option is unavailable".to_string());
        }
    }

    fn set_reasoning(&mut self, reasoning: &str) {
        if self
            .current_reasoning_options()
            .iter()
            .any(|option| option.stored_value == reasoning)
        {
            self.reasoning = reasoning.to_string();
        } else {
            self.error = Some("Reasoning option is unavailable".to_string());
        }
    }

    fn set_runtime_target(&mut self, target: gwt_agent::LaunchRuntimeTarget) {
        self.runtime_target = target;
        if self.runtime_target == gwt_agent::LaunchRuntimeTarget::Host {
            self.docker_service = None;
        } else if self.docker_service.is_none() {
            self.docker_service = self.preferred_docker_service().map(str::to_string);
        }
        self.sync_docker_lifecycle_default();
    }

    fn set_docker_service(&mut self, service: &str) {
        if self
            .docker_service_options()
            .iter()
            .any(|candidate| candidate == service)
        {
            self.docker_service = Some(service.to_string());
            self.sync_docker_lifecycle_default();
        } else {
            self.error = Some("Docker service is unavailable".to_string());
        }
    }

    fn set_docker_lifecycle(&mut self, intent: gwt_agent::DockerLifecycleIntent) {
        if self
            .docker_lifecycle_options()
            .iter()
            .any(|option| option.intent == intent)
        {
            self.docker_lifecycle_intent = intent;
        } else {
            self.error = Some("Docker lifecycle option is unavailable".to_string());
        }
    }

    fn set_version(&mut self, version: &str) {
        if self
            .current_version_options()
            .iter()
            .any(|option| option.value == version)
        {
            self.version = version.to_string();
        } else {
            self.error = Some("Version option is unavailable".to_string());
        }
    }

    fn set_execution_mode(&mut self, mode: &str) {
        if EXECUTION_MODE_OPTIONS
            .iter()
            .any(|option| option.value == mode)
        {
            self.mode = mode.to_string();
            if self.mode != "resume" {
                self.resume_session_id = None;
            }
        } else {
            self.error = Some("Execution mode is unavailable".to_string());
        }
    }

    fn quick_start_entries_view(&self) -> Vec<LaunchWizardQuickStartView> {
        self.quick_start_entries
            .iter()
            .enumerate()
            .map(|(index, entry)| LaunchWizardQuickStartView {
                index,
                tool_label: entry.tool_label.clone(),
                summary: quick_start_summary(entry),
                resume_session_id: entry.resume_session_id.clone(),
                reuse_action_label: entry.reuse_action_label().map(str::to_string),
            })
            .collect()
    }

    fn live_sessions_view(&self) -> Vec<LaunchWizardLiveSessionView> {
        self.context
            .live_sessions
            .iter()
            .enumerate()
            .map(|(index, entry)| LaunchWizardLiveSessionView {
                index,
                name: entry.name.clone(),
                detail: entry.detail.clone(),
                active: entry.active,
            })
            .collect()
    }

    fn agent_options_view(&self) -> Vec<LaunchWizardOptionView> {
        self.detected_agents
            .iter()
            .map(|agent| LaunchWizardOptionView {
                value: agent.id.clone(),
                label: agent.name.clone(),
                description: Some(agent_description(agent)),
                color: agent_option_color(&agent.id),
            })
            .collect()
    }

    fn model_options_view(&self) -> Vec<LaunchWizardOptionView> {
        model_display_options(self.effective_agent_id())
            .iter()
            .map(|option| LaunchWizardOptionView {
                value: option.label.to_string(),
                label: option.label.to_string(),
                description: Some(option.description.to_string()),
                color: None,
            })
            .collect()
    }

    fn reasoning_options_view(&self) -> Vec<LaunchWizardOptionView> {
        self.current_reasoning_options()
            .iter()
            .map(|option| LaunchWizardOptionView {
                value: option.stored_value.to_string(),
                label: option.label.to_string(),
                description: Some(option.description.to_string()),
                color: None,
            })
            .collect()
    }

    fn docker_service_options_view(&self) -> Vec<LaunchWizardOptionView> {
        self.docker_service_options()
            .into_iter()
            .map(|service| LaunchWizardOptionView {
                value: service.clone(),
                label: service,
                description: Some("Docker Compose service".to_string()),
                color: None,
            })
            .collect()
    }

    fn docker_lifecycle_options_view(&self) -> Vec<LaunchWizardOptionView> {
        self.docker_lifecycle_options()
            .iter()
            .map(|option| LaunchWizardOptionView {
                value: docker_lifecycle_value(option.intent).to_string(),
                label: option.label.to_string(),
                description: Some(option.description.to_string()),
                color: None,
            })
            .collect()
    }

    fn version_options_view(&self) -> Vec<LaunchWizardOptionView> {
        self.current_version_options()
            .into_iter()
            .map(|option| LaunchWizardOptionView {
                value: option.value,
                label: option.label,
                description: Some("Tool version".to_string()),
                color: None,
            })
            .collect()
    }

    fn launch_summary_view(&self) -> Vec<LaunchWizardSummaryView> {
        let mut summary = vec![
            LaunchWizardSummaryView {
                label: "Branch".to_string(),
                value: self.branch_name.clone(),
            },
            LaunchWizardSummaryView {
                label: "Target".to_string(),
                value: match self.launch_target {
                    LaunchTargetKind::Agent => "Agent".to_string(),
                    LaunchTargetKind::Shell => "Shell".to_string(),
                },
            },
        ];

        if self.launch_target_is_agent() {
            summary.push(LaunchWizardSummaryView {
                label: "Agent".to_string(),
                value: self
                    .selected_agent()
                    .map(|agent| agent.name.clone())
                    .unwrap_or_else(|| "Unavailable".to_string()),
            });
            if is_explicit_model_selection(&self.model) {
                summary.push(LaunchWizardSummaryView {
                    label: "Model".to_string(),
                    value: self.model.clone(),
                });
            }
            if let Some(reasoning) = self.reasoning_level_for_launch() {
                summary.push(LaunchWizardSummaryView {
                    label: if self.agent_is_codex() {
                        "Reasoning".to_string()
                    } else {
                        "Effort".to_string()
                    },
                    value: reasoning.to_string(),
                });
            }
            if !self.version.is_empty() {
                summary.push(LaunchWizardSummaryView {
                    label: "Version".to_string(),
                    value: self.version.clone(),
                });
            }
            summary.push(LaunchWizardSummaryView {
                label: "Mode".to_string(),
                value: self.mode.clone(),
            });
        }
        summary.push(LaunchWizardSummaryView {
            label: "Runtime".to_string(),
            value: if self.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
                self.docker_service
                    .as_ref()
                    .map(|service| format!("docker:{service}"))
                    .unwrap_or_else(|| "docker".to_string())
            } else {
                "host".to_string()
            },
        });
        if let Some(windows_shell) = self.windows_shell_for_launch() {
            summary.push(LaunchWizardSummaryView {
                label: "Shell".to_string(),
                value: windows_shell_option_label(windows_shell).to_string(),
            });
        }
        if self.launch_target_is_agent() {
            summary.push(LaunchWizardSummaryView {
                label: "Permissions".to_string(),
                value: if self.skip_permissions {
                    "skip".to_string()
                } else {
                    "prompt".to_string()
                },
            });
        }
        if self.agent_is_codex() {
            summary.push(LaunchWizardSummaryView {
                label: "Fast mode".to_string(),
                value: if self.codex_fast_mode {
                    "on".to_string()
                } else {
                    "off".to_string()
                },
            });
        }

        summary
    }

    fn selected_branch_type_prefix(&self) -> Option<&'static str> {
        BRANCH_TYPE_PREFIXES
            .iter()
            .find(|prefix| self.branch_name.starts_with(**prefix))
            .copied()
    }

    fn sync_selected_agent_options(&mut self) {
        let Some(agent) = self.selected_agent().cloned() else {
            return;
        };

        if self.agent_id.is_empty() {
            self.agent_id = agent.id.clone();
        }

        let models = current_model_options(&agent.id);
        if models.is_empty() {
            self.model.clear();
        } else if self.model.is_empty() || !models.iter().any(|model| model == &self.model) {
            self.model = models[0].to_string();
        }

        let version_options = self.current_version_options_for(&agent);
        if version_options.is_empty() {
            self.version.clear();
        } else if self.version.is_empty()
            || !version_options
                .iter()
                .any(|option| option.value == self.version)
        {
            self.version = if agent_has_npm_package(&agent.id) {
                "latest".to_string()
            } else {
                "installed".to_string()
            };
        }

        if !self.agent_is_codex() {
            self.codex_fast_mode = false;
        }
        self.sync_reasoning_state();
    }

    fn apply_saved_model(&mut self, model: Option<&str>) {
        let Some(model) = model else {
            return;
        };
        if current_model_options(self.effective_agent_id())
            .iter()
            .any(|candidate| candidate == &model)
        {
            self.model = model.to_string();
        }
    }

    fn sync_reasoning_state(&mut self) {
        let options = self.current_reasoning_options();
        if options.is_empty() {
            self.reasoning.clear();
            return;
        }
        if self.reasoning.is_empty()
            || !options
                .iter()
                .any(|option| option.stored_value == self.reasoning)
        {
            self.reasoning = options
                .iter()
                .find(|option| option.is_default)
                .map(|option| option.stored_value.to_string())
                .unwrap_or_default();
        }
    }

    fn sync_docker_lifecycle_default(&mut self) {
        let supported = self
            .docker_lifecycle_options()
            .iter()
            .any(|option| option.intent == self.docker_lifecycle_intent);
        if !supported {
            self.docker_lifecycle_intent =
                default_docker_lifecycle_intent(self.context.docker_service_status);
        }
    }

    fn reasoning_level_for_launch(&self) -> Option<&str> {
        match self.effective_agent_id() {
            "codex" if !self.reasoning.is_empty() => Some(self.reasoning.as_str()),
            "claude"
                if !self.reasoning.is_empty()
                    && is_claude_effort_capable_model(self.model.as_str()) =>
            {
                Some(self.reasoning.as_str())
            }
            _ => None,
        }
    }

    fn launch_target_is_agent(&self) -> bool {
        self.launch_target == LaunchTargetKind::Agent
    }

    fn launch_target_is_shell(&self) -> bool {
        self.launch_target == LaunchTargetKind::Shell
    }

    fn selected_agent(&self) -> Option<&AgentOption> {
        if self.step == LaunchWizardStep::AgentSelect {
            return self.detected_agents.get(self.selected);
        }
        if self.agent_id.is_empty() {
            self.detected_agents.first()
        } else {
            self.detected_agents
                .iter()
                .find(|agent| agent.id == self.agent_id)
        }
    }

    fn effective_agent_id(&self) -> &str {
        self.selected_agent()
            .map(|agent| agent.id.as_str())
            .unwrap_or(self.agent_id.as_str())
    }

    fn agent_is_codex(&self) -> bool {
        self.launch_target_is_agent() && self.effective_agent_id() == "codex"
    }

    fn agent_has_models(&self) -> bool {
        self.launch_target_is_agent()
            && matches!(self.effective_agent_id(), "claude" | "codex" | "gemini")
    }

    fn agent_uses_reasoning_step(&self) -> bool {
        if !self.launch_target_is_agent() {
            return false;
        }
        if self.agent_is_codex() {
            return true;
        }
        self.effective_agent_id() == "claude" && is_claude_effort_capable_model(self.model.as_str())
    }

    fn has_docker_workflow(&self) -> bool {
        self.context.docker_context.is_some()
    }

    fn show_windows_shell_selection(&self) -> bool {
        cfg!(windows) && self.windows_shell_for_launch().is_some()
    }

    fn windows_shell_for_launch(&self) -> Option<gwt_agent::WindowsShellKind> {
        (cfg!(windows) && self.runtime_target == gwt_agent::LaunchRuntimeTarget::Host)
            .then_some(self.windows_shell)
    }

    fn docker_service_prompt_required(&self) -> bool {
        self.context
            .docker_context
            .as_ref()
            .is_some_and(|ctx| ctx.services.len() > 1)
    }

    fn preferred_docker_service(&self) -> Option<&str> {
        self.docker_service.as_deref().or_else(|| {
            self.context
                .docker_context
                .as_ref()
                .and_then(|ctx| ctx.suggested_service.as_deref())
        })
    }

    fn docker_service_options(&self) -> Vec<String> {
        self.context
            .docker_context
            .as_ref()
            .map(|ctx| ctx.services.clone())
            .unwrap_or_default()
    }

    fn docker_lifecycle_options(&self) -> &'static [DockerLifecycleOption] {
        match self.context.docker_service_status {
            gwt_docker::ComposeServiceStatus::Unknown => &[DockerLifecycleOption {
                label: "Connect or start then launch",
                description: "Resolve the Docker service state at launch time",
                intent: gwt_agent::DockerLifecycleIntent::Start,
            }],
            gwt_docker::ComposeServiceStatus::Running => &[
                DockerLifecycleOption {
                    label: "Connect only",
                    description: "Reuse the running Docker service",
                    intent: gwt_agent::DockerLifecycleIntent::Connect,
                },
                DockerLifecycleOption {
                    label: "Restart then launch",
                    description: "Restart the Docker service before launching",
                    intent: gwt_agent::DockerLifecycleIntent::Restart,
                },
                DockerLifecycleOption {
                    label: "Recreate then launch",
                    description: "Force-recreate the Docker service before launching",
                    intent: gwt_agent::DockerLifecycleIntent::Recreate,
                },
            ],
            gwt_docker::ComposeServiceStatus::Stopped
            | gwt_docker::ComposeServiceStatus::Exited => &[
                DockerLifecycleOption {
                    label: "Start then launch",
                    description: "Start the existing Docker service",
                    intent: gwt_agent::DockerLifecycleIntent::Start,
                },
                DockerLifecycleOption {
                    label: "Recreate then launch",
                    description: "Force-recreate the Docker service before launching",
                    intent: gwt_agent::DockerLifecycleIntent::Recreate,
                },
            ],
            gwt_docker::ComposeServiceStatus::NotFound => &[DockerLifecycleOption {
                label: "Create and start then launch",
                description: "Create the Docker service and launch into it",
                intent: gwt_agent::DockerLifecycleIntent::CreateAndStart,
            }],
        }
    }

    fn current_version_options_for(&self, agent: &AgentOption) -> Vec<gwt_agent::VersionOption> {
        gwt_agent::build_version_options(
            agent.available,
            agent.installed_version.as_deref(),
            agent_has_npm_package(&agent.id),
            &agent.versions,
        )
    }

    fn current_version_options(&self) -> Vec<gwt_agent::VersionOption> {
        self.selected_agent()
            .map(|agent| self.current_version_options_for(agent))
            .unwrap_or_default()
    }

    fn current_reasoning_options(&self) -> &'static [ReasoningDisplayOption] {
        if self.agent_is_codex() {
            &CODEX_REASONING_OPTIONS
        } else if self.effective_agent_id() == "claude" && is_claude_opus_model(self.model.as_str())
        {
            &CLAUDE_OPUS_REASONING_OPTIONS
        } else if self.effective_agent_id() == "claude" && self.model == "sonnet" {
            &CLAUDE_SONNET_REASONING_OPTIONS
        } else {
            &[]
        }
    }

    fn selected_quick_start_action(&self) -> QuickStartAction {
        self.quick_start_actions()
            .get(self.selected)
            .copied()
            .unwrap_or(QuickStartAction::ChooseDifferent)
    }

    fn selected_quick_start_entry(&self) -> Option<&QuickStartEntry> {
        match self.selected_quick_start_action() {
            QuickStartAction::ReuseEntry { index } | QuickStartAction::StartNewEntry { index } => {
                self.quick_start_entries.get(index)
            }
            QuickStartAction::FocusExistingSession | QuickStartAction::ChooseDifferent => None,
        }
    }

    fn quick_start_actions(&self) -> Vec<QuickStartAction> {
        let mut actions = Vec::new();
        for (index, entry) in self.quick_start_entries.iter().enumerate() {
            if entry.can_reuse() {
                actions.push(QuickStartAction::ReuseEntry { index });
            }
            actions.push(QuickStartAction::StartNewEntry { index });
        }
        if !self.context.live_sessions.is_empty() {
            actions.push(QuickStartAction::FocusExistingSession);
        }
        actions.push(QuickStartAction::ChooseDifferent);
        actions
    }

    fn apply_quick_start_selection(&mut self) {
        let Some(entry) = self.selected_quick_start_entry().cloned() else {
            return;
        };

        self.launch_target = LaunchTargetKind::Agent;
        self.agent_id = entry.agent_id.clone();
        if let Some(index) = self
            .detected_agents
            .iter()
            .position(|agent| agent.id == entry.agent_id)
        {
            self.selected = index;
        }
        self.sync_selected_agent_options();

        self.apply_saved_model(entry.model.as_deref());
        if let Some(reasoning) = entry.reasoning {
            self.reasoning = reasoning;
        }
        if let Some(version) = entry.version {
            self.version = version;
        }
        self.skip_permissions = entry.skip_permissions;
        self.codex_fast_mode = entry.codex_fast_mode && self.agent_is_codex();
        self.runtime_target = entry.runtime_target;
        self.docker_service = entry.docker_service.clone();
        self.docker_lifecycle_intent = entry.docker_lifecycle_intent;
        self.sync_docker_lifecycle_default();

        match self.selected_quick_start_action() {
            QuickStartAction::ReuseEntry { .. } => {
                if let Some(window_id) = entry.live_window_id {
                    self.completion = Some(LaunchWizardCompletion::FocusWindow { window_id });
                } else if let Some(resume_session_id) = entry.resume_session_id {
                    self.mode = "resume".to_string();
                    self.resume_session_id = Some(resume_session_id);
                    match self.build_launch_request() {
                        Ok(config) => {
                            self.completion =
                                Some(LaunchWizardCompletion::Launch(Box::new(config)));
                        }
                        Err(error) => self.error = Some(error),
                    }
                } else {
                    self.error = Some("No saved session is available".to_string());
                }
            }
            QuickStartAction::StartNewEntry { .. } => {
                self.mode = "normal".to_string();
                self.resume_session_id = None;
                match self.build_launch_request() {
                    Ok(config) => {
                        self.completion = Some(LaunchWizardCompletion::Launch(Box::new(config)));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            QuickStartAction::FocusExistingSession | QuickStartAction::ChooseDifferent => {}
        }
    }

    fn current_options(&self) -> Vec<LaunchWizardOptionView> {
        match self.step {
            LaunchWizardStep::QuickStart => {
                let mut options = Vec::new();
                for (index, entry) in self.quick_start_entries.iter().enumerate() {
                    let summary = quick_start_summary(entry);
                    if let Some(reuse_action_label) = entry.reuse_action_label() {
                        options.push(LaunchWizardOptionView {
                            value: format!("reuse:{index}"),
                            label: format!("{reuse_action_label} {}", entry.tool_label),
                            description: Some(summary.clone()),
                            color: None,
                        });
                    }
                    options.push(LaunchWizardOptionView {
                        value: format!("start_new:{index}"),
                        label: format!("Start new with {}", entry.tool_label),
                        description: Some(summary),
                        color: None,
                    });
                }
                if !self.context.live_sessions.is_empty() {
                    options.push(LaunchWizardOptionView {
                        value: "focus_existing".to_string(),
                        label: "Focus existing session".to_string(),
                        description: Some("Jump to a running window on this branch".to_string()),
                        color: None,
                    });
                }
                options.push(LaunchWizardOptionView {
                    value: "choose_different".to_string(),
                    label: "Choose different".to_string(),
                    description: Some("Open the full launch wizard".to_string()),
                    color: None,
                });
                options
            }
            LaunchWizardStep::FocusExistingSession => self
                .context
                .live_sessions
                .iter()
                .map(|entry| LaunchWizardOptionView {
                    value: entry.window_id.clone(),
                    label: entry.name.clone(),
                    description: entry.detail.clone(),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::BranchAction => vec![
                LaunchWizardOptionView {
                    value: "use_selected".to_string(),
                    label: "Use selected branch".to_string(),
                    description: Some("Launch on the selected branch".to_string()),
                    color: None,
                },
                LaunchWizardOptionView {
                    value: "create_new".to_string(),
                    label: "Create new from selected".to_string(),
                    description: Some(
                        "Create a new branch based on the selected branch".to_string(),
                    ),
                    color: None,
                },
            ],
            LaunchWizardStep::BranchTypeSelect => BRANCH_TYPE_PREFIXES
                .iter()
                .map(|prefix| LaunchWizardOptionView {
                    value: (*prefix).to_string(),
                    label: (*prefix).to_string(),
                    description: Some(format!(
                        "Use {} as the branch prefix",
                        prefix.trim_end_matches('/')
                    )),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::LaunchTarget => launch_target_options_view(),
            LaunchWizardStep::AgentSelect => self
                .detected_agents
                .iter()
                .map(|agent| LaunchWizardOptionView {
                    value: agent.id.clone(),
                    label: agent.name.clone(),
                    description: Some(agent_description(agent)),
                    color: agent_option_color(&agent.id),
                })
                .collect(),
            LaunchWizardStep::ModelSelect => model_display_options(self.effective_agent_id())
                .iter()
                .map(|option| LaunchWizardOptionView {
                    value: option.label.to_string(),
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::ReasoningLevel => self
                .current_reasoning_options()
                .iter()
                .map(|option| LaunchWizardOptionView {
                    value: option.stored_value.to_string(),
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::RuntimeTarget => RUNTIME_TARGET_OPTIONS
                .iter()
                .map(|option| LaunchWizardOptionView {
                    value: option.label.to_ascii_lowercase(),
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::WindowsShell => windows_shell_options_view(),
            LaunchWizardStep::DockerServiceSelect => self
                .docker_service_options()
                .into_iter()
                .map(|service| LaunchWizardOptionView {
                    value: service.clone(),
                    label: service,
                    description: Some("Docker Compose service".to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::DockerLifecycle => self
                .docker_lifecycle_options()
                .iter()
                .map(|option| LaunchWizardOptionView {
                    value: docker_lifecycle_value(option.intent).to_string(),
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::VersionSelect => self
                .current_version_options()
                .into_iter()
                .map(|option| LaunchWizardOptionView {
                    value: option.value,
                    label: option.label,
                    description: Some("Tool version".to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::ExecutionMode => EXECUTION_MODE_OPTIONS
                .iter()
                .map(|option| LaunchWizardOptionView {
                    value: option.value.to_string(),
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::SkipPermissions => YES_NO_OPTIONS
                .iter()
                .map(|option| LaunchWizardOptionView {
                    value: option.label.to_ascii_lowercase(),
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::CodexFastMode => FAST_MODE_OPTIONS
                .iter()
                .map(|option| LaunchWizardOptionView {
                    value: option.label.to_ascii_lowercase(),
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::BranchNameInput => Vec::new(),
        }
    }
}

#[derive(Clone, Copy)]
struct ModelDisplayOption {
    label: &'static str,
    description: &'static str,
}

#[derive(Clone, Copy)]
struct ReasoningDisplayOption {
    label: &'static str,
    stored_value: &'static str,
    description: &'static str,
    is_default: bool,
}

#[derive(Clone, Copy)]
struct ChoiceOption {
    label: &'static str,
    description: &'static str,
}

#[derive(Clone, Copy)]
struct ExecutionModeOption {
    label: &'static str,
    description: &'static str,
    value: &'static str,
}

#[derive(Clone, Copy)]
struct DockerLifecycleOption {
    label: &'static str,
    description: &'static str,
    intent: gwt_agent::DockerLifecycleIntent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuickStartAction {
    ReuseEntry { index: usize },
    StartNewEntry { index: usize },
    FocusExistingSession,
    ChooseDifferent,
}

const CLAUDE_DEFAULT_MODEL_LABEL: &str = "Default (Opus 4.7)";

fn is_claude_opus_model(model: &str) -> bool {
    model == CLAUDE_DEFAULT_MODEL_LABEL || model == "opus"
}

fn is_claude_effort_capable_model(model: &str) -> bool {
    is_claude_opus_model(model) || model == "sonnet"
}

const CLAUDE_MODEL_OPTIONS: [ModelDisplayOption; 4] = [
    ModelDisplayOption {
        label: CLAUDE_DEFAULT_MODEL_LABEL,
        description: "Most capable for complex work",
    },
    ModelDisplayOption {
        label: "opus",
        description: "Deep reasoning for complex problems",
    },
    ModelDisplayOption {
        label: "sonnet",
        description: "Balanced speed and capability",
    },
    ModelDisplayOption {
        label: "haiku",
        description: "Fastest option for light tasks",
    },
];

const CODEX_MODEL_OPTIONS: [ModelDisplayOption; 7] = [
    ModelDisplayOption {
        label: "Default (Auto)",
        description: "Use Codex default model (gpt-5.5)",
    },
    ModelDisplayOption {
        label: "gpt-5.5",
        description: "Frontier model for complex coding, research, and real-world work",
    },
    ModelDisplayOption {
        label: "gpt-5.4",
        description: "Strong model for everyday coding",
    },
    ModelDisplayOption {
        label: "gpt-5.4-mini",
        description: "Small, fast, and cost-efficient model for simpler coding tasks",
    },
    ModelDisplayOption {
        label: "gpt-5.3-codex",
        description: "Coding-optimized model",
    },
    ModelDisplayOption {
        label: "gpt-5.3-codex-spark",
        description: "Ultra-fast coding model",
    },
    ModelDisplayOption {
        label: "gpt-5.2",
        description: "Optimized for professional work and long-running agents",
    },
];

const GEMINI_MODEL_OPTIONS: [ModelDisplayOption; 6] = [
    ModelDisplayOption {
        label: "Default (Auto)",
        description: "Use Gemini default model",
    },
    ModelDisplayOption {
        label: "gemini-3-pro-preview",
        description: "Preview pro model",
    },
    ModelDisplayOption {
        label: "gemini-3-flash-preview",
        description: "Preview flash model",
    },
    ModelDisplayOption {
        label: "gemini-2.5-pro",
        description: "Stable pro model",
    },
    ModelDisplayOption {
        label: "gemini-2.5-flash",
        description: "Balanced speed and reasoning",
    },
    ModelDisplayOption {
        label: "gemini-2.5-flash-lite",
        description: "Fastest Gemini option",
    },
];

const CLAUDE_OPUS_REASONING_OPTIONS: [ReasoningDisplayOption; 6] = [
    ReasoningDisplayOption {
        label: "Auto",
        stored_value: "auto",
        description: "Let the model choose the effort",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Low",
        stored_value: "low",
        description: "Fast responses for simple work",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Medium",
        stored_value: "medium",
        description: "Balanced reasoning for everyday work",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "High",
        stored_value: "high",
        description: "Deeper reasoning for complex work",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "xHigh",
        stored_value: "xhigh",
        description: "Best results for most coding tasks (Opus 4.7 default)",
        is_default: true,
    },
    ReasoningDisplayOption {
        label: "Max",
        stored_value: "max",
        description: "Deepest reasoning with no token-spending constraint",
        is_default: false,
    },
];

const CLAUDE_SONNET_REASONING_OPTIONS: [ReasoningDisplayOption; 4] = [
    ReasoningDisplayOption {
        label: "Auto",
        stored_value: "auto",
        description: "Let the model choose the effort",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Low",
        stored_value: "low",
        description: "Fast responses for simple work",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Medium",
        stored_value: "medium",
        description: "Balanced reasoning for everyday work",
        is_default: true,
    },
    ReasoningDisplayOption {
        label: "High",
        stored_value: "high",
        description: "Deeper reasoning for complex work",
        is_default: false,
    },
];

const CODEX_REASONING_OPTIONS: [ReasoningDisplayOption; 4] = [
    ReasoningDisplayOption {
        label: "Low",
        stored_value: "low",
        description: "Fast responses with lighter reasoning",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Medium",
        stored_value: "medium",
        description: "Balances speed and reasoning depth",
        is_default: true,
    },
    ReasoningDisplayOption {
        label: "High",
        stored_value: "high",
        description: "Greater reasoning depth",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Extra high",
        stored_value: "xhigh",
        description: "Maximum reasoning depth",
        is_default: false,
    },
];

const EXECUTION_MODE_OPTIONS: [ExecutionModeOption; 3] = [
    ExecutionModeOption {
        label: "Normal",
        description: "Start a new session",
        value: "normal",
    },
    ExecutionModeOption {
        label: "Continue",
        description: "Continue from the last session",
        value: "continue",
    },
    ExecutionModeOption {
        label: "Resume",
        description: "Resume the selected session metadata",
        value: "resume",
    },
];

const RUNTIME_TARGET_OPTIONS: [ChoiceOption; 2] = [
    ChoiceOption {
        label: "Host",
        description: "Run directly on the host",
    },
    ChoiceOption {
        label: "Docker",
        description: "Run inside the detected Docker service",
    },
];

const WINDOWS_SHELL_OPTIONS: [gwt_agent::WindowsShellKind; 3] = [
    gwt_agent::WindowsShellKind::CommandPrompt,
    gwt_agent::WindowsShellKind::WindowsPowerShell,
    gwt_agent::WindowsShellKind::PowerShell7,
];

const YES_NO_OPTIONS: [ChoiceOption; 2] = [
    ChoiceOption {
        label: "Yes",
        description: "Skip permission prompts",
    },
    ChoiceOption {
        label: "No",
        description: "Show permission prompts",
    },
];

const FAST_MODE_OPTIONS: [ChoiceOption; 2] = [
    ChoiceOption {
        label: "On",
        description: "Use Codex fast service tier",
    },
    ChoiceOption {
        label: "Off",
        description: "Use the standard service tier",
    },
];

fn default_docker_lifecycle_intent(
    status: gwt_docker::ComposeServiceStatus,
) -> gwt_agent::DockerLifecycleIntent {
    match status {
        gwt_docker::ComposeServiceStatus::Unknown => gwt_agent::DockerLifecycleIntent::Start,
        gwt_docker::ComposeServiceStatus::Running => gwt_agent::DockerLifecycleIntent::Connect,
        gwt_docker::ComposeServiceStatus::Stopped | gwt_docker::ComposeServiceStatus::Exited => {
            gwt_agent::DockerLifecycleIntent::Start
        }
        gwt_docker::ComposeServiceStatus::NotFound => {
            gwt_agent::DockerLifecycleIntent::CreateAndStart
        }
    }
}

struct LaunchWizardFlow<'a> {
    state: &'a LaunchWizardState,
}

impl<'a> LaunchWizardFlow<'a> {
    fn new(state: &'a LaunchWizardState) -> Self {
        Self { state }
    }

    fn next_step(&self, current: LaunchWizardStep) -> Option<LaunchWizardStep> {
        match current {
            LaunchWizardStep::QuickStart => match self.state.selected_quick_start_action() {
                QuickStartAction::ChooseDifferent => Some(LaunchWizardStep::BranchAction),
                QuickStartAction::FocusExistingSession => {
                    Some(LaunchWizardStep::FocusExistingSession)
                }
                QuickStartAction::ReuseEntry { .. } | QuickStartAction::StartNewEntry { .. } => {
                    Some(LaunchWizardStep::SkipPermissions)
                }
            },
            LaunchWizardStep::FocusExistingSession => None,
            LaunchWizardStep::BranchAction => {
                if self.state.selected == 0 {
                    Some(LaunchWizardStep::LaunchTarget)
                } else {
                    Some(LaunchWizardStep::BranchTypeSelect)
                }
            }
            LaunchWizardStep::BranchTypeSelect => Some(LaunchWizardStep::BranchNameInput),
            LaunchWizardStep::BranchNameInput => Some(LaunchWizardStep::LaunchTarget),
            LaunchWizardStep::LaunchTarget => self.next_after_launch_target(),
            LaunchWizardStep::AgentSelect => {
                if self.state.agent_has_models() {
                    Some(LaunchWizardStep::ModelSelect)
                } else {
                    self.next_after_agent_configuration()
                }
            }
            LaunchWizardStep::ModelSelect => {
                if self.state.agent_uses_reasoning_step() {
                    Some(LaunchWizardStep::ReasoningLevel)
                } else {
                    self.next_after_agent_configuration()
                }
            }
            LaunchWizardStep::ReasoningLevel => self.next_after_agent_configuration(),
            LaunchWizardStep::RuntimeTarget => self.next_after_runtime_target(),
            LaunchWizardStep::WindowsShell => self.next_after_windows_shell(),
            LaunchWizardStep::DockerServiceSelect => Some(LaunchWizardStep::DockerLifecycle),
            LaunchWizardStep::DockerLifecycle => self.next_after_docker_lifecycle(),
            LaunchWizardStep::VersionSelect => Some(LaunchWizardStep::ExecutionMode),
            LaunchWizardStep::ExecutionMode => Some(LaunchWizardStep::SkipPermissions),
            LaunchWizardStep::SkipPermissions => {
                if self.state.agent_is_codex() {
                    Some(LaunchWizardStep::CodexFastMode)
                } else {
                    None
                }
            }
            LaunchWizardStep::CodexFastMode => None,
        }
    }

    fn prev_step(&self, current: LaunchWizardStep) -> Option<LaunchWizardStep> {
        match current {
            LaunchWizardStep::QuickStart => None,
            LaunchWizardStep::FocusExistingSession => Some(LaunchWizardStep::QuickStart),
            LaunchWizardStep::BranchAction => {
                if !self.state.quick_start_entries.is_empty()
                    || !self.state.context.live_sessions.is_empty()
                {
                    Some(LaunchWizardStep::QuickStart)
                } else {
                    None
                }
            }
            LaunchWizardStep::BranchTypeSelect => Some(LaunchWizardStep::BranchAction),
            LaunchWizardStep::BranchNameInput => Some(LaunchWizardStep::BranchTypeSelect),
            LaunchWizardStep::LaunchTarget => {
                if self.state.is_new_branch {
                    Some(LaunchWizardStep::BranchNameInput)
                } else {
                    Some(LaunchWizardStep::BranchAction)
                }
            }
            LaunchWizardStep::AgentSelect => Some(LaunchWizardStep::LaunchTarget),
            LaunchWizardStep::ModelSelect => Some(LaunchWizardStep::AgentSelect),
            LaunchWizardStep::ReasoningLevel => Some(LaunchWizardStep::ModelSelect),
            LaunchWizardStep::RuntimeTarget => {
                if self.state.launch_target_is_shell() {
                    Some(LaunchWizardStep::LaunchTarget)
                } else {
                    self.previous_agent_configuration_step()
                }
            }
            LaunchWizardStep::WindowsShell => self.previous_before_windows_shell(),
            LaunchWizardStep::DockerServiceSelect => Some(LaunchWizardStep::RuntimeTarget),
            LaunchWizardStep::DockerLifecycle => {
                if self.state.docker_service_prompt_required() {
                    Some(LaunchWizardStep::DockerServiceSelect)
                } else {
                    Some(LaunchWizardStep::RuntimeTarget)
                }
            }
            LaunchWizardStep::VersionSelect => self.previous_before_version_select(),
            LaunchWizardStep::ExecutionMode => self.previous_before_execution_mode(),
            LaunchWizardStep::SkipPermissions => Some(LaunchWizardStep::ExecutionMode),
            LaunchWizardStep::CodexFastMode => Some(LaunchWizardStep::SkipPermissions),
        }
    }

    fn next_after_launch_target(&self) -> Option<LaunchWizardStep> {
        if self.state.launch_target_is_agent() {
            Some(LaunchWizardStep::AgentSelect)
        } else if self.state.has_docker_workflow() {
            Some(LaunchWizardStep::RuntimeTarget)
        } else {
            self.next_after_host_runtime()
        }
    }

    fn next_after_agent_configuration(&self) -> Option<LaunchWizardStep> {
        if self.state.has_docker_workflow() {
            Some(LaunchWizardStep::RuntimeTarget)
        } else {
            self.next_after_host_runtime()
        }
    }

    fn next_after_runtime_target(&self) -> Option<LaunchWizardStep> {
        if self.state.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker
            && self.state.docker_service_prompt_required()
        {
            Some(LaunchWizardStep::DockerServiceSelect)
        } else if self.state.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
            Some(LaunchWizardStep::DockerLifecycle)
        } else {
            self.next_after_host_runtime()
        }
    }

    fn next_after_host_runtime(&self) -> Option<LaunchWizardStep> {
        if self.state.show_windows_shell_selection() {
            Some(LaunchWizardStep::WindowsShell)
        } else {
            self.next_after_windows_shell()
        }
    }

    fn next_after_windows_shell(&self) -> Option<LaunchWizardStep> {
        if self.state.launch_target_is_shell() {
            None
        } else if agent_has_npm_package(self.state.effective_agent_id()) {
            Some(LaunchWizardStep::VersionSelect)
        } else {
            Some(LaunchWizardStep::ExecutionMode)
        }
    }

    fn next_after_docker_lifecycle(&self) -> Option<LaunchWizardStep> {
        if self.state.launch_target_is_shell() {
            None
        } else if agent_has_npm_package(self.state.effective_agent_id()) {
            Some(LaunchWizardStep::VersionSelect)
        } else {
            Some(LaunchWizardStep::ExecutionMode)
        }
    }

    fn previous_agent_configuration_step(&self) -> Option<LaunchWizardStep> {
        if self.state.agent_uses_reasoning_step() {
            Some(LaunchWizardStep::ReasoningLevel)
        } else if self.state.agent_has_models() {
            Some(LaunchWizardStep::ModelSelect)
        } else {
            Some(LaunchWizardStep::AgentSelect)
        }
    }

    fn previous_before_windows_shell(&self) -> Option<LaunchWizardStep> {
        if self.state.has_docker_workflow() {
            Some(LaunchWizardStep::RuntimeTarget)
        } else if self.state.launch_target_is_shell() {
            Some(LaunchWizardStep::LaunchTarget)
        } else {
            self.previous_agent_configuration_step()
        }
    }

    fn previous_before_version_select(&self) -> Option<LaunchWizardStep> {
        if self.state.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
            Some(LaunchWizardStep::DockerLifecycle)
        } else if self.state.show_windows_shell_selection() {
            Some(LaunchWizardStep::WindowsShell)
        } else if self.state.has_docker_workflow() {
            Some(LaunchWizardStep::RuntimeTarget)
        } else {
            self.previous_agent_configuration_step()
        }
    }

    fn previous_before_execution_mode(&self) -> Option<LaunchWizardStep> {
        if self.state.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
            Some(LaunchWizardStep::DockerLifecycle)
        } else if self.state.show_windows_shell_selection() {
            Some(LaunchWizardStep::WindowsShell)
        } else if agent_has_npm_package(self.state.effective_agent_id()) {
            Some(LaunchWizardStep::VersionSelect)
        } else if self.state.has_docker_workflow() {
            Some(LaunchWizardStep::RuntimeTarget)
        } else {
            self.previous_agent_configuration_step()
        }
    }
}

fn next_step(current: LaunchWizardStep, state: &LaunchWizardState) -> Option<LaunchWizardStep> {
    LaunchWizardFlow::new(state).next_step(current)
}

fn prev_step(current: LaunchWizardStep, state: &LaunchWizardState) -> Option<LaunchWizardStep> {
    LaunchWizardFlow::new(state).prev_step(current)
}

fn step_default_selection(step: LaunchWizardStep, state: &LaunchWizardState) -> usize {
    match step {
        LaunchWizardStep::QuickStart => 0,
        LaunchWizardStep::FocusExistingSession => 0,
        LaunchWizardStep::BranchAction => 0,
        LaunchWizardStep::BranchTypeSelect => 0,
        LaunchWizardStep::BranchNameInput => 0,
        LaunchWizardStep::LaunchTarget => usize::from(state.launch_target_is_shell()),
        LaunchWizardStep::AgentSelect => state
            .detected_agents
            .iter()
            .position(|agent| agent.id == state.agent_id)
            .unwrap_or(0),
        LaunchWizardStep::ModelSelect => current_model_options(state.effective_agent_id())
            .iter()
            .position(|model| model == &state.model)
            .unwrap_or(0),
        LaunchWizardStep::ReasoningLevel => state
            .current_reasoning_options()
            .iter()
            .position(|option| option.stored_value == state.reasoning)
            .unwrap_or_else(|| {
                state
                    .current_reasoning_options()
                    .iter()
                    .position(|option| option.is_default)
                    .unwrap_or(0)
            }),
        LaunchWizardStep::RuntimeTarget => {
            usize::from(state.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker)
        }
        LaunchWizardStep::WindowsShell => WINDOWS_SHELL_OPTIONS
            .iter()
            .position(|option| *option == state.windows_shell)
            .unwrap_or(0),
        LaunchWizardStep::DockerServiceSelect => state
            .preferred_docker_service()
            .and_then(|service| {
                state
                    .docker_service_options()
                    .iter()
                    .position(|option| option == service)
            })
            .unwrap_or(0),
        LaunchWizardStep::DockerLifecycle => state
            .docker_lifecycle_options()
            .iter()
            .position(|option| option.intent == state.docker_lifecycle_intent)
            .unwrap_or(0),
        LaunchWizardStep::VersionSelect => state
            .current_version_options()
            .iter()
            .position(|option| option.value == state.version)
            .unwrap_or(0),
        LaunchWizardStep::ExecutionMode => EXECUTION_MODE_OPTIONS
            .iter()
            .position(|option| option.value == state.mode)
            .unwrap_or(0),
        LaunchWizardStep::SkipPermissions => usize::from(!state.skip_permissions),
        LaunchWizardStep::CodexFastMode => usize::from(!state.codex_fast_mode),
    }
}

fn current_model_options(agent_id: &str) -> Vec<&'static str> {
    match agent_id {
        "claude" => CLAUDE_MODEL_OPTIONS
            .iter()
            .map(|option| option.label)
            .collect(),
        "codex" => CODEX_MODEL_OPTIONS
            .iter()
            .map(|option| option.label)
            .collect(),
        "gemini" => GEMINI_MODEL_OPTIONS
            .iter()
            .map(|option| option.label)
            .collect(),
        _ => Vec::new(),
    }
}

fn model_display_options(agent_id: &str) -> &'static [ModelDisplayOption] {
    match agent_id {
        "claude" => &CLAUDE_MODEL_OPTIONS,
        "codex" => &CODEX_MODEL_OPTIONS,
        "gemini" => &GEMINI_MODEL_OPTIONS,
        _ => &[],
    }
}

fn quick_start_summary(entry: &QuickStartEntry) -> String {
    let mut parts = vec![entry.tool_label.clone()];
    if let Some(model) = entry.model.as_deref() {
        parts.push(model.to_string());
    }
    if let Some(reasoning) = entry.reasoning.as_deref() {
        parts.push(reasoning.to_string());
    }
    if let Some(version) = entry.version.as_deref() {
        parts.push(version.to_string());
    }
    if entry.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
        parts.push(
            entry
                .docker_service
                .as_ref()
                .map(|service| format!("docker:{service}"))
                .unwrap_or_else(|| "docker".to_string()),
        );
    }
    parts.join(" · ")
}

fn branch_type_options_view() -> Vec<LaunchWizardOptionView> {
    BRANCH_TYPE_PREFIXES
        .iter()
        .map(|prefix| LaunchWizardOptionView {
            value: (*prefix).to_string(),
            label: (*prefix).to_string(),
            description: Some(format!(
                "Use {} as the branch prefix",
                prefix.trim_end_matches('/')
            )),
            color: None,
        })
        .collect()
}

fn launch_target_options_view() -> Vec<LaunchWizardOptionView> {
    vec![
        LaunchWizardOptionView {
            value: "agent".to_string(),
            label: "Agent".to_string(),
            description: Some("Launch a coding agent terminal".to_string()),
            color: None,
        },
        LaunchWizardOptionView {
            value: "shell".to_string(),
            label: "Shell".to_string(),
            description: Some("Open a plain shell terminal".to_string()),
            color: None,
        },
    ]
}

fn runtime_target_options_view() -> Vec<LaunchWizardOptionView> {
    RUNTIME_TARGET_OPTIONS
        .iter()
        .map(|option| LaunchWizardOptionView {
            value: option.label.to_ascii_lowercase(),
            label: option.label.to_string(),
            description: Some(option.description.to_string()),
            color: None,
        })
        .collect()
}

fn windows_shell_options_view() -> Vec<LaunchWizardOptionView> {
    WINDOWS_SHELL_OPTIONS
        .iter()
        .copied()
        .map(|shell| LaunchWizardOptionView {
            value: windows_shell_option_value(shell).to_string(),
            label: windows_shell_option_label(shell).to_string(),
            description: Some(windows_shell_option_description(shell).to_string()),
            color: None,
        })
        .collect()
}

fn windows_shell_option_value(shell: gwt_agent::WindowsShellKind) -> &'static str {
    match shell {
        gwt_agent::WindowsShellKind::CommandPrompt => "command_prompt",
        gwt_agent::WindowsShellKind::WindowsPowerShell => "windows_power_shell",
        gwt_agent::WindowsShellKind::PowerShell7 => "power_shell_7",
    }
}

fn windows_shell_option_label(shell: gwt_agent::WindowsShellKind) -> &'static str {
    match shell {
        gwt_agent::WindowsShellKind::CommandPrompt => "Command Prompt",
        gwt_agent::WindowsShellKind::WindowsPowerShell => "Windows PowerShell",
        gwt_agent::WindowsShellKind::PowerShell7 => "PowerShell 7",
    }
}

fn windows_shell_option_description(shell: gwt_agent::WindowsShellKind) -> &'static str {
    match shell {
        gwt_agent::WindowsShellKind::CommandPrompt => "Run through cmd.exe",
        gwt_agent::WindowsShellKind::WindowsPowerShell => "Run through Windows PowerShell",
        gwt_agent::WindowsShellKind::PowerShell7 => "Run through PowerShell 7",
    }
}

fn windows_shell_detection_command(shell: gwt_agent::WindowsShellKind) -> &'static str {
    match shell {
        gwt_agent::WindowsShellKind::CommandPrompt => "cmd.exe",
        gwt_agent::WindowsShellKind::WindowsPowerShell => "powershell",
        gwt_agent::WindowsShellKind::PowerShell7 => "pwsh",
    }
}

fn default_windows_shell_kind() -> gwt_agent::WindowsShellKind {
    default_windows_shell_kind_with(gwt_core::process::command_exists)
}

fn default_windows_shell_kind_with<F>(mut command_exists: F) -> gwt_agent::WindowsShellKind
where
    F: FnMut(&str) -> bool,
{
    if command_exists(windows_shell_detection_command(
        gwt_agent::WindowsShellKind::PowerShell7,
    )) {
        return gwt_agent::WindowsShellKind::PowerShell7;
    }
    if command_exists(windows_shell_detection_command(
        gwt_agent::WindowsShellKind::WindowsPowerShell,
    )) {
        return gwt_agent::WindowsShellKind::WindowsPowerShell;
    }
    gwt_agent::WindowsShellKind::CommandPrompt
}

fn execution_mode_options_view() -> Vec<LaunchWizardOptionView> {
    EXECUTION_MODE_OPTIONS
        .iter()
        .map(|option| LaunchWizardOptionView {
            value: option.value.to_string(),
            label: option.label.to_string(),
            description: Some(option.description.to_string()),
            color: None,
        })
        .collect()
}

fn execution_mode_value_from_session_mode(mode: gwt_agent::SessionMode) -> &'static str {
    match mode {
        gwt_agent::SessionMode::Normal => "normal",
        gwt_agent::SessionMode::Continue | gwt_agent::SessionMode::Resume => "continue",
    }
}

fn launch_target_value(target: LaunchTargetKind) -> &'static str {
    match target {
        LaunchTargetKind::Agent => "agent",
        LaunchTargetKind::Shell => "shell",
    }
}

fn runtime_target_value(target: gwt_agent::LaunchRuntimeTarget) -> &'static str {
    match target {
        gwt_agent::LaunchRuntimeTarget::Host => "host",
        gwt_agent::LaunchRuntimeTarget::Docker => "docker",
    }
}

fn docker_lifecycle_value(intent: gwt_agent::DockerLifecycleIntent) -> &'static str {
    match intent {
        gwt_agent::DockerLifecycleIntent::Connect => "connect",
        gwt_agent::DockerLifecycleIntent::Start => "start",
        gwt_agent::DockerLifecycleIntent::Restart => "restart",
        gwt_agent::DockerLifecycleIntent::Recreate => "recreate",
        gwt_agent::DockerLifecycleIntent::CreateAndStart => "create_and_start",
    }
}

#[cfg(test)]
fn apply_branch_prefix(seed: &str, prefix: &str) -> String {
    let trimmed = seed.trim();
    let suffix = BRANCH_TYPE_PREFIXES
        .iter()
        .find_map(|known| trimmed.strip_prefix(known))
        .unwrap_or(trimmed)
        .trim_matches('/');
    if suffix.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}{suffix}")
    }
}

fn is_explicit_model_selection(model: &str) -> bool {
    !model.is_empty() && !model.starts_with("Default")
}

fn agent_has_npm_package(agent_id: &str) -> bool {
    matches!(agent_id, "claude" | "codex" | "gemini")
}

fn agent_id_from_key(agent_id: &str) -> gwt_agent::AgentId {
    match agent_id {
        "claude" => gwt_agent::AgentId::ClaudeCode,
        "codex" => gwt_agent::AgentId::Codex,
        "gemini" => gwt_agent::AgentId::Gemini,
        "gh" => gwt_agent::AgentId::Copilot,
        other => gwt_agent::AgentId::Custom(other.to_string()),
    }
}

fn agent_description(agent: &AgentOption) -> String {
    match agent.installed_version.as_deref() {
        Some(version) => format!("Detected · {version}"),
        None if agent.custom_agent.is_some() => "Configured".to_string(),
        None => "Built-in".to_string(),
    }
}

fn load_global_custom_agents() -> Vec<gwt_agent::CustomCodingAgent> {
    if std::env::var_os(gwt_agent::DISABLE_GLOBAL_CUSTOM_AGENTS_ENV).is_some() {
        return Vec::new();
    }

    gwt_agent::load_custom_agents_from_path(&gwt_core::paths::gwt_config_path()).unwrap_or_default()
}

/// Map the raw agent option id (command name or custom agent id) to the
/// AgentColor rendered on the Launch Wizard candidate row.
/// SPEC #2133 FR-009 / シナリオ 2.
fn agent_option_color(agent_id: &str) -> Option<gwt_agent::AgentColor> {
    gwt_agent::resolve_agent_id(agent_id).map(|id| id.default_color())
}

pub fn default_wizard_version_cache_path() -> PathBuf {
    gwt_core::paths::gwt_cache_dir().join("agent-versions.json")
}

pub fn build_agent_options(
    detected_agents: Vec<gwt_agent::DetectedAgent>,
    cache: &gwt_agent::VersionCache,
    custom_agents: Vec<gwt_agent::CustomCodingAgent>,
) -> Vec<AgentOption> {
    let mut options = build_builtin_agent_options(detected_agents, cache);
    options.extend(custom_agents.into_iter().map(|agent| AgentOption {
        id: agent.id.clone(),
        name: agent.display_name.clone(),
        available: true,
        installed_version: None,
        versions: Vec::new(),
        custom_agent: Some(agent),
    }));
    options
}

pub fn load_agent_options(cache: &gwt_agent::VersionCache) -> Vec<AgentOption> {
    build_agent_options(Vec::new(), cache, load_global_custom_agents())
}

pub fn build_builtin_agent_options(
    detected_agents: Vec<gwt_agent::DetectedAgent>,
    cache: &gwt_agent::VersionCache,
) -> Vec<AgentOption> {
    const BUILTIN: [gwt_agent::AgentId; 4] = [
        gwt_agent::AgentId::ClaudeCode,
        gwt_agent::AgentId::Codex,
        gwt_agent::AgentId::Gemini,
        gwt_agent::AgentId::Copilot,
    ];

    BUILTIN
        .into_iter()
        .map(|agent_id| {
            let detected = detected_agents
                .iter()
                .find(|detected| detected.agent_id == agent_id);
            AgentOption {
                id: agent_id.command().to_string(),
                name: agent_id.display_name().to_string(),
                available: true,
                installed_version: detected.and_then(|detected| detected.version.clone()),
                versions: cache
                    .get(&agent_id)
                    .map(|versions| versions.to_vec())
                    .unwrap_or_default(),
                custom_agent: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{TimeZone, Utc};
    use tempfile::tempdir;

    use super::*;

    fn sample_agent_options() -> Vec<AgentOption> {
        vec![
            AgentOption {
                id: "claude".to_string(),
                name: "Claude Code".to_string(),
                available: true,
                installed_version: Some("1.0.0".to_string()),
                versions: vec!["0.9.0".to_string(), "1.0.0".to_string()],
                custom_agent: None,
            },
            AgentOption {
                id: "codex".to_string(),
                name: "Codex".to_string(),
                available: true,
                installed_version: Some("0.110.0".to_string()),
                versions: vec!["0.109.0".to_string(), "0.110.0".to_string()],
                custom_agent: None,
            },
        ]
    }

    fn sample_custom_agent(
        id: &str,
        display_name: &str,
        agent_type: gwt_agent::custom::CustomAgentType,
        command: impl Into<String>,
    ) -> gwt_agent::CustomCodingAgent {
        gwt_agent::CustomCodingAgent {
            id: id.to_string(),
            display_name: display_name.to_string(),
            agent_type,
            command: command.into(),
            default_args: vec!["--serve".to_string()],
            mode_args: Some(gwt_agent::custom::ModeArgs {
                normal: Vec::new(),
                continue_mode: vec!["--continue".to_string()],
                resume: vec!["--resume".to_string()],
            }),
            skip_permissions_args: vec!["--unsafe".to_string()],
            env: HashMap::from([("API_KEY".to_string(), "secret".to_string())]),
        }
    }

    #[test]
    fn agent_option_color_maps_known_ids_and_falls_back_to_gray() {
        assert_eq!(
            agent_option_color("claude"),
            Some(gwt_agent::AgentColor::Yellow)
        );
        assert_eq!(
            agent_option_color("codex"),
            Some(gwt_agent::AgentColor::Cyan)
        );
        assert_eq!(
            agent_option_color("gemini"),
            Some(gwt_agent::AgentColor::Magenta)
        );
        assert_eq!(
            agent_option_color("opencode"),
            Some(gwt_agent::AgentColor::Green)
        );
        assert_eq!(agent_option_color("gh"), Some(gwt_agent::AgentColor::Blue));
        assert_eq!(
            agent_option_color("my-custom"),
            Some(gwt_agent::AgentColor::Gray)
        );
        assert_eq!(agent_option_color(""), None);
    }

    #[test]
    fn build_agent_options_appends_config_backed_custom_agents_after_builtins() {
        let dir = tempdir().expect("tempdir");
        let available_path = dir.path().join("custom-agent");
        std::fs::write(&available_path, "echo custom").expect("write custom agent stub");
        let missing_path = dir.path().join("missing-agent");

        let options = build_agent_options(
            vec![gwt_agent::DetectedAgent {
                agent_id: gwt_agent::AgentId::ClaudeCode,
                version: Some("1.2.3".to_string()),
                path: PathBuf::from("/tmp/claude"),
            }],
            &gwt_agent::VersionCache::new(),
            vec![
                sample_custom_agent(
                    "proxy-agent",
                    "Claude Proxy",
                    gwt_agent::custom::CustomAgentType::Path,
                    available_path.display().to_string(),
                ),
                sample_custom_agent(
                    "missing-agent",
                    "Missing Agent",
                    gwt_agent::custom::CustomAgentType::Path,
                    missing_path.display().to_string(),
                ),
            ],
        );

        let proxy = options
            .iter()
            .position(|option| option.id == "proxy-agent")
            .expect("custom agent appended");
        let missing = options
            .iter()
            .position(|option| option.id == "missing-agent")
            .expect("missing custom agent appended");

        assert!(proxy > 0, "custom agents must appear after builtin options");
        assert!(missing > proxy, "custom agents should keep append order");
        assert_eq!(options[proxy].name, "Claude Proxy");
        assert!(options[proxy].available);
        assert!(
            options[missing].available,
            "configured custom agents must stay selectable; runtime preparation validates execution"
        );
    }

    fn branch(name: &str) -> BranchListEntry {
        BranchListEntry {
            name: name.to_string(),
            scope: crate::BranchScope::Local,
            is_head: false,
            upstream: Some(format!("origin/{name}")),
            ahead: 0,
            behind: 0,
            last_commit_date: None,
            cleanup_ready: true,
            cleanup: crate::BranchCleanupInfo::default(),
        }
    }

    fn context(branch: BranchListEntry, normalized: &str) -> LaunchWizardContext {
        LaunchWizardContext {
            selected_branch: branch,
            normalized_branch_name: normalized.to_string(),
            worktree_path: None,
            quick_start_root: PathBuf::from("/tmp/repo"),
            live_sessions: Vec::new(),
            docker_context: None,
            docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
            linked_issue_number: None,
            linked_issue_kind: None,
        }
    }

    fn context_with_linked_issue(
        branch: BranchListEntry,
        normalized: &str,
        kind: LinkedIssueKind,
        number: u64,
    ) -> LaunchWizardContext {
        let mut ctx = context(branch, normalized);
        ctx.linked_issue_kind = Some(kind);
        ctx.linked_issue_number = Some(number);
        ctx
    }

    fn sample_session(
        dir: &Path,
        branch: &str,
        worktree_path: &Path,
        agent_id: gwt_agent::AgentId,
        updated_at: chrono::DateTime<Utc>,
        resume_id: &str,
    ) {
        sample_session_with_resume(
            dir,
            branch,
            worktree_path,
            agent_id,
            updated_at,
            Some(resume_id),
        );
    }

    fn sample_session_with_resume(
        dir: &Path,
        branch: &str,
        worktree_path: &Path,
        agent_id: gwt_agent::AgentId,
        updated_at: chrono::DateTime<Utc>,
        resume_id: Option<&str>,
    ) {
        let mut session = gwt_agent::Session::new(worktree_path, branch, agent_id);
        session.display_name = session.agent_id.display_name().to_string();
        session.agent_session_id = resume_id.map(str::to_string);
        session.tool_version = Some("installed".to_string());
        session.model = Some("gpt-5.5".to_string());
        session.reasoning_level = Some("high".to_string());
        session.skip_permissions = true;
        session.codex_fast_mode = true;
        session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        session.docker_service = Some("gwt".to_string());
        session.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;
        session.created_at = updated_at;
        session.updated_at = updated_at;
        session.last_activity_at = updated_at;
        session.save(dir).expect("save session");
    }

    fn sample_session_record(
        branch: &str,
        worktree_path: &Path,
        agent_id: gwt_agent::AgentId,
        updated_at: chrono::DateTime<Utc>,
        resume_id: Option<&str>,
    ) -> gwt_agent::Session {
        let mut session = gwt_agent::Session::new(worktree_path, branch, agent_id);
        session.display_name = session.agent_id.display_name().to_string();
        session.agent_session_id = resume_id.map(str::to_string);
        session.tool_version = Some("installed".to_string());
        session.model = Some("gpt-5.5".to_string());
        session.reasoning_level = Some("high".to_string());
        session.skip_permissions = true;
        session.codex_fast_mode = true;
        session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        session.docker_service = Some("gwt".to_string());
        session.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;
        session.created_at = updated_at;
        session.updated_at = updated_at;
        session.last_activity_at = updated_at;
        session
    }

    fn init_repo_with_origin(path: &Path, origin: &str) {
        std::fs::create_dir_all(path).expect("repo dir");
        let status = gwt_core::process::hidden_command("git")
            .args(["init"])
            .current_dir(path)
            .status()
            .expect("git init");
        assert!(status.success(), "git init failed");
        let status = gwt_core::process::hidden_command("git")
            .args(["remote", "add", "origin", origin])
            .current_dir(path)
            .status()
            .expect("git remote add");
        assert!(status.success(), "git remote add failed");
    }

    fn quick_start_entry(
        session_id: &str,
        agent_id: &str,
        resume_session_id: Option<&str>,
        live_window_id: Option<&str>,
        runtime_target: gwt_agent::LaunchRuntimeTarget,
        docker_service: Option<&str>,
    ) -> QuickStartEntry {
        let (tool_label, model, reasoning, version, codex_fast_mode) = match agent_id {
            "claude" => (
                "Claude Code",
                Some("sonnet"),
                Some("medium"),
                Some("latest"),
                false,
            ),
            "codex" => (
                "Codex",
                Some("gpt-5.5"),
                Some("high"),
                Some("0.110.0"),
                true,
            ),
            _ => ("Custom", None, None, None, false),
        };
        QuickStartEntry {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            tool_label: tool_label.to_string(),
            model: model.map(str::to_string),
            reasoning: reasoning.map(str::to_string),
            version: version.map(str::to_string),
            resume_session_id: resume_session_id.map(str::to_string),
            live_window_id: live_window_id.map(str::to_string),
            skip_permissions: true,
            codex_fast_mode,
            runtime_target,
            docker_service: docker_service.map(str::to_string),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
        }
    }

    #[test]
    fn open_local_branch_without_quick_start_starts_at_branch_action() {
        let state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );

        assert_eq!(state.step, LaunchWizardStep::BranchAction);
        assert_eq!(state.branch_name, "feature/gui");
        assert!(!state.is_new_branch);
    }

    #[test]
    fn open_with_quick_start_prefers_quick_start_step() {
        let state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            vec![QuickStartEntry {
                session_id: "gwt-session-1".to_string(),
                agent_id: "codex".to_string(),
                tool_label: "Codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                resume_session_id: Some("resume-1".to_string()),
                live_window_id: None,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            }],
        );

        assert_eq!(state.step, LaunchWizardStep::QuickStart);
    }

    #[test]
    fn load_quick_start_entries_prefers_latest_session_per_agent() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        sample_session(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 0, 0).unwrap(),
            "older",
        );
        sample_session(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            "newer",
        );

        let entries = load_quick_start_entries(&worktree, dir.path(), "feature/gui");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "codex");
        assert_eq!(entries[0].resume_session_id.as_deref(), Some("newer"));
        assert_eq!(entries[0].docker_service.as_deref(), Some("gwt"));
    }

    #[test]
    fn load_quick_start_entries_uses_latest_resumable_session_when_latest_lacks_resume_id() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        sample_session(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 0, 0).unwrap(),
            "resume-older",
        );
        sample_session_with_resume(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            None,
        );

        let entries = load_quick_start_entries(&worktree, dir.path(), "feature/gui");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "codex");
        assert_eq!(
            entries[0].resume_session_id.as_deref(),
            Some("resume-older")
        );
    }

    #[test]
    fn load_quick_start_entries_does_not_reuse_resume_id_from_other_scope() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        let other_worktree = dir.path().join("other-repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        std::fs::create_dir_all(&other_worktree).expect("other repo dir");
        sample_session(
            dir.path(),
            "feature/other",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 0, 0).unwrap(),
            "wrong-branch",
        );
        sample_session(
            dir.path(),
            "feature/gui",
            &other_worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 30, 0).unwrap(),
            "wrong-worktree",
        );
        sample_session_with_resume(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            None,
        );

        let entries = load_quick_start_entries(&worktree, dir.path(), "feature/gui");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "codex");
        assert!(entries[0].resume_session_id.is_none());
    }

    #[test]
    fn collect_quick_start_entries_from_sessions_reuses_resumable_session_profile() {
        let worktree = PathBuf::from("/tmp/repo");
        let mut older = sample_session_record(
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 0, 0).unwrap(),
            Some("resume-older"),
        );
        older.tool_version = Some("0.110.0".to_string());
        older.model = Some("gpt-5.5".to_string());
        older.reasoning_level = Some("high".to_string());
        older.skip_permissions = true;
        older.codex_fast_mode = true;
        older.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        older.docker_service = Some("gwt".to_string());

        let mut newer = sample_session_record(
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            None,
        );
        newer.tool_version = Some("0.111.0".to_string());
        newer.model = Some("gpt-5.4-mini".to_string());
        newer.reasoning_level = Some("low".to_string());
        newer.skip_permissions = false;
        newer.codex_fast_mode = false;
        newer.runtime_target = gwt_agent::LaunchRuntimeTarget::Host;
        newer.docker_service = None;

        let entries = super::quick_start::collect_quick_start_entries_from_sessions(
            &worktree,
            "feature/gui",
            vec![older.clone(), newer],
        );

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].session_id, older.id);
        assert_eq!(entries[0].agent_id, "codex");
        assert_eq!(
            entries[0].resume_session_id.as_deref(),
            Some("resume-older")
        );
        assert_eq!(entries[0].model.as_deref(), Some("gpt-5.5"));
        assert_eq!(entries[0].reasoning.as_deref(), Some("high"));
        assert_eq!(entries[0].version.as_deref(), Some("0.110.0"));
        assert_eq!(
            entries[0].runtime_target,
            gwt_agent::LaunchRuntimeTarget::Docker
        );
        assert_eq!(entries[0].docker_service.as_deref(), Some("gwt"));
        assert!(entries[0].skip_permissions);
        assert!(entries[0].codex_fast_mode);
    }

    #[test]
    fn load_previous_launch_profile_uses_latest_session_for_repo_without_reusing_branch() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        let mut older = sample_session_record(
            "feature/old",
            &worktree,
            gwt_agent::AgentId::ClaudeCode,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 0, 0).unwrap(),
            None,
        );
        older.session_mode = gwt_agent::SessionMode::Normal;
        older.save(dir.path()).expect("save older session");

        let mut newer = sample_session_record(
            "feature/previous",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            Some("resume-ignored"),
        );
        newer.tool_version = Some("0.110.0".to_string());
        newer.model = Some("gpt-5.5".to_string());
        newer.reasoning_level = Some("high".to_string());
        newer.session_mode = gwt_agent::SessionMode::Continue;
        newer.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        newer.docker_service = Some("gwt".to_string());
        newer.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;
        newer.save(dir.path()).expect("save newer session");

        let profile =
            load_previous_launch_profile(&worktree, dir.path()).expect("previous profile");

        assert_eq!(profile.agent_id, "codex");
        assert_eq!(profile.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(profile.reasoning.as_deref(), Some("high"));
        assert_eq!(profile.version.as_deref(), Some("0.110.0"));
        assert_eq!(profile.session_mode, gwt_agent::SessionMode::Continue);
        assert_eq!(
            profile.runtime_target,
            gwt_agent::LaunchRuntimeTarget::Docker
        );
        assert_eq!(profile.docker_service.as_deref(), Some("gwt"));
    }

    #[test]
    fn previous_launch_profile_tie_breaks_equal_timestamps_by_session_id() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        let timestamp = Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap();
        let mut lower_id = sample_session_record(
            "feature/lower",
            &worktree,
            gwt_agent::AgentId::Codex,
            timestamp,
            None,
        );
        lower_id.id = "session-a".to_string();
        lower_id.model = Some("gpt-5.4".to_string());
        let mut higher_id = sample_session_record(
            "feature/higher",
            &worktree,
            gwt_agent::AgentId::Codex,
            timestamp,
            None,
        );
        higher_id.id = "session-b".to_string();
        higher_id.model = Some("gpt-5.5".to_string());

        let profile = previous_launch_profile_from_sessions(
            &worktree,
            &[higher_id.clone(), lower_id.clone()],
        )
        .expect("profile");
        assert_eq!(profile.model.as_deref(), Some("gpt-5.5"));

        let profile = previous_launch_profile_from_sessions(&worktree, &[lower_id, higher_id])
            .expect("profile");
        assert_eq!(profile.model.as_deref(), Some("gpt-5.5"));
    }

    #[test]
    fn load_previous_launch_profile_matches_deleted_worktree_by_persisted_repo_hash() {
        let dir = tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let origin = "https://github.com/example/project.git";
        init_repo_with_origin(&repo, origin);
        let removed_worktree = dir.path().join("removed-worktree");
        let mut session = sample_session_record(
            "feature/removed",
            &removed_worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            None,
        );
        session.repo_hash = Some(
            gwt_core::repo_hash::compute_repo_hash(origin)
                .as_str()
                .to_string(),
        );
        session
            .save(dir.path())
            .expect("save removed worktree session");

        let profile = load_previous_launch_profile(&repo, dir.path())
            .expect("profile should match persisted repo identity");

        assert_eq!(profile.agent_id, "codex");
    }

    #[test]
    fn branch_action_create_new_from_selected_sets_base_branch() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );

        state.apply(LaunchWizardAction::Select { index: 1 });

        assert_eq!(state.step, LaunchWizardStep::BranchTypeSelect);
        assert!(state.is_new_branch);
        assert_eq!(state.base_branch_name.as_deref(), Some("feature/gui"));
    }

    // SPEC-2014 FR-024/025: branch name auto-seed from SPEC/Issue window.

    #[test]
    fn branch_seed_uses_issue_kind_when_create_new_then_feature_prefix() {
        let mut state = LaunchWizardState::open_with(
            context_with_linked_issue(branch("develop"), "develop", LinkedIssueKind::Issue, 42),
            sample_agent_options(),
            Vec::new(),
        );
        state.apply(LaunchWizardAction::SetBranchMode { create_new: true });
        state.apply(LaunchWizardAction::SetBranchType {
            prefix: "feature/".to_string(),
        });
        assert_eq!(state.branch_name, "feature/issue-42");
    }

    #[test]
    fn branch_seed_uses_spec_kind_when_create_new_then_feature_prefix() {
        let mut state = LaunchWizardState::open_with(
            context_with_linked_issue(branch("develop"), "develop", LinkedIssueKind::Spec, 2014),
            sample_agent_options(),
            Vec::new(),
        );
        state.apply(LaunchWizardAction::SetBranchMode { create_new: true });
        state.apply(LaunchWizardAction::SetBranchType {
            prefix: "feature/".to_string(),
        });
        assert_eq!(state.branch_name, "feature/spec-2014");
    }

    #[test]
    fn branch_seed_uses_issue_kind_when_alternative_prefix_selected() {
        let mut state = LaunchWizardState::open_with(
            context_with_linked_issue(branch("develop"), "develop", LinkedIssueKind::Issue, 10),
            sample_agent_options(),
            Vec::new(),
        );
        state.apply(LaunchWizardAction::SetBranchMode { create_new: true });
        state.apply(LaunchWizardAction::SetBranchType {
            prefix: "bugfix/".to_string(),
        });
        assert_eq!(state.branch_name, "bugfix/issue-10");
    }

    #[test]
    fn branch_seed_omits_when_no_linked_issue_kind() {
        let mut state = LaunchWizardState::open_with(
            context(branch("develop"), "develop"),
            sample_agent_options(),
            Vec::new(),
        );
        state.apply(LaunchWizardAction::SetBranchMode { create_new: true });
        state.apply(LaunchWizardAction::SetBranchType {
            prefix: "feature/".to_string(),
        });
        assert_eq!(state.branch_name, "feature/");
    }

    #[test]
    fn branch_seed_respects_user_edit_when_prefix_changes() {
        let mut state = LaunchWizardState::open_with(
            context_with_linked_issue(branch("develop"), "develop", LinkedIssueKind::Issue, 42),
            sample_agent_options(),
            Vec::new(),
        );
        state.apply(LaunchWizardAction::SetBranchMode { create_new: true });
        state.apply(LaunchWizardAction::SetBranchType {
            prefix: "feature/".to_string(),
        });
        // user replaces the auto-seeded suffix
        state.apply(LaunchWizardAction::SetBranchName {
            value: "feature/custom-name".to_string(),
        });
        // switching prefix should preserve the user-entered suffix
        state.apply(LaunchWizardAction::SetBranchType {
            prefix: "bugfix/".to_string(),
        });
        assert_eq!(state.branch_name, "bugfix/custom-name");
    }

    #[test]
    fn branch_seed_omits_title_slug_for_spec_proposal_a() {
        // SPEC-2014 FR-025 (Proposal A): SPEC seed must be `spec-{n}` only,
        // not the legacy gwt-tui `feature/spec-{n}-{title-slug}` form.
        let ctx =
            context_with_linked_issue(branch("develop"), "develop", LinkedIssueKind::Spec, 2014);
        let suffix = ctx
            .linked_issue_branch_suffix()
            .expect("linked issue branch suffix");
        assert_eq!(suffix, "spec-2014");
    }

    #[test]
    fn branch_seed_create_new_then_default_prefix_seeds_branch_name() {
        // SetBranchMode { create_new: true } currently primes branch_name to
        // BRANCH_TYPE_PREFIXES[0] (=feature/). With a linked issue context the
        // prime should include the auto-seeded suffix as well.
        let mut state = LaunchWizardState::open_with(
            context_with_linked_issue(branch("develop"), "develop", LinkedIssueKind::Issue, 7),
            sample_agent_options(),
            Vec::new(),
        );
        state.apply(LaunchWizardAction::SetBranchMode { create_new: true });
        assert_eq!(state.branch_name, "feature/issue-7");
    }

    #[test]
    fn build_launch_config_for_codex_resume_uses_resume_session_id() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );
        state.agent_id = "codex".to_string();
        state.model = "gpt-5.5".to_string();
        state.reasoning = "high".to_string();
        state.version = "0.110.0".to_string();
        state.mode = "resume".to_string();
        state.resume_session_id = Some("session-123".to_string());
        state.skip_permissions = true;
        state.codex_fast_mode = true;
        state.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        state.docker_service = Some("gwt".to_string());
        state.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;

        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.agent_id, gwt_agent::AgentId::Codex);
        assert_eq!(config.branch.as_deref(), Some("feature/gui"));
        assert_eq!(config.resume_session_id.as_deref(), Some("session-123"));
        assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
        assert_eq!(config.reasoning_level.as_deref(), Some("high"));
        assert_eq!(config.tool_version.as_deref(), Some("0.110.0"));
        assert_eq!(config.docker_service.as_deref(), Some("gwt"));
        assert!(config.skip_permissions);
        assert!(config.codex_fast_mode);
    }

    #[test]
    fn panel_quick_start_resume_populates_launch_state() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            vec![QuickStartEntry {
                session_id: "gwt-session-1".to_string(),
                agent_id: "codex".to_string(),
                tool_label: "Codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                resume_session_id: Some("resume-1".to_string()),
                live_window_id: None,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
                docker_service: Some("gwt".to_string()),
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
            }],
        );

        state.apply(LaunchWizardAction::ApplyQuickStart {
            index: 0,
            mode: QuickStartLaunchMode::Resume,
        });

        assert_eq!(state.agent_id, "codex");
        assert_eq!(state.model, "gpt-5.5");
        assert_eq!(state.reasoning, "high");
        assert_eq!(state.version, "0.110.0");
        assert_eq!(state.mode, "resume");
        assert_eq!(state.resume_session_id.as_deref(), Some("resume-1"));
        assert_eq!(state.runtime_target, gwt_agent::LaunchRuntimeTarget::Docker);
        assert_eq!(state.docker_service.as_deref(), Some("gwt"));
        assert!(state.skip_permissions);
        assert!(state.codex_fast_mode);
    }

    #[test]
    fn quick_start_with_removed_codex_model_falls_back_to_auto() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            vec![QuickStartEntry {
                session_id: "gwt-session-1".to_string(),
                agent_id: "codex".to_string(),
                tool_label: "Codex".to_string(),
                model: Some("gpt-5.2-codex".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                resume_session_id: Some("resume-1".to_string()),
                live_window_id: None,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            }],
        );

        state.apply(LaunchWizardAction::ApplyQuickStart {
            index: 0,
            mode: QuickStartLaunchMode::Resume,
        });

        assert_eq!(state.model, "Default (Auto)");
        match state.completion.as_ref() {
            Some(LaunchWizardCompletion::Launch(config)) => match config.as_ref() {
                LaunchWizardLaunchRequest::Agent(config) => {
                    assert!(config.model.is_none());
                }
                other => panic!("expected agent launch request, got {other:?}"),
            },
            other => panic!("expected launch completion, got {other:?}"),
        }
    }

    #[test]
    fn quick_start_reuse_prefers_live_window_focus() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.live_sessions = vec![LiveSessionEntry {
            session_id: "gwt-session-1".to_string(),
            window_id: "window-1".to_string(),
            agent_id: "codex".to_string(),
            kind: "agent".to_string(),
            name: "Codex".to_string(),
            detail: Some("/tmp/repo".to_string()),
            active: true,
        }];

        let mut state = LaunchWizardState::open_with(
            ctx,
            sample_agent_options(),
            vec![QuickStartEntry {
                session_id: "gwt-session-1".to_string(),
                agent_id: "codex".to_string(),
                tool_label: "Codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                resume_session_id: Some("resume-1".to_string()),
                live_window_id: None,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            }],
        );

        let view = state.view();
        assert_eq!(
            view.quick_start_entries[0].reuse_action_label.as_deref(),
            Some("Focus")
        );

        state.apply(LaunchWizardAction::ApplyQuickStart {
            index: 0,
            mode: QuickStartLaunchMode::Resume,
        });

        match state.completion.as_ref() {
            Some(LaunchWizardCompletion::FocusWindow { window_id }) => {
                assert_eq!(window_id, "window-1");
            }
            other => panic!("expected focus completion, got {other:?}"),
        }
    }

    #[test]
    fn quick_start_start_new_keeps_live_window_available_but_does_not_focus_it() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.live_sessions = vec![LiveSessionEntry {
            session_id: "gwt-session-1".to_string(),
            window_id: "window-1".to_string(),
            agent_id: "codex".to_string(),
            kind: "agent".to_string(),
            name: "Codex".to_string(),
            detail: Some("/tmp/repo".to_string()),
            active: true,
        }];

        let mut state = LaunchWizardState::open_with(
            ctx,
            sample_agent_options(),
            vec![QuickStartEntry {
                session_id: "gwt-session-1".to_string(),
                agent_id: "codex".to_string(),
                tool_label: "Codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                resume_session_id: Some("resume-1".to_string()),
                live_window_id: None,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            }],
        );

        state.apply(LaunchWizardAction::ApplyQuickStart {
            index: 0,
            mode: QuickStartLaunchMode::StartNew,
        });

        assert_eq!(state.mode, "normal");
        assert!(state.resume_session_id.is_none());
        match state.completion.as_ref() {
            Some(LaunchWizardCompletion::Launch(config)) => match config.as_ref() {
                LaunchWizardLaunchRequest::Agent(config) => {
                    assert_eq!(config.session_mode, gwt_agent::SessionMode::Normal);
                    assert!(config.resume_session_id.is_none());
                }
                other => panic!("expected agent launch request, got {other:?}"),
            },
            other => panic!("expected launch completion, got {other:?}"),
        }
    }

    #[test]
    fn quick_start_view_hides_reuse_action_without_live_or_saved_session() {
        let state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            vec![QuickStartEntry {
                session_id: "gwt-session-1".to_string(),
                agent_id: "codex".to_string(),
                tool_label: "Codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                resume_session_id: None,
                live_window_id: None,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            }],
        );

        let view = state.view();
        assert!(view.quick_start_entries[0].reuse_action_label.is_none());
    }

    #[test]
    fn open_with_previous_profile_restores_full_profile_without_reusing_branch() {
        let mut ctx = context(branch("feature/current"), "feature/current");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "gwt".to_string()],
            suggested_service: Some("api".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let state = LaunchWizardState::open_with_previous_profile(
            ctx,
            sample_agent_options(),
            Vec::new(),
            Some(LaunchWizardPreviousProfile {
                agent_id: "codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                session_mode: gwt_agent::SessionMode::Continue,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
                docker_service: Some("gwt".to_string()),
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
                windows_shell: None,
            }),
        );

        let view = state.view();
        assert_eq!(view.branch_name, "feature/current");
        assert_eq!(view.selected_agent_id, "codex");
        assert_eq!(view.selected_model, "gpt-5.5");
        assert_eq!(view.selected_reasoning, "high");
        assert_eq!(view.selected_version, "0.110.0");
        assert_eq!(view.selected_execution_mode, "continue");
        assert_eq!(view.selected_runtime_target, "docker");
        assert_eq!(view.selected_docker_service.as_deref(), Some("gwt"));
        assert_eq!(view.selected_docker_lifecycle, "restart");
        assert!(view.skip_permissions);
        assert!(view.codex_fast_mode);

        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.branch.as_deref(), Some("feature/current"));
        assert_eq!(config.session_mode, gwt_agent::SessionMode::Continue);
        assert!(config.resume_session_id.is_none());
        assert_eq!(config.linked_issue_number, None);
    }

    #[test]
    fn previous_profile_docker_service_falls_back_to_current_suggestion() {
        let mut ctx = context(branch("feature/current"), "feature/current");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "worker".to_string()],
            suggested_service: Some("worker".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let state = LaunchWizardState::open_with_previous_profile(
            ctx,
            sample_agent_options(),
            Vec::new(),
            Some(LaunchWizardPreviousProfile {
                agent_id: "codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                session_mode: gwt_agent::SessionMode::Normal,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
                docker_service: Some("missing".to_string()),
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
                windows_shell: None,
            }),
        );

        let view = state.view();
        assert_eq!(view.selected_runtime_target, "docker");
        assert_eq!(view.selected_docker_service.as_deref(), Some("worker"));
    }

    #[test]
    fn previous_profile_docker_runtime_falls_back_to_host_without_context() {
        let state = LaunchWizardState::open_with_previous_profile(
            context(branch("feature/current"), "feature/current"),
            sample_agent_options(),
            Vec::new(),
            Some(LaunchWizardPreviousProfile {
                agent_id: "codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                session_mode: gwt_agent::SessionMode::Normal,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
                docker_service: Some("gwt".to_string()),
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
                windows_shell: None,
            }),
        );

        let view = state.view();
        assert_eq!(view.selected_runtime_target, "host");
        assert!(view.selected_docker_service.is_none());
        assert!(!view.show_docker_lifecycle);
    }

    #[test]
    fn previous_profile_keeps_saved_builtin_agent_without_host_detection() {
        let mut options = sample_agent_options();
        options
            .iter_mut()
            .find(|option| option.id == "codex")
            .expect("codex option")
            .available = false;
        let state = LaunchWizardState::open_with_previous_profile(
            context(branch("feature/current"), "feature/current"),
            options,
            Vec::new(),
            Some(LaunchWizardPreviousProfile {
                agent_id: "codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                session_mode: gwt_agent::SessionMode::Normal,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
                windows_shell: None,
            }),
        );

        assert_eq!(state.view().selected_agent_id, "codex");
        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.agent_id, gwt_agent::AgentId::Codex);
    }

    #[test]
    fn previous_profile_uses_builtin_agent_even_when_none_are_host_detected() {
        let mut options = sample_agent_options();
        for option in &mut options {
            option.available = false;
        }
        let state = LaunchWizardState::open_with_previous_profile(
            context(branch("feature/current"), "feature/current"),
            options,
            Vec::new(),
            Some(LaunchWizardPreviousProfile {
                agent_id: "codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                session_mode: gwt_agent::SessionMode::Normal,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
                windows_shell: None,
            }),
        );

        assert_eq!(state.view().selected_agent_id, "codex");
        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.agent_id, gwt_agent::AgentId::Codex);
    }

    #[test]
    fn set_agent_keeps_launch_config_on_selected_agent_when_index_is_stale() {
        let mut options = sample_agent_options();
        options
            .iter_mut()
            .find(|option| option.id == "claude")
            .expect("claude option")
            .available = false;
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/current"), "feature/current"),
            options,
            Vec::new(),
        );
        state.step = LaunchWizardStep::AgentSelect;
        state.selected = 0;

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "codex".to_string(),
        });

        assert_eq!(state.error, None);
        assert_eq!(state.agent_id, "codex");
        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.agent_id, gwt_agent::AgentId::Codex);
    }

    #[test]
    fn hydration_syncs_docker_lifecycle_when_previous_profile_is_not_applicable() {
        let mut state = LaunchWizardState::open_loading(
            context(branch("feature/gui"), "feature/gui"),
            Vec::new(),
        );
        state.apply_hydration(LaunchWizardHydration {
            selected_branch: Some(branch("origin/feature/gui")),
            normalized_branch_name: "feature/gui".to_string(),
            worktree_path: Some(PathBuf::from("/tmp/repo-feature")),
            quick_start_root: PathBuf::from("/tmp/repo-feature"),
            docker_context: Some(DockerWizardContext {
                services: vec!["app".to_string()],
                suggested_service: Some("app".to_string()),
            }),
            docker_service_status: gwt_docker::ComposeServiceStatus::Running,
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profile: Some(LaunchWizardPreviousProfile {
                agent_id: "missing-agent".to_string(),
                model: None,
                reasoning: None,
                version: None,
                session_mode: gwt_agent::SessionMode::Normal,
                skip_permissions: false,
                codex_fast_mode: false,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::CreateAndStart,
                windows_shell: None,
            }),
        });

        assert_eq!(
            state.docker_lifecycle_intent,
            gwt_agent::DockerLifecycleIntent::Connect
        );
    }

    #[test]
    fn panel_submit_requires_branch_name_for_new_branch() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );

        state.apply(LaunchWizardAction::SetBranchMode { create_new: true });
        state.apply(LaunchWizardAction::SetBranchName {
            value: "".to_string(),
        });
        state.apply(LaunchWizardAction::Submit);

        assert!(state.completion.is_none());
        assert_eq!(state.error.as_deref(), Some("Branch name is required"));
    }

    #[test]
    fn panel_view_exposes_selected_values_and_summary() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );
        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "codex".to_string(),
        });
        state.apply(LaunchWizardAction::SetModel {
            model: "gpt-5.5".to_string(),
        });
        state.apply(LaunchWizardAction::SetReasoning {
            reasoning: "high".to_string(),
        });
        state.apply(LaunchWizardAction::SetRuntimeTarget {
            target: gwt_agent::LaunchRuntimeTarget::Host,
        });
        state.apply(LaunchWizardAction::SetVersion {
            version: "0.110.0".to_string(),
        });
        state.apply(LaunchWizardAction::SetSkipPermissions { enabled: true });
        state.apply(LaunchWizardAction::SetCodexFastMode { enabled: true });

        let view = state.view();

        assert_eq!(view.branch_mode, "use_selected");
        assert_eq!(view.selected_agent_id, "codex");
        assert_eq!(view.selected_model, "gpt-5.5");
        assert_eq!(view.selected_reasoning, "high");
        assert_eq!(view.selected_runtime_target, "host");
        assert_eq!(view.selected_version, "0.110.0");
        assert!(view.show_reasoning);
        assert!(view.show_version);
        assert!(view.show_codex_fast_mode);
        assert!(view
            .launch_summary
            .iter()
            .any(|item| item.label == "Agent" && item.value == "Codex"));
        assert!(view
            .launch_summary
            .iter()
            .any(|item| item.label == "Fast mode" && item.value == "on"));
    }

    #[test]
    fn mutator_methods_validate_and_normalize_launch_options() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "worker".to_string()],
            suggested_service: Some("worker".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let mut state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());

        state.set_branch_mode(true);
        assert!(state.is_new_branch);
        assert_eq!(state.base_branch_name.as_deref(), Some("feature/gui"));
        assert_eq!(state.branch_name, "feature/");

        state.branch_name = "feature/coverage".to_string();
        state.set_branch_type("bugfix/");
        assert_eq!(state.branch_name, "bugfix/coverage");
        state.set_branch_type("fix/");
        assert_eq!(state.error.as_deref(), Some("Branch type is unavailable"));

        state.mode = "resume".to_string();
        state.resume_session_id = Some("resume-1".to_string());
        state.skip_permissions = true;
        state.codex_fast_mode = true;
        state.set_launch_target(LaunchTargetKind::Shell);
        assert_eq!(state.mode, "normal");
        assert!(state.resume_session_id.is_none());
        assert!(!state.skip_permissions);
        assert!(!state.codex_fast_mode);

        state.set_launch_target(LaunchTargetKind::Agent);
        state.set_agent_id("codex");
        assert_eq!(state.agent_id, "codex");
        state.set_agent_id("missing");
        assert_eq!(state.error.as_deref(), Some("Agent option is unavailable"));

        state.set_model("gpt-5.5");
        assert_eq!(state.model, "gpt-5.5");
        state.set_model("bad-model");
        assert_eq!(state.error.as_deref(), Some("Model option is unavailable"));

        state.set_reasoning("high");
        assert_eq!(state.reasoning, "high");
        state.set_reasoning("extreme");
        assert_eq!(
            state.error.as_deref(),
            Some("Reasoning option is unavailable")
        );

        state.set_runtime_target(gwt_agent::LaunchRuntimeTarget::Docker);
        assert_eq!(state.runtime_target, gwt_agent::LaunchRuntimeTarget::Docker);
        assert_eq!(state.docker_service.as_deref(), Some("worker"));

        state.set_docker_service("api");
        assert_eq!(state.docker_service.as_deref(), Some("api"));
        state.set_docker_service("missing");
        assert_eq!(
            state.error.as_deref(),
            Some("Docker service is unavailable")
        );

        state.set_docker_lifecycle(gwt_agent::DockerLifecycleIntent::Connect);
        assert_eq!(
            state.docker_lifecycle_intent,
            gwt_agent::DockerLifecycleIntent::Connect
        );
        state.set_docker_lifecycle(gwt_agent::DockerLifecycleIntent::CreateAndStart);
        assert_eq!(
            state.error.as_deref(),
            Some("Docker lifecycle option is unavailable")
        );

        state.error = None;
        state.set_version("0.110.0");
        assert_eq!(state.version, "0.110.0");
        state.set_version("definitely-missing");
        assert_eq!(
            state.error.as_deref(),
            Some("Version option is unavailable")
        );

        state.resume_session_id = Some("resume-2".to_string());
        state.error = None;
        state.set_execution_mode("continue");
        assert_eq!(state.mode, "continue");
        assert!(state.resume_session_id.is_none());
        state.set_execution_mode("invalid");
        assert_eq!(
            state.error.as_deref(),
            Some("Execution mode is unavailable")
        );
    }

    #[test]
    fn private_selection_and_completion_helpers_cover_focus_and_submit_paths() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "worker".to_string()],
            suggested_service: Some("api".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        ctx.live_sessions = vec![LiveSessionEntry {
            session_id: "session-1".to_string(),
            window_id: "window-1".to_string(),
            agent_id: "codex".to_string(),
            kind: "agent".to_string(),
            name: "Codex".to_string(),
            detail: Some("/tmp/repo".to_string()),
            active: true,
        }];
        let mut state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());

        state.step = LaunchWizardStep::FocusExistingSession;
        state.selected = 0;
        state.apply_selection();
        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::FocusWindow { window_id }) if window_id == "window-1"
        ));

        state.completion = None;
        state.selected = 9;
        state.apply_selection();
        assert_eq!(
            state.error.as_deref(),
            Some("No running session is available")
        );

        state.step = LaunchWizardStep::BranchAction;
        state.selected = 1;
        state.apply_selection();
        assert!(state.is_new_branch);

        state.step = LaunchWizardStep::BranchTypeSelect;
        state.selected = 1;
        state.apply_selection();
        assert!(state.branch_name.starts_with("bugfix/"));

        state.step = LaunchWizardStep::LaunchTarget;
        state.selected = 1;
        state.apply_selection();
        assert!(state.launch_target_is_shell());

        state.step = LaunchWizardStep::LaunchTarget;
        state.selected = 0;
        state.apply_selection();
        state.step = LaunchWizardStep::AgentSelect;
        state.selected = 1;
        state.apply_selection();
        assert_eq!(state.agent_id, "codex");

        state.step = LaunchWizardStep::ModelSelect;
        state.selected = 1;
        state.apply_selection();
        assert_eq!(state.model, "gpt-5.5");

        state.step = LaunchWizardStep::ReasoningLevel;
        state.selected = 1;
        state.apply_selection();
        assert!(!state.reasoning.is_empty());

        state.step = LaunchWizardStep::RuntimeTarget;
        state.selected = 1;
        state.apply_selection();
        assert_eq!(state.runtime_target, gwt_agent::LaunchRuntimeTarget::Docker);

        state.step = LaunchWizardStep::DockerServiceSelect;
        state.selected = 0;
        state.apply_selection();
        assert_eq!(state.docker_service.as_deref(), Some("api"));

        state.step = LaunchWizardStep::DockerLifecycle;
        state.selected = 0;
        state.apply_selection();

        state.step = LaunchWizardStep::VersionSelect;
        state.selected = 0;
        state.apply_selection();
        assert!(!state.version.is_empty());

        state.step = LaunchWizardStep::ExecutionMode;
        state.selected = 1;
        state.apply_selection();
        assert_eq!(state.mode, "continue");

        state.step = LaunchWizardStep::SkipPermissions;
        state.selected = 0;
        state.apply_selection();
        assert!(state.skip_permissions);

        state.step = LaunchWizardStep::CodexFastMode;
        state.selected = 0;
        state.apply_selection();
        assert!(state.codex_fast_mode);

        state.completion = None;
        state.step = LaunchWizardStep::CodexFastMode;
        state.advance_after_current_step();
        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::Launch(_))
        ));

        state.completion = None;
        state.set_launch_target(LaunchTargetKind::Shell);
        state.submit_panel();
        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::Launch(config))
                if matches!(config.as_ref(), LaunchWizardLaunchRequest::Shell(_))
        ));

        state.step = LaunchWizardStep::BranchNameInput;
        state.completion = None;
        state.error = None;
        state.apply(LaunchWizardAction::SubmitText {
            value: "  hotfix/coverage  ".to_string(),
        });
        assert_eq!(state.branch_name, "hotfix/coverage");
    }

    #[test]
    fn shell_target_hides_agent_specific_controls_and_builds_shell_request() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.worktree_path = Some(PathBuf::from("/tmp/repo-feature"));
        let mut state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());

        state.apply(LaunchWizardAction::SetLaunchTarget {
            target: LaunchTargetKind::Shell,
        });

        let view = state.view();
        assert_eq!(view.selected_launch_target, "shell");
        assert!(!view.show_agent_settings);
        assert!(!view.show_execution_mode);
        assert!(!view.show_skip_permissions);
        assert!(!view.show_version);
        assert!(view
            .launch_summary
            .iter()
            .any(|item| item.label == "Target" && item.value == "Shell"));
        assert!(!view.launch_summary.iter().any(|item| item.label == "Agent"));

        match state.build_launch_request().expect("shell launch request") {
            LaunchWizardLaunchRequest::Shell(config) => {
                assert_eq!(
                    config.working_dir.as_deref(),
                    Some(Path::new("/tmp/repo-feature"))
                );
                assert_eq!(config.branch.as_deref(), Some("feature/gui"));
                assert_eq!(config.display_name, "Shell");
                assert_eq!(config.runtime_target, gwt_agent::LaunchRuntimeTarget::Host);
            }
            other => panic!("expected shell launch request, got {other:?}"),
        }
    }

    #[test]
    fn default_windows_shell_kind_prefers_pwsh_then_windows_powershell_then_cmd() {
        let shell = default_windows_shell_kind_with(|command| command == "pwsh");
        assert_eq!(shell, gwt_agent::WindowsShellKind::PowerShell7);

        let shell = default_windows_shell_kind_with(|command| command == "powershell");
        assert_eq!(shell, gwt_agent::WindowsShellKind::WindowsPowerShell);

        let shell = default_windows_shell_kind_with(|_| false);
        assert_eq!(shell, gwt_agent::WindowsShellKind::CommandPrompt);
    }

    #[test]
    fn windows_shell_option_metadata_is_owned_by_launch_wizard() {
        assert_eq!(
            windows_shell_option_value(gwt_agent::WindowsShellKind::CommandPrompt),
            "command_prompt"
        );
        assert_eq!(
            windows_shell_option_label(gwt_agent::WindowsShellKind::WindowsPowerShell),
            "Windows PowerShell"
        );
        assert_eq!(
            windows_shell_option_description(gwt_agent::WindowsShellKind::PowerShell7),
            "Run through PowerShell 7"
        );
    }

    #[test]
    fn launch_wizard_flow_policy_centralizes_host_shell_step() {
        let state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );
        let flow = LaunchWizardFlow::new(&state);
        let expected_host_tail = if cfg!(windows) {
            Some(LaunchWizardStep::WindowsShell)
        } else if agent_has_npm_package(state.effective_agent_id()) {
            Some(LaunchWizardStep::VersionSelect)
        } else {
            Some(LaunchWizardStep::ExecutionMode)
        };

        assert_eq!(flow.next_after_agent_configuration(), expected_host_tail);

        let mut docker = state.clone();
        docker.context.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "worker".to_string()],
            suggested_service: Some("api".to_string()),
        });
        docker.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;

        assert_ne!(
            LaunchWizardFlow::new(&docker).next_after_runtime_target(),
            Some(LaunchWizardStep::WindowsShell)
        );
    }

    #[test]
    fn windows_shell_selection_flows_to_agent_and_shell_launch_requests() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.worktree_path = Some(PathBuf::from("/tmp/repo-feature"));
        let mut state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());

        state.apply(LaunchWizardAction::SetWindowsShell {
            shell: gwt_agent::WindowsShellKind::PowerShell7,
        });

        let view = state.view();
        assert_eq!(view.windows_shell_options.len(), 3);
        assert!(view
            .windows_shell_options
            .iter()
            .any(|option| option.label == "PowerShell 7"));
        if cfg!(windows) {
            assert_eq!(
                view.selected_windows_shell.as_deref(),
                Some("power_shell_7")
            );
            assert!(view
                .launch_summary
                .iter()
                .any(|item| item.label == "Shell" && item.value == "PowerShell 7"));
        } else {
            assert_eq!(view.selected_windows_shell.as_deref(), None);
        }

        let config = state.build_launch_config().expect("agent config");
        if cfg!(windows) {
            assert_eq!(
                config.windows_shell,
                Some(gwt_agent::WindowsShellKind::PowerShell7)
            );
        } else {
            assert_eq!(config.windows_shell, None);
        }

        state.apply(LaunchWizardAction::SetLaunchTarget {
            target: LaunchTargetKind::Shell,
        });

        match state.build_launch_request().expect("shell request") {
            LaunchWizardLaunchRequest::Shell(config) => {
                if cfg!(windows) {
                    assert_eq!(
                        config.windows_shell,
                        Some(gwt_agent::WindowsShellKind::PowerShell7)
                    );
                } else {
                    assert_eq!(config.windows_shell, None);
                }
            }
            other => panic!("expected shell launch request, got {other:?}"),
        }
    }

    #[test]
    fn docker_runtime_omits_windows_shell_from_launch_requests() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string()],
            suggested_service: Some("api".to_string()),
        });
        let mut state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());

        state.apply(LaunchWizardAction::SetWindowsShell {
            shell: gwt_agent::WindowsShellKind::CommandPrompt,
        });
        state.apply(LaunchWizardAction::SetRuntimeTarget {
            target: gwt_agent::LaunchRuntimeTarget::Docker,
        });

        let config = state.build_launch_config().expect("agent config");
        assert_eq!(
            config.runtime_target,
            gwt_agent::LaunchRuntimeTarget::Docker
        );
        assert_eq!(config.windows_shell, None);

        state.apply(LaunchWizardAction::SetLaunchTarget {
            target: LaunchTargetKind::Shell,
        });
        match state.build_launch_request().expect("shell request") {
            LaunchWizardLaunchRequest::Shell(config) => {
                assert_eq!(
                    config.runtime_target,
                    gwt_agent::LaunchRuntimeTarget::Docker
                );
                assert_eq!(config.windows_shell, None);
            }
            other => panic!("expected shell launch request, got {other:?}"),
        }
    }

    #[test]
    fn build_launch_config_preserves_linked_issue_number() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.linked_issue_number = Some(1234);

        let state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());

        let config = state.build_launch_config().expect("config");

        assert_eq!(config.linked_issue_number, Some(1234));
    }

    #[test]
    fn build_launch_config_for_custom_agent_uses_stored_definition() {
        let dir = tempdir().expect("tempdir");
        let custom_path = dir.path().join("custom-agent");
        std::fs::write(&custom_path, "echo custom").expect("write custom agent stub");

        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            build_agent_options(
                Vec::new(),
                &gwt_agent::VersionCache::new(),
                vec![sample_custom_agent(
                    "proxy-agent",
                    "Claude Proxy",
                    gwt_agent::custom::CustomAgentType::Path,
                    custom_path.display().to_string(),
                )],
            ),
            Vec::new(),
        );
        state.set_agent_id("proxy-agent");
        state.set_execution_mode("resume");
        state.resume_session_id = Some("resume-1".to_string());
        state.skip_permissions = true;

        let config = state.build_launch_config().expect("custom launch config");

        assert_eq!(config.command, custom_path.display().to_string());
        assert_eq!(config.display_name, "Claude Proxy");
        assert!(config.args.contains(&"--serve".to_string()));
        assert!(config.args.contains(&"--resume".to_string()));
        assert!(config.args.contains(&"--unsafe".to_string()));
        assert_eq!(
            config.env_vars.get("API_KEY").map(String::as_str),
            Some("secret")
        );
    }

    #[test]
    fn build_launch_config_allows_configured_custom_agent_without_host_detection() {
        let missing_path = PathBuf::from("/tmp/nonexistent-custom-agent");
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            build_agent_options(
                Vec::new(),
                &gwt_agent::VersionCache::new(),
                vec![sample_custom_agent(
                    "missing-agent",
                    "Missing Agent",
                    gwt_agent::custom::CustomAgentType::Path,
                    missing_path.display().to_string(),
                )],
            ),
            Vec::new(),
        );
        state.set_agent_id("missing-agent");

        let config = state
            .build_launch_config()
            .expect("configured custom agent should reach runtime preparation");
        assert_eq!(config.command, missing_path.display().to_string());
        assert_eq!(config.display_name, "Missing Agent");
    }

    #[test]
    fn quick_start_resume_for_custom_agent_uses_config_backed_definition() {
        let dir = tempdir().expect("tempdir");
        let custom_path = dir.path().join("custom-agent");
        std::fs::write(&custom_path, "echo custom").expect("write custom agent stub");

        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            build_agent_options(
                Vec::new(),
                &gwt_agent::VersionCache::new(),
                vec![sample_custom_agent(
                    "proxy-agent",
                    "Claude Proxy",
                    gwt_agent::custom::CustomAgentType::Path,
                    custom_path.display().to_string(),
                )],
            ),
            vec![QuickStartEntry {
                session_id: "gwt-session-1".to_string(),
                agent_id: "proxy-agent".to_string(),
                tool_label: "Claude Proxy".to_string(),
                model: None,
                reasoning: None,
                version: None,
                resume_session_id: Some("resume-1".to_string()),
                live_window_id: None,
                skip_permissions: true,
                codex_fast_mode: false,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            }],
        );

        state.apply(LaunchWizardAction::ApplyQuickStart {
            index: 0,
            mode: QuickStartLaunchMode::Resume,
        });

        match state.completion.as_ref() {
            Some(LaunchWizardCompletion::Launch(config)) => match config.as_ref() {
                LaunchWizardLaunchRequest::Agent(config) => {
                    assert_eq!(config.command, custom_path.display().to_string());
                    assert_eq!(config.display_name, "Claude Proxy");
                    assert!(config.args.contains(&"--resume".to_string()));
                    assert!(config.args.contains(&"--unsafe".to_string()));
                }
                other => panic!("expected agent launch request, got {other:?}"),
            },
            other => panic!("expected quick start launch completion, got {other:?}"),
        }
    }

    #[test]
    fn open_loading_marks_wizard_as_hydrating() {
        let state = LaunchWizardState::open_loading(
            context(branch("feature/gui"), "feature/gui"),
            Vec::new(),
        );

        let view = state.view();
        assert!(state.is_hydrating);
        assert!(view.is_hydrating);
        assert!(state.quick_start_entries.is_empty());
        assert!(!view.show_runtime_target);
        assert!(view.hydration_error.is_none());
    }

    #[test]
    fn helper_value_functions_cover_docker_and_agent_variants() {
        assert_eq!(
            default_docker_lifecycle_intent(gwt_docker::ComposeServiceStatus::Running),
            gwt_agent::DockerLifecycleIntent::Connect
        );
        assert_eq!(
            default_docker_lifecycle_intent(gwt_docker::ComposeServiceStatus::Stopped),
            gwt_agent::DockerLifecycleIntent::Start
        );
        assert_eq!(
            default_docker_lifecycle_intent(gwt_docker::ComposeServiceStatus::NotFound),
            gwt_agent::DockerLifecycleIntent::CreateAndStart
        );
        assert_eq!(launch_target_value(LaunchTargetKind::Agent), "agent");
        assert_eq!(launch_target_value(LaunchTargetKind::Shell), "shell");
        assert_eq!(
            runtime_target_value(gwt_agent::LaunchRuntimeTarget::Host),
            "host"
        );
        assert_eq!(
            runtime_target_value(gwt_agent::LaunchRuntimeTarget::Docker),
            "docker"
        );
        assert_eq!(
            docker_lifecycle_value(gwt_agent::DockerLifecycleIntent::Restart),
            "restart"
        );
        assert_eq!(
            docker_lifecycle_value(gwt_agent::DockerLifecycleIntent::CreateAndStart),
            "create_and_start"
        );
        assert_eq!(
            apply_branch_prefix("feature/coverage", "fix/"),
            "fix/coverage"
        );
        assert_eq!(apply_branch_prefix("  ", "chore/"), "chore/");
        assert!(is_explicit_model_selection("gpt-5.5"));
        assert!(!is_explicit_model_selection("Default (Installed)"));
        assert!(agent_has_npm_package("codex"));
        assert!(!agent_has_npm_package("custom"));
        assert_eq!(agent_id_from_key("gh"), gwt_agent::AgentId::Copilot);
        assert_eq!(
            agent_id_from_key("custom"),
            gwt_agent::AgentId::Custom("custom".to_string())
        );
        assert_eq!(
            agent_description(&sample_agent_options()[0]),
            "Detected · 1.0.0".to_string()
        );
    }

    #[test]
    fn option_views_and_model_catalogs_expose_expected_labels() {
        let branch_types = branch_type_options_view();
        assert!(branch_types.iter().any(|option| option.value == "feature/"));
        assert!(branch_types
            .iter()
            .all(|option| option.description.as_deref().is_some()));

        let launch_targets = launch_target_options_view();
        assert_eq!(launch_targets[0].value, "agent");
        assert_eq!(launch_targets[1].value, "shell");

        let runtime_targets = runtime_target_options_view();
        assert!(runtime_targets.iter().any(|option| option.value == "host"));
        assert!(runtime_targets
            .iter()
            .any(|option| option.value == "docker"));

        let execution_modes = execution_mode_options_view();
        assert!(execution_modes
            .iter()
            .any(|option| option.value == "normal"));
        assert!(execution_modes
            .iter()
            .any(|option| option.value == "resume"));

        assert!(current_model_options("claude").contains(&"sonnet"));
        assert_eq!(
            current_model_options("codex"),
            vec![
                "Default (Auto)",
                "gpt-5.5",
                "gpt-5.4",
                "gpt-5.4-mini",
                "gpt-5.3-codex",
                "gpt-5.3-codex-spark",
                "gpt-5.2",
            ]
        );
        assert!(current_model_options("gemini").contains(&"gemini-2.5-pro"));
        assert!(current_model_options("custom").is_empty());
        assert!(model_display_options("custom").is_empty());
        assert!(!model_display_options("codex").is_empty());
    }

    #[test]
    fn quick_start_summary_includes_runtime_metadata() {
        let summary = quick_start_summary(&QuickStartEntry {
            session_id: "gwt-session-1".to_string(),
            agent_id: "codex".to_string(),
            tool_label: "Codex".to_string(),
            model: Some("gpt-5.5".to_string()),
            reasoning: Some("high".to_string()),
            version: Some("0.110.0".to_string()),
            resume_session_id: Some("resume-1".to_string()),
            live_window_id: None,
            skip_permissions: true,
            codex_fast_mode: true,
            runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
            docker_service: Some("gwt".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
        });

        assert_eq!(summary, "Codex · gpt-5.5 · high · 0.110.0 · docker:gwt");
    }

    #[test]
    fn step_navigation_and_default_selection_follow_runtime_state() {
        let mut docker_context = context(branch("feature/gui"), "feature/gui");
        docker_context.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "worker".to_string()],
            suggested_service: Some("worker".to_string()),
        });
        docker_context.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let mut state =
            LaunchWizardState::open_with(docker_context, sample_agent_options(), Vec::new());

        state.selected = 1;
        assert_eq!(
            next_step(LaunchWizardStep::BranchAction, &state),
            Some(LaunchWizardStep::BranchTypeSelect)
        );

        state.launch_target = LaunchTargetKind::Shell;
        assert_eq!(
            next_step(LaunchWizardStep::LaunchTarget, &state),
            Some(LaunchWizardStep::RuntimeTarget)
        );

        state.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        assert_eq!(
            next_step(LaunchWizardStep::RuntimeTarget, &state),
            Some(LaunchWizardStep::DockerServiceSelect)
        );
        assert_eq!(
            prev_step(LaunchWizardStep::DockerLifecycle, &state),
            Some(LaunchWizardStep::DockerServiceSelect)
        );
        assert_eq!(
            step_default_selection(LaunchWizardStep::DockerServiceSelect, &state),
            1
        );

        state.launch_target = LaunchTargetKind::Agent;
        state.agent_id = "codex".to_string();
        state.model = "gpt-5.5".to_string();
        state.reasoning = "high".to_string();
        state.version = "0.110.0".to_string();
        state.mode = "resume".to_string();
        state.skip_permissions = true;
        state.codex_fast_mode = true;

        assert_eq!(
            next_step(LaunchWizardStep::AgentSelect, &state),
            Some(LaunchWizardStep::ModelSelect)
        );
        assert_eq!(
            next_step(LaunchWizardStep::ModelSelect, &state),
            Some(LaunchWizardStep::ReasoningLevel)
        );
        assert_eq!(
            step_default_selection(LaunchWizardStep::ModelSelect, &state),
            current_model_options("codex")
                .iter()
                .position(|model| model == &"gpt-5.5")
                .unwrap()
        );
        assert_eq!(
            step_default_selection(LaunchWizardStep::ExecutionMode, &state),
            EXECUTION_MODE_OPTIONS
                .iter()
                .position(|option| option.value == "resume")
                .unwrap()
        );
        assert_eq!(
            step_default_selection(LaunchWizardStep::SkipPermissions, &state),
            0
        );
        assert_eq!(
            step_default_selection(LaunchWizardStep::CodexFastMode, &state),
            0
        );
    }

    #[test]
    fn apply_hydration_updates_docker_defaults_and_quick_start_entries() {
        let mut state = LaunchWizardState::open_loading(
            context(branch("feature/gui"), "feature/gui"),
            Vec::new(),
        );
        let worktree = PathBuf::from("/tmp/repo-feature");
        state.apply_hydration(LaunchWizardHydration {
            selected_branch: Some(branch("origin/feature/gui")),
            normalized_branch_name: "feature/gui".to_string(),
            worktree_path: Some(worktree.clone()),
            quick_start_root: worktree.clone(),
            docker_context: Some(DockerWizardContext {
                services: vec!["app".to_string(), "worker".to_string()],
                suggested_service: Some("app".to_string()),
            }),
            docker_service_status: gwt_docker::ComposeServiceStatus::Running,
            agent_options: sample_agent_options(),
            quick_start_entries: vec![QuickStartEntry {
                session_id: "gwt-session-1".to_string(),
                agent_id: "codex".to_string(),
                tool_label: "Codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                resume_session_id: Some("resume-1".to_string()),
                live_window_id: None,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            }],
            previous_profile: None,
        });

        let view = state.view();
        assert!(!state.is_hydrating);
        assert_eq!(
            state.context.worktree_path.as_deref(),
            Some(worktree.as_path())
        );
        assert_eq!(state.context.normalized_branch_name, "feature/gui");
        assert_eq!(state.runtime_target, gwt_agent::LaunchRuntimeTarget::Docker);
        assert_eq!(state.docker_service.as_deref(), Some("app"));
        assert_eq!(
            state.docker_lifecycle_intent,
            gwt_agent::DockerLifecycleIntent::Connect
        );
        assert_eq!(state.quick_start_entries.len(), 1);
        assert!(view.show_runtime_target);
        assert!(!view.is_hydrating);
        assert_eq!(view.selected_agent_id, "claude");
        assert_eq!(view.agent_options.len(), 2);
        assert_eq!(view.selected_runtime_target, "docker");
    }

    #[test]
    fn build_launch_config_rejects_loading_state() {
        let state = LaunchWizardState::open_loading(
            context(branch("feature/gui"), "feature/gui"),
            Vec::new(),
        );

        let error = state
            .build_launch_config()
            .expect_err("loading must block launch");
        assert_eq!(error, "Launch options are still loading");
    }

    #[test]
    fn claude_opus_reasoning_options_include_xhigh() {
        let values: Vec<&str> = super::CLAUDE_OPUS_REASONING_OPTIONS
            .iter()
            .map(|option| option.stored_value)
            .collect();
        assert_eq!(values, ["auto", "low", "medium", "high", "xhigh", "max"]);
    }

    #[test]
    fn claude_opus_reasoning_default_is_xhigh() {
        let default = super::CLAUDE_OPUS_REASONING_OPTIONS
            .iter()
            .find(|option| option.is_default)
            .expect("Opus reasoning options must have a default row");
        assert_eq!(default.stored_value, "xhigh");
    }

    #[test]
    fn claude_sonnet_reasoning_options_exclude_xhigh_and_max() {
        let values: Vec<&str> = super::CLAUDE_SONNET_REASONING_OPTIONS
            .iter()
            .map(|option| option.stored_value)
            .collect();
        assert_eq!(values, ["auto", "low", "medium", "high"]);
        assert!(!values.contains(&"xhigh"));
        assert!(!values.contains(&"max"));
    }

    #[test]
    fn claude_sonnet_reasoning_default_is_medium() {
        let default = super::CLAUDE_SONNET_REASONING_OPTIONS
            .iter()
            .find(|option| option.is_default)
            .expect("Sonnet reasoning options must have a default row");
        assert_eq!(default.stored_value, "medium");
    }

    #[test]
    fn open_and_quick_start_helpers_cover_real_sessions_and_errors() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        sample_session(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 11, 0, 0).unwrap(),
            "resume-1",
        );

        let mut ctx = context(branch("origin/feature/gui"), "feature/gui");
        ctx.quick_start_root = worktree;
        let state = LaunchWizardState::open(ctx, dir.path(), &dir.path().join("versions.json"));

        assert_eq!(state.step, LaunchWizardStep::QuickStart);
        assert_eq!(state.quick_start_entries.len(), 1);
        assert!(state.quick_start_entries[0].can_reuse());
        assert_eq!(
            state.quick_start_entries[0].reuse_action_label(),
            Some("Resume")
        );
        assert!(matches!(
            state.quick_start_actions().as_slice(),
            [
                QuickStartAction::ReuseEntry { index: 0 },
                QuickStartAction::StartNewEntry { index: 0 },
                QuickStartAction::ChooseDifferent
            ]
        ));
        assert!(matches!(
            state.selected_quick_start_action(),
            QuickStartAction::ReuseEntry { index: 0 }
        ));
        assert_eq!(
            state
                .selected_quick_start_entry()
                .map(|entry| entry.agent_id.as_str()),
            Some("codex")
        );
        assert_eq!(
            state.view().quick_start_entries[0]
                .reuse_action_label
                .as_deref(),
            Some("Resume")
        );

        let mut loading = LaunchWizardState::open_loading(
            context(branch("feature/gui"), "feature/gui"),
            Vec::new(),
        );
        loading.set_hydration_error("network failed".to_string());
        assert!(!loading.is_hydrating);
        assert_eq!(loading.hydration_error.as_deref(), Some("network failed"));

        let mut resumable = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            state.quick_start_entries.clone(),
        );
        resumable.apply(LaunchWizardAction::ApplyQuickStart {
            index: 0,
            mode: QuickStartLaunchMode::Resume,
        });
        assert!(matches!(
            resumable.completion.as_ref(),
            Some(LaunchWizardCompletion::Launch(config))
                if matches!(
                    config.as_ref(),
                    LaunchWizardLaunchRequest::Agent(config)
                        if config.resume_session_id.as_deref() == Some("resume-1")
                )
        ));

        let mut missing = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            vec![quick_start_entry(
                "session-2",
                "codex",
                None,
                None,
                gwt_agent::LaunchRuntimeTarget::Host,
                None,
            )],
        );
        missing.apply(LaunchWizardAction::ApplyQuickStart {
            index: 0,
            mode: QuickStartLaunchMode::Resume,
        });
        assert_eq!(
            missing.error.as_deref(),
            Some("No saved session is available")
        );
    }

    #[test]
    fn current_options_cover_all_steps_and_reasoning_variants() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.live_sessions = vec![LiveSessionEntry {
            session_id: "session-live".to_string(),
            window_id: "window-1".to_string(),
            agent_id: "codex".to_string(),
            kind: "agent".to_string(),
            name: "Codex".to_string(),
            detail: Some("/tmp/repo".to_string()),
            active: true,
        }];
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "worker".to_string()],
            suggested_service: Some("worker".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let mut state = LaunchWizardState::open_with(
            ctx,
            sample_agent_options(),
            vec![
                quick_start_entry(
                    "session-1",
                    "codex",
                    Some("resume-1"),
                    Some("window-1"),
                    gwt_agent::LaunchRuntimeTarget::Docker,
                    Some("worker"),
                ),
                quick_start_entry(
                    "session-2",
                    "claude",
                    None,
                    None,
                    gwt_agent::LaunchRuntimeTarget::Host,
                    None,
                ),
            ],
        );

        let quick_options = state.current_options();
        assert_eq!(quick_options[0].value, "reuse:0");
        assert!(quick_options
            .iter()
            .any(|option| option.value == "focus_existing"));
        assert!(quick_options
            .iter()
            .any(|option| option.value == "choose_different"));
        state.selected = 999;
        assert!(matches!(
            state.selected_quick_start_action(),
            QuickStartAction::ChooseDifferent
        ));
        assert!(state.selected_quick_start_entry().is_none());

        state.step = LaunchWizardStep::FocusExistingSession;
        assert_eq!(state.current_options()[0].value, "window-1");

        state.step = LaunchWizardStep::BranchAction;
        assert_eq!(state.current_options().len(), 2);

        state.step = LaunchWizardStep::BranchTypeSelect;
        assert!(state
            .current_options()
            .iter()
            .any(|option| option.value == "release/"));

        state.step = LaunchWizardStep::LaunchTarget;
        assert_eq!(state.current_options()[1].value, "shell");

        state.step = LaunchWizardStep::AgentSelect;
        assert_eq!(state.current_options().len(), 2);

        state.agent_id = "claude".to_string();
        state.step = LaunchWizardStep::ModelSelect;
        assert!(state
            .current_options()
            .iter()
            .any(|option| option.value == "sonnet"));

        state.model = "opus".to_string();
        state.step = LaunchWizardStep::ReasoningLevel;
        assert!(state
            .current_options()
            .iter()
            .any(|option| option.value == "xhigh"));

        state.model = "sonnet".to_string();
        assert!(!state
            .current_options()
            .iter()
            .any(|option| option.value == "xhigh"));

        state.step = LaunchWizardStep::RuntimeTarget;
        assert!(state
            .current_options()
            .iter()
            .any(|option| option.value == "docker"));

        state.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        state.step = LaunchWizardStep::DockerServiceSelect;
        assert_eq!(state.current_options()[1].value, "worker");

        state.step = LaunchWizardStep::DockerLifecycle;
        assert!(state
            .current_options()
            .iter()
            .any(|option| option.value == "connect"));

        state.context.docker_service_status = gwt_docker::ComposeServiceStatus::Exited;
        assert_eq!(state.current_options()[0].value, "start");

        state.context.docker_service_status = gwt_docker::ComposeServiceStatus::NotFound;
        assert_eq!(state.current_options()[0].value, "create_and_start");

        state.context.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        state.agent_id = "missing".to_string();
        state.step = LaunchWizardStep::VersionSelect;
        assert!(state.current_options().is_empty());

        state.agent_id = "codex".to_string();
        state.version = "0.110.0".to_string();
        assert!(state
            .current_options()
            .iter()
            .any(|option| option.value == "0.110.0" || option.value == "latest"));

        state.step = LaunchWizardStep::ExecutionMode;
        assert!(state
            .current_options()
            .iter()
            .any(|option| option.value == "resume"));

        state.step = LaunchWizardStep::SkipPermissions;
        assert_eq!(state.current_options()[0].value, "yes");

        state.step = LaunchWizardStep::CodexFastMode;
        assert_eq!(state.current_options()[0].value, "on");

        state.step = LaunchWizardStep::BranchNameInput;
        assert!(state.current_options().is_empty());
    }

    #[test]
    fn navigation_and_apply_actions_cover_cancel_back_and_focus_paths() {
        let mut quick_ctx = context(branch("feature/gui"), "feature/gui");
        quick_ctx.live_sessions = vec![LiveSessionEntry {
            session_id: "session-live".to_string(),
            window_id: "window-1".to_string(),
            agent_id: "codex".to_string(),
            kind: "agent".to_string(),
            name: "Codex".to_string(),
            detail: Some("/tmp/repo".to_string()),
            active: true,
        }];
        let mut state = LaunchWizardState::open_with(
            quick_ctx,
            sample_agent_options(),
            vec![quick_start_entry(
                "session-1",
                "codex",
                Some("resume-1"),
                Some("window-1"),
                gwt_agent::LaunchRuntimeTarget::Host,
                None,
            )],
        );

        assert_eq!(
            next_step(LaunchWizardStep::QuickStart, &state),
            Some(LaunchWizardStep::SkipPermissions)
        );
        state.selected = 2;
        assert_eq!(
            next_step(LaunchWizardStep::QuickStart, &state),
            Some(LaunchWizardStep::FocusExistingSession)
        );
        state.selected = 3;
        assert_eq!(
            next_step(LaunchWizardStep::QuickStart, &state),
            Some(LaunchWizardStep::BranchAction)
        );
        assert_eq!(
            prev_step(LaunchWizardStep::BranchAction, &state),
            Some(LaunchWizardStep::QuickStart)
        );
        assert_eq!(
            prev_step(LaunchWizardStep::FocusExistingSession, &state),
            Some(LaunchWizardStep::QuickStart)
        );

        state.apply(LaunchWizardAction::FocusExistingSession { index: 0 });
        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::FocusWindow { window_id }) if window_id == "window-1"
        ));

        state.completion = None;
        state.error = None;
        state.apply(LaunchWizardAction::FocusExistingSession { index: 99 });
        assert_eq!(
            state.error.as_deref(),
            Some("No running session is available")
        );

        state.completion = None;
        state.step = LaunchWizardStep::QuickStart;
        state.apply(LaunchWizardAction::Back);
        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::Cancelled)
        ));

        let mut plain = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );
        assert_eq!(
            next_step(LaunchWizardStep::LaunchTarget, &plain),
            Some(LaunchWizardStep::AgentSelect)
        );
        plain.launch_target = LaunchTargetKind::Shell;
        let expected_shell_step = if cfg!(windows) {
            Some(LaunchWizardStep::WindowsShell)
        } else {
            None
        };
        assert_eq!(
            next_step(LaunchWizardStep::LaunchTarget, &plain),
            expected_shell_step
        );
        plain.apply(LaunchWizardAction::SetLinkedIssue { issue_number: 42 });
        assert_eq!(plain.linked_issue_number, Some(42));
        plain.apply(LaunchWizardAction::ClearLinkedIssue);
        assert_eq!(plain.linked_issue_number, None);
        plain.step = LaunchWizardStep::BranchNameInput;
        plain.apply(LaunchWizardAction::SubmitText {
            value: "   ".to_string(),
        });
        assert_eq!(plain.error.as_deref(), Some("Branch name is required"));
        plain.error = None;
        plain.step = LaunchWizardStep::BranchAction;
        plain.apply(LaunchWizardAction::SubmitText {
            value: "ignored".to_string(),
        });
        assert!(plain.error.is_none());
        assert_eq!(plain.branch_name, "feature/gui");

        let mut docker_ctx = context(branch("feature/gui"), "feature/gui");
        docker_ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "worker".to_string()],
            suggested_service: Some("worker".to_string()),
        });
        docker_ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let mut docker =
            LaunchWizardState::open_with(docker_ctx, sample_agent_options(), Vec::new());
        docker.agent_id = "claude".to_string();
        docker.model = "sonnet".to_string();
        docker.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        assert_eq!(
            prev_step(LaunchWizardStep::RuntimeTarget, &docker),
            Some(LaunchWizardStep::ReasoningLevel)
        );
        assert_eq!(
            prev_step(LaunchWizardStep::DockerLifecycle, &docker),
            Some(LaunchWizardStep::DockerServiceSelect)
        );

        docker.apply(LaunchWizardAction::SetBranchType {
            prefix: "release/".to_string(),
        });
        assert!(docker.branch_name.starts_with("release/"));
        docker.apply(LaunchWizardAction::SetDockerService {
            service: "api".to_string(),
        });
        assert_eq!(docker.docker_service.as_deref(), Some("api"));
        docker.apply(LaunchWizardAction::SetDockerLifecycle {
            intent: gwt_agent::DockerLifecycleIntent::Restart,
        });
        assert_eq!(
            docker.docker_lifecycle_intent,
            gwt_agent::DockerLifecycleIntent::Restart
        );
        docker.apply(LaunchWizardAction::SetExecutionMode {
            mode: "continue".to_string(),
        });
        assert_eq!(docker.mode, "continue");
    }
}
