## エンティティ

### IssueExactCacheEntry

個別 Issue のメタデータキャッシュエントリ。

| フィールド | 型 | 説明 |
|-----------|---|------|
| `number` | `u64` | Issue 番号（キー） |
| `title` | `String` | Issue タイトル |
| `url` | `String` | Issue HTML URL |
| `state` | `String` | `"open"` \| `"closed"` |
| `labels` | `Vec<String>` | ラベル名リスト |
| `updated_at` | `String` | GitHub 側の最終更新時刻（ISO 8601） |
| `fetched_at` | `i64` | ローカル取得時刻（Unix millis） |

**不変条件**: `number` は 1 以上。`fetched_at` は `updated_at` 以降。

### IssueExactCache

リポジトリ単位の exact cache コンテナ。永続化単位。

| フィールド | 型 | 説明 |
|-----------|---|------|
| `entries` | `HashMap<u64, IssueExactCacheEntry>` | Issue 番号 → エントリ |
| `sync_state` | `IssueCacheSyncState` | 同期状態 |

**ストレージ**: `~/.gwt/cache/issue-exact/<repo-hash>.json`
**repo-hash**: `SHA256(canonical_repo_path)[..16]`（既存パターン踏襲）

### IssueCacheSyncState

同期の進行状態。diff sync の watermark と最終結果を保持。

| フィールド | 型 | 説明 |
|-----------|---|------|
| `last_diff_sync_at` | `Option<i64>` | 最終 diff sync 時刻（Unix millis） |
| `last_full_sync_at` | `Option<i64>` | 最終 full sync 時刻（Unix millis） |
| `last_issue_updated_at` | `Option<String>` | diff sync の watermark（ISO 8601）。`since` パラメータに使用 |
| `last_result` | `Option<SyncResult>` | 最終同期結果 |

### SyncResult

同期実行結果。UI 表示と診断に使用。

| フィールド | 型 | 説明 |
|-----------|---|------|
| `sync_type` | `SyncType` | `Diff` \| `Full` |
| `updated_count` | `u32` | 更新（追加含む）件数 |
| `deleted_count` | `u32` | 削除件数（full sync のみ非ゼロ） |
| `duration_ms` | `u64` | 所要時間 |
| `completed_at` | `i64` | 完了時刻（Unix millis） |
| `error` | `Option<String>` | エラーメッセージ（部分失敗時） |

### WorktreeIssueLinkEntry

Worktree（branch）と Issue の紐付けエントリ。

| フィールド | 型 | 説明 |
|-----------|---|------|
| `branch_name` | `String` | ブランチ名（キー） |
| `issue_number` | `u64` | 紐付け先 Issue 番号 |
| `source` | `LinkSource` | 紐付けソース |
| `linked_at` | `i64` | 初回紐付け時刻（Unix millis） |
| `updated_at` | `i64` | 最終更新時刻（Unix millis） |

### LinkSource

紐付けの由来を示す enum。

| 値 | 説明 |
|---|------|
| `GitHubLinkage` | `gh issue develop` による GitHub 側の紐付け |
| `BranchParse` | branch 名 `issue-<number>` パターンからの推定 |
| `Manual` | ユーザーによる手動紐付け |

**優先度**: `GitHubLinkage` > `Manual` > `BranchParse`（同一 branch に複数ソースがある場合、高優先度で上書き）

### WorktreeIssueLinkStore

リポジトリ単位の linkage ストア。永続化単位。

| フィールド | 型 | 説明 |
|-----------|---|------|
| `links` | `HashMap<String, WorktreeIssueLinkEntry>` | branch 名 → エントリ |

**ストレージ**: `~/.gwt/cache/issue-links/<repo-hash>.json`

## ライフサイクル

### Exact Cache

```
初回プロジェクトオープン → auto full sync → 全 Issue 取り込み
  ↓
通常 UI 描画 → cache hit → 即返却（online lookup なし）
  ↓
cache miss → fetch_issue_detail (gh → REST fallback) → cache 更新
  ↓
ユーザー操作 → manual diff sync → since watermark 以降の更新取り込み
  ↓
ユーザー操作 → manual full sync → stale 削除 + 全件更新
```

### Linkage

```
プロジェクトオープン → bootstrap → 全 worktree branch をスキャン
  ↓
issue-<number> パターン発見 → BranchParse linkage 作成
  ↓
gh issue develop 実行時 → GitHubLinkage で上書き
  ↓
branch 削除 → linkage は残留（closed worktree の表示名保持のため）
```
