## 現状分析

### Issue 取得パス

- `fetch_issue_detail()` (`gwt-core/src/git/issue.rs:855-883`): `gh issue view --json` 毎回実行。REST fallback はコメント数 hydration のみ
- `fetch_branch_linked_issue()` (`gwt-tauri/src/commands/issue.rs:596-611`): branch 名から issue 番号抽出 → `get_spec_issue_detail()` → `gh issue view` 毎回実行
- rate limit / 403 / transport error 時のリカバリパスが無い（コメント数以外）

### 既存キャッシュ

- `IssueListCacheEntry` (`gwt-tauri/src/commands/issue.rs`): ページ単位のリストキャッシュ。TTL 120s (in-memory) / 30d (disk)
- キャッシュキー: `page={page}&per_page={per_page}&state={state}&category={category}&include_body={include_body}`
- 個別 Issue の exact lookup には構造的に使えない（ページレスポンス全体を保存）

### Worktree-Issue 紐付け

- `extract_issue_number_from_branch_name()` (`issue.rs:1022-1037`): branch 名の `issue-<number>` パース。唯一の紐付け手段
- `create_or_verify_linked_branch()` (`issue.rs:1367-1402`): `gh issue develop` で GitHub linkage を作成。ローカル永続化なし
- GitHub linkage 確認: `gh issue develop --list <number>` のパース (`issue.rs:1319-1330`)

### ストレージパターン

- `~/.gwt/cache/issues/<repo-hash>.json`: SHA256(canonical_path) の先頭 16 文字でキー化
- `AppState` に `Mutex<HashMap>` で in-memory cache + disk 永続化の 2 層構造
- inflight 重複排除: `project_issue_list_inflight` HashSet で同時 fetch を防止

## Canonical Relationship

- `#1643`: Issue/Spec search の canonical spec
- `#1354`: Issue タブ画面の canonical spec
- `#1520`: Files index / recovery の canonical spec
- `#1714`: local issue cache / linkage の canonical spec

`#1684` は `#1643` に統合された historical reference として扱う。

## トレードオフ決定

### T-001: exact cache を既存ページキャッシュと分離

- **選択**: 分離（`~/.gwt/cache/issue-exact/` に新ディレクトリ）
- **理由**: 既存ページキャッシュは TTL ベースの揮発性キャッシュ。exact cache は永続性・stale cleanup・watermark 管理が必要で、ライフサイクルが異なる

### T-002: REST fallback に `gh api` を使用（直接 HTTP ではなく）

- **選択**: `gh api repos/{slug}/issues/{number}` を使用
- **理由**: `gh` のトークン管理・プロキシ設定をそのまま活用。直接 HTTP は認証管理の二重化になる

### T-003: Linkage store を exact cache と分離

- **選択**: 分離（`~/.gwt/cache/issue-links/`）
- **理由**: linkage はブランチ名 → Issue 番号のマッピング、cache は Issue 番号 → メタデータのマッピング。更新タイミングとライフサイクルが異なる

### T-004: Diff sync は GitHub REST `since` パラメータで全 Issue 差分取得

- **選択**: `gh api repos/{slug}/issues?since={watermark}&state=all&per_page=100` + ページネーション
- **理由**: キャッシュ済みエントリのみ再検証する方式では、新規 Issue を自動取得できない。`since` パラメータなら新規・更新をまとめて取得可能

### T-005: Auto full sync はプロジェクトオープン時のみ

- **選択**: タイマーベースの定期実行は初期実装では見送り、プロジェクトオープン時の 1 回のみ
- **理由**: プロジェクトオープンで全 Issue を取り込めば、セッション中は diff sync + cache hit で十分。タイマーは将来拡張で追加可能

## 外部制約

- GitHub REST API rate limit: authenticated で 5,000 req/hour。full sync で 100 件/ページなら 1,000 Issue でも 10 リクエスト
- `gh` CLI の `--json` オプション: Issue の `updatedAt` フィールドは ISO 8601 形式（`2024-01-15T10:30:00Z`）
- `gh api` の `since` パラメータ: ISO 8601 timestamp（例: `2024-01-15T00:00:00Z`）
