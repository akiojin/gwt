# 実装計画: PR Status Preview（GUI）

**仕様ID**: `SPEC-d6949f99` | **日付**: 2026-02-14 | **仕様書**: `specs/SPEC-d6949f99/spec.md`

## 目的

- GitHub Webを開かずにgwt上でPR/CIステータスを確認可能にする
- Worktreeツリーの展開式UIでCI Workflow Run一覧を視覚的に表示
- Session SummaryにPR詳細（メタデータ・レビューコメント・変更サマリー）セクションを追加

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-core/` + `crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **外部連携**: GitHub CLI（`gh`）経由のGraphQL API + CLIコマンド
- **テスト**: `cargo test`（Rust） / `vitest`（フロントエンド）
- **前提**: gh CLIインストール済み・認証済み（未対応時はグレースフルデグレード）

## 既存資産

- `crates/gwt-core/src/git/pullrequest.rs`: `PrCache` + `PullRequest`構造体（PR番号・タイトル・ステータス・URL）
- `crates/gwt-core/src/git/gh_cli.rs`: `gh_command()` / `is_gh_available()` ヘルパー
- `crates/gwt-core/src/git/issue.rs`: GitHub Issue取得パターン（`GhCliStatus`、JSON解析、ページネーション）
- `gwt-gui/src/lib/components/Sidebar.svelte`: ブランチ一覧（`BranchInfo`ベース、safety dot、divergence表示）
- `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`: Session Summary（AI要約、Quick Start）
- `gwt-gui/src/lib/types.ts`: フロントエンド型定義

## 実装方針

### Phase 1: バックエンド — GraphQL PR/CI 一括取得

**対象ファイル**: `crates/gwt-core/src/git/`

1. `pullrequest.rs` を拡張して `PrStatusInfo` 構造体を追加
   - PR メタデータ: number, title, state, url, mergeable, author, base/head branch, labels, assignees, milestone, linked issues
   - Check Suites: workflow 名, status (queued/in_progress/completed), conclusion (success/failure/neutral/...)
   - Review 情報: reviewer 名, state (APPROVED/CHANGES_REQUESTED/COMMENTED/PENDING)
   - Review コメント: inline comments (file path, line, body, code snippet)
   - 変更サマリー: changed files count, additions, deletions

2. `graphql.rs`（新規）: GraphQLクエリビルダー
   - ブランチ名リストを受け取り、1回のGraphQLクエリで全PRの情報を一括取得
   - クエリ: `repository > pullRequests(headRefName)` + `commits > checkSuites > checkRuns` + `reviews` + `reviewThreads`
   - `gh api graphql -f query='...'` を `gh_command()` 経由で実行

3. `pullrequest.rs` の `PrCache` を拡張
   - `PrStatusInfo` をキャッシュし、ポーリングでリフレッシュ
   - レートリミットエラー時はキャッシュ維持

### Phase 2: バックエンド — Tauri コマンド

**対象ファイル**: `crates/gwt-tauri/src/`

1. `fetch_pr_status` コマンド: 全Worktreeブランチ分のPR/CI情報を一括取得して返す
2. `fetch_pr_detail` コマンド: 特定PRの詳細情報（レビューコメント含む）を取得
3. `fetch_ci_log` コマンド: `gh run view <run_id> --log` を実行しログテキストを返す
4. `check_gh_status` コマンド: 既存の `GhCliStatus` パターンを流用

### Phase 3: フロントエンド — Worktreeツリー展開

**対象ファイル**: `gwt-gui/src/lib/`

1. `types.ts` に新型定義を追加
   - `PrStatusInfo`, `WorkflowRunInfo`, `ReviewInfo`, `ReviewComment`, `PrChangeSummary`

2. `Sidebar.svelte` のブランチ一覧をツリー化
   - 各 `branch-item` の左に専用トグルアイコン（▶/▼）を追加
   - トグルクリックで展開/折りたたみ（既存のブランチクリック動作は維持）
   - 展開時にWorkflow Run一覧を表示（pass=緑, fail=赤, running=黄, pending=グレー）
   - PRステータスバッジ（`#42 Open` / `No PR`）をブランチ名横に表示

3. Workflow Runクリックでxterm.jsターミナルタブを開く
   - 既存のタブシステム（`Tab` 型の `type: "terminal"`）を活用
   - `gh run view <run_id> --log` を実行するPTYセッションを起動

### Phase 4: フロントエンド — Session Summary PRセクション

**対象ファイル**: `gwt-gui/src/lib/components/`

1. `PrStatusSection.svelte`（新規コンポーネント）
   - メタデータ表示: タイトル、作成者、base/head、ラベル、アサイニー、マイルストーン
   - Mergeable ステータスバッジ
   - Reviews サブセクション: レビューアー承認状態 + inline コメント表示
   - コードスニペットのシンタックスハイライト（フロントエンド処理）
   - Changes サブセクション: ファイル一覧、追加/削除行数、コミット一覧

2. `WorktreeSummaryPanel.svelte` にPR Statusセクションを統合
   - 既存の AI Summary セクションと並列で「PR Status」セクションを追加
   - `selectedBranch` に紐づくPR情報を表示

### Phase 5: ポーリング + フォーカス管理

**対象ファイル**: `gwt-gui/src/lib/`

1. `prPolling.ts`（新規）: ポーリングロジック
   - 30秒固定間隔で `fetch_pr_status` を呼び出し
   - `document.visibilitychange` イベントでフォーカスロス時にポーリング停止
   - フォアグラウンド復帰時に即座リフレッシュ + ポーリング再開
   - Svelte 5 runes（`$state` / `$effect`）でリアクティブに管理

2. グレースフルデグレード
   - `check_gh_status` の結果に応じてPR UI全体を「GitHub not connected」に切り替え

## テスト

### バックエンド（Rust）

- GraphQL JSON パース: 正常/空/エラー各ケース
- `PrStatusInfo` 構造体の各フィールド検証
- `PrCache` のキャッシュ動作（ポピュレート/クリア/レートリミット時維持）
- gh CLI未対応時のグレースフルフォールバック

### フロントエンド（vitest）

- `PrStatusSection` のレンダリング（メタデータ、レビュー、変更サマリー各パターン）
- ツリー展開/折りたたみのインタラクション
- ポーリング開始/停止のライフサイクル
- グレースフルデグレード（`GhCliStatus.authenticated = false` 時）
