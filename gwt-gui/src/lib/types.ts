export interface BranchInfo {
  name: string;
  commit: string;
  is_current: boolean;
  is_agent_running: boolean;
  ahead: number;
  behind: number;
  divergence_status: string; // "UpToDate" | "Ahead" | "Behind" | "Diverged"
  last_tool_usage?: string | null;
}

export interface ProjectInfo {
  path: string;
  repo_name: string;
  current_branch: string | null;
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

export interface AgentModeMessage {
  role: "user" | "assistant" | "system" | "tool";
  kind?: "message" | "thought" | "action" | "observation" | "error";
  content: string;
  timestamp: number;
}

export interface AgentModeState {
  messages: AgentModeMessage[];
  ai_ready: boolean;
  ai_error?: string | null;
  last_error?: string | null;
  is_waiting: boolean;
  session_name?: string | null;
  llm_call_count: number;
  estimated_tokens: number;
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
  docker_force_host: boolean;
  ui_font_size: number;
  terminal_font_size: number;
  voice_input: VoiceInputSettings;
}

export interface VoiceInputSettings {
  enabled: boolean;
  hotkey: string;
  language: "auto" | "ja" | "en" | (string & {});
  model: string;
}

export interface AISettings {
  endpoint: string;
  api_key: string;
  model: string;
  summary_enabled: boolean;
}

export interface Profile {
  name: string;
  env: Record<string, string>;
  disabled_env: string[];
  description: string;
  ai?: AISettings | null;
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
  type: "summary" | "agent" | "settings" | "versionHistory" | "agentMode" | "terminal";
  paneId?: string;
  cwd?: string;
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

export interface DockerContext {
  worktree_path?: string | null;
  file_type: "compose" | "devcontainer" | "dockerfile" | "none";
  compose_services: string[];
  docker_available: boolean;
  compose_available: boolean;
  daemon_running: boolean;
  force_host: boolean;
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
  ahead: number;
  behind: number;
  is_gone: boolean;
  last_tool_usage: string | null;
  safety_level: "safe" | "warning" | "danger" | "disabled";
}

export interface CleanupResult {
  branch: string;
  success: boolean;
  error: string | null;
}

export interface CleanupProgress {
  branch: string;
  status: "deleting" | "deleted" | "failed";
  error?: string;
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

// GitHub Issue types (SPEC-c6ba640a)

export interface GitHubIssueInfo {
  number: number;
  title: string;
  updatedAt: string;
  labels: string[];
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
}
