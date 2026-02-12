# 実装計画: GUI Session Summary のスクロールバック要約（実行中対応）

**仕様ID**: `SPEC-3a1b7c2d` | **日付**: 2026-02-12 | **仕様書**: `specs/SPEC-3a1b7c2d/spec.md`

## 目的

- session_id 未保存でも Summary を表示できるようにする
- 同一Worktreeの複数paneから最新出力のpaneを選択する
- session_id 確定後は既存フローへ戻す

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **要約生成**: `gwt-core::ai::summarize_scrollback` を使用

## 実装方針

### Phase 1: バックエンド（sessions command 拡張）

- `get_branch_session_summary` に **スクロールバック fallback** を追加
  - session_id が無い場合、起動中paneの scrollback を要約入力にする
  - 取得対象は **最終出力が最新のpane**
- `ScrollbackSummaryJob` を追加して非同期生成
  - `session-summary-updated` イベントに `pane:` の擬似session_idを付与
- キャッシュは `SessionSummaryCache` を再利用

### Phase 2: フロントエンド（表示ラベル）

- `sessionId` が `pane:` の場合は `Live (pane summary)` 表示に切り替える

### Phase 3: テスト

- Rust: 最新pane選定ロジックのユニットテストを追加
- Rust: scrollback fallback が job を返すことを検証

## テスト

### Rust（gwt-tauri）

- latest pane 選定のユニットテスト
- scrollback fallback の job 生成テスト
