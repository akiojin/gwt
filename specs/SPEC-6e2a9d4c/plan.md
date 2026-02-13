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
- Windows かつ起動コマンドが `.cmd` / `.bat` の場合は `cmd.exe /d /s /c` ラップに変換する。
- それ以外は既存の直接起動を維持する。

### Phase 2: PTY read error 時の空タブ防止

- `crates/gwt-core/src/terminal/pane.rs` にエラー状態設定メソッドを追加し、`PaneStatus::Error` を運用経路で使う。
- `crates/gwt-tauri/src/commands/terminal.rs` の `stream_pty_output` で read error を捕捉し、
  - pane 状態を `Error` に更新
  - 失敗メッセージをスクロールバックに保存
  - `terminal-output` イベントで UI に通知
  - `Press Enter to close this tab.` を表示
  を実施する。

### Phase 3: テストと回帰確認

- `pty.rs` にラップ変換のユニットテストを追加する。
- `pane.rs` に `Error` 状態設定のユニットテストを追加する。
- 変更対象クレートのテストを実行し、回帰がないことを確認する。

## テスト

- `cargo test -q -p gwt-core terminal::pty`
- `cargo test -q -p gwt-core terminal::pane`
- `cargo test -q -p gwt-tauri commands::terminal`

## 非目標

- Launch UI のデザイン変更
- Docker 実行ロジックの仕様変更
