# TODO: Issue一覧の無限スクロール "Loading more" フリーズ修正

## 背景

Issue一覧パネルの無限スクロールで「Loading more」中にUIがフリーズする。
根本原因: 同期 Tauri コマンドが IPC スレッドをブロック + O(n^2) ページネーション + ブランチリンク検索のブロック + IntersectionObserver 再発火不良。

## 実装ステップ

- [x] T001 GitHub Issue 仕様策定（gwt-spec ラベル）→ #1408
- [x] T002 TDD テスト作成（RED 確認）
  - [x] T002a Rust: Search API エンドポイント生成テスト（4件）
  - [x] T002b Rust: Search API レスポンスパーステスト（7件）
  - [x] T002c Frontend: 無限スクロール継続ロードテスト（2件）
- [x] T003 Fix 1: Issue コマンドの async 化（IPC ブロック解消）
- [x] T004 Fix 2+3: フロントエンド改善（非同期ブランチリンク + IO 修正）
- [x] T005 Fix 4: O(1) ページネーション（REST Search API）
- [x] T006 全テスト GREEN 確認 + lint + 型チェック

## 検証結果

- [x] `cargo test -p gwt-core --lib` — 1422 tests passed（新規11件含む）
- [x] `cargo test -p gwt-tauri` — 32 tests passed
- [x] `cargo clippy --all-targets --all-features -- -D warnings` — 警告なし
- [x] `cd gwt-gui && pnpm test` — 34 tests passed（新規2件含む）
- [x] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` — 0 errors
