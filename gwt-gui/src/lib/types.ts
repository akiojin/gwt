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

export interface AgentInfo {
  id: "claude" | "codex" | "gemini" | (string & {});
  name: string;
  version: string;
  path?: string;
  authenticated: boolean;
  available: boolean;
}

export interface SettingsData {
  protected_branches: string[];
  default_base_branch: string;
  worktree_root: string;
  debug: boolean;
  log_dir?: string | null;
  log_retention_days: number;
  web_port: number;
  web_address: string;
  web_cors: boolean;
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

export interface DockerContext {
  worktree_path?: string | null;
  file_type: "compose" | "none";
  compose_services: string[];
  docker_available: boolean;
  compose_available: boolean;
  daemon_running: boolean;
  force_host: boolean;
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
