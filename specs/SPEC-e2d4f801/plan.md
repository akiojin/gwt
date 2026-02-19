# 実装計画: OS環境変数の自動継承

## 目的

- ユーザーのシェルプロファイル（`.bashrc`/`.zshrc`等）で設定された環境変数を、gwtから起動するエージェントに自動継承する
- 三層マージ構造（OS → Profile → Overrides/GLM設定）を実現する
- EnvSnapshotを廃止してシンプル化する
- デバッグ用の環境変数確認機能を追加する

## 実装方針

### Phase 1: OS環境変数キャプチャモジュール（gwt-core）

`crates/gwt-core/src/config/os_env.rs` を新規作成:

- `ShellType` enum: Bash, Zsh, Fish, Nushell, Sh（フォールバック）
- `detect_shell_type(shell_path: &str) -> ShellType`: パスからシェル種別を判定
- `build_env_capture_command(shell_type: ShellType, shell_path: &str) -> Command`: シェル種別に応じたコマンド構築
  - bash/zsh/sh: `[shell_path, "-l", "-c", "env -0"]`
  - fish: `["fish", "-l", "-c", "env -0"]`
  - nushell: `["nu", "-l", "-c", "$env | to json"]`
- `parse_env_null_separated(bytes: &[u8]) -> HashMap<String, String>`: NUL区切りパーサー
- `parse_env_json(json: &str) -> HashMap<String, String>`: nushell JSON出力パーサー
- `capture_login_shell_env() -> (HashMap<String, String>, EnvSource)`: 非同期メイン関数
  - `EnvSource`: `LoginShell` | `StdEnvFallback { reason: String }`
  - Unix: `$SHELL` 検出 → シェル起動（5秒タイムアウト）→ パース
  - `$SHELL` 未設定: `/bin/sh` で試行
  - エラー/タイムアウト: `std::env::vars()` フォールバック
  - Windows: 直接 `std::env::vars()` を返す

### Phase 2: アプリ起動時の非同期取得（gwt-tauri）

- `OsEnvState` 構造体:
  - `env: Arc<tokio::sync::OnceCell<HashMap<String, String>>>`
  - `source: Arc<tokio::sync::OnceCell<EnvSource>>`
- Tauri セットアップ時に `capture_login_shell_env()` をバックグラウンドタスクとして起動
- `get_os_env()` メソッド: 取得完了を await して環境変数を返す
- フォールバック発生時:
  - `warn!` ログ出力
  - Tauri Event で GUI にフォールバック通知を送信

### Phase 3: 三層マージロジック（gwt-tauri）

`launch_agent` の環境変数構築ロジックを変更:

1. `OsEnvState.get_os_env().await` でOS環境変数を取得（待機）
2. profiles.toml の `disabled_env` に含まれる変数を削除
3. profiles.toml の `env` で上書き
4. gwt コンテキスト変数（`GWT_PROJECT_ROOT`等）を追加
5. GLMプロバイダー変数（`ANTHROPIC_BASE_URL`等）を追加（常にOS環境変数を上書き）
6. `env_overrides` で最終上書き

### Phase 4: EnvSnapshot廃止（gwt-core）

`runner.rs` から `EnvSnapshot` 構造体と関連ロジックを削除:

- `resolve_command_path_with_env()` をOS環境変数ベースに書き換え
- PATH検索は OS環境変数から取得した PATH を使用
- 個別の HOME/BUN_INSTALL フォールバックは不要に
- 共通パス（/usr/local/bin等）のフォールバック検索は残す

### Phase 5: GUI対応

- エージェント起動時の "Loading environment..." ステータス表示
- メニュー: `Debug > Show Captured Environment`
  - 環境変数一覧（ソート済み）
  - ソース情報（login shell / std::env fallback + 理由）
- フォールバック時のトースト通知

## テスト

### unit test（gwt-core / os_env.rs）

- `parse_env_null_separated`: 正常系、値に改行、値に=、空入力、=なしエントリ（MOTD混入）
- `parse_env_json`: nushell JSON出力のパース
- `detect_shell_type`: bash/zsh/fish/nushell/不明シェルの判定
- `build_env_capture_command`: 各シェル種別のコマンド構築

### unit test（gwt-tauri）

- 三層マージ: OS + Profile(env + disabled_env) + overrides の優先順位テスト
- disabled_env: OS変数が正しく除外されること
- GLM設定がOS環境変数を上書きすること

### unit test（gwt-core / runner.rs）

- EnvSnapshot廃止後のコマンド解決テスト更新

### 手動確認

- macOS (zsh): Dock起動で `.zshrc` の変数が反映されること
- macOS (bash): `.bashrc` の変数が反映されること
- nvm/pyenv ユーザー: PATH にツールチェインが含まれること
- Debug メニューで環境変数一覧とソースが正しく表示されること
- フォールバック時にトースト通知が表示されること
