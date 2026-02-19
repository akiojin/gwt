# 実装計画: Windows シェル選択（Launch Agent / New Terminal）

**仕様ID**: `SPEC-1350124a` | **日付**: 2026-02-19 | **仕様書**: `specs/SPEC-1350124a/spec.md`

## 目的

- Windows 環境で Launch Agent および New Terminal の実行シェルを PowerShell / Command Prompt / WSL から選択可能にする
- サイドバー（WorktreeSummaryPanel）に New Terminal ボタンを追加し、全 OS でターミナルタブの即座起動を可能にする
- Settings 画面に Terminal タブを新設し、デフォルトシェルを永続化する

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-core/`, `crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **ターミナル**: xterm.js v6（TerminalView.svelte）
- **PTY**: portable-pty クレート
- **設定**: ~/.gwt/config.toml（figment による読み込み）
- **前提**: SPEC-f490dded（シンプルターミナルタブ）の `spawn_shell` コマンドが存在する

## 実装方針

### Phase 1: データモデルとバックエンド基盤

#### 1-1. TerminalSettings の追加

`crates/gwt-core/src/config/settings.rs`:

- `TerminalSettings` 構造体を追加（`default_shell: Option<String>`）
- `Settings` に `pub terminal: TerminalSettings` フィールドを追加
- `SettingsData`（Tauri ↔ Frontend の DTO）に `default_shell: Option<String>` を追加

#### 1-2. シェル列挙型の定義

`crates/gwt-core/src/terminal/shell.rs`（新規ファイル）:

- `WindowsShell` enum: `PowerShell`, `Cmd`, `Wsl`
- 各バリアントに `id()`, `display_name()`, `is_available()` メソッドを実装
- `is_available()` は `which::which()` や `wsl.exe --list --quiet` でチェック
- PowerShell バージョン検出: `pwsh --version` / `powershell -Command "$PSVersionTable.PSVersion.ToString()"`

#### 1-3. `get_available_shells` Tauri コマンド

`crates/gwt-tauri/src/commands/terminal.rs`:

- Windows: `WindowsShell` の全バリアントを `is_available()` でフィルタし、ID・表示名・バージョンを返す
- macOS/Linux: 空配列を返す
- 戻り値: `Vec<ShellInfo>` (`id: String, name: String, version: Option<String>`)

#### 1-4. Windows パス → WSL パス変換

`crates/gwt-core/src/terminal/shell.rs`:

- `pub fn windows_to_wsl_path(win_path: &str) -> Result<String>` 関数
- `C:\Users\foo` → `/mnt/c/Users/foo` のバイト操作変換
- ドライブレター（A-Z）を小文字化、バックスラッシュをスラッシュに変換
- UNC パスはエラーを返す

### Phase 2: シェル選択のバックエンド統合

#### 2-1. `spawn_shell` コマンドの拡張

`crates/gwt-tauri/src/commands/terminal.rs`:

- `shell: Option<String>` 引数を追加
- `shell` が未指定の場合は Settings の `default_shell` → 自動検出順でフォールバック
- `shell == "wsl"` の場合: `wsl.exe` を PtyConfig のコマンドとして指定、working_dir を WSL パスに変換、`--cd` 引数を渡す
- `shell == "cmd"` の場合: `cmd.exe` を PtyConfig のコマンドとして指定
- `shell == "powershell"` の場合: 既存の `resolve_windows_shell()` ロジックを使用

#### 2-2. `LaunchAgentRequest` の拡張

`crates/gwt-tauri/src/commands/terminal.rs`:

- `terminal_shell: Option<String>` フィールドを追加
- `launch_agent_for_project_root()` 内で、シェルが指定されている場合は `resolve_spawn_command_for_platform()` のロジックを上書き
- WSL の場合: PTY で `wsl.exe` を起動 → プロンプト検出 → コマンド文字列を書き込み

#### 2-3. WSL プロンプト検出と PTY 書き込み

`crates/gwt-tauri/src/commands/terminal.rs`:

- WSL PTY 起動後、出力バイトストリームを監視
- プロンプトパターン（行末の `$`, `#`, `>` + 空白）を検出
- 検出後、`cd /mnt/c/... && bunx claude ...` のコマンド文字列を PTY に書き込み
- タイムアウト（3秒）を設定し、プロンプトが検出できない場合は非インタラクティブ方式（`wsl.exe -e bash -lc 'command'`）にフォールバック

#### 2-4. WSL 環境変数のハイブリッド処理

- WSL 起動時は `os_env` のキャプチャ結果を使用しない（WSL 側のログインシェルに委任）
- GWT Profile の `env_overrides` のみ WSL PTY 起動時の環境変数に含める
- 具体的には `wsl.exe` の起動時に `--exec env KEY=VALUE ... bash -l` のように渡すか、PTY 書き込みで `export KEY=VALUE` を先行投入

### Phase 3: フロントエンド — New Terminal ボタン

#### 3-1. WorktreeSummaryPanel に New Terminal ボタン追加

`gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`:

- `onNewTerminal` コールバック props を追加
- Launch Agent ボタンの横にターミナルアイコンボタンを配置
- クリック時に `onNewTerminal()` を呼び出し

#### 3-2. Sidebar → App への接続

`gwt-gui/src/lib/components/Sidebar.svelte` → `gwt-gui/src/App.svelte`:

- Sidebar props に `onNewTerminal` を追加
- App.svelte で `spawn_shell` コマンドを呼び出すハンドラを実装

### Phase 4: フロントエンド — AgentLaunchForm シェル選択

#### 4-1. OS 検出とシェル一覧取得

`gwt-gui/src/lib/components/AgentLaunchForm.svelte`:

- コンポーネント初期化時に `invoke("get_available_shells")` を呼び出し
- 結果が空配列（= macOS/Linux）の場合はシェル選択 UI を非表示

#### 4-2. Advanced Options にシェル選択ドロップダウン追加

- Advanced Options セクション内に `<select>` を追加
- Docker モード時は `disabled` 状態にし、"Container default" テキストを表示
- 選択値を `LaunchAgentRequest.terminal_shell` に設定

#### 4-3. シェル選択のデフォルト値

- `agentLaunchDefaults.ts` にシェル選択の永続化を追加
- Settings の `default_shell` をデフォルト値とし、フォーム上でオーバーライド可能

### Phase 5: フロントエンド — Settings Terminal タブ

#### 5-1. SettingsPanel にターミナルタブ追加

`gwt-gui/src/lib/components/SettingsPanel.svelte`:

- `SettingsTabId` に `"terminal"` を追加
- タブバーに "Terminal" ボタンを追加
- Windows 環境でのみ Terminal タブの内容（Default Shell セクション）を表示

#### 5-2. Default Shell セクション UI

- `get_available_shells` で取得したシェル一覧をラジオボタンまたはドロップダウンで表示
- 各シェルの表示名＋バージョンをサブテキストで表示
- 保存時に `SettingsData.default_shell` を更新

#### 5-3. SettingsData / Settings の双方向変換

- `SettingsData` に `default_shell: Option<String>` を追加
- `SettingsData::from(&Settings)` と `SettingsData::to_settings()` を更新

### Phase 6: cmd.exe 対応

#### 6-1. cmd.exe コマンドラッパー

`crates/gwt-core/src/terminal/pty.rs`:

- `resolve_spawn_command_for_platform()` に `shell` パラメータを追加
- `shell == "cmd"` の場合: `cmd.exe /C "command args..."` 形式で構築
- 初回はシンプルな `/C` 方式で実装。エスケープ問題が発生したら専用ビルダーを追加

## リスク

| ID | リスク | 影響 | 軽減策 |
|---|---|---|---|
| RISK-001 | `portable-pty` が `wsl.exe` を正常に PTY コマンドとして起動できない | WSL 機能が動作しない | ConPTY 経由の起動を検証、代替として `wsl.exe` を non-PTY で起動 |
| RISK-002 | WSL プロンプト検出が特殊なシェル設定で失敗 | エージェントコマンドが実行されない | 3秒タイムアウト後に非インタラクティブ方式（`wsl.exe -e bash -lc 'command'`）にフォールバック |
| RISK-003 | cmd.exe のエスケープ問題 | 特殊文字を含むコマンドが壊れる | 段階的対応（初回シンプル、問題発生時に専用ビルダー追加） |
| RISK-004 | WSL パス変換がネットワークドライブやシンボリックリンクで破綻 | 正しいディレクトリで起動できない | サポート範囲を明示し、エラーメッセージで案内 |

## 依存関係

- Phase 1 のデータモデルが全 Phase の基盤
- Phase 2 の `spawn_shell` 拡張が Phase 3 の New Terminal ボタンに必要
- Phase 2 の `LaunchAgentRequest` 拡張が Phase 4 の AgentLaunchForm に必要
- Phase 1 の `get_available_shells` が Phase 4, 5 のフロントエンドに必要
- Phase 5 は Phase 4 と並行可能

## マイルストーン

| マイルストーン | 内容 | 完了条件 |
|---|---|---|
| M1: データモデル | Settings + Shell 列挙 + 利用可能シェル API | FR-001, FR-002 |
| M2: PowerShell/cmd 切替 | spawn_shell と launch_agent で PS/cmd を選択可能 | SC-001（PS, cmd） |
| M3: WSL 基本動作 | WSL PTY 起動 + パス変換 + プロンプト検出 | SC-001（WSL）, SC-002 |
| M4: New Terminal ボタン | WorktreeSummaryPanel にボタン追加、全 OS 動作 | SC-005 |
| M5: AgentLaunchForm 統合 | Advanced Options にシェル選択 + Docker 連動 | SC-004, SC-006 |
| M6: Settings Terminal タブ | デフォルトシェルの永続化 | SC-003 |

## テスト

### バックエンド

- `WindowsShell` enum のユニットテスト（ID, 表示名, シリアライズ）
- `windows_to_wsl_path()` のユニットテスト（正常系、ドライブレター、UNC パス）
- `TerminalSettings` の config.toml 読み書きテスト
- `get_available_shells` の戻り値検証（モック環境）

### フロントエンド

- AgentLaunchForm のシェル選択ドロップダウン表示/非表示テスト
- Docker モード時の disabled 状態テスト
- SettingsPanel Terminal タブの表示テスト
- WorktreeSummaryPanel New Terminal ボタンのクリックテスト
