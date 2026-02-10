export interface BranchInfo {
  name: string;
  commit: string;
  is_current: boolean;
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

export interface AgentInfo {
  id: "claude" | "codex" | "gemini" | (string & {});
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
  type: "summary" | "agent" | "settings";
  paneId?: string;
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
  file_type: "compose" | "none";
  compose_services: string[];
  docker_available: boolean;
  compose_available: boolean;
  daemon_running: boolean;
  force_host: boolean;
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

export interface LaunchAgentRequest {
  agentId: string;
  branch: string;
  profile?: string;
  model?: string;
  agentVersion?: string;
  mode?: "normal" | "continue" | "resume";
  skipPermissions?: boolean;
  reasoningLevel?: string;
  collaborationModes?: boolean;
  extraArgs?: string[];
  envOverrides?: Record<string, string>;
  resumeSessionId?: string;
  createBranch?: { name: string; base?: string | null };
  dockerService?: string;
  dockerForceHost?: boolean;
  dockerRecreate?: boolean;
  dockerBuild?: boolean;
  dockerKeep?: boolean;
}
