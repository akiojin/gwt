/**
 * アプリケーション全体で使用する定数
 */

// ブランチタイプ
export const BRANCH_TYPES = {
  FEATURE: 'feature',
  HOTFIX: 'hotfix',
  RELEASE: 'release',
  MAIN: 'main',
  DEVELOP: 'develop',
  OTHER: 'other'
} as const;

// ブランチプレフィックス
export const BRANCH_PREFIXES = {
  FEATURE: 'feature/',
  HOTFIX: 'hotfix/',
  RELEASE: 'release/'
} as const;

// メインブランチ名
export const MAIN_BRANCHES = ['main', 'master'] as const;
export const DEVELOP_BRANCHES = ['develop', 'dev'] as const;

// 表示設定
export const DISPLAY_CONFIG = {
  MAX_BRANCH_NAME_LENGTH: 50,
  TABLE_PADDING: 2,
  CHANGES_COLUMN_WIDTH: 10
} as const;

// プロンプト設定
export const PROMPT_CONFIG = {
  PAGE_SIZE: 15,
  SEARCH_ENABLED: true
} as const;

// Git設定
export const GIT_CONFIG = {
  DEFAULT_BASE_BRANCH: 'main',
  FETCH_TIMEOUT: 30000, // 30秒
  PUSH_RETRY_COUNT: 3
} as const;

// GitHub設定
export const GITHUB_CONFIG = {
  PR_FETCH_LIMIT: 100,
  DEBUG_ENV_VAR: 'DEBUG_CLEANUP'
} as const;

// エラーメッセージ
export const ERROR_MESSAGES = {
  NOT_GIT_REPO: 'このディレクトリはGitリポジトリではありません',
  GIT_COMMAND_FAILED: 'Gitコマンドの実行に失敗しました',
  WORKTREE_CREATE_FAILED: 'worktreeの作成に失敗しました',
  GITHUB_CLI_NOT_AVAILABLE: 'GitHub CLIがインストールされていません',
  GITHUB_AUTH_REQUIRED: 'GitHub認証が必要です。gh auth login を実行してください'
} as const;

// 成功メッセージ
export const SUCCESS_MESSAGES = {
  WORKTREE_CREATED: 'worktreeを作成しました',
  BRANCH_CREATED: 'ブランチを作成しました',
  CHANGES_COMMITTED: '変更をコミットしました',
  CHANGES_PUSHED: '変更をプッシュしました',
  CLEANUP_COMPLETED: 'クリーンアップが完了しました'
} as const;