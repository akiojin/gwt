use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::BranchListEntry;

mod launch_request;
mod options;
mod profiles;
mod quick_start;
mod state;
#[cfg(test)]
mod test_support;
mod view_model;

use options::*;

pub use options::{
    build_agent_options, build_builtin_agent_options, default_wizard_version_cache_path,
    load_agent_options,
};
pub use profiles::{
    load_previous_launch_profile, load_previous_launch_profiles,
    previous_launch_profile_from_sessions, previous_launch_profiles_for_repo_from_sessions,
    previous_launch_profiles_from_sessions, quick_start_entries_from_sessions,
};
pub use quick_start::{load_quick_start_entries, load_sessions};

const DEFAULT_NEW_BRANCH_BASE_BRANCH: &str = "develop";
const BRANCH_TYPE_PREFIXES: [&str; 4] = ["feature/", "bugfix/", "hotfix/", "release/"];

/// Distinguishes the source bridge so branch names seed as `issue-{n}` vs
/// `spec-{n}` (kept independent of `linked_issue_number` because Branches-window
/// callers can know the number from a linkage store but not the source kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkedIssueKind {
    #[default]
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
pub enum LaunchWizardMode {
    Branch,
    StartWork,
    Knowledge,
}

pub fn knowledge_launch_target_branch_name(kind: LinkedIssueKind, number: u64) -> String {
    match kind {
        LinkedIssueKind::Issue => format!("work/issue-{number}"),
        LinkedIssueKind::Spec => format!("feature/spec-{number}"),
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

/// SPEC-2014 FR-126/FR-128: progress rail クリックジャンプ（GotoStep）の対象フェーズ。
/// ManualSetup（ConfigureAndStart）の Setup 3ステップ + 入口を表す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WizardPhase {
    Path,
    Settings,
    Runtime,
    Confirm,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchWizardLaunchPath {
    QuickStart,
    ManualSetup,
    FocusSession,
}

impl LaunchWizardLaunchPath {
    fn value(self) -> &'static str {
        match self {
            LaunchWizardLaunchPath::QuickStart => "quick_start",
            LaunchWizardLaunchPath::ManualSetup => "manual_setup",
            LaunchWizardLaunchPath::FocusSession => "focus_session",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchWizardStartMethodKind {
    ConfigureAndStart,
    StartWithLastSettings,
    ContinueLastSession,
    OpenSessionPicker,
    FocusRunningSession,
}

impl LaunchWizardStartMethodKind {
    fn value(self) -> &'static str {
        match self {
            LaunchWizardStartMethodKind::ConfigureAndStart => "configure_and_start",
            LaunchWizardStartMethodKind::StartWithLastSettings => "start_with_last_settings",
            LaunchWizardStartMethodKind::ContinueLastSession => "continue_last_session",
            LaunchWizardStartMethodKind::OpenSessionPicker => "open_session_picker",
            LaunchWizardStartMethodKind::FocusRunningSession => "focus_running_session",
        }
    }
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
pub struct LaunchWizardStartMethodView {
    pub kind: String,
    pub label: String,
    pub badge: String,
    pub group: String,
    pub recommended: bool,
    pub summary: String,
    pub detail: Option<String>,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
}

/// SPEC-2359 US-42 — Workspace Resume Picker entry.
///
/// One row in the modal that appears when the user clicks Resume on a
/// Workspace card. Each entry maps to a previously-assigned agent whose
/// `session_id` we can spawn with `claude --resume <uuid>` or
/// `codex resume <uuid>`. We deliberately keep this view backend-driven
/// so the picker can render without re-deriving runtime metadata from
/// storage on the client.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct ResumableAgentView {
    pub session_id: String,
    pub agent_id: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    /// `"session"` means we found a Session toml on disk with a non-empty
    /// `agent_session_id`, so the launcher can pass `--resume <uuid>` and
    /// the agent will pick up the previous conversation. `"metadata_only"`
    /// means we only have Workspace projection metadata (no Session toml
    /// for that id), so a fresh agent will be started while preserving
    /// the Workspace title / owner.
    pub resume_kind: ResumableAgentResumeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_status: Option<ResumableAgentLifecycleStatus>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResumableAgentResumeKind {
    Session,
    NativePicker,
    MetadataOnly,
}

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResumableAgentLifecycleStatus {
    Active,
    Interrupted,
    Running,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchWizardLiveSessionView {
    pub index: usize,
    pub name: String,
    pub detail: Option<String>,
    pub active: bool,
    pub runtime_status: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchWizardSummaryView {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchWizardProgressStepView {
    pub key: String,
    pub label: String,
    pub state: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchWizardView {
    pub title: String,
    pub mode: LaunchWizardMode,
    pub branch_name: String,
    pub selected_branch_name: String,
    /// SPEC-2359 US-83 / FR-444: existing remote branches offered by the "open
    /// existing branch" picker (each dispatches `SelectExistingBranch`).
    pub open_branch_candidates: Vec<String>,
    pub linked_issue_number: Option<u64>,
    pub is_hydrating: bool,
    pub runtime_context_resolved: bool,
    pub hydration_error: Option<String>,
    pub start_methods: Vec<LaunchWizardStartMethodView>,
    pub quick_start_entries: Vec<LaunchWizardQuickStartView>,
    pub live_sessions: Vec<LaunchWizardLiveSessionView>,
    pub selected_launch_path: String,
    pub selected_quick_start_index: Option<usize>,
    pub selected_live_session_index: Option<usize>,
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
    pub show_fast_mode: bool,
    /// Legacy Codex-only compatibility field for older frontend payloads.
    pub show_codex_fast_mode: bool,
    /// SPEC-3152: render the Hermes launch-options section (provider / model /
    /// profile / advanced) in the Settings form.
    pub show_hermes_options: bool,
    pub hermes_needs_setup: bool,
    /// SPEC-3151 FR-008: render the OpenCode launch-options section (free-text
    /// `provider/model`) in the Settings form.
    pub show_opencode_options: bool,
    /// SPEC-3151 FR-009: `true` when OpenCode has no AI provider configured, so
    /// the wizard shows a non-blocking "OpenCode is not set up" hint with an
    /// in-pane setup launcher. Only meaningful for the OpenCode agent.
    pub opencode_needs_setup: bool,
    pub hermes_provider: String,
    pub hermes_provider_options: Vec<String>,
    pub hermes_profile: String,
    pub hermes_toolsets: String,
    pub hermes_skills: String,
    pub hermes_max_turns: String,
    pub hermes_safe_mode: bool,
    pub show_branch_controls: bool,
    pub show_start_methods: bool,
    pub show_back_button: bool,
    pub show_manual_setup: bool,
    pub show_runtime_confirmation: bool,
    /// SPEC-2014 FR-127: ManualSetup の Confirm ステップ（読み取りサマリ + Launch）。
    pub show_confirm: bool,
    // SPEC-2014 Amendment 2026-05-20 (FR-057): gate the Linked issue section
    // so it only renders when the wizard was opened through the Knowledge
    // Issue Bridge (linked_issue_kind == Some(Issue) AND number is some).
    pub show_linked_issue: bool,
    pub runtime_resolution_pending: bool,
    pub runtime_resolution_message: Option<String>,
    pub launch_materialization_pending: bool,
    pub launch_materialization_message: Option<String>,
    pub primary_action_label: String,
    pub primary_action_enabled: bool,
    pub progress_steps: Vec<LaunchWizardProgressStepView>,
    pub fast_mode: bool,
    /// Legacy Codex-only compatibility field for older frontend payloads.
    pub codex_fast_mode: bool,
    pub launch_summary: Vec<LaunchWizardSummaryView>,
    /// SPEC-2014 FR-126/FR-128: 現在のウィザードフェーズ（rail 表示・クリック判定用）。
    pub phase: WizardPhase,
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LaunchWizardPreviousProfiles {
    default_agent_id: Option<String>,
    by_agent: HashMap<String, LaunchWizardPreviousProfile>,
    /// SPEC-2014 FR-032/FR-035: repo-local 最新 successful session から得られる
    /// runtime_target / docker_service / docker_lifecycle_intent の復元元。
    /// agent 識別系 (by_agent) は cross-repo の global preference を表すのに対し、
    /// repo_local は per-repo の runtime/Docker 永続化を担う。
    repo_local: Option<LaunchWizardPreviousProfile>,
}

impl LaunchWizardPreviousProfiles {
    pub fn from_profile(profile: Option<LaunchWizardPreviousProfile>) -> Self {
        let Some(profile) = profile else {
            return Self::default();
        };
        let default_agent_id = Some(profile.agent_id.clone());
        let by_agent = HashMap::from([(profile.agent_id.clone(), profile.clone())]);
        Self {
            default_agent_id,
            by_agent,
            repo_local: Some(profile),
        }
    }

    /// SPEC-2014 FR-032: repo-local previous profile を別途差し込む。
    /// テスト・production 双方で agent map とは独立に runtime 復元元を構成できる。
    pub fn with_repo_local(mut self, profile: Option<LaunchWizardPreviousProfile>) -> Self {
        self.repo_local = profile;
        self
    }

    pub fn preferred_agent_id(&self) -> Option<&str> {
        self.default_agent_id.as_deref()
    }

    pub fn profile_for(&self, agent_id: &str) -> Option<&LaunchWizardPreviousProfile> {
        self.by_agent.get(agent_id)
    }

    /// SPEC-2014 FR-032/FR-035: repo-local previous profile を返す。
    pub fn repo_local(&self) -> Option<&LaunchWizardPreviousProfile> {
        self.repo_local.as_ref()
    }

    pub fn preferred_profile(&self) -> Option<&LaunchWizardPreviousProfile> {
        self.preferred_agent_id()
            .and_then(|agent_id| self.profile_for(agent_id))
            .or_else(|| self.repo_local())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentLaunchDraft {
    model: String,
    reasoning: String,
    version: String,
    mode: String,
    resume_session_id: Option<String>,
    skip_permissions: bool,
    codex_fast_mode: bool,
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
    /// SPEC-3151 FR-010: when `Some`, run this command on the host instead of
    /// the detected interactive shell (e.g. `bunx`/`opencode` for the OpenCode
    /// setup launcher). `None` keeps the default shell behavior.
    pub command_override: Option<String>,
    /// SPEC-3151 FR-010: arguments for `command_override`. Ignored when
    /// `command_override` is `None`.
    pub command_args_override: Option<Vec<String>>,
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
    pub runtime_status: crate::WindowProcessStatus,
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
    /// Whether the locally installed Claude Code can offer the opt-in
    /// `ultracode` reasoning option. Used only when selected version is
    /// `installed`; npm-backed `latest` and pinned versions are evaluated from
    /// the selected version string at render time. Defaults to `false`.
    pub ultracode_supported: bool,
    /// Whether Claude Code dynamic workflows are enabled in the current
    /// environment. This gate applies to installed, `latest`, and pinned
    /// versions.
    pub claude_workflows_enabled: bool,
    /// SPEC-3214 Phase 3: when `Some(base_ref)`, this wizard launches an
    /// ephemeral **intake session** — the agent runs in a detached, throwaway
    /// worktree based on `base_ref` (e.g. `origin/develop`) and creates no
    /// branch. `None` is the normal branch-based launch.
    pub ephemeral_base_ref: Option<String>,
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
    pub previous_profiles: Option<LaunchWizardPreviousProfiles>,
    /// SPEC-2359 US-83 / FR-444: eligible existing remote branches the user can
    /// pick to continue on (the "open existing branch" picker). Empty when no
    /// branch listing was available. Preserved across runtime re-resolution.
    pub open_branch_candidates: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum LaunchWizardLaunchRequest {
    Agent(Box<gwt_agent::LaunchConfig>),
    Shell(Box<ShellLaunchConfig>),
}

impl LaunchWizardLaunchRequest {
    /// SPEC #3200 T-040/FR-006: when the project opted into unattended autonomous
    /// mode, force `skip_permissions` on a monitor-launched implementation agent
    /// so it runs without stalling on a permission prompt. A no-op when
    /// autonomous mode is off (preserves SPEC #3165 human-gated behavior exactly)
    /// and for non-agent (shell) launches.
    pub fn force_skip_permissions_for_autonomous(&mut self, autonomous_mode: bool) {
        if autonomous_mode {
            if let LaunchWizardLaunchRequest::Agent(config) = self {
                config.skip_permissions = true;
            }
        }
    }
}

#[cfg(test)]
mod autonomous_launch_tests {
    use super::LaunchWizardLaunchRequest;

    fn agent_request(skip_permissions: bool) -> LaunchWizardLaunchRequest {
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode).build();
        config.skip_permissions = skip_permissions;
        LaunchWizardLaunchRequest::Agent(Box::new(config))
    }

    #[test]
    fn autonomous_launch_forces_skip_permissions() {
        // SPEC #3200 T-040/FR-006: autonomous mode forces skip_permissions on an
        // agent launch so the unattended agent never stalls on a prompt.
        let mut request = agent_request(false);
        request.force_skip_permissions_for_autonomous(true);
        match request {
            LaunchWizardLaunchRequest::Agent(config) => assert!(config.skip_permissions),
            LaunchWizardLaunchRequest::Shell(_) => panic!("expected agent request"),
        }
    }

    #[test]
    fn non_autonomous_launch_leaves_skip_permissions_untouched() {
        // Default OFF must not change the human-gated launch's skip_permissions.
        let mut request = agent_request(false);
        request.force_skip_permissions_for_autonomous(false);
        match request {
            LaunchWizardLaunchRequest::Agent(config) => assert!(
                !config.skip_permissions,
                "off ⇒ unchanged (SPEC #3165 preserved)"
            ),
            LaunchWizardLaunchRequest::Shell(_) => panic!("expected agent request"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum LaunchWizardCompletion {
    Launch(Box<LaunchWizardLaunchRequest>),
    ResolveRuntime(Box<LaunchWizardLaunchRequest>),
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
    UseStartMethod {
        method: LaunchWizardStartMethodKind,
    },
    SetLaunchPath {
        path: LaunchWizardLaunchPath,
    },
    SelectQuickStart {
        index: usize,
    },
    SelectLiveSession {
        index: usize,
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
    /// SPEC-2359 US-83 / FR-444: pick an existing branch to continue on it. The
    /// worktree materializes on `<branch_name>` (tracking origin/<branch_name>
    /// when only the remote exists) without minting a new work/* branch, by
    /// switching the wizard out of the "new work branch" mode. `branch_name`
    /// may arrive as `origin/X` / `refs/remotes/origin/X` and is normalized.
    SelectExistingBranch {
        branch_name: String,
    },
    /// SPEC-2359 US-80: the optional Start Work intake prompt describing the
    /// work about to begin. Drives the duplicate-work advisory query.
    SetInitialPrompt {
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
    SetFastMode {
        enabled: bool,
    },
    SetCodexFastMode {
        enabled: bool,
    },
    /// SPEC-3152: set a Hermes free-text launch option by field key
    /// (`provider` | `profile` | `toolsets` | `skills` | `max_turns`).
    SetHermesOption {
        field: String,
        value: String,
    },
    /// SPEC-3152: toggle Hermes `--safe-mode`.
    SetHermesSafeMode {
        enabled: bool,
    },
    /// SPEC-3151 FR-010: launch `<opencode runner> auth login` in an in-pane
    /// host shell so the user can sign in to an AI provider without leaving the
    /// wizard. OpenCode auth is host-global, so this always runs on the host.
    ///
    /// Explicit serde rename: the default snake_case of `RunOpenCodeSetup` is
    /// `run_open_code_setup`, but the frontend and the action-label use the
    /// `opencode` convention, so the wire tag is `run_opencode_setup`.
    #[serde(rename = "run_opencode_setup")]
    RunOpenCodeSetup,
    Submit,
    /// SPEC-2014 FR-128: progress rail クリックで指定フェーズへ直接移動する。
    GotoStep {
        phase: WizardPhase,
    },
}

#[derive(Debug, Clone)]
pub struct LaunchWizardState {
    pub context: LaunchWizardContext,
    pub wizard_mode: LaunchWizardMode,
    pub step: LaunchWizardStep,
    pub selected: usize,
    pub launch_path: LaunchWizardLaunchPath,
    pub selected_quick_start_index: Option<usize>,
    pub selected_live_session_index: Option<usize>,
    pub detected_agents: Vec<AgentOption>,
    pub quick_start_entries: Vec<QuickStartEntry>,
    previous_profiles: LaunchWizardPreviousProfiles,
    pub is_new_branch: bool,
    pub base_branch_name: Option<String>,
    pub launch_target: LaunchTargetKind,
    pub agent_id: String,
    agent_drafts: HashMap<String, AgentLaunchDraft>,
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
    /// SPEC-3152: Hermes-specific launch options. Free-text fields map to
    /// `--provider` / `--profile` / `--toolsets` / `--skills` / `--max-turns`;
    /// `hermes_safe_mode` maps to `--safe-mode`. Only consumed for the Hermes
    /// agent; ignored otherwise.
    pub hermes_provider: String,
    pub hermes_profile: String,
    pub hermes_toolsets: String,
    pub hermes_skills: String,
    pub hermes_max_turns: String,
    pub hermes_safe_mode: bool,
    /// SPEC-3152: providers enumerated from the user's `~/.hermes/config.yaml`
    /// (model.provider + `providers:` keys), populated at wizard open. Empty
    /// in tests / when no config exists; the wizard then offers only the
    /// "use config default" and free-text "Other" provider entries.
    pub hermes_provider_choices: Vec<String>,
    /// SPEC-3152 FR-005: `true` when the user's global Hermes home has no
    /// resolvable credentials, so the wizard shows a non-blocking "Hermes is
    /// not set up" hint. Populated at wizard open; never blocks launch.
    pub hermes_needs_setup: bool,
    /// SPEC-3151 FR-009: `true` when OpenCode has no AI provider configured in
    /// its global data home, so the wizard shows a non-blocking "OpenCode is
    /// not set up" hint with an in-pane setup launcher. Populated at wizard
    /// open; never blocks launch.
    pub opencode_needs_setup: bool,
    pub branch_name: String,
    /// SPEC-2359 US-80: optional Start Work intake prompt (always skippable).
    /// Empty string means the step was skipped or left blank.
    pub initial_prompt: String,
    pub completion: Option<LaunchWizardCompletion>,
    pub error: Option<String>,
    pub is_hydrating: bool,
    pub runtime_context_resolved: bool,
    pub runtime_resolution_pending: bool,
    pub runtime_resolution_message: Option<String>,
    pub launch_materialization_pending: bool,
    pub launch_materialization_message: Option<String>,
    pub hydration_error: Option<String>,
    pub linked_issue_number: Option<u64>,
    start_method_selected: bool,
    manual_setup_initialized: bool,
    /// SPEC-2014 FR-126/FR-127: ManualSetup で Runtime ステップから Confirm へ
    /// 進んだか。Runtime(編集) と Confirm(サマリ+Launch) を区別する。QuickStart /
    /// 即起動系では使用しない（常に false）。
    runtime_confirmed: bool,
    /// SPEC-2014 FR-128: 解決済み状態のまま Settings フォームへ戻った（Back/rail）か。
    /// resolved を破棄せず Settings を再表示し、branch 不変なら再解決を避ける（SC-082）。
    settings_revisited: bool,
    /// SPEC-2014 FR-128: 最後に runtime を解決した branch 名。branch 変更検出に使う。
    resolved_branch_name: Option<String>,
    /// SPEC-2359 US-83 / FR-444: eligible existing remote branches offered by the
    /// "open existing branch" picker. Set at hydration / Start Work open.
    pub open_branch_candidates: Vec<String>,
}
