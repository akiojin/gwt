/** Terminal pane information. */
export interface TerminalInfo {
  pane_id: string;
  agent_name: string;
  branch_name: string;
  status: string;
}

/** Branch information. */
export interface BranchInfo {
  canonical_name: string;
  display_name: string;
  branch_type: string;
  is_current: boolean;
  is_worktree: boolean;
  worktree_path: string | null;
  has_running_agent: boolean;
  issue_number: number | null;
  issue_title: string | null;
}

/** Worktree information. */
export interface WorktreeInfo {
  path: string;
  branch: string | null;
  is_main: boolean;
  is_bare: boolean;
  is_detached: boolean;
}

/** Agent Canvas viewport. */
export interface Viewport {
  x: number;
  y: number;
  zoom: number;
}

/** Agent Canvas tile layout. */
export interface TileLayout {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Tab in the agent canvas. */
export interface Tab {
  id: string;
  label: string;
  type: "assistant" | "worktree" | "agent";
  branchName?: string;
  paneId?: string;
}

/** Application settings. */
export interface AppSettings {
  theme: string;
  ai_model: string | null;
  ai_api_key_configured: boolean;
  [key: string]: unknown;
}

/** Project information. */
export interface ProjectInfo {
  path: string;
  name: string;
  is_git_repo: boolean;
  current_branch: string | null;
}

/** Structured error from the server. */
export interface StructuredError {
  severity: string;
  code: string;
  message: string;
  command: string;
  category: string;
  suggestions: string[];
  timestamp: string;
}
