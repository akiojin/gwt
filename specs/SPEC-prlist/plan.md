# 実装計画: PR Dashboard

**仕様ID**: SPEC-prlist

## 実装フェーズ

### Phase 1: Backend - データ取得・アクション

1. `gh_cli.rs` に `fetch_pr_list()`, `fetch_authenticated_user()`, `merge_pr()`, `review_pr()`, `mark_pr_ready()` を追加
2. `pullrequest.rs` (gwt-core) に `PrListItem` 型を追加
3. `pullrequest.rs` (gwt-tauri/commands) に対応 Tauri コマンドを追加
4. `app.rs` の `invoke_handler` にコマンド登録

### Phase 2: Frontend - 型定義・メニュー・タブルーティング

1. `types.ts` に `PrListItem` 型追加、`Tab.type` に `"prs"` 追加
2. `menu.rs` に `MENU_ID_GIT_PULL_REQUESTS` 追加
3. `App.svelte` に `openPullRequestsTab()` と menu handler 追加
4. `MainArea.svelte` に `"prs"` タブ対応追加

### Phase 3: Frontend - メインコンポーネント

1. `PrListPanel.svelte` 新規作成（IssueListPanel パターン踏襲）
2. `MergeDialog.svelte` 新規作成
3. `ReviewDialog.svelte` 新規作成

### Phase 4: 検証

1. `cargo clippy --all-targets --all-features -- -D warnings`
2. `cargo fmt`
3. `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json`
4. `cargo test`
5. `cd gwt-gui && pnpm test`

## 既存コード再利用

- `gh_cli.rs` の `gh_command()`, `wait_with_timeout()` パターン
- `pullrequest.rs` (gwt-tauri) の `spawn_blocking`, キャッシュ, `StructuredError` パターン
- `IssueListPanel.svelte` のUI パターン（フィルター、ページネーション、IntersectionObserver）
- `PrStatusSection.svelte` のバッジ表示パターン
- `MarkdownRenderer.svelte` のMarkdown表示
- `prPolling.svelte.ts` のポーリングパターン
- `openExternalUrl.ts` の外部URL開き
