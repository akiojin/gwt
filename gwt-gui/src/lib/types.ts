export interface BranchInfo {
  name: string;
  commit: string;
  is_current: boolean;
  ahead: number;
  behind: number;
  divergence_status: string; // "UpToDate" | "Ahead" | "Behind" | "Diverged"
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
  name: string;
  agent_type: string;
  available: boolean;
}

export interface SettingsData {
  log_retention_days: number;
  protected_branches: string[];
}

export interface Tab {
  id: string;
  label: string;
  type: "summary" | "agent";
  paneId?: string;
}
