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

// 日本語メッセージ（デフォルト）
export const MESSAGES_JA = {
  [MESSAGE_KEYS.ERROR.NOT_GIT_REPO]:
    "このディレクトリはGitリポジトリではありません",
  [MESSAGE_KEYS.ERROR.GIT_COMMAND_FAILED]: "Gitコマンドの実行に失敗しました",
  [MESSAGE_KEYS.ERROR.WORKTREE_CREATE_FAILED]: "worktreeの作成に失敗しました",
  [MESSAGE_KEYS.ERROR.GITHUB_CLI_NOT_AVAILABLE]:
    "GitHub CLIがインストールされていません",
  [MESSAGE_KEYS.ERROR.GITHUB_AUTH_REQUIRED]:
    "GitHub認証が必要です。gh auth login を実行してください",
  [MESSAGE_KEYS.SUCCESS.WORKTREE_CREATED]: "worktreeを作成しました",
  [MESSAGE_KEYS.SUCCESS.BRANCH_CREATED]: "ブランチを作成しました",
  [MESSAGE_KEYS.SUCCESS.CHANGES_COMMITTED]: "変更をコミットしました",
  [MESSAGE_KEYS.SUCCESS.CHANGES_PUSHED]: "変更をプッシュしました",
  [MESSAGE_KEYS.SUCCESS.CLEANUP_COMPLETED]: "クリーンアップが完了しました",
  [MESSAGE_KEYS.INFO.LOADING]: "読み込み中...",
  [MESSAGE_KEYS.INFO.PROCESSING]: "処理中...",
  [MESSAGE_KEYS.INFO.FETCHING_DATA]: "データを取得中...",
} as const;

// メッセージ取得関数（将来的に多言語対応可能）
export function getMessage(key: string): string {
  // 現時点では日本語のみサポート
  return MESSAGES_JA[key as keyof typeof MESSAGES_JA] || key;
}
