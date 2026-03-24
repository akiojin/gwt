### 背景

現在の gwt は `gh` CLI を介して全 GitHub 操作を実行している。Unity 6 移行に伴い、これらの GitHub 操作を C# で再実装する必要がある。

対象は 30 以上のコマンドに及び、GitHub Issues の取得・リンク管理、Pull Request の一覧・マージ・レビュー、CI/CD ワークフローのステータス、Spec Issue（`gwt-spec` ラベル）の CRUD、認証状態の検出を含む。

gh CLI は外部依存であるため、未インストール・未認証時のフォールバックとガイダンス表示も重要な要件となる。

#### gh CLI ラッパーパターン

```csharp
public async UniTask> ListIssues(string repo, CancellationToken ct)
{
    var result = await RunGhCommand($"issue list --repo {repo} --json number,title,body", ct);
    return JsonConvert.DeserializeObject>(result.Stdout);
}
```

#### Lead PR操作権限

- LeadはPR作成・merge操作を自律的に実行する権限を持つ
- これは #1574（Lead自律ワークツリー管理）のワークフロー（worktree作成→push→PR作成→merge→worktree削除）の一環として必要
- PR作成時のタイトルフォーマット: `[Lead] {task.Title}`
- merge判定: `PrStatusInfo.Mergeable == "MERGEABLE"` の場合のみ実行

#### クラッシュレポート: GitHub Issue自動作成

- アプリケーションクラッシュ時にGitHub Issueを自動作成する機能を提供する
- **オプトイン方式**: ユーザーが明示的に有効化した場合のみ動作（デフォルトOFF）
- クラッシュログ・スタックトレース・環境情報を含むIssueを自動生成
- プライバシー配慮: 送信前にユーザーが内容を確認・編集できるUIを提供

#### 再実装対象コマンド

| カテゴリ | コマンド |
|---------|---------|
| Issues | `fetch_github_issues`, `fetch_github_issue_detail`, `fetch_branch_linked_issue`, `find_existing_issue_branch`, `find_existing_issue_branches_bulk`, `link_branch_to_issue`, `rollback_issue_branch`, `classify_issue_branch_prefix` |
| PRs | `fetch_pr_status`, `fetch_pr_detail`, `fetch_latest_branch_pr`, `fetch_ci_log`, `update_pr_branch`, `fetch_branch_pr_preflight`, `merge_pull_request`, `merge_pr`, `fetch_pr_list`, `fetch_github_user`, `review_pr`, `mark_pr_ready` |
| Spec Issues | `create_spec_issue_cmd`, `update_spec_issue_cmd`, `upsert_spec_issue_cmd`, `get_spec_issue_detail_cmd`, `append_spec_contract_comment_cmd`, `upsert_spec_issue_artifact_comment_cmd`, `list_spec_issue_artifact_comments_cmd`, `delete_spec_issue_artifact_comment_cmd`, `close_spec_issue_cmd`, `sync_spec_issue_project_cmd` |
| Auth | `check_gh_cli_status`, `check_gh_available` |
| Crash Report | `create_crash_report_issue` (新規) |

#### 主要データ型

| 型名 | フィールド |
|------|-----------|
| `GitHubIssueInfo` | number, title, body, state, labels, assignees, created_at, updated_at, url |
| `GitHubLabel` | name, color, description |
| `GitHubAssignee` | login, avatar_url |
| `PrStatusInfo` | number, title, state, branch, base, mergeable, review_decision, checks, url |
| `PrPreflightResult` | has_changes, is_pushed, has_remote, conflicts, ci_status |
| `ReviewInfo` | author, state, body, submitted_at |
| `ReviewComment` | author, body, path, line, created_at |
| `WorkflowRunInfo` | id, name, status, conclusion, url, created_at |
| `FetchIssuesResponse` | issues, total_count, has_next_page |
| `FetchPrListResponse` | prs, total_count, has_next_page |
| `CrashReportPayload` | title, body, labels, stack_trace, environment_info, user_notes |

#### インタビュー確定事項（2026-03-10追記）

**アプリ内認証ガイド:**
- gh CLI未認証時のガイダンスは**アプリ内ガイド**として実装
- 外部ドキュメントへのリンクではなく、アプリ内でステップバイステップの `gh auth login` 手順を表示
- ターミナルオーバーレイで直接 `gh auth login` を実行可能にする

### ユーザーシナリオ

- **US-1** [P0]: 2D スタジオ内で GitHub Issues が浮遊マーカーとして表示され、クリックで詳細が見られる
  - テスト: Issues 一覧がスタジオ内に浮遊マーカーとして配置されること
  - テスト: Issue クリックで title, body, labels, assignees が表示されること
- **US-2** [P0]: PR の状態（マージ可能、コンフリクト、レビュー待ち）が視覚的に確認できる
  - テスト: PR バッジがステータスに応じた色・アニメーションで表示されること
  - テスト: CI チェック状態が視覚的にわかること
- **US-3** [P0]: 2D スタジオから PR のマージ・レビュー操作ができる
  - テスト: マージ操作実行後、PR の state が merged に変わること
  - テスト: レビュー送信後、review_decision が更新されること
- **US-4** [P1]: Issue からブランチ・worktree を作成できる
  - テスト: Issue 選択→ブランチ名自動生成→worktree 作成の一連のフローが動作すること
  - テスト: 作成されたブランチが Issue にリンクされること
- **US-5** [P1]: PR 作成前のプリフライトチェック（変更有無、push 済み判定、コンフリクト検出）が実行される
  - テスト: 未 push のブランチでプリフライトが警告を返すこと
  - テスト: コンフリクトがある場合にプリフライトが検出すること
- **US-6** [P1]: クラッシュ発生時にオプトインでGitHub Issueが自動作成される
  - テスト: オプトイン設定OFF時にIssueが作成されないこと
  - テスト: オプトイン設定ON時にクラッシュレポートIssueが正しいフォーマットで作成されること
  - テスト: 送信前にユーザーが内容を確認・編集できること

### 機能要件

- **FR-001**: gh CLI をプロセス実行して全 GitHub 操作を行う C# ラッパーを実装する
- **FR-002**: GitHub Issues の取得・詳細表示・ブランチリンクをサポートする
- **FR-003**: Pull Request の一覧・詳細・マージ・レビュー・ステータス更新をサポートする
- **FR-004**: CI/CD ワークフローのステータス取得・ログ閲覧をサポートする
- **FR-005**: Spec Issue の CRUD（`gwt-spec` ラベル管理）をサポートする
- **FR-006**: gh CLI の認証状態を検出・表示する
- **FR-007**: Issue-ブランチ間のリンク管理をサポートする
- **FR-008**: PR のマージ可能性判定（コンフリクト、レビュー、CI チェック）を表示する
- **FR-009**: VContainer で `IGitHubService` として DI 登録する
- **FR-010**: PR 作成前のプリフライトチェック（変更有無、push 済み判定、リモート存在確認、コンフリクト検出、CI ステータス）をサポートする
- **FR-011**: Lead向けPR操作（PR作成・merge）を自律的に実行可能にする（#1574 Lead自律ワークツリー管理と連携）
- **FR-012**: クラッシュレポートのGitHub Issue自動作成機能をオプトインで提供する（クラッシュログ・スタックトレース・環境情報を含む）

### 非機能要件

- **NFR-001**: gh CLI 呼び出しは非同期（`async/await`）で実行し、Unity メインスレッドをブロックしない
- **NFR-002**: GitHub API のレートリミットを考慮し、キャッシュ戦略を実装する（TTL: Issues 60秒、PR ステータス 30秒）
- **NFR-003**: gh CLI 未インストール時はグレースフルデグラデーション（Git 操作のみ有効）を行う
- **NFR-004**: 未認証時のガイダンスメッセージをユーザーに提示する
- **NFR-005**: ページネーション対応（大量 Issue/PR リポジトリでの安定動作）

### 成功基準

- **SC-001**: 現在の gwt（Rust 版）と同等の GitHub 操作が全て実行可能
- **SC-002**: gh CLI 未認証時に適切なガイダンスを表示する
- **SC-003**: 2D スタジオ内で Issue/PR が視覚的に表現される
- **SC-004**: 全 FR に対応するユニットテストが存在し、パスする
- **SC-005**: PR プリフライトチェックが正しく動作する
- **SC-006**: LeadからのPR作成・merge操作が正しく動作する
- **SC-007**: クラッシュレポートIssue自動作成がオプトイン設定に従って動作する
