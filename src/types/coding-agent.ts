/**
 * Coding Agent Types
 *
 * コーディングエージェント（Claude, Codex, Gemini）の共通型定義。
 */

/**
 * コーディングエージェントの基本情報
 */
export interface CodingAgentInfo {
  /** ツールID */
  id: string;
  /** 表示名 */
  name: string;
  /** コマンド名 */
  command: string;
  /** npmパッケージ名（グローバルインストール用） */
  packageName?: string;
}

/**
 * コーディングエージェントの起動オプション（共通部分）
 */
export interface CodingAgentLaunchOptions {
  /** 作業ディレクトリ */
  cwd: string;
  /** セッションID（継続時） */
  sessionId?: string;
  /** 継続モード */
  continueSession?: boolean;
  /** 再開モード */
  resumeSession?: boolean;
  /** モデルID */
  model?: string;
  /** 環境変数 */
  env?: Record<string, string>;
}

/**
 * コーディングエージェントの起動結果
 */
export interface CodingAgentLaunchResult {
  /** セッションID */
  sessionId?: string;
  /** 終了コード */
  exitCode: number;
  /** 終了シグナル */
  signal?: string;
}

/**
 * コーディングエージェントのセッション検出結果
 */
export interface CodingAgentSessionInfo {
  /** セッションID */
  sessionId: string;
  /** セッションパス */
  path?: string;
  /** 最終更新日時 */
  lastModified?: Date;
}

/**
 * コーディングエージェント登録情報
 */
export const CODING_AGENTS: Record<string, CodingAgentInfo> = {
  claude: {
    id: "claude",
    name: "Claude Code",
    command: "claude",
    packageName: "@anthropic-ai/claude-code",
  },
  codex: {
    id: "codex",
    name: "Codex CLI",
    command: "codex",
    packageName: "codex",
  },
  gemini: {
    id: "gemini",
    name: "Gemini CLI",
    command: "gemini",
    packageName: "@anthropic-ai/gemini-cli",
  },
};
