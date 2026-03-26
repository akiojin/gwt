## Summary

Worktree-Issue ローカルリンクとローカル Issue キャッシュを実装する。現状の `gh issue view` 都度取得を、local exact cache + REST fallback に置き換え、UI 描画のオフライン安定性を確保する。リポジトリ全 Issue を対象とし、diff sync / auto full sync / manual sync の 3 戦略を提供する。

## Technical Context

### 現状アーキテクチャ

- **Issue 取得**: `fetch_issue_detail()` → `gh issue view --json` を毎回実行。REST fallback はコメント数 hydration のみ
- **Issue リスト キャッシュ**: `~/.gwt/cache/issues/<repo-hash>.json` にページ単位のレスポンスキャッシュ（TTL 120s / 30d）。個別 Issue の exact lookup には使えない
- **Worktree-Issue 紐付け**: branch 名パース（`issue-<number>`）のみ。永続化された linkage は無い
- **Display name 解決**: `fetch_branch_linked_issue()` → `get_spec_issue_detail()` → `gh issue view` を毎回呼ぶ
- **Tauri 状態管理**: 既存 `AppState` の issue list 用 in-memory cache / inflight 管理は維持する。本仕様の exact cache / linkage は各コマンドから file-based store を `load/save` し、新規 `AppState` フィールドは追加しない

### 影響ファイル/モジュール

| レイヤー | ファイル | 変更内容 |
|---------|---------|---------|
| gwt-core | `src/git/issue_cache.rs` (新規) | ExactCache / SyncState / diff sync / full sync |
| gwt-core | `src/git/issue_linkage.rs` (新規) | WorktreeIssueLinkStore / bootstrap / CRUD |
| gwt-core | `src/git/issue.rs` | `fetch_issue_detail()` に REST fallback 追加 |
| gwt-core | `src/git/mod.rs` | 新モジュール re-export |
| gwt-tauri | `src/commands/issue.rs` | 新コマンド追加、`fetch_branch_linked_issue` を cache-first に変更 |
| gwt-tauri | `src/commands/project.rs` | `open_project` 内で auto full sync / linkage bootstrap を起動 |
| gwt-tauri | `src/app.rs` | 新コマンドを `tauri::Builder` に登録 |
| gwt-gui | `src/lib/types.ts` | SyncResult / SyncStatus 型追加 |
| gwt-gui | `src/lib/components/SettingsPanel.svelte` | 手動更新 UI 追加 |

### 前提と制約

- `gh` CLI が利用可能かつ認証済み（既存前提を踏襲）
- REST fallback は `gh api repos/{slug}/issues/{number}` で実装（`gh` のトークン管理を利用、直接 HTTP は不要）
- ストレージは既存パターンに従い `~/.gwt/cache/` 配下に repo ハッシュで分離した JSON ファイル
- 既存の `IssueListCacheEntry`（ページキャッシュ）はそのまま維持。exact cache は別系統

## Constitution Check

| ルール | 適合状況 |
|-------|---------|
| 1. Spec Before Implementation | ✅ spec.md clarified (rev2), plan.md / tasks.md completed |
| 2. Test-First Delivery | ✅ 各フェーズで先にテストを書く。受け入れシナリオ 8 件すべてにテスト対応 |
| 3. No Workaround-First Changes | ✅ 根本原因（毎回 gh 呼び出し / linkage 未永続化）を直接解消 |
| 4. Minimal Complexity | ✅ 下記 Complexity Tracking 参照 |
| 5. Verifiable Completion | ✅ SC-001〜SC-006 すべてテスト可能 |

### Required Plan Gates

1. **影響ファイル/モジュール**: 上記テーブル参照
2. **適用される constitution ルール**: 全 5 ルール適合
3. **受容するリスク・複雑性**: Complexity Tracking 参照
4. **受け入れシナリオの検証方法**: Rust unit test + Tauri integration test + Frontend vitest + E2E

## Project Structure

```text
crates/gwt-core/src/git/
├── issue.rs              # 既存: fetch_issue_detail に REST fallback 追加
├── issue_cache.rs        # 新規: IssueExactCache, diff/full sync
├── issue_linkage.rs      # 新規: WorktreeIssueLinkStore, bootstrap
├── issue_spec.rs         # 既存: 変更なし
└── mod.rs                # 既存: 新モジュール re-export

crates/gwt-tauri/src/
├── commands/issue.rs     # 既存: 新コマンド追加、cache-first 化
├── commands/project.rs   # 既存: open_project に自動同期/bootstrapping 追加
└── app.rs                # 既存: Tauri command 登録

gwt-gui/src/lib/
├── types.ts              # 既存: SyncResult 型追加
└── components/           # 既存: 手動更新 UI 追加

~/.gwt/cache/
├── issues/               # 既存: ページキャッシュ（変更なし）
├── issue-exact/          # 新規: exact cache (repo-hash.json)
└── issue-links/          # 新規: linkage store (repo-hash.json)
```

## Complexity Tracking

| 追加複雑性 | 理由 | 代替案と棄却理由 |
|-----------|------|----------------|
| 新規モジュール 2 つ (`issue_cache.rs`, `issue_linkage.rs`) | 既存 `issue.rs` は 1400 行超で責務過多。cache と linkage は独立した関心事 | 1ファイルに追加: 可読性・テスタビリティが劣化するため棄却 |
| `~/.gwt/cache/` に新ディレクトリ 2 つ | 既存ページキャッシュと exact cache は構造が異なる。混在させると TTL 管理が複雑化 | 同一ファイルに統合: キー衝突・サイズ肥大のリスクで棄却 |
| `open_project` でのバックグラウンド同期フック | 初回表示時に cache freshness と linkage bootstrap を保証する必要がある | 手動同期のみ: 初回表示の安定性要件を満たせないため棄却 |

## Phased Implementation

### Phase 1: データモデルと永続化 (gwt-core)

- `IssueExactCacheEntry`, `IssueCacheSyncState`, `SyncResult` 構造体を `issue_cache.rs` に定義
- `IssueExactCache` の JSON シリアライズ/デシリアライズ
- ファイルパス計算（既存 `issue_cache_file_path` パターン踏襲）
- `WorktreeIssueLinkEntry`, `LinkSource`, `WorktreeIssueLinkStore` を `issue_linkage.rs` に定義
- 永続化の read/write
- **テスト**: シリアライズ往復、ファイル I/O、空データ初期化

### Phase 2: REST fallback (gwt-core)

- `fetch_issue_detail()` に rate limit / 403 / transport error 検出を追加
- 失敗時に `gh api repos/{slug}/issues/{number}` で再取得
- `run_gh_output_with_repair` の既存パターンを活用
- **テスト**: 正常パス、gh 失敗 → REST 成功、両方失敗

### Phase 3: Exact cache lookup chain (gwt-core)

- `resolve_issue_from_cache(repo_path, issue_number)` → cache hit なら即返却
- cache miss → `fetch_issue_detail()` (with REST fallback) → 成功時に cache 更新
- cache hit + online fail → 既存 cache を返す
- cache miss + online fail → `None` / typed error
- **テスト**: 4 パターンの lookup chain

### Phase 4: Linkage bootstrap & store (gwt-core)

- `bootstrap_linkage_from_branches(repo_path)` → 全 worktree の branch 名をスキャンし linkage 生成
- `main` / `master` / `develop` 除外フィルタ
- GitHub linkage 確認（`gh issue develop --list`）による source 判定
- linkage CRUD: get / set / delete
- **テスト**: branch パース、除外フィルタ、CRUD

### Phase 5: Sync strategies (gwt-core)

- **diff sync**: `gh api repos/{slug}/issues?since={watermark}&state=all&per_page=100` + ページネーション → cache 更新、watermark 更新
- **full sync**: 全 Issue 取得 → cache 突合 → stale 削除 → watermark 更新。ネットワーク断時は部分反映のみ（stale cleanup スキップ）
- `SyncResult` 生成（更新件数、削除件数、所要時間、エラー）
- **テスト**: diff sync（新規追加・更新）、full sync（stale 削除）、ネットワーク断

### Phase 6: Tauri コマンド & 自動同期 (gwt-tauri)

- `resolve_worktree_issue(project_path, branch)` → linkage → exact cache → title 返却
- `sync_issue_cache(project_path, mode)` → diff / full を実行し `SyncResult` 返却
- `get_issue_cache_sync_status(project_path)` → 最終同期時刻・結果を返却
- `fetch_branch_linked_issue()` を cache-first に改修
- プロジェクトオープン時の auto full sync フック
- `IssueExactCache` / `WorktreeIssueLinkStore` は各コマンド内で `load/save` する file-based アプローチを採用し、`AppState` 拡張は行わない
- **テスト**: Tauri command 統合テスト

### Phase 7: フロントエンド (gwt-gui)

- `SyncResult` / `SyncStatus` TypeScript 型追加
- 手動更新 UI: 「Diff Sync」「Full Sync」ボタン + 結果表示（更新/削除件数、所要時間、エラー）
- consumer 側は既存 `fetch_branch_linked_issue()` 経路を維持しつつ、その内部を cache-first 化する。必要箇所では新 `resolve_worktree_issue` コマンドを再利用可能とする
- **テスト**: vitest でコンポーネントテスト

### Phase 8: Consumer 統合 & E2E

- #1644 (Sidebar display name) が exact cache 経由で title 解決することを確認
- #1687 (Tab label) が同じ chain を使うことを確認
- #1520 向け: exact cache データを外部から読み取れるインターフェース確認
- E2E: cache hit → title 表示、manual sync 操作
