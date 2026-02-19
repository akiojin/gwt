# データモデル: Issue タブ

**仕様ID**: `SPEC-ca4b5b07`

## バックエンド型定義（Rust）

### GitHubLabel（拡張）

```text
GitHubLabel {
  name: String        // ラベル名
  color: String       // 16進カラーコード（例: "d73a4a"）
}
```

### GitHubAssignee（新規）

```text
GitHubAssignee {
  login: String       // GitHub ユーザー名
  avatar_url: String  // アバター画像 URL
}
```

### GitHubMilestone（新規）

```text
GitHubMilestone {
  title: String       // マイルストーン名
  number: u32         // マイルストーン番号
}
```

### GitHubIssueInfo（拡張）

```text
GitHubIssueInfo {
  number: u32                           // Issue 番号
  title: String                         // タイトル
  body: Option<String>                  // 本文（Markdown）
  state: String                         // "open" | "closed"
  updated_at: String                    // ISO 8601 日時
  html_url: String                      // GitHub Issue URL
  labels: Vec<GitHubLabel>              // ラベル（色付き）
  assignees: Vec<GitHubAssignee>        // アサイニー一覧
  comments_count: u32                   // コメント数
  milestone: Option<GitHubMilestone>    // マイルストーン
}
```

### GitHubIssueDetail（新規）

```text
GitHubIssueDetail {
  // GitHubIssueInfo の全フィールド + 追加情報があれば
  // 現時点では GitHubIssueInfo と同一構造
}
```

## フロントエンド型定義（TypeScript）

### GitHubLabel（拡張）

```text
GitHubLabel {
  name: string
  color: string       // 16進カラーコード
}
```

### GitHubAssignee（新規）

```text
GitHubAssignee {
  login: string
  avatarUrl: string
}
```

### GitHubMilestone（新規）

```text
GitHubMilestone {
  title: string
  number: number
}
```

### GitHubIssueInfo（拡張）

```text
GitHubIssueInfo {
  number: number
  title: string
  body?: string
  state: "open" | "closed"
  updatedAt: string
  htmlUrl: string
  labels: GitHubLabel[]
  assignees: GitHubAssignee[]
  commentsCount: number
  milestone?: GitHubMilestone
}
```

### Tab type 拡張

```text
Tab.type に "issues" を追加
```

## データフロー

```text
Git メニュー → Issues クリック
  ↓
App.svelte: シングルトンタブ作成/フォーカス
  ↓
IssueListPanel: check_gh_cli_status → エラー or 一覧取得
  ↓
fetch_github_issues(project_path, page, per_page, state)
  ↓
gh issue list --json ... --state {state} --limit {per_page}
  ↓
Issue 一覧表示（フィルタ・無限スクロール）
  ↓
Issue クリック → fetch_github_issue_detail(project_path, number)
  ↓
詳細ビュー（GFM Markdown / IssueSpecPanel）
  ↓
「Work on this」→ AgentLaunchForm（プリフィル）
```
