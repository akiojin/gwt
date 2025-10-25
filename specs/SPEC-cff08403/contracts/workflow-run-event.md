# GitHub Actions workflow_run イベント契約

**仕様ID**: `SPEC-cff08403` | **日付**: 2025-10-25

## 概要

`workflow_run`イベントは、指定したワークフローが完了したときにトリガーされます。このドキュメントは、自動マージワークフローが依存するイベントペイロードの構造を定義します。

## イベントトリガー設定

```yaml
on:
  workflow_run:
    workflows: ["Test", "Lint"]
    types:
      - completed
    branches:
      - main
      - develop
```

## イベントペイロード構造

### 主要フィールド

```yaml
github.event.workflow_run:
  # ワークフロー実行結果
  conclusion: string  # "success" | "failure" | "cancelled" | "skipped" | "timed_out" | "action_required" | null

  # ワークフロー情報
  name: string       # ワークフロー名（例: "Test", "Lint"）
  event: string      # トリガーイベント（例: "pull_request", "push"）

  # ブランチ情報
  head_branch: string  # ソースブランチ名（例: "feature/auto-merge"）
  head_sha: string     # コミットSHA（例: "abc123..."）

  # その他
  id: number          # ワークフロー実行ID
  run_number: number  # ワークフロー実行番号
  created_at: string  # 作成日時（ISO 8601形式）
  updated_at: string  # 更新日時（ISO 8601形式）
```

## 使用例

### 1. ワークフロー成功確認

```yaml
if: github.event.workflow_run.conclusion == 'success'
```

**値**:
- `success`: ワークフロー成功
- `failure`: ワークフロー失敗
- `cancelled`: キャンセル
- `skipped`: スキップ
- `timed_out`: タイムアウト
- `action_required`: アクション必要
- `null`: 結論未確定

**自動マージの条件**: `conclusion == 'success'`のみ

### 2. PRイベント確認

```yaml
if: github.event.workflow_run.event == 'pull_request'
```

**値**:
- `pull_request`: PRに対する実行
- `push`: プッシュに対する実行
- その他: その他のイベント

**自動マージの条件**: `event == 'pull_request'`のみ

### 3. ブランチ名取得

```yaml
steps:
  - name: Get PR number
    run: |
      PR_NUMBER=$(gh pr list --head ${{ github.event.workflow_run.head_branch }} --json number --jq '.[0].number')
```

**使用目的**: ブランチ名からPR番号を特定

## 実際のペイロード例

### 成功したワークフロー

```json
{
  "workflow_run": {
    "conclusion": "success",
    "name": "Test",
    "event": "pull_request",
    "head_branch": "feature/auto-merge",
    "head_sha": "a1b2c3d4e5f6",
    "id": 12345678,
    "run_number": 42,
    "created_at": "2025-10-25T12:00:00Z",
    "updated_at": "2025-10-25T12:05:00Z"
  }
}
```

### 失敗したワークフロー

```json
{
  "workflow_run": {
    "conclusion": "failure",
    "name": "Lint",
    "event": "pull_request",
    "head_branch": "feature/auto-merge",
    "head_sha": "a1b2c3d4e5f6",
    "id": 12345679,
    "run_number": 43,
    "created_at": "2025-10-25T12:00:00Z",
    "updated_at": "2025-10-25T12:03:00Z"
  }
}
```

## 制約事項

### トリガータイミング

- ワークフローが完了した直後にトリガー
- 遅延は通常数秒〜数十秒程度
- ワークフローが失敗した場合でもトリガーされる（`conclusion`で判別）

### ブランチフィルター

- `branches`で指定したブランチへのPRのみ対象
- フォークからのPRも含まれる（権限に注意）

## エラーケース

### 1. conclusionがnull

**発生条件**: ワークフローがまだ実行中または状態不明

**対処**: 自動マージをスキップ

### 2. head_branchが空

**発生条件**: 極めて稀（API異常）

**対処**: PR番号取得失敗として扱い、スキップ

## セキュリティ考慮事項

### フォークからのPR

- `workflow_run`イベントは、フォークからのPRに対しても本リポジトリの権限で実行される
- `pull_request`イベントとは異なり、書き込み権限を持つ
- 自動マージ機能では、ブランチ保護ルールによってフォークからのPRを制御することを推奨

### シークレットへのアクセス

- `workflow_run`でトリガーされたワークフローは、リポジトリのシークレットにアクセス可能
- 自動マージワークフローでは、`GITHUB_TOKEN`のみ使用し、追加のシークレットは使用しない

## 参考資料

- [GitHub Actions: workflow_run イベント](https://docs.github.com/en/actions/using-workflows/events-that-trigger-workflows#workflow_run)
- [GitHub Actions: Contexts](https://docs.github.com/en/actions/learn-github-actions/contexts#github-context)
