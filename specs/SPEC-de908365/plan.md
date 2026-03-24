### 技術コンテキスト

| ファイル | 変更内容 |
|---|---|
| `crates/gwt-tauri/src/commands/issue.rs` | Fix 1: async化、spawn_blocking |
| `crates/gwt-core/src/git/issue.rs` | Fix 4: Search API ページネーション + パーサー |
| `gwt-gui/src/lib/components/IssueListPanel.svelte` | Fix 2+3: 非同期ブランチリンク、IO修正 |
| `gwt-gui/src/lib/components/IssueListPanel.test.ts` | 無限スクロール継続ロードのテスト追加 |

### 実装アプローチ

1. **Fix 1**: PR コマンド（pullrequest.rs）で既に使われている `tauri::async_runtime::spawn_blocking` パターンを Issue コマンドに適用
2. **Fix 2**: `fetchIssues` 内の `await loadBranchLinks()` を `void loadBranchLinks()` に変更（fire-and-forget）
3. **Fix 3**: IntersectionObserver の root をスクロールコンテナに設定 + ロード完了後のセンチネル可視性チェック
4. **Fix 4**: `gh issue list --limit (per_page*page+1)` を `gh api search/issues` に置換
