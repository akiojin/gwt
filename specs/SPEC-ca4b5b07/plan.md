# 実装計画: Issue タブ — GitHub Issue 一覧・詳細・フルフロー

**仕様ID**: `SPEC-ca4b5b07` | **日付**: 2026-02-17 | **仕様書**: `specs/SPEC-ca4b5b07/spec.md`

## 目的

- Git メニューから GitHub Issue 一覧を表示するタブをメインエリアに追加する
- Issue 詳細を GFM Markdown でレンダリングして表示する
- Issue から AgentLaunchForm（worktree 作成）へのフルフローを実現する

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）— 既存 `fetch_github_issues` / `check_gh_cli_status` / `find_existing_issue_branch` コマンドを拡張
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）— 新規 IssueListPanel コンポーネント + MarkdownRenderer コンポーネント
- **ストレージ/外部連携**: GitHub CLI (`gh`) 経由の GitHub API — `gh issue list --json`, `gh issue view --json`
- **テスト**: cargo test（バックエンド）/ vitest + @testing-library/svelte（フロントエンド）
- **前提**: GFM Markdown レンダリングライブラリ（marked + DOMPurify）の新規導入が必要

## 実装方針

### Phase 1: バックエンド拡張

- `GitHubIssueInfo` 構造体を拡張: `body`, `assignees`（`login` + `avatar_url`）, `comments_count`, `milestone`, `state`, `html_url` フィールド追加
- `GitHubLabel` 構造体を拡張: `color` フィールド追加（現在は `name` のみ）
- `fetch_github_issues` コマンドに `state` パラメータ（`"open"` / `"closed"`）を追加
- `fetch_github_issue_detail` コマンドを新規追加: 単一 Issue の本文・メタ情報を取得
- `gh issue list --json number,title,labels,assignees,updatedAt,comments,milestone,state,url,body --state {state} --limit {per_page}` 形式で呼び出し

### Phase 2: メニュー・タブ基盤

- `menu.rs` に `MENU_ID_GIT_ISSUES` 定数を追加し、Git メニューに「Issues」を追加
- `types.ts` の `Tab["type"]` ユニオンに `"issues"` を追加
- `App.svelte` にメニューアクションハンドラを追加（シングルトンロジック: 既存タブがあればフォーカス、なければ新規作成）
- `MainArea.svelte` に `IssueListPanel` のタブレンダリング分岐を追加

### Phase 3: GFM Markdown レンダリング基盤

- `gwt-gui` に `marked`（GFM パーサー）+ `dompurify`（XSS サニタイズ）をインストール
- `MarkdownRenderer.svelte` コンポーネントを作成:
  - `marked` で GFM → HTML 変換（チェックボックス・テーブル・取り消し線対応）
  - `DOMPurify` で HTML サニタイズ
  - リンクは `target="_blank"` + `rel="noopener noreferrer"` を付与
  - コードブロックのシンタックスハイライトは将来スコープ

### Phase 4: Issue 一覧コンポーネント

- `IssueListPanel.svelte` を新規作成:
  - リッチ一覧表示: #番号・タイトル・ラベル（色付きバッジ）・アサイニーアバター・更新日時（相対表示）・コメント数アイコン・マイルストーン・worktree インジケーター
  - テキスト検索バー（クライアントサイドフィルタ）
  - ラベルフィルタ（クリックトグル）
  - open/closed トグルボタン（サーバーサイド: state パラメータで再取得）
  - 無限スクロール（IntersectionObserver でセンチネル要素を監視）
  - 手動リフレッシュボタン
  - ローディングスピナー・エラー表示（gh CLI 未対応ガイド含む）・空状態メッセージ
  - worktree 紐づきインジケーター（`find_existing_issue_branch` API を全 Issue に対して呼び出し）

### Phase 5: Issue 詳細ビュー

- `IssueListPanel` 内に詳細ビューを実装（一覧 → 詳細切替ナビゲーション）:
  - 戻るボタン（一覧に戻る、フィルタ状態保持）
  - ヘッダー: タイトル・ステータスバッジ・ラベル・アサイニーアバター・マイルストーン・コメント数
  - 本文: `MarkdownRenderer` で GFM レンダリング
  - Spec Issue 判定: ラベルに `spec` が含まれれば `IssueSpecPanel` のセクション解析ビューを表示
  - 「Open in GitHub」ボタン: `shell.open(html_url)` で外部ブラウザ
  - `fetch_github_issue_detail` で詳細データを取得

### Phase 6: フルフロー連携（AgentLaunchForm プリフィル）

- `App.svelte` の `requestAgentLaunch()` に Issue 情報渡しインターフェースを追加
- `AgentLaunchForm.svelte` に Issue プリフィルロジックを追加:
  - New Branch モードを自動選択
  - prefix: ラベルから推定マッピング（`bug` → `bugfix/`, `enhancement`/`feature` → `feature/`, `hotfix` → `hotfix/`, デフォルト → `feature/`）
  - suffix: `issue-{number}`
  - `issueNumber`: プリフィル
- 紐づき worktree 判定:
  - `find_existing_issue_branch` → worktree 存在チェック
  - 存在する場合:「Switch to worktree」ボタンに変更 → worktree 切替処理

## テスト

### バックエンド

- `GitHubIssueInfo` / `GitHubLabel` 拡張型のシリアライズ/デシリアライズテスト
- `fetch_github_issue_detail` コマンドの正常系・エラー系テスト
- `state` パラメータによるフィルタテスト

### フロントエンド

- `MarkdownRenderer.svelte`: GFM 各要素レンダリング + XSS サニタイズテスト
- `IssueListPanel.svelte`:
  - Issue 一覧レンダリング（各表示項目の存在確認）
  - テキスト検索・ラベルフィルタ・open/closed トグル
  - 無限スクロール（IntersectionObserver mock）
  - gh CLI エラー表示・空状態表示
  - 詳細ビュー遷移・戻る・フィルタ保持
  - Spec Issue 判定切替
  - 「Work on this」/ 「Switch to worktree」ボタン表示切替
