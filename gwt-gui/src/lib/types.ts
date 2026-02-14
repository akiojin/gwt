export type AgentStatusValue = "unknown" | "running" | "waiting_input" | "stopped";

export interface StructuredError {
  severity: "info" | "warning" | "error" | "critical";
  code: string;
  message: string;
  command: string;
  category: string;
  suggestions: string[];
  timestamp: string;
}

export interface BranchInfo {
  name: string;
  commit: string;
  is_current: boolean;
  is_agent_running: boolean;
  agent_status: AgentStatusValue;
  ahead: number;
  behind: number;
  divergence_status: string; // "UpToDate" | "Ahead" | "Behind" | "Diverged"
  commit_timestamp?: number | null;
  last_tool_usage?: string | null;
}

export interface ProjectInfo {
  path: string;
  repo_name: string;
  current_branch: string | null;
}

export interface OpenProjectResult {
  info: ProjectInfo;
  action: "opened" | "focusedExisting";
  focusedWindowLabel?: string | null;
}

export interface CloneProgress {
  stage: string; // "receiving" | "resolving"
  percent: number; // 0-100
}

export interface TerminalInfo {
  pane_id: string;
  agent_name: string;
  branch_name: string;
  status: string;
}

export interface TerminalAnsiProbe {
  pane_id: string;
  bytes_scanned: number;
  esc_count: number;
  sgr_count: number;
  color_sgr_count: number;
  has_256_color: boolean;
  has_true_color: boolean;
}

export interface ProjectModeMessage {
  role: "user" | "assistant" | "system" | "tool";
  kind?: "message" | "thought" | "action" | "observation" | "error";
  content: string;
  timestamp: number;
}

export interface ProjectModeState {
  messages: ProjectModeMessage[];
  ai_ready: boolean;
  ai_error?: string | null;
  last_error?: string | null;
  is_waiting: boolean;
  session_name?: string | null;
  project_mode_session_id?: string | null;
  lead_status?: string | null;
  llm_call_count: number;
  estimated_tokens: number;
  active_spec_id?: string | null;
  active_spec_issue_number?: number | null;
  active_spec_issue_url?: string | null;
  active_spec_issue_etag?: string | null;
}

export interface AgentSidebarSubAgent {
  id: string;
  name: string;
  toolId: string;
  status: "running" | "completed" | "failed" | (string & {});
  model?: string | null;
  branch: string;
  worktreeRelPath: string;
  worktreeAbsPath?: string | null;
}

export interface AgentSidebarTask {
  id: string;
  title: string;
  status: "running" | "pending" | "failed" | "completed" | (string & {});
  subAgents: AgentSidebarSubAgent[];
}

export interface AgentSidebarView {
  specId?: string | null;
  tasks: AgentSidebarTask[];
}

export interface SendKeysRequest {
  paneId: string;
  text: string;
}

export interface CaptureScrollbackRequest {
  paneId: string;
  maxBytes?: number;
}

export interface AgentInfo {
  id: "claude" | "codex" | "gemini" | "opencode" | (string & {});
  name: string;
  version: string;
  path?: string;
  authenticated: boolean;
  available: boolean;
}

export type ClaudeAgentProvider = "anthropic" | "glm";

export interface ClaudeGlmConfig {
  base_url: string;
  auth_token: string;
  api_timeout_ms: string;
  default_opus_model: string;
  default_sonnet_model: string;
  default_haiku_model: string;
}

export interface ClaudeAgentConfig {
  provider: ClaudeAgentProvider;
  glm: ClaudeGlmConfig;
}

export interface AgentConfig {
  version: number;
  claude: ClaudeAgentConfig;
}

export interface ShellInfo {
  id: string;
  name: string;
  version?: string;
}

export interface SettingsData {
  protected_branches: string[];
  default_base_branch: string;
  worktree_root: string;
  debug: boolean;
  log_dir?: string | null;
  log_retention_days: number;
  agent_default?: string | null;
  agent_claude_path?: string | null;
  agent_codex_path?: string | null;
  agent_gemini_path?: string | null;
  agent_auto_install_deps: boolean;
  agent_github_project_id?: string | null;
  agent_skill_registration_default_scope?: SkillRegistrationScope | null;
  agent_skill_registration_codex_scope?: SkillRegistrationScope | null;
  agent_skill_registration_claude_scope?: SkillRegistrationScope | null;
  agent_skill_registration_gemini_scope?: SkillRegistrationScope | null;
  docker_force_host: boolean;
  ui_font_size: number;
  terminal_font_size: number;
  ui_font_family: string;
  terminal_font_family: string;
  app_language: "auto" | "ja" | "en" | (string & {});
  voice_input: VoiceInputSettings;
  default_shell?: string | null;
  os_env_capture_mode?: "login_shell" | "process_env" | null;
}

export interface VoiceInputSettings {
  enabled: boolean;
  engine: "qwen3-asr" | (string & {});
  hotkey: string;
  ptt_hotkey: string;
  language: "auto" | "ja" | "en" | (string & {});
  quality: "fast" | "balanced" | "accurate" | (string & {});
  model: string;
}

export type SkillRegistrationScope = "user" | "project" | "local";

export interface SkillAgentRegistrationStatus {
  agent_id: string;
  label: string;
  skills_path?: string | null;
  registered: boolean;
  missing_skills: string[];
  error_code?: string | null;
  error_message?: string | null;
}

export interface SkillRegistrationStatus {
  overall: "ok" | "degraded" | "failed" | (string & {});
  agents: SkillAgentRegistrationStatus[];
  last_checked_at: number;
  last_error_message?: string | null;
}

export interface AISettings {
  endpoint: string;
  api_key: string;
  model: string;
  language: "auto" | "ja" | "en" | (string & {});
  summary_enabled: boolean;
}

export interface Profile {
  name: string;
  env: Record<string, string>;
  disabled_env: string[];
  description: string;
  ai?: AISettings | null;
  ai_enabled?: boolean | null;
}

export interface ProfilesConfig {
  version: number;
  active?: string | null;
  default_ai?: AISettings | null;
  profiles: Record<string, Profile>;
}

export interface Tab {
  id: string;
  label: string;
  agentId?: "claude" | "codex" | "gemini" | "opencode";
  type:
    | "summary"
    | "agent"
    | "settings"
    | "versionHistory"
    | "projectMode"
    | "terminal"
    | "issueSpec"
    | "issues";
  paneId?: string;
  cwd?: string;
  issueNumber?: number;
  specId?: string;
}

export interface ToolSessionEntry {
  branch: string;
  worktree_path?: string | null;
  tool_id: string;
  tool_label: string;
  session_id?: string | null;
  mode?: string | null;
  model?: string | null;
  reasoning_level?: string | null;
  skip_permissions?: boolean | null;
  tool_version?: string | null;
  collaboration_modes?: boolean | null;
  docker_service?: string | null;
  docker_force_host?: boolean | null;
  docker_recreate?: boolean | null;
  docker_build?: boolean | null;
  docker_keep?: boolean | null;
  /** Name of the Docker container launched for this tool session. */
  docker_container_name?: string | null;
  /** CLI args used in `docker-compose` launch for this session. */
  docker_compose_args?: string[] | null;
  timestamp: number;
}

export interface ProjectVersions {
  items: VersionItem[];
}

export interface VersionItem {
  id: string; // "unreleased" | "vX.Y.Z"
  label: string;
  range_from?: string | null;
  range_to: string; // "HEAD" | "vX.Y.Z"
  commit_count: number;
}

export interface VersionHistoryResult {
  status: "ok" | "generating" | "error" | "disabled";
  version_id: string;
  label: string;
  range_from?: string | null;
  range_to: string;
  commit_count: number;
  summary_markdown?: string | null;
  changelog_markdown?: string | null;
  error?: string | null;
}

export interface SessionSummaryResult {
  status: "ok" | "ai-not-configured" | "disabled" | "no-session" | "error";
  generating: boolean;
  toolId?: string | null;
  sessionId?: string | null;
  language?: "auto" | "ja" | "en" | (string & {}) | null;
  sourceType?: "session" | "scrollback" | null;
  inputMtimeMs?: number | null;
  summaryUpdatedMs?: number | null;
  markdown?: string | null;
  taskOverview?: string | null;
  shortSummary?: string | null;
  bulletPoints: string[];
  warning?: string | null;
  error?: string | null;
}

export interface BranchSuggestResult {
  status: "ok" | "ai-not-configured" | "error";
  suggestions: string[];
  error?: string | null;
}

export interface ClassifyResult {
  status: "ok" | "ai-not-configured" | "error";
  prefix?: string;
  error?: string;
}

export interface DockerContext {
  worktree_path?: string | null;
  file_type: "compose" | "devcontainer" | "dockerfile" | "none";
  compose_services: string[];
  docker_available: boolean;
  compose_available: boolean;
  daemon_running: boolean;
  force_host: boolean;
  container_status?: "running" | "stopped" | "not_found" | null;
  images_exist?: boolean | null;
}

export interface ProbePathResult {
  kind:
    | "gwtProject"
    | "migrationRequired"
    | "emptyDir"
    | "notFound"
    | "invalid"
    | "notGwtProject";
  projectPath?: string | null;
  migrationSourceRoot?: string | null;
  message?: string | null;
}

export interface MigrationProgressPayload {
  jobId: string;
  state: string;
  current?: number | null;
  total?: number | null;
}

export interface MigrationFinishedPayload {
  jobId: string;
  ok: boolean;
  error?: string | null;
  projectPath?: string | null;
}

export interface LaunchProgressPayload {
  jobId: string;
  step: string;
  detail?: string | null;
}

export interface LaunchFinishedPayload {
  jobId: string;
  status: "ok" | "cancelled" | "error";
  paneId?: string | null;
  error?: string | null;
}

export interface WorktreeInfo {
  path: string;
  branch: string | null;
  commit: string;
  status: string; // "active" | "locked" | "prunable" | "missing"
  is_main: boolean;
  has_changes: boolean;
  has_unpushed: boolean;
  is_current: boolean;
  is_protected: boolean;
  is_agent_running: boolean;
  agent_status: AgentStatusValue;
  ahead: number;
  behind: number;
  is_gone: boolean;
  last_tool_usage: string | null;
  safety_level: "safe" | "warning" | "danger" | "disabled";
}

export type PrStatus = "merged" | "open" | "closed" | "none" | "unknown";

export interface CleanupResult {
  branch: string;
  success: boolean;
  error: string | null;
  remote_success: boolean | null;
  remote_error: string | null;
}

export interface CleanupProgress {
  branch: string;
  status: "deleting" | "deleted" | "failed";
  error?: string;
  remote_status?: string;
}

export interface CleanupSettings {
  delete_remote_branches: boolean;
}

export interface CapturedEnvEntry {
  key: string;
  value: string;
}

export interface CapturedEnvInfo {
  entries: CapturedEnvEntry[];
  source: string;
  reason: string | null;
  ready: boolean;
}

export interface OsEnvCaptureStatus {
  configuredMode?: "login_shell" | "process_env" | null;
  activeSource: string;
  reason: string | null;
  ready: boolean;
  captureInFlight: boolean;
}

export type UpdateState =
  | {
      state: "up_to_date";
      checked_at?: string | null;
    }
  | {
      state: "available";
      current: string;
      latest: string;
      release_url: string;
      asset_url?: string | null;
      checked_at: string;
    }
  | {
      state: "failed";
      message: string;
      failed_at: string;
    };

export type FileChangeKind = "Added" | "Modified" | "Deleted" | "Renamed";

export interface FileChange {
  path: string;
  kind: FileChangeKind;
  additions: number;
  deletions: number;
  is_binary: boolean;
}

export interface FileDiff {
  content: string;
  truncated: boolean;
}

export interface CommitEntry {
  sha: string;
  message: string;
  timestamp: number;
  author: string;
}

export interface StashEntry {
  index: number;
  message: string;
  file_count: number;
}

export interface WorkingTreeEntry {
  path: string;
  status: FileChangeKind;
  is_staged: boolean;
}

export interface GitChangeSummary {
  file_count: number;
  commit_count: number;
  stash_count: number;
  base_branch: string;
}

// GitHub Issue types (SPEC-c6ba640a, SPEC-ca4b5b07)

export interface GitHubLabel {
  name: string;
  color: string;
}

export interface GitHubAssignee {
  login: string;
  avatarUrl: string;
}

export interface GitHubMilestone {
  title: string;
  number: number;
}

export interface GitHubIssueInfo {
  number: number;
  title: string;
  body?: string;
  state: "open" | "closed";
  updatedAt: string;
  htmlUrl: string;
  labels: GitHubLabel[];
  assignees: GitHubAssignee[];
  commentsCount: number;
  milestone?: GitHubMilestone;
}

export interface BranchLinkedIssueInfo {
  number: number;
  title: string;
  updatedAt: string;
  labels: string[];
  url: string;
}

export interface GhCliStatus {
  available: boolean;
  authenticated: boolean;
}

export interface FetchIssuesResponse {
  issues: GitHubIssueInfo[];
  hasNextPage: boolean;
}

export interface RollbackResult {
  localDeleted: boolean;
  remoteDeleted: boolean;
  error: string | null;
}

export interface LaunchAgentRequest {
  agentId: string;
  branch: string;
  profile?: string;
  model?: string;
  agentVersion?: string;
  mode?: "normal" | "continue" | "resume";
  skipPermissions?: boolean;
  reasoningLevel?: string;
  extraArgs?: string[];
  envOverrides?: Record<string, string>;
  resumeSessionId?: string;
  createBranch?: { name: string; base?: string | null };
  dockerService?: string;
  dockerForceHost?: boolean;
  dockerRecreate?: boolean;
  dockerBuild?: boolean;
  dockerKeep?: boolean;
  issueNumber?: number;
  terminalShell?: string;
}

// PR Status types (SPEC-d6949f99)

export interface PrStatusInfo {
  number: number;
  title: string;
  state: "OPEN" | "CLOSED" | "MERGED";
  url: string;
  mergeable: "MERGEABLE" | "CONFLICTING" | "UNKNOWN";
  author: string;
  baseBranch: string;
  headBranch: string;
  labels: string[];
  assignees: string[];
  milestone: string | null;
  linkedIssues: number[];
  checkSuites: WorkflowRunInfo[];
  reviews: ReviewInfo[];
  reviewComments: ReviewComment[];
  changedFilesCount: number;
  additions: number;
  deletions: number;
  mergeStateStatus?: "BEHIND" | "BLOCKED" | "CLEAN" | "DIRTY" | "DRAFT" | "HAS_HOOKS" | "UNKNOWN" | "UNSTABLE" | null;
}

export interface PrStatusLite {
  number: number;
  state: "OPEN" | "CLOSED" | "MERGED";
  url: string;
  mergeable: "MERGEABLE" | "CONFLICTING" | "UNKNOWN";
  baseBranch: string;
  headBranch: string;
  checkSuites: WorkflowRunInfo[];
}

export interface BranchPrReference {
  number: number;
  title: string;
  state: "OPEN" | "CLOSED" | "MERGED" | (string & {});
  url: string | null;
}

export interface WorkflowRunInfo {
  workflowName: string;
  runId: number;
  status: "queued" | "in_progress" | "completed";
  conclusion:
    | "success"
    | "failure"
    | "neutral"
    | "cancelled"
    | "timed_out"
    | "action_required"
    | "skipped"
    | null;
  isRequired?: boolean;
}

export interface ReviewInfo {
  reviewer: string;
  state:
    | "APPROVED"
    | "CHANGES_REQUESTED"
    | "COMMENTED"
    | "PENDING"
    | "DISMISSED";
}

export interface ReviewComment {
  author: string;
  body: string;
  filePath: string | null;
  line: number | null;
  codeSnippet: string | null;
  createdAt: string;
}

export interface PrStatusResponse {
  statuses: Record<string, PrStatusLite | null>;
  ghStatus: GhCliStatus;
}

// === Project Mode 3-Layer Type Definitions ===

export interface ProjectModeWorkspaceState {
  sessionId: string;
  status: "active" | "paused" | "completed" | "failed";
  lead: LeadState;
  issues: ProjectIssue[];
  developerAgentType: "claude" | "codex" | "gemini";
}

export interface LeadState {
  messages: LeadMessage[];
  status: "idle" | "thinking" | "waiting_approval" | "orchestrating" | "error";
  llmCallCount: number;
  estimatedTokens: number;
}

export interface LeadMessage {
  role: "user" | "assistant" | "system";
  kind: "message" | "thought" | "action" | "observation" | "error" | "progress";
  content: string;
  timestamp: number;
}

export interface ProjectIssue {
  id: string;
  githubIssueNumber: number;
  githubIssueUrl: string;
  title: string;
  status: "pending" | "planned" | "in_progress" | "ci_fail" | "completed" | "failed";
  coordinator?: CoordinatorState;
  tasks: ProjectTask[];
}

export type DashboardIssue = ProjectIssue & {
  expanded: boolean;
  taskCompletedCount: number;
  taskTotalCount: number;
};

export interface CoordinatorState {
  paneId: string;
  status: "starting" | "running" | "completed" | "crashed" | "restarting";
}

export interface ProjectTask {
  id: string;
  name: string;
  status: "pending" | "ready" | "running" | "completed" | "failed" | "cancelled";
  developers: DeveloperState[];
  testStatus?: "not_run" | "running" | "passed" | "failed";
  pullRequest?: { number: number; url: string; ciStatus?: string };
  retryCount: number;
}

export type DashboardTask = ProjectTask & {
  developerCount: number;
};

export interface DeveloperState {
  id: string;
  agentType: "claude" | "codex" | "gemini";
  paneId: string;
  status: "starting" | "running" | "completed" | "error";
  worktree: { branchName: string; path: string };
}
