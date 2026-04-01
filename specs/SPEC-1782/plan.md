# Plan: SPEC-1782 — Quick Start ワンクリックエージェント起動

## Summary

gwt-tui の Quick Start 機能を再設計・実装する。ブランチ単位で前回のエージェント設定と session_id を記憶し、ワンクリックでセッション再開（`--resume <id>`）または新規起動（前回設定の Normal モード）を可能にする。

gwt-cli から `detect_session_id_for_tool()` を gwt-core に移植し、gwt-tui の Wizard / app.rs 統合レイヤーを接続する。

## Technical Context

### 影響ファイル

| ファイル | 変更内容 |
|---------|---------|
| `crates/gwt-core/src/ai/session_detect.rs` | **新規** — detect_session_id_for_tool() 移植 |
| `crates/gwt-core/src/ai/mod.rs` | session_detect モジュール export 追加 |
| `crates/gwt-tui/src/screens/wizard.rs` | Quick Start 仕様変更、ExecutionMode ステップ削除 |
| `crates/gwt-tui/src/app.rs` | 履歴読み込み、SessionMode ブリッジ、session_id 検出・保存 |

### 既存資産（再利用）

| 資産 | 場所 | 用途 |
|------|------|------|
| `encode_claude_project_path()` | `gwt-core/src/ai/claude_paths.rs` | Claude のプロジェクトパスエンコード |
| `get_branch_tool_history()` | `gwt-core/src/config/ts_session.rs` | ブランチ別ツール履歴取得 |
| `save_session_entry()` | `gwt-core/src/config/ts_session.rs` | セッション履歴保存 |
| `AgentLaunchBuilder.session_mode()` | `gwt-core/src/agent/launch.rs` | SessionMode 設定 |
| `AgentLaunchBuilder.resume_session_id()` | `gwt-core/src/agent/launch.rs` | Resume session ID 設定 |
| `find_agent_def()` | `gwt-core/src/agent/launch.rs` | エージェント定義取得 |

### 前提

- gwt-cli の `detect_session_id_for_tool()` は gwt-cli にのみ存在し gwt-core に未移植
- gwt-tui には SessionWatcher 統合がないが、本 SPEC では SessionWatcher は使用しない（直接ファイルスキャン方式）
- `--continue` フラグは使用しない（`--resume <id>` のみ）

## Constitution Check

| Rule | Status | Notes |
|------|--------|-------|
| 1. Spec Before Implementation | OK | SPEC-1782 spec.md 策定済み |
| 2. Test-First Delivery | OK | 各フェーズで TDD を実施 |
| 3. No Workaround-First Changes | OK | 根本原因（統合レイヤー未接続）を修正 |
| 4. Minimal Complexity | OK | 既存 API（AgentLaunchBuilder, get_branch_tool_history 等）を再利用 |
| 5. Verifiable Completion | OK | 自動テスト + 手動検証で SC-001〜SC-006 を確認 |
| 6. SPEC vs Issue Separation | OK | 機能仕様として SPEC-1782 で管理 |

## Project Structure

```
crates/gwt-core/src/ai/
  session_detect.rs    ← 新規: detect_session_id_for_tool()
  claude_paths.rs      ← 既存: encode_claude_project_path() を再利用
  mod.rs               ← export 追加

crates/gwt-tui/src/
  screens/wizard.rs    ← Quick Start 再設計、ExecutionMode 削除
  app.rs               ← 履歴読み込み、SessionMode ブリッジ、session_id 保存
```

## Complexity Tracking

| 追加要素 | 理由 |
|---------|------|
| `session_detect.rs` 新規モジュール | gwt-cli からの移植。各エージェントのセッションファイル形式が異なるため独立モジュールが必要 |
| バックグラウンド session_id 検出 | エージェント起動後に検出するため非同期処理が必要。ただし既存の PTY reader スレッドパターンを踏襲 |

## Phased Implementation

### Phase 1: session_id 検出基盤 (gwt-core)

**目的**: gwt-cli の `detect_session_id_for_tool()` を gwt-core に移植する

1. `crates/gwt-core/src/ai/session_detect.rs` を新規作成
2. Claude / Codex / Gemini / OpenCode の session_id 検出を実装
3. `crates/gwt-core/src/ai/mod.rs` から pub export
4. テスト: 各エージェントのセッションファイル形式に対するユニットテスト

### Phase 2: Quick Start 仕様変更 (gwt-tui Wizard)

**目的**: Wizard の Quick Start ステップを再設計し、ExecutionMode ステップを削除する

1. `wizard.rs`: Quick Start の表示条件変更（session_id がある 1 ツールのみ）
2. `wizard.rs`: フラット 3 項目 UI（Resume / Start New / Choose Different）
3. `wizard.rs`: apply_quick_start_selection() の全設定復元
4. `wizard.rs`: Resume / Start New でワンクリック起動（WizardAction::Complete）
5. `wizard.rs`: ExecutionMode ステップ削除、next_step() / prev_step() 更新
6. `wizard.rs`: WizardExecutionMode::Continue 削除

### Phase 3: 統合レイヤー (gwt-tui app.rs)

**目的**: Wizard と gwt-core を正しく接続する

1. `app.rs`: load_quick_start_history() — session_id がある最新ツールをフィルタ
2. `app.rs`: spawn_agent_session() の SessionMode ブリッジ
3. `app.rs`: save_session_entry() の改善（tool_label, session_id, mode, reasoning_level）
4. `app.rs`: エージェント起動後の session_id 検出・保存（バックグラウンド）

### Phase 4: 統合検証

**目的**: 全変更の整合性を検証する

1. `cargo build -p gwt-tui` / `cargo test` / `cargo clippy`
2. 手動 E2E: Quick Start 表示 → Resume → Start New → Choose Different
3. 手動 E2E: session_id 検出フロー確認
