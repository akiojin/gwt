> **ℹ️ TUI MIGRATION NOTE**: This SPEC was completed during the gwt-tauri era. The gwt-tauri frontend has been replaced by gwt-tui (SPEC-1776). GUI-specific references are historical.

### 背景

Issue一覧パネル (`IssueListPanel.svelte`) の無限スクロールで「Loading more」状態でUIが固まる。
アニメーションは動くがタブ変更等の操作ができなくなる = Tauri IPC スレッドがブロックされている。

根本原因は4つ:
1. 同期コマンドが IPC スレッドをブロック（主因）
2. O(n^2) バックエンドページネーション
3. ブランチリンク検索がローディング状態をブロック
4. IntersectionObserver が再発火しない

### ユーザーシナリオ

- **US1 (P0)**: Issue一覧で下スクロールして「Loading more」が表示された際、タブ変更やボタンクリック等のUI操作が即座にレスポンスする
- **US2 (P1)**: 3ページ以上の連続スクロールで、ページ3のロード時間がページ1と同等
- **US3 (P1)**: Loading more 完了後にセンチネルがまだ可視領域にある場合、自動的に次ページがロードされる
- **US4 (P1)**: loadingMore フラグが Issue データ取得直後に false になる（ブランチリンク検索完了前に）

### 機能要件

- **FR-001**: Issue 関連 Tauri コマンド（fetch_github_issues, find_existing_issue_branches_bulk, fetch_github_issue_detail, fetch_branch_linked_issue）が IPC スレッドをブロックしない（async + spawn_blocking 化）
- **FR-002**: ブランチリンク検索（loadBranchLinks）が loadingMore 状態をブロックしない（fire-and-forget）
- **FR-003**: IntersectionObserver がロード完了後も正しく再発火する（root をスクロールコンテナに設定 + 手動可視性チェック）
- **FR-004**: ページネーションが O(1)/ページで動作する（GitHub REST Search API 使用）

### 成功基準

- **SC-001**: Loading more 実行中でもタブ変更等の UI 操作が即座にレスポンスする
- **SC-002**: 複数ページの連続ロードが途中で止まらない
- **SC-003**: 全既存テストが通過する
