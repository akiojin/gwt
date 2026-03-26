# Agent Canvas メモタイル — tasks.md

## Phase 1: タイル型定義 + 基本レンダリング (US-1, US-2)

### 1.1 型定義の拡張

- [ ] **[TEST]** `gwt-gui/src/lib/agentCanvas.test.ts` に memo カード型のテストを追加（AgentCanvasMemoCard の構造検証、cards 配列への memo 追加）
- [ ] `gwt-gui/src/lib/agentCanvas.ts` に `AgentCanvasMemoCard` 型と `AgentCanvasCardType` への `"memo"` 追加
- [ ] `AgentCanvasCard` union に `AgentCanvasMemoCard` を追加
- [ ] `AgentCanvasGraph` に `memoCards: AgentCanvasMemoCard[]` を追加
- [ ] `buildAgentCanvasState` で memo カードを cards 配列に含める

### 1.2 MemoCardContent コンポーネント

- [ ] **[TEST]** `gwt-gui/src/lib/components/MemoCardContent.test.ts` を作成（編集モード表示、プレビューモード表示、トグル切り替え、ダブルクリックで編集復帰）
- [ ] `gwt-gui/src/lib/components/MemoCardContent.svelte` を新規作成
  - 編集モード: textarea + プレビュー切り替えボタン
  - プレビューモード: MarkdownRenderer + 編集切り替えボタン
  - ダブルクリックで編集モードに戻る

### 1.3 AgentCanvasPanel への memo レンダリング統合

- [ ] **[TEST]** `gwt-gui/src/lib/components/AgentCanvasPanel.test.ts` に memo カードレンダリングのテスト追加
- [ ] `AgentCanvasPanel.svelte` の `RenderableCanvasCard` / `CanvasCardKind` に memo を追加
- [ ] memo カード用のレンダリングブランチ（MemoCardContent を使用）を追加

## Phase 2: Rust バックエンド永続化 (US-3)

### 2.1 gwt-core memo モジュール

- [ ] **[TEST]** `crates/gwt-core/tests/memo_test.rs`（または `#[cfg(test)]` モジュール）を作成（create / read / update / delete / list の各操作テスト）
- [ ] `crates/gwt-core/src/memo.rs` モジュールを新規作成
  - `MemoEntry` 構造体（id, title, content, created_at, updated_at）
  - `MemoIndex` 構造体（メモ ID リスト）
  - project_hash 算出（プロジェクトパスの SHA-256 先頭 16 文字）
  - CRUD 関数: `create_memo`, `read_memo`, `update_memo`, `delete_memo`, `list_memos`
  - 保存先: `~/.gwt/memos/{project_hash}/{memo_id}.md`、インデックス: `index.json`
- [ ] `crates/gwt-core/src/lib.rs` に `pub mod memo;` を追加

### 2.2 gwt-tauri Tauri コマンド [P]

- [ ] **[TEST]** Tauri コマンドの単体テスト（正常系・エラー系）
- [ ] `crates/gwt-tauri/src/commands/` に memo コマンドを追加（`memo_create`, `memo_read`, `memo_update`, `memo_delete`, `memo_list`）
- [ ] Tauri アプリのコマンドハンドラ登録

### 2.3 フロントエンド Tauri invoke 連携 [P]

- [ ] **[TEST]** `gwt-gui/src/lib/memoApi.test.ts` を作成（invoke ラッパーのテスト）
- [ ] `gwt-gui/src/lib/memoApi.ts` を新規作成（Tauri invoke ラッパー: createMemo, readMemo, updateMemo, deleteMemo, listMemos）

## Phase 3: Canvas 統合 + UI (US-1, US-3, US-5)

### 3.1 メモカード作成 UI

- [ ] **[TEST]** 「Add Memo」ボタン / コンテキストメニュー表示のテスト
- [ ] Canvas ツールバーに「Add Memo」ボタンを追加
- [ ] クリック時に memo_create → 新しい memo カードをデフォルト位置・サイズ（320x240）で生成 → 即編集モード

### 3.2 メモカード削除 UI

- [ ] **[TEST]** 削除ボタン表示・確認ダイアログ表示のテスト
- [ ] memo カードヘッダーに削除ボタンを追加
- [ ] 内容がある場合は確認ダイアログを表示
- [ ] 削除時に memo_delete + tileLayouts からの除去

### 3.3 起動時の復元

- [ ] **[TEST]** アプリ起動時の memo カード復元テスト
- [ ] アプリ起動時に memo_list → AgentCanvasState の cards に memo カードを追加
- [ ] tileLayouts から位置・サイズを復元

## Phase 3.5: Relation Edge (US-6)

### 3.5.1 手動 edge 接続・切断

- [ ] **[TEST]** メモ-worktree 手動 edge 接続・切断のユニットテスト
- [ ] メモタイルのコンテキストメニューに「Link to worktree」操作を追加
- [ ] relation edge の描画（既存の worktree→session edge と同じ仕組みを再利用）
- [ ] edge 切断操作の実装

### 3.5.2 worktree 削除時のメモ保持

- [ ] **[TEST]** worktree 削除時にメモタイルが独立メモとして残るテスト
- [ ] worktree 削除時のメモ保持ロジック

### 3.5.3 edge 永続化

- [ ] **[TEST]** edge 状態の永続化・復元テスト
- [ ] edge 状態を tileLayouts 永続化データに含める

## Phase 4: UX ポリッシュ (US-2, US-4)

### 4.1 自動保存

- [ ] **[TEST]** デバウンス保存のテスト（500ms 以内の連続入力で 1 回だけ保存）
- [ ] 編集時にデバウンス（500ms）付き自動保存を実装

### 4.2 リサイズ

- [ ] **[TEST]** memo カードのリサイズテスト（サイズ変更 + 永続化）
- [ ] memo カード用のデフォルトサイズ定数（MEMO_CARD_WIDTH = 320, MEMO_CARD_HEIGHT = 240）を追加
- [ ] 既存のリサイズ機構を memo カードに適用

### 4.3 キーボード・インタラクション

- [ ] Escape キーで編集モードを終了しプレビューモードに切り替え
- [ ] プレビューモードでダブルクリック → 編集モード

## 完了チェックリスト

- [ ] 全テスト通過（`cargo test` + `cd gwt-gui && pnpm test`）
- [ ] Lint 通過（`cargo clippy` + `svelte-check`）
- [ ] フォーマット通過（`cargo fmt --check`）
- [ ] spec.md の全 Success Criteria を満たす
- [ ] gwt-spec Issue #1767 のステータスを更新

## Notes

- `[P]` は並列実行可能なタスクを示す
- `[TEST]` はテストファーストで先に作成するタスクを示す
- Phase 1 と Phase 2 は一部並列実行可能（型定義は Phase 1、Rust バックエンドは Phase 2）
