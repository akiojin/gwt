# 実装計画: gwt GUI コーディングエージェント機能のTUI完全移行（Quick Start / Mode / Skip / Reasoning / Version）

**仕様ID**: `SPEC-90217e33` | **日付**: 2026-02-09 | **仕様書**: `specs/SPEC-90217e33/spec.md`

## 概要

本実装では、GUI 版のエージェント起動を TUI と同等まで引き上げる。

- 起動オプション（Mode/Skip/Reasoning/Version/Extra Args/Env overrides）を GUI 起動ウィザードに追加する
- Quick Start を Summary へ追加し、ブランチごとの前回設定で Continue/New を即時実行できるようにする
- ブランチ一覧へ直近利用ツール（例: `Codex@latest`）を表示し、ツールごとの色で識別可能にする
- OpenCode をビルトインとして検出・起動できるようにする
- 履歴（ts_session）への記録と、終了後の sessionId 検出を追加する（ベストエフォート、失敗しても起動フローをブロックしない）

## 技術コンテキスト

- **GUI**: Tauri v2 + Svelte 5 + Vite（`gwt-gui/`）
- **Backend**: Rust（`crates/gwt-tauri/`）
- **Core**: Rust（`crates/gwt-core/`）
- **Terminal**: portable-pty + xterm.js
- **履歴**: `gwt-core` の TypeScript互換セッション（`crates/gwt-core/src/config/ts_session.rs`）
- **セッション解析**: `gwt-core` の session_parser（`crates/gwt-core/src/ai/session_parser/`）

## 原則チェック

- Spec-first/TDD を遵守（仕様 → テスト → 実装）
- GUI 表示文言は英語のみ
- 設定/履歴ファイル読み込み時にディスク副作用を書かない（Save/Launch 時のみ書き込み）
- 既存コードの改修を優先し、必要最小限の新規追加に留める

## 実装方針（決定事項）

### 1) DTO と起動引数組み立て（Backend）

- `launch_agent` DTO を拡張し、Mode/Skip/Reasoning/Version/Extra Args/Env overrides/ResumeSessionId を受け取る
- エージェント別の引数を組み立てる（Codex は `gwt_core::agent::codex::*` の既存ヘルパーを優先利用）
- `installed`/`bunx|npx` の切替を `auto`（既存）に加えて明示指定できるようにする

### 2) Quick Start と履歴（Backend + Frontend）

- `get_branch_quick_start(projectPath, branch)` を追加し、`ts_session::get_branch_tool_history` を返す（UI 表示用に必要最小限へ整形）
- `launch_agent` 実行時に `save_session_entry` で履歴を追記する
- 終了後に `session_parser` を使って sessionId をベストエフォートで検出し、成功時のみ履歴へ追記する

### 3) ブランチ一覧の直近ツール表示（Frontend）

- `BranchInfo` に `last_tool_usage` を追加し、Sidebar で右寄せ表示する
- 色分けは toolId/label から決定する（Claude: yellow / Codex: cyan / Gemini: magenta / OpenCode: green）

### 4) UI（AgentLaunchForm / Summary）

- 起動ウィザードに Session Mode/Skip/Reasoning/Runner/Version/Extra Args/Env overrides を追加する
- Summary に Quick Start を追加し、履歴がある場合はウィザードを開かずに Continue/New を実行できるようにする

## テスト戦略

- Rust:
  - `launch_agent` の引数組み立てヘルパーをユニットテスト
  - sessionId 検出の補助関数（Claude path encoding など）をユニットテスト
- Frontend:
  - `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json`
- 最終ゲート:
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`

## リスクと緩和策

- **CLI フラグ互換性**: エージェント CLI の引数が将来変化する可能性
  - **緩和策**: 既存の version gate ヘルパー（Codex）を利用し、未知はベストエフォートで起動（失敗時は UI にエラー表示）
- **sessionId 検出の精度**: 複数セッションが並行更新されると誤検出の可能性
  - **緩和策**: 起動開始時刻以降で最も新しいセッションを優先する（可能な範囲で）
