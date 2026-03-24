## 背景

Worktree 一覧や tab 表示名は Issue タイトルを必要とするが、現状は `gh issue view` の都度取得と branch 名パースに強く依存しており、rate limit・通信失敗・linkage の曖昧さに弱い。現在の Chroma `issues` index は `gwt-spec` 限定の semantic search 用であり、一般 Issue の exact cache としては使えない。

本仕様では、GitHub linkage を source of truth としつつ、UI が直接 online lookup に依存しないように、Worktree-Issue ローカルリンクと local exact issue cache を定義する。Project Index / Issues semantic search は #1520 の責務とし、その index データは本仕様の local cache を元に派生生成する。

## 境界

- 本仕様は worktree-issue linkage と local exact issue cache persistence の正本である
- branch/ref/worktree inventory、local Git cache invalidation、cleanup などの local Git backend は `#1644` が正本である
- GitHub Issue/Spec search や version history の discovery 導線は `#1643` が正本である

## 用語定義

- **GitHub linkage**: `gh issue develop` コマンドで作成される GitHub 側の development branch と Issue の紐付け。GitHub API 経由で問い合わせ可能
- **local linkage**: ローカルに永続化された worktree-issue 対応表。source of truth は GitHub linkage だが、branch 名パース（`issue-<number>`）による bootstrap / fallback も持つ
- **exact cache**: `issue_number -> issue metadata` のローカル永続キャッシュ。リポジトリの全 Issue（open / closed）を対象とし、UI 描画・semantic search の元データとなる
- **diff sync**: GitHub API の `since` パラメータで最終同期以降に更新された全 Issue（新規含む）を取得し、cache を更新する。stale entry の削除は行わない
- **full sync**: リポジトリの全 Issue を取得し、cache と突合して stale entry を削除・更新する

## ユーザーシナリオ

### US-1: Worktree 表示名を安定して表示したい (Priority: P0)
- Given: Worktree が Issue に関連付いている
- When: Sidebar / Summary / tab で表示名を解決する
- Then: local cache にある Issue タイトルを使って安定表示できる

### US-2: GitHub への都度アクセスを避けたい (Priority: P0)
- Given: Issue タイトルを頻繁に参照する UI がある
- When: Worktree 一覧や tab bar を再描画する
- Then: local cache を優先し、`gh` / REST を毎回叩かない

### US-3: branch 命名規則に依存せず Worktree と Issue を結びたい (Priority: P0)
- Given: branch 名が `issue-1234` でない、または将来 rename される
- When: Worktree-Issue 関連を解決する
- Then: GitHub linkage または local linkage によって正しく解決できる

### US-4: stale な Issue 情報を手動で更新したい (Priority: P0)
- Given: local cache が古い、または fetch が失敗した
- When: ユーザーが手動更新を実行する
- Then: 差分更新またはフル同期を選んで cache/index を更新できる

### US-5: semantic search も同じ元データから作りたい (Priority: P1)
- Given: Project Index / Issue search を使う
- When: Issues の semantic search を行う
- Then: local exact cache の内容を consumer（#1520）が vector index の入力データとして利用できる

## 受け入れシナリオ

1. Given linkage 済み Worktree, When 表示名を解決する, Then local linkage -> exact cache -> Issue タイトルで表示できる
2. Given exact cache hit, When Worktree 一覧を再表示する, Then `gh issue view` を呼ばない
3. Given `gh issue view` が rate limit で失敗する, When exact cache miss が起こる, Then `gh api repos/{slug}/issues/{number}` による REST fallback で取得し cache を更新する
4. Given stale entry が存在する, When full sync を実行する, Then stale issue が削除または更新される
5. Given old branch-based linkage のみ存在する, When bootstrap を行う, Then local linkage に保存される
6. Given プロジェクトを開く, When アプリが初期化を完了する, Then 自動フル同期が実行されリポジトリの全 Issue が exact cache に取り込まれる
7. Given `gh` と REST の両方が失敗する, When exact cache に既存エントリがある, Then 既存 cache を返す
8. Given exact cache miss かつ online lookup 全失敗, When consumer が title を要求する, Then `None` / typed error を返し consumer 側で fallback 表示を使える

## Edge Cases

- `main` / `master` / `develop` 系 branch は linkage 自動生成対象外とする
- GitHub linkage が無く、branch 名にも issue 番号が無い場合は linkage 不能として扱う
- `gh` と REST の両方が失敗した場合でも、既存 exact cache があればそれを返す
- exact cache miss かつ online lookup 失敗時は consumer 側が `AI -> branch` fallback を使えるよう `None` / typed error を返す
- 差分更新では stale entry を削除しない。stale 解消は full sync でのみ行う
- diff sync の `since` クエリで新規 Issue が見つかった場合、cache に追加する
- full sync 実行中にネットワーク断が発生した場合、部分取得分は cache に反映するが stale cleanup は行わない（完全取得時のみ）
- closed issue も cache 対象とする（closed worktree の表示名解決に必要）
- #1520 / #1644 / #1687 の consumer 側での最終受け入れは各 SPEC で担保し、本仕様では reusable cache/linkage primitive と interface 提供までを範囲とする

## 機能要件

- FR-001: Worktree-Issue 関連は local persisted linkage を保持しなければならない。linkage source は `github_linkage` | `branch_parse` | `manual` の 3 種を区別する
- FR-002: linkage の source of truth は GitHub linkage（`gh issue develop` による紐付け）とし、branch 名パース（`issue-<number>`）は bootstrap / fallback 専用としなければならない
- FR-003: Issue exact cache は `issue_number -> issue metadata` の厳密 lookup を提供しなければならない。対象はリポジトリの全 Issue（open / closed）とする
- FR-004: Worktree / Sidebar / Summary / tab の Issue タイトル参照は local exact cache を最優先で使わなければならない
- FR-005: `gh issue view` は online source の 1 つとし、rate limit / 403 / transport error 時は `gh api repos/{slug}/issues/{number}` による direct REST fallback を試行しなければならない
- FR-006: online lookup が成功した場合、exact cache を更新しなければならない
- FR-007: linkage bootstrap はプロジェクトオープン時に、linkage 未解決の worktree に対して実行しなければならない。既存 `issue-<number>` branch から初回取り込みを行う
- FR-008: update strategy は以下の 3 戦略を提供しなければならない
  - **差分更新（diff sync）**: GitHub API の `since` パラメータで最終同期以降に更新された全 Issue を取得する。新規 Issue の自動取り込みを含む。stale entry は削除しない
  - **自動フル同期**: プロジェクトオープン時に自動実行する。リポジトリの全 Issue を取得し、stale entry を削除・更新する
  - **手動更新**: ユーザー操作で差分更新またはフル同期を明示的に実行する
- FR-009: 手動更新 UI は少なくとも `差分更新` と `フル同期` を選べなければならない
- FR-010: 手動更新結果は更新件数、削除件数、所要時間、最終更新時刻、失敗理由を返さなければならない
- FR-011: local exact cache の内容は #1520 の Issue semantic search index が利用できる入力インターフェース（例: `all_entries()`）として提供されなければならない
- FR-012: #1644 と #1687 は本仕様を参照し、Issue タイトル解決の upstream dependency として扱わなければならない

## 非機能要件

- NFR-001: UI の通常描画は online lookup を必須としてはならない
- NFR-002: exact cache は repo 単位で永続化され、アプリ再起動後も再利用できなければならない
- NFR-003: 差分更新の通常経路では stale cleanup を行わず、処理量を抑えなければならない
- NFR-004: full sync は stale cleanup を保証しなければならない（ただしネットワーク断による部分取得時は除く）
- NFR-005: Issue 検索 index の再構築は local exact cache を元に deterministic に行われなければならない
- NFR-006: `gh` / REST の一時失敗で既存 cache を破壊してはならない

## 成功基準

- SC-001: Worktree / Sidebar / tab が exact cache 経由で Issue タイトルを安定表示できる
- SC-002: `gh` rate limit 時でも REST fallback により title lookup が継続できる
- SC-003: 差分更新で更新分（新規含む）を取り込み、フル同期で stale entry を解消できる
- SC-004: 手動更新 UI から diff/full を実行できる
- SC-005: #1520 の Issue search index が local exact cache を元データとして利用できる入力インターフェースが提供される
- SC-006: プロジェクトオープン時に自動フル同期が実行され、cache が最新状態になる
