# タスクリスト: Windows シェル選択（Launch Agent / New Terminal）

## ストーリー間依存関係

```text
US4（シェル検出）─→ US1（Launch Agent選択）─→ US5（Docker無効化）
       │                    │
       ├─→ US3（Settings）  ├─→ US7（WSL環境変数）
       │                    │
       └─→ US2（New Terminal）
                    │
                    └─→ US6（WSLパス変換）
```

- US4（シェル検出）は全ストーリーの前提（Phase 2 で実装）
- US6（WSL パス変換）は US1, US2 の WSL 対応で使用（Phase 2 で実装）
- US1 と US2 は US4 完了後に並行可能
- US5 は US1 の AgentLaunchForm 内で対応（小規模）
- US7 は US1 の WSL 統合で対応

## Phase 1: セットアップ

- [x] T001 [P] `crates/gwt-core/src/terminal/shell.rs` を新規作成し、`crates/gwt-core/src/terminal/mod.rs` に `pub mod shell;` を追加する `crates/gwt-core/src/terminal/shell.rs` `crates/gwt-core/src/terminal/mod.rs`

## Phase 2: 基盤（US4 シェル検出 + US6 WSL パス変換）

- [x] T002 [US4] `WindowsShell` enum（`PowerShell`, `Cmd`, `Wsl`）を定義する。`id() -> &str` と `display_name() -> &str` メソッドを実装する。Serialize/Deserialize を derive する `crates/gwt-core/src/terminal/shell.rs`
- [x] T003 [US4] `WindowsShell` の `id()` と `display_name()` のユニットテストを作成する（TDD: RED）。テストケース: (1) PowerShell の id は "powershell" (2) Cmd の id は "cmd" (3) Wsl の id は "wsl" (4) 各 display_name の検証 (5) シリアライズ/デシリアライズの往復 `crates/gwt-core/src/terminal/shell.rs`
- [x] T004 [US4] `WindowsShell::is_available()` メソッドを実装する。PowerShell: `which::which("pwsh")` or `which::which("powershell")`。Cmd: `which::which("cmd")`。WSL: `which::which("wsl")` && `wsl --list --quiet` が空でないこと `crates/gwt-core/src/terminal/shell.rs`
- [x] T005 [US4] `WindowsShell::detect_version()` メソッドを実装する。PowerShell のみ: `pwsh --version` または `powershell -Command "$PSVersionTable.PSVersion.ToString()"` を実行しバージョン文字列を返す。Cmd/Wsl は None `crates/gwt-core/src/terminal/shell.rs`
- [x] T006 [US6] `windows_to_wsl_path()` 関数のユニットテストを作成する（TDD: RED）。テストケース: (1) `C:\Users\foo` → `/mnt/c/Users/foo` (2) `D:\projects\repo` → `/mnt/d/projects/repo` (3) 小文字ドライブレター `c:\foo` (4) UNC パス `\\server\share` でエラー (5) 既に `/mnt/` 形式ならそのまま返す `crates/gwt-core/src/terminal/shell.rs`
- [x] T007 [US6] `windows_to_wsl_path(win_path: &str) -> Result<String>` 関数を実装する。ドライブレター小文字化、バックスラッシュ→スラッシュ変換のバイト操作。UNC パスはエラー。T006 のテストが GREEN になることを確認 `crates/gwt-core/src/terminal/shell.rs`
- [x] T008 [US4] `TerminalSettings` 構造体（`default_shell: Option<String>`）を追加する。`Default` trait を実装（`default_shell: None`）。`Settings` に `pub terminal: TerminalSettings` フィールドを追加する `crates/gwt-core/src/config/settings.rs`
- [x] T009 [US4] `TerminalSettings` の config.toml 読み書きテストを作成する（TDD: RED → GREEN）。テストケース: (1) `[terminal]` セクション未設定時に `default_shell` が None (2) `default_shell = "wsl"` の保存と読み込み (3) 既存設定との後方互換性 `crates/gwt-core/src/config/settings.rs`
- [x] T010 [US4] `ShellInfo` 構造体（`id: String, name: String, version: Option<String>`）を定義し、`get_available_shells` Tauri コマンドを実装する。Windows: `WindowsShell` の全バリアントを `is_available()` でフィルタ。macOS/Linux: 空配列を返す `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T011 [US4] `get_available_shells` を `generate_handler!` マクロに登録する `crates/gwt-tauri/src/commands/mod.rs`
- [x] T012 [P] [US4] `gwt-gui/src/lib/types.ts` に `ShellInfo` インターフェース（`id: string, name: string, version?: string`）を追加する。`SettingsData` に `default_shell?: string | null` を追加する。`LaunchAgentRequest` に `terminal_shell?: string` を追加する `gwt-gui/src/lib/types.ts`

## Phase 3: US3 Settings Terminal タブ

- [x] T013 [US3] `SettingsData` に `default_shell: Option<String>` フィールドを追加し、`SettingsData::from(&Settings)` と `SettingsData::to_settings()` の双方向変換を更新する `crates/gwt-tauri/src/commands/settings.rs`
- [x] T014 [US3] SettingsPanel の `SettingsTabId` に `"terminal"` を追加し、タブバーに "Terminal" ボタンを追加する `gwt-gui/src/lib/components/SettingsPanel.svelte`
- [x] T015 [US3] Terminal タブの内容を実装する。`invoke("get_available_shells")` で取得したシェル一覧をドロップダウンで表示し、各シェルの名前＋バージョンをサブテキスト表示する。選択値を `SettingsData.default_shell` に保存する `gwt-gui/src/lib/components/SettingsPanel.svelte`
- [x] T016 [US3] Terminal タブを Windows 環境のみで表示する。`availableShells` が空（macOS/Linux）の場合はタブ自体を非表示にする `gwt-gui/src/lib/components/SettingsPanel.svelte`
- [x] T017 [US3] SettingsPanel Terminal タブのユニットテストを作成する（TDD: RED → GREEN）。テストケース: (1) availableShells が空の場合 Terminal タブが非表示 (2) availableShells がある場合 Terminal タブが表示 (3) Default Shell 変更と保存 `gwt-gui/src/lib/components/SettingsPanel.test.ts`

## Phase 4: US2 New Terminal ボタン

- [x] T018 [US2] `spawn_shell` コマンドに `shell: Option<String>` 引数を追加する。`shell` 未指定時は Settings の `default_shell` → `resolve_shell_launch_spec()` フォールバック。`"wsl"` 時: `wsl.exe` を PtyConfig コマンドに設定し `working_dir` を `windows_to_wsl_path()` で変換。`"cmd"` 時: `cmd.exe` を使用。`"powershell"` 時: 既存の `resolve_windows_shell()` を使用 `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T019 [US2] WorktreeSummaryPanel のユニットテストを作成する（TDD: RED）。テストケース: (1) New Terminal ボタン（`>_`）が表示される (2) クリックで `onNewTerminal` コールバックが呼ばれる `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`
- [x] T020 [US2] WorktreeSummaryPanel に `onNewTerminal` コールバック props を追加し、Launch Agent ボタンの横にターミナルアイコンボタン（`>_`）を配置する。クリック時に `onNewTerminal()` を呼び出す `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`
- [x] T021 [US2] Sidebar.svelte に `onNewTerminal` props を追加し、WorktreeSummaryPanel に接続する `gwt-gui/src/lib/components/Sidebar.svelte`
- [x] T022 [US2] App.svelte で `handleNewTerminal` ハンドラを実装する。`invoke("spawn_shell", { workingDir, shell: null })` を呼び出し、ターミナルタブを作成する。Sidebar に `onNewTerminal` を接続する `gwt-gui/src/App.svelte`

## Phase 5: US1 Launch Agent シェル選択

- [x] T023 [US1] `LaunchAgentRequest` に `terminal_shell: Option<String>` フィールドを追加する（`#[serde(default)]`） `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T024 [US1] `launch_agent_for_project_root()` で `terminal_shell` が指定されている場合のシェル切替ロジックを実装する。`"powershell"`: 既存ロジック。`"cmd"`: `cmd.exe /C` ラッパー方式。`"wsl"`: Phase 6 で実装（この段階では TODO コメント） `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T025 [US1] `resolve_spawn_command_for_platform()` に `shell: Option<&str>` パラメータを追加する。`shell == Some("cmd")` の場合: `cmd.exe /C "command args..."` 形式で構築する `crates/gwt-core/src/terminal/pty.rs`
- [x] T026 [US1] AgentLaunchForm のシェル選択テストを作成する（TDD: RED）。テストケース: (1) availableShells が空の場合ドロップダウン非表示 (2) availableShells がある場合ドロップダウン表示 (3) Docker モード時に disabled `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T027 [US1] AgentLaunchForm の初期化時に `invoke("get_available_shells")` を呼び出し、結果を `availableShells` state に保存する `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T028 [US1] [US5] Advanced Options セクションにシェル選択ドロップダウンを追加する。`availableShells` が空の場合は非表示。Docker モード時は disabled にし "Container default" テキストを表示する `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T029 [US1] `handleLaunch()` で選択されたシェルを `LaunchAgentRequest.terminal_shell` に設定する `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T030 [US1] `agentLaunchDefaults.ts` の `LaunchDefaults` 型に `selectedShell: string` を追加し、`loadLaunchDefaults` / `saveLaunchDefaults` を更新する `gwt-gui/src/lib/agentLaunchDefaults.ts`

## Phase 6: WSL PTY 統合（US1 WSL + US7 環境変数）

- [x] T031 [US1] WSL エージェント起動の PTY 書き込み方式を実装する。(1) `wsl.exe` を PTY コマンドとして起動（`--cd /mnt/c/...` で作業ディレクトリ指定） (2) PTY 出力を監視してプロンプトパターン（行末 `$`, `#`, `>` + 空白）を検出 (3) 検出後にエージェントコマンド文字列を PTY に書き込み `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T032 [US1] WSL プロンプト検出のタイムアウト処理を実装する。3秒以内にプロンプトが検出できない場合、非インタラクティブ方式（`wsl.exe -e bash -lc 'command'`）にフォールバックする `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T033 [US7] WSL 起動時の環境変数ハイブリッド処理を実装する。ベース環境変数は WSL のログインシェルに委任し、GWT Profile の `env_overrides` のみ PTY 書き込みで `export KEY=VALUE` を先行投入する `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T034 [US1] T024 の WSL TODO を解消し、WSL シェル指定時に T031-T032 の PTY 書き込み方式を呼び出すよう統合する `crates/gwt-tauri/src/commands/terminal.rs`

## Phase 7: 仕上げ・横断

- [x] T035 [P] [共通] `specs/specs.md` の現行仕様テーブルに SPEC-1350124a のステータスを「実装中」に更新する `specs/specs.md`
- [x] T036 [P] [共通] `cargo clippy --all-targets --all-features -- -D warnings` でバックエンド全体の lint を通す
- [x] T037 [P] [共通] `cargo fmt --check` でフォーマットを検証する
- [x] T038 [P] [共通] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` でフロントエンドの型チェックを通す
- [x] T039 [共通] `cargo test` で全バックエンドテストを通す
- [x] T040 [共通] `cd gwt-gui && pnpm test` で全フロントエンドテストを通す
