import type { AgentId } from "./agentUtils";

export type AgentStatusValue =
  | "unknown"
  | "running"
  | "waiting_input"
  | "stopped";

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
  display_name?: string | null;
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
  agent_skill_registration_enabled?: boolean | null;
  agent_inject_claude_md?: boolean | null;
  agent_inject_agents_md?: boolean | null;
  agent_inject_gemini_md?: boolean | null;
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
  language: "auto" | "ja" | "en" | (string & {});
  quality: "fast" | "balanced" | "accurate" | (string & {});
  model: string;
}

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
  profiles: Record<string, Profile>;
}

export interface ProjectIndexSearchResult {
  path: string;
  description: string;
  distance: number | null;
  fileType: string | null;
  size: number | null;
}

export interface GitHubIssueSearchResult {
  number: number;
  title: string;
  url: string;
  state: string;
  labels: string[];
  distance: number | null;
}

export interface Tab {
  id: string;
  label: string;
  branchName?: string;
  agentId?: AgentId;
  type:
    | "summary"
    | "agent"
    | "settings"
    | "versionHistory"
    | "terminal"
    | "issueSpec"
    | "issues"
    | "prs"
    | "projectIndex"
    | "assistant";
  paneId?: string;
  cwd?: string;
  issueNumber?: number;
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
  suggestion: string;
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

// GitHub Issue types (gwt-spec issue)

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

export interface IssueBranchMatch {
  issueNumber: number;
  branchName: string;
}

export const ISSUE_BRANCH_LOOKUP_UNKNOWN =
  "__gwt_issue_branch_lookup_unknown__";
export type IssueBranchLookupState =
  | string
  | null
  | typeof ISSUE_BRANCH_LOOKUP_UNKNOWN;

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
  fastMode?: boolean;
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
  aiBranchDescription?: string;
}

// PR Status types (gwt-spec issue)

export type MergeUiState =
  | "merged"
  | "closed"
  | "checking"
  | "blocked"
  | "conflicting"
  | "mergeable";

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
  mergeStateStatus?:
    | "BEHIND"
    | "BLOCKED"
    | "CLEAN"
    | "DIRTY"
    | "DRAFT"
    | "HAS_HOOKS"
    | "UNKNOWN"
    | "UNSTABLE"
    | null;
  /** UI-oriented merge state synthesized by backend. */
  mergeUiState?: MergeUiState;
  /** True when only non-required checks are failing. */
  nonRequiredChecksWarning?: boolean;
  /** True while backend retry is in progress for UNKNOWN merge status. */
  retrying?: boolean;
}

export interface PrStatusLite {
  number: number;
  state: "OPEN" | "CLOSED" | "MERGED";
  url: string;
  mergeable: "MERGEABLE" | "CONFLICTING" | "UNKNOWN";
  baseBranch: string;
  headBranch: string;
  checkSuites: WorkflowRunInfo[];
  /** UI-oriented merge state synthesized by backend. */
  mergeUiState?: MergeUiState;
  /** True when only non-required checks are failing. */
  nonRequiredChecksWarning?: boolean;
  /** True while backend retry is in progress for UNKNOWN merge status. */
  retrying?: boolean;
}

export interface BranchPrReference {
  number: number;
  title: string;
  state: "OPEN" | "CLOSED" | "MERGED" | (string & {});
  url: string | null;
}

export interface BranchPrPreflight {
  baseBranch: string;
  aheadBy: number;
  behindBy: number;
  status: "up_to_date" | "ahead" | "behind" | "diverged";
  blockingReason: string | null;
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
  /** Repository identity key used to match `pr-status-updated` events. */
  repoKey?: string | null;
}

// PR List types (gwt-spec issue)

export interface PrListItem {
  number: number;
  title: string;
  state: "OPEN" | "CLOSED" | "MERGED";
  isDraft: boolean;
  headRefName: string;
  baseRefName: string;
  author: { login: string };
  labels: Array<{ name: string; color: string }>;
  createdAt: string;
  updatedAt: string;
  url: string;
  body: string;
  reviewRequests: Array<{ login: string }>;
  assignees: Array<{ login: string }>;
}

export interface FetchPrListResponse {
  items: PrListItem[];
  ghStatus: GhCliStatus;
}

export interface GitHubUserResponse {
  login: string;
  ghStatus: GhCliStatus;
}

// === Assistant Mode Types ===

export interface AssistantMessage {
  role: "user" | "assistant" | "system" | "tool";
  kind: "text" | "tool_use" | (string & {});
  content: string;
  timestamp: number;
}

export interface PaneDashboard {
  paneId: string;
  agentName: string;
  status: string;
}

export interface GitDashboard {
  branch: string;
  uncommittedCount: number;
  unpushedCount: number;
}

export interface DashboardData {
  panes: PaneDashboard[];
  git: GitDashboard;
}

export interface AssistantState {
  messages: AssistantMessage[];
  aiReady: boolean;
  isThinking: boolean;
  sessionId?: string | null;
  llmCallCount: number;
  estimatedTokens: number;
  startupStatus: "idle" | "analyzing" | "ready" | "failed" | (string & {});
  startupSummaryReady: boolean;
  startupFailureKind?:
    | "resource_guard"
    | "ai_not_configured"
    | "llm_error"
    | "unknown"
    | (string & {})
    | null;
  startupFailureDetail?: string | null;
  startupRecoveryHints: string[];
  workingGoal?: string | null;
  goalConfidence?: "high" | "medium" | "low" | (string & {}) | null;
  currentStatus?:
    | "analyzing"
    | "awaiting_goal_confirmation"
    | "awaiting_user_choice"
    | "monitoring"
    | "blocked"
    | (string & {})
    | null;
  blockers: string[];
  recommendedNextActions: string[];
  queuedMessageCount: number;
}
