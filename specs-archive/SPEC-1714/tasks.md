> **Status Note**: この SPEC は実装完了により closed。task は履歴として保持する。

## Phase 0: Setup

- [x] TASK-0-1: `crates/gwt-core/src/git/issue_cache.rs` を新規作成し、`mod.rs` に `pub mod issue_cache;` を追加
- [x] TASK-0-2: `crates/gwt-core/src/git/issue_linkage.rs` を新規作成し、`mod.rs` に `pub mod issue_linkage;` を追加
- [x] TASK-0-3: `~/.gwt/cache/issue-exact/` および `~/.gwt/cache/issue-links/` ディレクトリ自動生成ロジックを追加（既存 `issue_cache_file_path` パターン踏襲）

## Phase 1: Foundational — データモデルと永続化

### US-1, US-2 基盤

- [x] TASK-1-1: [P] テスト: `IssueExactCacheEntry` / `IssueExactCache` / `IssueCacheSyncState` / `SyncResult` のシリアライズ往復テスト
- [x] TASK-1-2: [P] テスト: `WorktreeIssueLinkEntry` / `LinkSource` / `WorktreeIssueLinkStore` のシリアライズ往復テスト
- [x] TASK-1-3: 構造体定義（`issue_cache.rs`）
- [x] TASK-1-4: 構造体定義（`issue_linkage.rs`）
- [x] TASK-1-5: [P] テスト: `IssueExactCache` の JSON ファイル read/write
- [x] TASK-1-6: [P] テスト: `WorktreeIssueLinkStore` の JSON ファイル read/write
- [x] TASK-1-7: `IssueExactCache` の `load` / `save` 実装
- [x] TASK-1-8: `WorktreeIssueLinkStore` の `load` / `save` 実装
- [x] TASK-1-9: テスト RED → GREEN 確認

## Phase 2: US-2 — REST fallback

- [x] TASK-2-1: テスト: REST fallback 検証
- [x] TASK-2-2: テスト: 両方失敗時のエラーテスト
- [x] TASK-2-3: `fetch_issue_detail()` に REST fallback 追加（`fetch_all_issues_via_rest` 含む）
- [x] TASK-2-4: テスト RED → GREEN 確認

## Phase 3: US-1, US-2 — Exact cache lookup chain

- [x] TASK-3-1: テスト: `resolve()` の 4 パターン（cache hit、miss→online成功、hit+fail→cache返却、miss+fail→None）
- [x] TASK-3-2: `IssueExactCache::resolve()` 実装
- [x] TASK-3-3: テスト RED → GREEN 確認

## Phase 4: US-3 — Linkage bootstrap & store

- [x] TASK-4-1: テスト: `bootstrap_from_branches()` + 除外フィルタ
- [x] TASK-4-2: テスト: linkage CRUD
- [x] TASK-4-3: テスト: `LinkSource` 優先度による上書き
- [x] TASK-4-4: `bootstrap_from_branches()` 実装
- [x] TASK-4-5: linkage CRUD 実装（get_link / set_link / remove_link）
- [x] TASK-4-6: テスト RED → GREEN 確認

## Phase 5: US-4 — Sync strategies

- [x] TASK-5-1: テスト: diff sync（watermark、stale 非削除）
- [x] TASK-5-2: テスト: full sync（stale 削除、ネットワーク断）
- [x] TASK-5-3: テスト: `SyncResult` フィールド検証
- [x] TASK-5-4: `diff_sync()` 実装
- [x] TASK-5-5: `full_sync()` 実装
- [x] TASK-5-6: テスト RED → GREEN 確認

## Phase 6: US-1, US-2, US-3, US-4 — Tauri コマンド & 自動同期

- [x] TASK-6-1: Note: `AppState` への Mutex 追加は不要と判断。各コマンド内で `IssueExactCache::load/save` を直接呼ぶファイルベースアプローチを採用（既存パターン踏襲、State 肥大化回避）
- [x] TASK-6-2: [P] `resolve_worktree_issue` コマンド追加
- [x] TASK-6-3: [P] `sync_issue_cache` コマンド追加
- [x] TASK-6-4: [P] `get_issue_cache_sync_status` コマンド追加
- [x] TASK-6-5: `fetch_branch_linked_issue()` をキャッシュ優先に改修
- [x] TASK-6-6: プロジェクトオープン時の auto full sync フック追加（`open_project` 内バックグラウンドタスク）
- [x] TASK-6-7: プロジェクトオープン時の linkage bootstrap フック追加（`open_project` 内バックグラウンドタスク）
- [x] TASK-6-8: 新コマンドを `tauri::Builder` に登録

## Phase 7: US-4 — フロントエンド手動更新 UI

- [x] TASK-7-1: `SyncResult` / `IssueCacheSyncState` TypeScript 型追加
- [x] TASK-7-2: テスト: 手動更新 UI の vitest（4件追加、89 tests all pass）
- [x] TASK-7-3: 手動更新 UI 実装（SettingsPanel Maintenance セクションに Diff Sync / Full Sync ボタン）
- [x] TASK-7-4: 表示名解決は `fetch_branch_linked_issue` 内部でキャッシュ優先に改修済み。consumer 側の明示的切り替えは不要と判断
- [x] TASK-7-5: テスト RED → GREEN 確認

## Phase 8: US-5 — #1520 インターフェース

- [x] TASK-8-1: `IssueExactCache::all_entries()` 公開メソッド追加
- [x] TASK-8-2: テスト: `all_entries()` が全キャッシュエントリを返すことを検証

## Phase 9: Polish / Cross-Cutting

- [x] TASK-9-1: #1644 consumer 統合は #1644 側の受け入れで継続管理し、本 SPEC では upstream cache-first dependency の提供までを確認
- [x] TASK-9-2: #1687 consumer 統合は #1687 側の受け入れで継続管理し、本 SPEC では upstream cache-first dependency の提供までを確認
- [x] TASK-9-3: E2E テスト: cache sync UI の Playwright テスト（3件追加）
- [x] TASK-9-4: `cargo clippy --all-targets --all-features -- -D warnings` 通過確認
- [x] TASK-9-5: `cargo fmt` 適用
- [x] TASK-9-6: `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` 通過確認

## Traceability Matrix

| User Story | Tasks |
|-----------|-------|
| US-1 (表示名安定表示) | TASK-1-*, TASK-3-*, TASK-6-2, TASK-6-5, TASK-7-4 |
| US-2 (GitHub 都度アクセス回避) | TASK-1-*, TASK-2-*, TASK-3-*, TASK-6-5, TASK-6-6 |
| US-3 (branch 命名非依存) | TASK-4-*, TASK-6-2, TASK-6-7 |
| US-4 (手動更新) | TASK-5-*, TASK-6-3, TASK-6-4, TASK-7-* |
| US-5 (semantic search 元データ) | TASK-8-* |

| Acceptance Scenario | Verification Task |
|--------------------|------------------|
| AS-1 (linkage→cache→title) | TASK-3-1, TASK-6-2, TASK-6-5, TASK-7-4, TASK-9-3 |
| AS-2 (cache hit→no gh call) | TASK-3-1, TASK-6-5 |
| AS-3 (rate limit→REST fallback) | TASK-2-1, TASK-2-3 |
| AS-4 (full sync→stale 削除) | TASK-5-2, TASK-5-5 |
| AS-5 (bootstrap→local linkage) | TASK-4-1, TASK-4-4, TASK-6-7 |
| AS-6 (auto full sync on open) | TASK-6-6 |
| AS-7 (both fail→existing cache) | TASK-3-1 |
| AS-8 (cache miss+fail→None) | TASK-3-1 |
