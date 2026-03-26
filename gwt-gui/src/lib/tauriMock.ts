/**
 * Mock responses for Tauri commands when running in browser dev mode.
 * Provides sensible defaults so the UI renders without the Tauri runtime.
 */

import type {
  BranchInfo,
  BranchInventorySnapshotEntry,
  GitHubIssueInfo,
  ProjectInfo,
  SettingsData,
  Tab,
  WorktreeInfo,
} from "./types";

const MOCK_PROJECT_PATH = "/Users/demo/projects/my-app";

const MOCK_BRANCHES: BranchInfo[] = [
  {
    name: "feature/auth-system",
    display_name: "Auth System",
    commit: "abc1234",
    is_current: false,
    is_agent_running: true,
    agent_status: "running",
    ahead: 3,
    behind: 0,
    divergence_status: "ahead",
    last_tool_usage: null,
  } as unknown as BranchInfo,
  {
    name: "feature/dashboard",
    display_name: "Dashboard UI",
    commit: "def5678",
    is_current: false,
    is_agent_running: false,
    agent_status: "idle",
    ahead: 1,
    behind: 2,
    divergence_status: "diverged",
    last_tool_usage: null,
  } as unknown as BranchInfo,
  {
    name: "fix/login-bug",
    display_name: "Login Bug Fix",
    commit: "ghi9012",
    is_current: true,
    is_agent_running: false,
    agent_status: "idle",
    ahead: 0,
    behind: 0,
    divergence_status: "up_to_date",
    last_tool_usage: null,
  } as unknown as BranchInfo,
];

const MOCK_WORKTREES: WorktreeInfo[] = [
  {
    path: `${MOCK_PROJECT_PATH}`,
    branch: "main",
    commit: "aaa1111",
    status: "clean",
    is_main: true,
    has_changes: false,
    has_unpushed: false,
    is_current: true,
    is_protected: true,
    agent_status: "idle",
  } as unknown as WorktreeInfo,
  {
    path: `${MOCK_PROJECT_PATH}/../my-app-feature-auth`,
    branch: "feature/auth-system",
    commit: "abc1234",
    status: "modified",
    is_main: false,
    has_changes: true,
    has_unpushed: true,
    is_current: false,
    is_protected: false,
    agent_status: "running",
  } as unknown as WorktreeInfo,
];

const MOCK_SETTINGS: SettingsData = {
  protected_branches: ["main", "master", "develop"],
  default_base_branch: "main",
  worktree_root: "",
  ui_font_size: 13,
  terminal_font_size: 14,
  ui_font_family: "",
  terminal_font_family: "",
  app_language: "auto",
  voice_input: {
    enabled: false,
    engine: "qwen3-asr",
    language: "auto",
    quality: "balanced",
    model: "Qwen/Qwen3-ASR-1.7B",
  },
  debug: false,
  profiling: false,
} as SettingsData;

const MOCK_RECENT_PROJECTS = [
  {
    path: MOCK_PROJECT_PATH,
    repo_name: "my-app",
    lastOpened: new Date(Date.now() - 1000 * 60 * 5).toISOString(),
  },
  {
    path: "/Users/demo/projects/backend-api",
    repo_name: "backend-api",
    lastOpened: new Date(Date.now() - 1000 * 60 * 60).toISOString(),
  },
  {
    path: "/Users/demo/projects/docs-site",
    repo_name: "docs-site",
    lastOpened: new Date(Date.now() - 1000 * 60 * 60 * 24).toISOString(),
  },
];

const MOCK_INVENTORY: BranchInventorySnapshotEntry[] = [
  {
    id: "feature/auth-system",
    canonical_name: "feature/auth-system",
    primary_branch: "feature/auth-system",
    local_branch: "feature/auth-system",
    remote_branch: "origin/feature/auth-system",
    has_local: true,
    has_remote: true,
    worktree_path: `${MOCK_PROJECT_PATH}/../my-app-feature-auth`,
    worktree_count: 1,
  } as unknown as BranchInventorySnapshotEntry,
];

/** Mock command responses keyed by command name */
const MOCK_RESPONSES: Record<string, unknown> = {
  get_settings: MOCK_SETTINGS,
  get_recent_projects: MOCK_RECENT_PROJECTS,
  get_report_system_info: { osName: "macos", osVersion: "15.0" },
  get_system_info: {
    cpu_usage_percent: 12.5,
    memory_used_bytes: 8_000_000_000,
    memory_total_bytes: 16_000_000_000,
    gpus: [],
  },
  get_startup_diagnostics: {
    startup_trace: false,
    disable_tray: false,
    disable_login_shell_capture: false,
    disable_heartbeat_watchdog: false,
    disable_session_watcher: false,
    disable_startup_update_check: true,
    disable_profiling: false,
    disable_tab_restore: false,
  },
  get_http_ipc_port: 0,
  list_worktree_branches: MOCK_BRANCHES,
  list_worktrees: MOCK_WORKTREES,
  list_branch_inventory: MOCK_INVENTORY,
  get_branch_inventory_detail: null,
  list_terminals: [],
  heartbeat: null,
  report_frontend_metrics: null,
  get_project_info: {
    path: MOCK_PROJECT_PATH,
    repo_name: "my-app",
    current_branch: "main",
  } as ProjectInfo,
  open_project: { action: "opened", info: { path: MOCK_PROJECT_PATH, repo_name: "my-app", current_branch: "main" } },
  probe_path: { kind: "gwtProject", projectPath: MOCK_PROJECT_PATH },
  is_git_repo: true,
  get_current_window_label: "main",
  try_acquire_window_restore_leader: true,
  release_window_restore_leader: null,
  sync_window_agent_tabs: null,
  sync_window_active_tab: null,
  get_skill_registration_status_cmd: { status: "ok", issues: [] },
  detect_agents: [],
  list_agent_versions: { tags: [], versions: [] },
  get_profiles: { version: 1, profiles: {} },
  get_captured_environment: {},
  is_os_env_ready: true,
  check_app_update: null,
  check_gh_available: true,
  get_voice_capability: { available: false, reason: "not_configured" },
  fetch_github_issues: [],
  fetch_pr_list: [],
  get_stats: {
    total_worktrees: 3,
    total_agents: 1,
    total_terminals: 0,
  },
  get_git_change_summary: { files: [], insertions: 0, deletions: 0 },
  get_branch_diff_files: [],
  get_branch_commits: [],
  get_working_tree_status: [],
  get_stash_list: [],
  get_base_branch_candidates: ["main", "develop"],
};

/** Event listeners registered in mock mode (never fire, just return noop) */
const NOOP_UNLISTEN = () => {};

export function isBrowserDevMode(): boolean {
  if (typeof window === "undefined") return true;
  return (
    typeof (window as Window & { __TAURI_INTERNALS__?: unknown })
      .__TAURI_INTERNALS__ === "undefined"
  );
}

export function getMockResponse<T>(command: string): T {
  if (command in MOCK_RESPONSES) {
    return MOCK_RESPONSES[command] as T;
  }
  // Return null for unknown commands rather than throwing
  console.debug(`[mock] No mock for command: ${command}`);
  return null as T;
}

export function createMockListen() {
  return async (_event: string, _handler: (event: unknown) => void) => {
    return NOOP_UNLISTEN;
  };
}
