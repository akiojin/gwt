/**
 * アプリケーション全体で使用する定数
 */

// ブランチタイプ
export const BRANCH_TYPES = {
  FEATURE: "feature",
  HOTFIX: "hotfix",
  RELEASE: "release",
  MAIN: "main",
  DEVELOP: "develop",
  OTHER: "other",
} as const;

// ブランチプレフィックス
export const BRANCH_PREFIXES = {
  FEATURE: "feature/",
  HOTFIX: "hotfix/",
  RELEASE: "release/",
} as const;

// メインブランチ名
export const MAIN_BRANCHES = ["main", "master"] as const;
export const DEVELOP_BRANCHES = ["develop", "dev"] as const;

// 表示設定
export const DISPLAY_CONFIG = {
  MAX_BRANCH_NAME_LENGTH: 50,
  TABLE_PADDING: 2,
  CHANGES_COLUMN_WIDTH: 10,
} as const;

// プロンプト設定
export const PROMPT_CONFIG = {
  PAGE_SIZE: 15,
  SEARCH_ENABLED: true,
} as const;

// Git設定
export const GIT_CONFIG = {
  DEFAULT_BASE_BRANCH: "main",
  FETCH_TIMEOUT: 30000, // 30秒
  PUSH_RETRY_COUNT: 3,
} as const;

// GitHub設定
export const GITHUB_CONFIG = {
  PR_FETCH_LIMIT: 100,
  DEBUG_ENV_VAR: "DEBUG_CLEANUP",
} as const;

// メッセージキー（国際化対応の基盤）
export const MESSAGE_KEYS = {
  // エラーメッセージ
  ERROR: {
    NOT_GIT_REPO: "error.not_git_repo",
    GIT_COMMAND_FAILED: "error.git_command_failed",
    WORKTREE_CREATE_FAILED: "error.worktree_create_failed",
    GITHUB_CLI_NOT_AVAILABLE: "error.github_cli_not_available",
    GITHUB_AUTH_REQUIRED: "error.github_auth_required",
  },
  // 成功メッセージ
  SUCCESS: {
    WORKTREE_CREATED: "success.worktree_created",
    BRANCH_CREATED: "success.branch_created",
    CHANGES_COMMITTED: "success.changes_committed",
    CHANGES_PUSHED: "success.changes_pushed",
    CLEANUP_COMPLETED: "success.cleanup_completed",
  },
  // 情報メッセージ
  INFO: {
    LOADING: "info.loading",
    PROCESSING: "info.processing",
    FETCHING_DATA: "info.fetching_data",
  },
} as const;

// English messages (default)
export const MESSAGES_EN = {
  [MESSAGE_KEYS.ERROR.NOT_GIT_REPO]: "This directory is not a Git repository",
  [MESSAGE_KEYS.ERROR.GIT_COMMAND_FAILED]: "Failed to run the Git command",
  [MESSAGE_KEYS.ERROR.WORKTREE_CREATE_FAILED]: "Failed to create the worktree",
  [MESSAGE_KEYS.ERROR.GITHUB_CLI_NOT_AVAILABLE]: "GitHub CLI is not installed",
  [MESSAGE_KEYS.ERROR.GITHUB_AUTH_REQUIRED]:
    "GitHub authentication is required. Please run 'gh auth login'",
  [MESSAGE_KEYS.SUCCESS.WORKTREE_CREATED]: "Worktree created",
  [MESSAGE_KEYS.SUCCESS.BRANCH_CREATED]: "Branch created",
  [MESSAGE_KEYS.SUCCESS.CHANGES_COMMITTED]: "Changes committed",
  [MESSAGE_KEYS.SUCCESS.CHANGES_PUSHED]: "Changes pushed",
  [MESSAGE_KEYS.SUCCESS.CLEANUP_COMPLETED]: "Cleanup completed",
  [MESSAGE_KEYS.INFO.LOADING]: "Loading...",
  [MESSAGE_KEYS.INFO.PROCESSING]: "Processing...",
  [MESSAGE_KEYS.INFO.FETCHING_DATA]: "Fetching data...",
} as const;

// Message lookup helper (multi-language ready)
export function getMessage(key: string): string {
  // Currently only English is provided
  return MESSAGES_EN[key as keyof typeof MESSAGES_EN] || key;
}
