# クイックスタート: PR自動マージ機能

**仕様ID**: `SPEC-cff08403` | **日付**: 2025-10-25

## 概要

PR自動マージ機能の開発、テスト、デプロイ方法を説明します。この機能は、CIが成功し競合がない場合に、すべてのPRを自動的にMerge commitでマージします。

## 前提条件

### 必要な環境

- GitHub Actions が有効化されているリポジトリ
- 既存のCIワークフロー（Test、Lint）が正常に動作している
- リポジトリへの書き込み権限

### 必要な知識

- GitHub Actionsの基本的な知識
- YAMLの記述方法
- GitHub CLIの基本的な使用方法

## セットアップ

### 1. ワークフローファイルの配置

`.github/workflows/auto-merge.yml`を作成します：

```yaml
name: Auto Merge

on:
  workflow_run:
    workflows: ["Test", "Lint"]
    types:
      - completed

jobs:
  auto-merge:
    name: Auto Merge PR
    runs-on: ubuntu-latest
    if: github.event.workflow_run.conclusion == 'success' && github.event.workflow_run.event == 'pull_request'

    permissions:
      contents: write
      pull-requests: write

    steps:
      - name: Checkout code
        uses: actions/checkout@v4

      - name: Get PR number
        id: pr
        run: |
          PR_NUMBER=$(gh pr list --head ${{ github.event.workflow_run.head_branch }} --json number --jq '.[0].number')
          if [ -z "$PR_NUMBER" ]; then
            echo "No PR found for branch"
            exit 0
          fi
          echo "number=$PR_NUMBER" >> $GITHUB_OUTPUT
        env:
          GH_TOKEN: ${{ github.token }}

      - name: Check merge conditions
        if: steps.pr.outputs.number
        id: check
        run: |
          PR_NUMBER=${{ steps.pr.outputs.number }}
          PR_DATA=$(gh pr view $PR_NUMBER --json mergeable,mergeStateStatus,isDraft)

          MERGEABLE=$(echo "$PR_DATA" | jq -r '.mergeable')
          MERGE_STATE=$(echo "$PR_DATA" | jq -r '.mergeStateStatus')
          IS_DRAFT=$(echo "$PR_DATA" | jq -r '.isDraft')

          if [ "$IS_DRAFT" = "true" ]; then
            echo "PR is a draft, skipping"
            exit 0
          fi

          if [ "$MERGEABLE" != "MERGEABLE" ]; then
            echo "PR has conflicts, skipping"
            exit 0
          fi

          if [ "$MERGE_STATE" != "CLEAN" ] && [ "$MERGE_STATE" != "UNSTABLE" ]; then
            echo "Merge state is not clean: $MERGE_STATE"
            exit 0
          fi

          echo "ready=true" >> $GITHUB_OUTPUT
        env:
          GH_TOKEN: ${{ github.token }}

      - name: Merge PR
        if: steps.check.outputs.ready == 'true'
        run: |
          PR_NUMBER=${{ steps.pr.outputs.number }}
          echo "Auto-merging PR #$PR_NUMBER"
          gh pr merge $PR_NUMBER --merge --auto
        env:
          GH_TOKEN: ${{ github.token }}
```

### 2. コミットとプッシュ

```bash
git add .github/workflows/auto-merge.yml
git commit -m "feat: PR自動マージ機能を追加"
git push origin main
```

## 開発ワークフロー

### ローカルでの検証

GitHub Actionsワークフローはローカルで実行できないため、実際のPRを使用してテストします。

### テストPRの作成

#### 1. テストブランチの作成

```bash
git checkout -b test/auto-merge
```

#### 2. テスト用の変更

簡単な変更を加えます（例：READMEにコメント追加）：

```bash
echo "<!-- Test auto-merge -->" >> README.md
git add README.md
git commit -m "test: 自動マージ機能のテスト"
git push origin test/auto-merge
```

#### 3. PRの作成

```bash
gh pr create --title "test: 自動マージ機能のテスト" --body "CIが成功したら自動的にマージされるはずです"
```

#### 4. 動作確認

1. GitHub ActionsでTest、Lintワークフローが実行される
2. すべてのCIが成功する
3. Auto Mergeワークフローが起動する
4. PRが自動的にマージされる

#### 5. ログの確認

GitHub Actionsのワークフローログで以下を確認：

- PRが正しく検出されたか
- マージ条件チェックが正しく動作したか
- マージが成功したか

## よくある操作

### 自動マージをスキップする方法

#### ドラフトPRとして作成

```bash
gh pr create --draft --title "WIP: 作業中の機能" --body "ドラフトPRは自動マージされません"
```

#### 競合を意図的に作成

ベースブランチと競合する変更を加えることで、自動マージをスキップできます。

### 自動マージの状態確認

#### PRの状態を確認

```bash
gh pr view <pr-number> --json mergeable,mergeStateStatus,isDraft
```

#### ワークフローの実行履歴を確認

```bash
gh run list --workflow="Auto Merge"
```

### 特定のワークフローを再実行

```bash
gh run rerun <run-id>
```

## トラブルシューティング

### PRが自動マージされない

#### チェック1: CIが成功しているか

```bash
gh pr checks <pr-number>
```

すべてのチェックが成功していることを確認。

#### チェック2: PRに競合がないか

```bash
gh pr view <pr-number> --json mergeable
```

`MERGEABLE`が返されることを確認。

#### チェック3: ドラフトPRでないか

```bash
gh pr view <pr-number> --json isDraft
```

`false`が返されることを確認。

#### チェック4: ワークフローのログを確認

GitHub Actionsのワークフローログで、Auto Mergeワークフローが実行されているか、どのステップでスキップされたかを確認。

### 権限エラー

#### エラーメッセージ

```
Error: HTTP 403: Resource not accessible by integration
```

#### 原因

ワークフローに必要な権限が付与されていない。

#### 解決方法

ワークフローファイルに権限を追加：

```yaml
permissions:
  contents: write
  pull-requests: write
```

### ワークフローがトリガーされない

#### 原因1: ブランチ設定

自動マージワークフローは、`main`と`develop`ブランチへのPRのみ対象です。

#### 原因2: workflow_runイベントの設定ミス

`workflows`配列のワークフロー名が正しいか確認：

```yaml
workflows: ["Test", "Lint"]  # 正確なワークフロー名
```

## パフォーマンス

### 期待される動作時間

- CI完了からワークフロートリガーまで: 数秒〜数十秒
- PR情報取得: 1秒以内
- マージ実行: 1秒以内

**合計**: CIワークフロー完了から1分以内にマージ完了（目標5分以内を大幅にクリア）

## セキュリティ

### ブランチ保護ルールとの併用

自動マージ機能は、ブランチ保護ルールを尊重します。以下の設定を推奨：

1. **必須ステータスチェック**: Test、Lintを必須に設定
2. **レビュー承認**: 必要に応じて設定
3. **管理者による強制プッシュの禁止**: 有効化

### フォークからのPRの扱い

`workflow_run`イベントは、フォークからのPRに対しても本リポジトリの権限で実行されます。ブランチ保護ルールを適切に設定することで、不適切なマージを防止できます。

## 次のステップ

### Phase 2: タスク生成

実装タスクを生成するために、以下のコマンドを実行：

```bash
/speckit.tasks
```

### Phase 3: 実装

タスクリストに従って実装を開始：

```bash
/speckit.implement
```

## 参考資料

- [research.md](./research.md): 技術調査
- [data-model.md](./data-model.md): データモデル定義
- [contracts/](./contracts/): API契約
- [GitHub Actions公式ドキュメント](https://docs.github.com/en/actions)
- [GitHub CLI公式ドキュメント](https://cli.github.com/manual/)
