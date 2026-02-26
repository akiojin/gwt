# 実装計画: Host OS 起動時の空タブ防止（Issue #1029）

**仕様ID**: `SPEC-6e2a9d4c` | **日付**: 2026-02-13 | **仕様書**: `specs/SPEC-6e2a9d4c/spec.md`

## 目的

- Windows Host OS 起動時の空タブ（無表示・入力不能）を解消する。
- 起動失敗時の可観測性を上げ、必ずクローズ可能な状態にする。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-core`, `crates/gwt-tauri`）
- **フロントエンド**: Svelte 5（実装変更は原則不要）
- **対象経路**: `start_launch_job` -> `launch_agent_for_project_root` -> `launch_with_config` -> `stream_pty_output`
- **テスト**: `cargo test -p gwt-core`, `cargo test -p gwt-tauri`

## 実装方針

### Phase 1: Windows Host 実行の安定化

- `crates/gwt-core/src/terminal/pty.rs` に起動コマンド解決ヘルパーを追加する。
- Windows Host OS 起動時は一律 PowerShell ラップではなく、明示シェルとコマンド種別で起動経路を分岐する。
  - `powershell` 明示: PowerShell `-Command` ラップ
  - `.cmd/.bat`: `cmd.exe /C`
  - それ以外: 直接実行
- `crates/gwt-tauri/src/commands/terminal.rs` で Windows + Claude + Host runtime の shell auto 時は `cmd` を既定として補完する。
- Windows 以外は既存の直接起動を維持する。

### Phase 2: PTY read error 時の空タブ防止

- `crates/gwt-core/src/terminal/pane.rs` にエラー状態設定メソッドを追加し、`PaneStatus::Error` を運用経路で使う。
- `crates/gwt-tauri/src/commands/terminal.rs` の `stream_pty_output` で read error を捕捉し、
  - pane 状態を `Error` に更新
  - 失敗メッセージをスクロールバックに保存
  - `terminal-output` イベントで UI に通知
  - `Press Enter to close this tab.` を表示
  を実施する。

### Phase 3: テストと回帰確認

- `pty.rs` に Windows 起動経路分岐（PowerShell 明示 / cmd スクリプト / 直接実行）のユニットテストを追加する。
- `pane.rs` に `Error` 状態設定のユニットテストを追加する。
- `terminal.rs` に Claude 既定 shell 補完と Windows EOF 防御のユニットテストを追加する。
- 変更対象クレートのテストを実行し、回帰がないことを確認する。

## テスト

- `cargo test -q -p gwt-core terminal::pty`
- `cargo test -q -p gwt-core terminal::pane`
- `cargo test -q -p gwt-tauri commands::terminal`

### Phase 4: Backend-gated event emission（根本修正）

PTY出力とフロントエンドリスナーのレースコンディションを根本的に解消する。

- `crates/gwt-core/src/terminal/pane.rs` に `frontend_ready` フィールドとアクセサ、`read_scrollback_tail_raw` メソッドを追加する。
- `crates/gwt-tauri/src/commands/terminal.rs` の `stream_pty_output` メインreadループに ready gate を追加し、`frontend_ready` が false の間は `terminal-output` イベントを emit しない。
- `wsl_prompt_detect_and_inject` の reader スレッドにも同一の ready gate を適用する。
- `terminal_ready` Tauri コマンドを追加し、scrollback の raw bytes（ANSI 除去なし）を返しつつ `frontend_ready` を true に設定する。
- `crates/gwt-tauri/src/app.rs` の invoke_handler に `terminal_ready` を登録する。
- `gwt-gui/src/lib/terminal/TerminalView.svelte` の onMount を簡素化し、バッファリング機構を削除して `terminal_ready` ベースのフローに置き換える。
- テスト更新: `TerminalView.test.ts` と `pane.rs` のテストを新しいフローに合わせる。

## テスト

- `cargo test -q -p gwt-core terminal::pty`
- `cargo test -q -p gwt-core terminal::pane`
- `cargo test -q -p gwt-tauri commands::terminal`
- `cd gwt-gui && pnpm test src/lib/terminal/TerminalView.test.ts`

## 非目標

- Launch UI のデザイン変更
- Docker 実行ロジックの仕様変更
