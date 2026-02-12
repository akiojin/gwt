# タスクリスト: GUI Session Summary のスクロールバック要約（実行中対応）

## Phase 1: バックエンド（Tauri）

- [ ] T1: `crates/gwt-tauri/src/commands/sessions.rs` に scrollback fallback を追加
- [ ] T2: latest pane 選定ヘルパと `ScrollbackSummaryJob` を実装
- [ ] T3: `session-summary-updated` で `pane:` 擬似session_idを通知

## Phase 2: フロントエンド（Svelte）

- [ ] T4: `gwt-gui/src/lib/components/MainArea.svelte` の表示を `Live (pane summary)` に変更

## Phase 3: テスト

- [ ] T5: Rust ユニットテスト追加（latest pane 選定）
- [ ] T6: Rust ユニットテスト追加（scrollback fallback job 生成）
