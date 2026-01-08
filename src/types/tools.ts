/**
 * コーディングエージェント対応機能の型定義
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
 * コーディングエージェント定義
 *
 * tools.jsonファイルで定義される個別のエージェント設定。
 */
export interface CodingAgent {
  /**
   * エージェントの一意識別子
   *
   * 小文字英数字とハイフンのみ使用可能（パターン: ^[a-z0-9-]+$）
   * ビルトインエージェント（claude-code, codex-cli）との重複は不可。
   */
  id: string;

  /**
   * UI表示名
   *
   * エージェント選択画面で表示される名前。日本語も使用可能。
   */
  displayName: string;

  /**
   * アイコン文字（オプション）
   *
   * エージェント選択画面で表示されるUnicode文字。
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
   * エージェント実行時に常に付与される引数。
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
   * エージェント起動時に設定される環境変数。
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
 * コーディングエージェント設定ファイル全体
 *
 * ~/.gwt/tools.json のスキーマ。
 */
export interface CodingAgentsConfig {
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
   * すべてのエージェントで共有する環境変数
   */
  env?: Record<string, string>;

  /**
   * カスタムコーディングエージェント定義の配列
   *
   * 空配列も許可（ビルトインエージェントのみ使用）。
   */
  customCodingAgents: CodingAgent[];
}

// ============================================================================
// 内部使用の型定義
// ============================================================================

/**
 * 統合コーディングエージェント設定
 *
 * ビルトインエージェントとカスタムエージェントを統合して扱うための内部型。
 * getAllCodingAgents() 関数がこの型の配列を返します。
 */
export interface CodingAgentConfig {
  /**
   * エージェントID
   *
   * ビルトイン: "claude-code" | "codex-cli"
   * カスタム: CodingAgent.id
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
   * ビルトインエージェントかどうか
   *
   * true: Claude Code または Codex CLI
   * false: カスタムエージェント
   */
  isBuiltin: boolean;

  /**
   * カスタムエージェントの場合、元の設定
   *
   * isBuiltin=false の場合のみ存在。
   */
  customConfig?: CodingAgent;
}

/**
 * コーディングエージェント起動オプション
 *
 * launchCodingAgent() 関数の引数として使用。
 */
/**
 * バージョン選択
 *
 * - "installed": bunxのデフォルト動作（キャッシュ優先）
 * - "latest": 常に最新版をダウンロード
 * - その他: 具体的なバージョン番号（例: "1.0.3", "2.1.0-beta.1"）
 */
export type VersionSelection = "installed" | "latest" | string;

export interface CodingAgentLaunchOptions {
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
   * エージェント起動時のcwdとして使用されます。
   */
  cwd?: string;

  /**
   * 共有環境変数（共通env + ローカル取り込み）
   */
  sharedEnv?: Record<string, string>;

  /**
   * バージョン選択
   *
   * bunxタイプのエージェントでのみ使用。
   * - "installed": bunxのデフォルト動作（キャッシュ優先）
   * - "latest": 常に最新版をダウンロード
   * - その他: 具体的なバージョン番号
   */
  version?: VersionSelection;
}
