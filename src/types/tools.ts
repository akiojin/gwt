/**
 * カスタムAIツール対応機能の型定義
 *
 * この型定義ファイルは、設定ファイル（tools.json）のスキーマと
 * 内部で使用するデータ構造を定義します。
 */

// ============================================================================
// 設定ファイルのスキーマ
// ============================================================================

/**
 * ツール実行方式
 *
 * - path: 絶対パスで直接実行（例: /usr/local/bin/my-tool）
 * - bunx: bunx経由でパッケージを実行（例: @org/package@latest）
 * - command: PATH環境変数から探して実行（例: aider）
 */
export type ToolExecutionType = "path" | "bunx" | "command";

/**
 * 実行モード別引数
 *
 * 各モードで使用する引数の配列を定義します。
 * 少なくとも1つのモードを定義する必要があります。
 */
export interface ModeArgs {
  /**
   * 通常モード時の引数
   */
  normal?: string[];

  /**
   * 継続モード時の引数
   */
  continue?: string[];

  /**
   * 再開モード時の引数
   */
  resume?: string[];
}

/**
 * カスタムAIツール定義
 *
 * tools.jsonファイルで定義される個別のツール設定。
 */
export interface CustomAITool {
  /**
   * ツールの一意識別子
   *
   * 小文字英数字とハイフンのみ使用可能（パターン: ^[a-z0-9-]+$）
   * ビルトインツール（claude-code, codex-cli）との重複は不可。
   */
  id: string;

  /**
   * UI表示名
   *
   * ツール選択画面で表示される名前。日本語も使用可能。
   */
  displayName: string;

  /**
   * アイコン文字（オプション）
   *
   * ツール選択画面で表示されるUnicode文字。
   */
  icon?: string;

  /**
   * 説明文（オプション）
   */
  description?: string;

  /**
   * 実行方式
   *
   * - "path": 絶対パスで直接実行
   * - "bunx": bunx経由でパッケージを実行
   * - "command": PATH環境変数から探して実行
   */
  type: ToolExecutionType;

  /**
   * 実行パス/パッケージ名/コマンド名
   *
   * typeに応じた値を設定：
   * - type="path": 絶対パス（例: /usr/local/bin/my-tool）
   * - type="bunx": パッケージ名（例: @org/package@latest）
   * - type="command": コマンド名（例: aider）
   */
  command: string;

  /**
   * デフォルト引数（オプション）
   *
   * ツール実行時に常に付与される引数。
   * 最終的な引数は: defaultArgs + modeArgs[mode] + permissionSkipArgs + extraArgs
   */
  defaultArgs?: string[];

  /**
   * モード別引数
   *
   * normal/continue/resumeの各モードで使用する引数。
   * 少なくとも1つのモードを定義する必要があります。
   */
  modeArgs: ModeArgs;

  /**
   * 権限スキップ時の引数（オプション）
   *
   * ユーザーが権限スキップを有効にした場合に追加される引数。
   */
  permissionSkipArgs?: string[];

  /**
   * 環境変数（オプション）
   *
   * ツール起動時に設定される環境変数。
   * APIキーや設定ファイルパスなどを指定。
   */
  env?: Record<string, string>;

  /**
   * 作成日時（ISO8601）。tools.jsonのメタデータとして使用。
   */
  createdAt?: string;

  /**
   * 更新日時（ISO8601）。tools.jsonのメタデータとして使用。
   */
  updatedAt?: string;
}

/**
 * ツール設定ファイル全体
 *
 * ~/.claude-worktree/tools.json のスキーマ。
 */
export interface ToolsConfig {
  /**
   * 設定フォーマットのバージョン
   *
   * セマンティックバージョニング形式。
   */
  version: string;

  /**
   * 設定ファイルの最終更新日時（ISO8601）
   */
  updatedAt?: string;

  /**
   * すべてのツールで共有する環境変数
   */
  env?: Record<string, string>;

  /**
   * カスタムツール定義の配列
   *
   * 空配列も許可（ビルトインツールのみ使用）。
   */
  customTools: CustomAITool[];
}

// ============================================================================
// 内部使用の型定義
// ============================================================================

/**
 * 統合ツール設定
 *
 * ビルトインツールとカスタムツールを統合して扱うための内部型。
 * getAllTools() 関数がこの型の配列を返します。
 */
export interface AIToolConfig {
  /**
   * ツールID
   *
   * ビルトイン: "claude-code" | "codex-cli"
   * カスタム: CustomAITool.id
   */
  id: string;

  /**
   * UI表示名
   */
  displayName: string;

  /**
   * アイコン文字（オプション）
   */
  icon?: string;

  /**
   * ビルトインツールかどうか
   *
   * true: Claude Code または Codex CLI
   * false: カスタムツール
   */
  isBuiltin: boolean;

  /**
   * カスタムツールの場合、元の設定
   *
   * isBuiltin=false の場合のみ存在。
   */
  customConfig?: CustomAITool;
}

/**
 * ツール起動オプション
 *
 * launchCustomAITool() 関数の引数として使用。
 */
export interface LaunchOptions {
  /**
   * 実行モード
   */
  mode?: "normal" | "continue" | "resume";

  /**
   * 権限スキップを有効にするか
   *
   * true の場合、permissionSkipArgs が追加されます。
   */
  skipPermissions?: boolean;

  /**
   * 追加引数
   *
   * コマンドラインから -- 以降に渡された引数。
   */
  extraArgs?: string[];

  /**
   * 作業ディレクトリ（ワークツリーパス）
   *
   * ツール起動時のcwdとして使用されます。
   */
  cwd?: string;

  /**
   * 共有環境変数（共通env + ローカル取り込み）
   */
  sharedEnv?: Record<string, string>;
}
