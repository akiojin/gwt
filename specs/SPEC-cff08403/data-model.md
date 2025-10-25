# データモデル: PR自動マージ機能

**仕様ID**: `SPEC-cff08403` | **日付**: 2025-10-25 | **フェーズ**: Phase 1

## 概要

PR自動マージ機能で使用する主要なデータモデルを定義します。このシステムはGitHub APIが提供するデータを使用し、独自のデータストレージは持ちません。

## エンティティ定義

### 1. PullRequest（プルリクエスト）

PRの状態とマージ可能性を表現するモデル。

**属性**:

| 属性名 | 型 | 説明 | 取得元 |
|--------|-----|------|--------|
| `number` | integer | PR番号 | GitHub API |
| `state` | enum | PRの状態（OPEN, CLOSED, MERGED） | GitHub API |
| `isDraft` | boolean | ドラフトPRかどうか | GitHub API |
| `mergeable` | enum | マージ可能性（MERGEABLE, CONFLICTING, UNKNOWN） | GitHub API |
| `mergeStateStatus` | enum | マージ状態（CLEAN, UNSTABLE, BLOCKED, DIRTY, UNKNOWN） | GitHub API |
| `headBranch` | string | PRのソースブランチ名 | GitHub API |
| `baseBranch` | string | PRのターゲットブランチ名 | GitHub API |

**状態遷移**:

```
[作成] → OPEN
OPEN → MERGED (自動マージ成功)
OPEN → CLOSED (手動クローズまたはマージ失敗)
```

**検証ルール**:

- `state == OPEN` : マージ対象
- `isDraft == false` : ドラフトPRは除外
- `mergeable == MERGEABLE` : 競合なし
- `mergeStateStatus in [CLEAN, UNSTABLE]` : 全体的にマージ可能

### 2. WorkflowRun（ワークフロー実行）

CIワークフローの実行結果を表現するモデル。

**属性**:

| 属性名 | 型 | 説明 | 取得元 |
|--------|-----|------|--------|
| `workflowName` | string | ワークフロー名（"Test", "Lint"） | GitHub Actions event |
| `conclusion` | enum | 実行結果（SUCCESS, FAILURE, CANCELLED, SKIPPED） | GitHub Actions event |
| `event` | string | トリガーイベント種別（"pull_request", "push"） | GitHub Actions event |
| `headBranch` | string | 実行対象のブランチ名 | GitHub Actions event |
| `headSha` | string | 実行対象のコミットSHA | GitHub Actions event |

**検証ルール**:

- `conclusion == SUCCESS` : ワークフローが成功
- `event == "pull_request"` : PRに対する実行

**複数ワークフローの状態**:

GitHub APIの`mergeStateStatus`が全ワークフローの結果を集約するため、個別の追跡は不要。最後に完了したワークフローの時点で、PRの`mergeStateStatus`がすべてのチェック結果を反映する。

### 3. MergeDecision（マージ判定）

自動マージの実行可否を決定するための判定モデル。

**属性**:

| 属性名 | 型 | 説明 | 計算方法 |
|--------|-----|------|----------|
| `shouldMerge` | boolean | マージを実行すべきか | 以下の条件すべてを満たす |
| `skipReason` | string? | スキップする理由（該当時のみ） | 条件チェック結果 |

**判定ロジック**:

```
shouldMerge = true IF:
  1. WorkflowRun.conclusion == SUCCESS
  2. WorkflowRun.event == "pull_request"
  3. PullRequest is found (number is not null)
  4. PullRequest.state == OPEN
  5. PullRequest.isDraft == false
  6. PullRequest.mergeable == MERGEABLE
  7. PullRequest.mergeStateStatus in [CLEAN, UNSTABLE]

ELSE:
  shouldMerge = false
  skipReason = <specific reason>
```

**skipReasonの値**:

- `"workflow_failed"`: CIワークフローが失敗
- `"not_pull_request"`: PRイベントではない
- `"pr_not_found"`: 対象PRが見つからない
- `"pr_is_draft"`: ドラフトPR
- `"pr_has_conflicts"`: マージ競合あり
- `"merge_blocked"`: マージがブロックされている
- `null`: マージ実行

## データフロー

### 1. ワークフロー完了時

```
[CI Workflow] → [workflow_run event] → [Auto-merge workflow]
   Test/Lint         ↓                          ↓
                 WorkflowRun                    ↓
                 (conclusion)             PullRequest取得
                                               ↓
                                          MergeDecision
                                               ↓
                                     shouldMerge == true?
                                         ↙         ↘
                                     Yes          No
                                      ↓            ↓
                        gh api graphql mergePullRequest    Log + Skip
```

### 2. PRデータ取得フロー

```
[workflow_run.head_branch]
         ↓
   gh pr list --head <branch>
         ↓
   PullRequest.number
         ↓
   gh pr view <number> --json <fields>
         ↓
   PullRequest (full data)
         ↓
   MergeDecision
```

## API契約

### GitHub CLI出力形式

#### PR一覧取得

```bash
gh pr list --head <branch> --json number
```

**出力形式**:
```json
[
  {
    "number": 123
  }
]
```

#### PR詳細取得

```bash
gh pr view <number> --json mergeable,mergeStateStatus,isDraft
```

**出力形式**:
```json
{
  "mergeable": "MERGEABLE",
  "mergeStateStatus": "CLEAN",
  "isDraft": false
}
```

### GitHub Actions Eventペイロード

#### workflow_run

```yaml
github.event.workflow_run:
  conclusion: "success"
  event: "pull_request"
  head_branch: "feature/my-feature"
  head_sha: "abc123..."
```

## エッジケースとデータ処理

### 1. PR未発見時

**条件**: `gh pr list`が空の結果を返す

**処理**:
```bash
if [ -z "$PR_NUMBER" ]; then
  echo "No PR found for branch"
  exit 0
fi
```

**理由**: PRが既にマージまたはクローズされた可能性

### 2. マージ状態が不明（UNKNOWN）

**条件**: `mergeable == UNKNOWN` または `mergeStateStatus == UNKNOWN`

**処理**: スキップ（安全側に倒す）

**理由**: GitHub APIがまだ状態を計算中、または一時的なエラー

### 3. 複数PRが同じブランチに存在

**条件**: `gh pr list --head <branch>`が複数のPRを返す

**処理**: 最初のPR（`.[0].number`）を使用

**理由**: 通常は1つのブランチに1つのオープンPRのみ存在

## データ永続性

このシステムはステートレスであり、データを永続化しません：

- すべてのデータはGitHub APIから動的に取得
- ワークフロー実行ごとに状態を再評価
- 履歴はGitHub Actionsのワークフローログに記録

## セキュリティ考慮事項

### 機密データ

このシステムで扱うデータに機密情報は含まれません：

- PR番号、ブランチ名、マージ状態はすべて公開情報
- GitHub Actionsトークンは環境変数として安全に管理
- ログに機密情報を出力しない

### データアクセス制御

- GitHub Actionsトークンは自動的に有効期限が設定される
- 必要最小限の権限（`contents: write`, `pull-requests: write`）のみ使用
- ブランチ保護ルールによる追加の保護

## 次のステップ

データモデル定義完了。次は：
- contracts/: 詳細なAPI契約定義
- quickstart.md: 開発者向けガイド
