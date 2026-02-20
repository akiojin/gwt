# リサーチ: Windows シェル選択（Launch Agent / New Terminal）

**仕様ID**: `SPEC-1350124a` | **日付**: 2026-02-19

## 調査結果

### 1. 既存のシェル解決ロジック

#### PTY レイヤー (`crates/gwt-core/src/terminal/pty.rs`)

- `resolve_windows_shell()`: `pwsh`（PowerShell 7）を優先、なければ `powershell.exe`（5.1）にフォールバック
- `resolve_spawn_command_for_platform()`: Windows 時に PowerShell ラッパー（`-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command`）で包む
- `PtyConfig`: `command`, `args`, `working_dir`, `env_vars`, `rows`, `cols` を保持

#### Tauri コマンドレイヤー (`crates/gwt-tauri/src/commands/terminal.rs`)

- `resolve_shell_launch_spec(is_windows)`: `SHELL` 環境変数（Unix）/ `COMSPEC`（Windows）→ フォールバック（`/bin/sh` / `cmd.exe`）
- `spawn_shell()`: `resolve_shell_launch_spec()` でシェルを決定、`BuiltinLaunchConfig` を構築して PTY 起動
- `launch_agent_for_project_root()`: `LaunchAgentRequest` を受け取り、エージェント起動の全フローを統括

### 2. 設定アーキテクチャ

#### Settings 構造体 (`crates/gwt-core/src/config/settings.rs`)

- 現在のフィールド: `agent`, `docker`, `appearance`, `voice_input` 等
- ターミナル関連の設定フィールドは未存在
- figment ベースの TOML デシリアライズ、`~/.gwt/config.toml` に保存
- `save_global()` でアトミック書き込み（temp + rename）

#### SettingsData DTO（Tauri ↔ Frontend）

- `crates/gwt-tauri/src/commands/settings.rs` で `SettingsData` を定義
- `from(&Settings)` / `to_settings()` の双方向変換

### 3. フロントエンド構造

#### AgentLaunchForm

- Advanced Options: Extra Args テキストエリア + Env Overrides テキストエリア
- `handleLaunch()`: フォーム状態から `LaunchAgentRequest` を構築 → `onLaunch` コールバック → `saveLaunchDefaults()` で永続化
- Docker モード時の条件分岐が既に存在

#### WorktreeSummaryPanel

- `onLaunchAgent` コールバック props でボタンクリックを親に通知
- Launch Agent ボタン: `<button class="launch-btn">`
- Quick Launch ボタン（Continue / New）が隣接

#### SettingsPanel

- タブ: `"appearance" | "voiceInput" | "mcpBridge" | "profiles"`
- `invoke("get_settings")` / `invoke("save_settings")` パターン

#### Sidebar → App 接続

- `onLaunchAgent`, `onQuickLaunch` を props で伝播
- App.svelte がモーダル制御とコマンド呼び出しを統括

#### agentLaunchDefaults.ts

- `LaunchDefaults` 型: エージェント・モデル・バージョン・Docker 設定等を localStorage に永続化
- キー: `gwt.launchDefaults.v1`
- ここにシェル選択を追加すれば FR-014 を満たせる

### 4. portable-pty と WSL

#### WSL PTY 起動の実現可能性

- `portable-pty` は Windows では ConPTY バックエンドを使用
- `wsl.exe` は通常のコンソールアプリケーションとして ConPTY 経由で起動可能
- `PtyConfig.command = "wsl.exe"` で PTY を生成し、`--cd /mnt/c/...` で作業ディレクトリを指定する方式が最もシンプル
- WSL 内のシェル（bash/zsh）が起動するため、プロンプト表示後にコマンドを PTY に書き込む方式で動作

#### WSL パス変換

- `C:\Users\foo` → `/mnt/c/Users/foo`: ドライブレター小文字化 + バックスラッシュ→スラッシュ
- `PathBuf::display()` で得られる Windows パス文字列を直接処理
- UNC パス（`\\server\share`）はサポート外

### 5. LaunchAgentRequest の拡張ポイント

- 既存フィールドに `terminal_shell: Option<String>` を追加
- Rust 側: `#[serde(default)]` で後方互換
- TypeScript 側: `terminal_shell?: string` で Optional
- `launch_agent_for_project_root()` 内で `terminal_shell` に基づいてシェル解決を分岐

### 6. spawn_shell の拡張ポイント

- 現在の引数: `working_dir: Option<String>`
- `shell: Option<String>` を追加
- `shell` 未指定時: Settings の `default_shell` → 既存の `resolve_shell_launch_spec()` フォールバック
- WSL 時: `wsl.exe` を `BuiltinLaunchConfig.command` に設定、`working_dir` を WSL パスに変換

## 解消済みの要確認事項

| 項目 | 解決策 |
|---|---|
| portable-pty の WSL 対応 | ConPTY 経由で `wsl.exe` を起動可能。フォールバックとして非インタラクティブ方式を用意 |
| WSL プロンプト検出タイムアウト | 3 秒に設定。超過時は非インタラクティブ方式（`wsl.exe -e bash -lc 'command'`）にフォールバック |
| Settings Terminal タブの配置 | 新規 "terminal" タブを SettingsTabId に追加。macOS/Linux では非表示 |
| シェル選択の永続化 | agentLaunchDefaults に `selectedShell` を追加（FR-014） + Settings に `default_shell`（FR-001） |
| New Terminal ボタンのアイコン | `>_` Unicode シンボル |
