> **Historical Status**: この closed SPEC は旧 Unity/C# 実装前提の履歴仕様である。未完了 task は旧 backlog の保存であり、現行の完了条件ではない。現行の local Git backend は `SPEC-1644` を参照する。

# Git 操作レイヤー

## Background

現在の gwt-core (Rust) は `std::process::Command` によるプロセス実行で git CLI を呼び出し、全 Git 操作を実装している。Unity 6 移行に伴い、これらの Git 操作を C# で再実装する必要がある。

対象となる操作は、Worktree の CRUD、ブランチ管理、Diff/Status 取得、コミット履歴、スタッシュ、クリーンアップ、バージョン履歴（タグ・changelog 生成）など、gwt の全 Git 機能に及ぶ。

gwt は bare リポジトリ + worktree アーキテクチャを前提としており、C# ラッパーもこのアーキテクチャを正しくサポートする必要がある。

### 非同期プロセス実行パターン

```csharp
// Rust: tokio::process::Command → C#: Process + UniTask
public async UniTask RunGitCommand(string args, CancellationToken ct)
{
    var psi = new ProcessStartInfo("git", args)
    {
        RedirectStandardOutput = true,
        RedirectStandardError = true,
        UseShellExecute = false,
        CreateNoWindow = true,
        WorkingDirectory = _workingDirectory
    };
    // ... UniTask でラップ
}
```

### Git 操作 UI

- 全 Git 操作はターミナル経由。エージェントが実行するか、ユーザーがプレーンターミナルで直接実行
- ゲームライクな Git UI は不要: 現行 gwt と同様、worktree でターミナルのみを起動する機能を継承

### Lead Git権限

- Leadはworktreeライフサイクル全体を自律的に操作する権限を持つ: worktree作成 → push → PR作成 → merge → worktree削除
- **禁止操作**: force push (`git push --force`) および rebase (`git rebase`) はLeadに許可しない
- これはLeadが安全にワークツリー管理を自動化するための権限設計であり、破壊的操作を明示的に排除する

### CI/CD 連携

- Worktree（デスク）単位で CI 状態を表示。GitHub Actions API でポーリング

### 再実装対象コマンド

| カテゴリ | コマンド |
|---------|---------|
| Worktree | `list_worktrees`, `create_worktree`, `delete_worktree`, worktree status tracking |
| Branch | `list_branches` (local/remote/worktree), `get_current_branch`, branch protection check |
| Diff/Status | `get_git_change_summary`, `get_branch_diff_files`, `get_file_diff`, `get_working_tree_status` |
| History | `get_branch_commits`, `get_stash_list`, `get_base_branch_candidates` |
| Cleanup | `cleanup_worktrees`, `cleanup_single_worktree`, `get_cleanup_pr_statuses`, `get_cleanup_settings` |
| Version | `list_project_versions`, `get_project_version_history`, `prefetch_version_history` |

### 主要データ型

| 型名 | フィールド |
|------|-----------|
| `Worktree` | path, branch, commit, status (Active/Locked/Prunable/Missing), is_main, has_changes, has_unpushed |
| `Branch` | name, commit, is_current, has_remote, upstream, ahead, behind, is_gone |
| `FileChange` | path, status (Added/Modified/Deleted/Renamed), old_path |
| `FileDiff` | file_path, hunks, additions, deletions |
| `CommitEntry` | hash, short_hash, author, date, message |
| `StashEntry` | index, message, branch, date |
| `WorkingTreeEntry` | path, index_status, worktree_status |
| `GitChangeSummary` | files_changed, insertions, deletions, untracked_count |

## Interview Notes

**gwt側Git操作の範囲:**
- gwt側のGit操作は**読み取り専用が主**（diff表示、コミット履歴表示、stash一覧、ブランチ状態取得）
- 書き込みGit操作（commit, push, merge等）はエージェントまたはユーザーがプレーンターミナルで直接実行
- gwt側の書き込み操作はworktree CRUD（作成・削除）とブランチ作成のみ
- これにより、Git lock contention（.git/index.lock等）のリスクを最小化

## User Stories

- **US-1** [P0]: プロジェクトを開くと全 worktree とブランチ情報が 2D スタジオに反映される
  - テスト: プロジェクトロード後、全 worktree がスタジオ内のデスクオブジェクトとして表示されること
  - テスト: 各 worktree のブランチ名・ステータスが正しく表示されること
- **US-2** [P0]: worktree の作成・削除が 2D スタジオから実行できる
  - テスト: 新規 worktree 作成後、デスクオブジェクトが追加されること
  - テスト: worktree 削除後、デスクオブジェクトが除去されること
- **US-3** [P0]: ブランチの変更状況（diff, commits, stash）を確認できる
  - テスト: worktree 選択時に変更ファイル一覧・差分が表示されること
  - テスト: コミット履歴がタイムライン表示されること
- **US-4** [P1]: 不要な worktree を一括クリーンアップできる
  - テスト: マージ済み PR の worktree が検出・一括削除されること
  - テスト: 保護ブランチの worktree は削除対象から除外されること

## Functional Requirements

- **FR-001**: git CLI をプロセス実行して全 Git 操作を行う C# ラッパーを実装する
- **FR-002**: Worktree の CRUD（作成・一覧・削除・ステータス取得）をサポートする
- **FR-003**: ブランチ一覧（ローカル・リモート・worktree）を取得できる
- **FR-004**: Diff/Status（変更サマリー、ファイル差分、ワーキングツリー状態）を取得できる
- **FR-005**: コミット履歴・スタッシュ一覧を取得できる
- **FR-006**: ベースブランチ候補を自動検出できる
- **FR-007**: Worktree クリーンアップ（PR ステータス連動、ブランチ保護チェック）をサポートする
- **FR-008**: バージョン履歴（タグ一覧、バージョン間の変更）を取得できる
- **FR-009**: VContainer で `IGitService` として DI 登録する
- **FR-010**: bare リポジトリ＋worktree のアーキテクチャをサポートする（プロジェクトマイグレーション含む）
- **FR-011**: Lead向けGit権限として、worktreeライフサイクル全体（作成→push→PR作成→merge→worktree削除）を許可し、force push/rebaseを禁止するアクセス制御を実装する

## Non-Functional Requirements

- **NFR-001**: git CLI 呼び出しは非同期（`async/await`）で実行し、Unity メインスレッドをブロックしない
- **NFR-002**: git コマンドのタイムアウトを設定可能にする（デフォルト 30 秒）
- **NFR-003**: git CLI が存在しない場合は起動時に検出し、ユーザーに通知する
- **NFR-004**: 大量の worktree（50+）でもパフォーマンスが劣化しないよう、並列取得を実装する
- **NFR-005**: エラーメッセージは git の stderr をそのまま伝搬し、デバッグ容易性を確保する
- **NFR-006**: 全ての Git コマンド実行は CancellationToken を受け取り、プロセスタイムアウト（デフォルト30秒）を設定する

## Success Criteria

- **SC-001**: macOS / Windows / Linux で git CLI 呼び出しが正しく動作する
- **SC-002**: 現在の gwt（Rust 版）と同等の Git 操作が全て実行可能
- **SC-003**: bare リポジトリの検出・操作が正しく動作する
- **SC-004**: 全 FR に対応するユニットテストが存在し、パスする
