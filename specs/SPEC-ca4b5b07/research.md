# リサーチ: Issue タブ

**仕様ID**: `SPEC-ca4b5b07`

## 既存バックエンドコマンド調査

### fetch_github_issues

- **場所**: `crates/gwt-tauri/src/commands/issue.rs`
- **パラメータ**: `project_path`, `page`, `per_page`
- **返却型**: `Vec<GitHubIssueInfo>` — 現在は `number`, `title`, `updatedAt`, `labels: Vec<String>` のみ
- **内部実装**: `gh issue list --json number,title,labels,updatedAt --limit {per_page}`
- **拡張必要**: `body`, `assignees`, `comments`, `milestone`, `state`, `url` の追加、`labels` を `Vec<GitHubLabel>` に変更

### check_gh_cli_status

- **場所**: `crates/gwt-tauri/src/commands/issue.rs`
- **返却型**: `GhCliStatus { available: bool, authenticated: bool }`
- **拡張不要**: そのまま活用可能

### find_existing_issue_branch

- **場所**: `crates/gwt-tauri/src/commands/issue.rs`
- **パラメータ**: `project_path`, `issue_number`
- **返却型**: `Option<String>`（ブランチ名）
- **拡張不要**: worktree 紐づきチェックに活用

## GFM Markdown ライブラリ選定

### 候補比較

| ライブラリ | サイズ (gzip) | GFM 対応 | XSS 対策 | メンテナンス |
|---|---|---|---|---|
| marked | ~9KB | 組込み | DOMPurify 併用 | 活発 |
| markdown-it | ~14KB | プラグイン | 別途必要 | 活発 |
| remark/rehype | ~30KB+ | プラグイン | rehype-sanitize | 活発 |

### 採用: marked + DOMPurify

- **理由**: 軽量、GFM 組込みサポート、広く使われている、設定が簡単
- `marked.use(markedGfm())` で GFM 対応
- `DOMPurify.sanitize(html)` で XSS 防止

## 既存フロントエンドコンポーネント調査

### IssueSpecPanel

- **場所**: `gwt-gui/src/lib/components/IssueSpecPanel.svelte`
- **機能**: Spec Issue のセクション解析（spec/plan/tasks/TDD 等のタブ表示）
- **統合方針**: `spec` ラベル付き Issue の詳細表示で再利用

### AgentLaunchForm

- **場所**: `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- **入力項目**: ブランチ選択（既存/新規）、prefix/suffix、base ブランチ、エージェント選択、セッションモード、Docker 設定、詳細オプション
- **Issue プリフィル対象**: New Branch モード、prefix（ラベル推定）、suffix（`issue-{number}`）、issueNumber

## GitHub CLI JSON フィールド

`gh issue list --json` で取得可能なフィールド:

- `number`, `title`, `body`, `state`, `url`
- `labels` (array: `name`, `color`, `description`)
- `assignees` (array: `login`, `name`, `avatarUrl`)
- `comments` (array、ただし `--json comments` は件数ではなく全コメント返却)
- `milestone` (object: `title`, `number`)
- `updatedAt`, `createdAt`

**注意**: `comments` フィールドは全コメントオブジェクトを返すため、一覧では件数のみ（`comments` 配列の length）を使用する。
