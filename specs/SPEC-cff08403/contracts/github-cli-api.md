# GitHub CLI API契約

**仕様ID**: `SPEC-cff08403` | **日付**: 2025-10-25

## 概要

自動マージワークフローで使用するGitHub CLI（`gh`）コマンドのAPI契約を定義します。

## 前提条件

### 認証

```yaml
env:
  GH_TOKEN: ${{ github.token }}
```

GitHub Actionsの`github.token`を使用して自動的に認証されます。

### 必要な権限

- `contents: write`: ブランチへの書き込み
- `pull-requests: write`: PRのマージ

## コマンド仕様

### 1. PR一覧取得

#### コマンド

```bash
gh pr list --head <branch-name> --json number --jq '.[0].number'
```

#### パラメータ

- `--head <branch-name>`: ソースブランチ名で絞り込み
- `--json number`: JSON形式で`number`フィールドのみ取得
- `--jq '.[0].number'`: 最初のPRの番号を抽出

#### 出力形式

**成功時（PRが存在）**:
```
123
```

**PRが存在しない場合**:
```
(空文字列)
```

#### エラーケース

- **ネットワークエラー**: `exit 1`で終了、エラーメッセージを標準エラー出力
- **認証エラー**: `exit 1`で終了、認証失敗メッセージ

#### 使用例

```bash
PR_NUMBER=$(gh pr list --head feature/auto-merge --json number --jq '.[0].number')
if [ -z "$PR_NUMBER" ]; then
  echo "No PR found for branch feature/auto-merge"
  exit 0
fi
```

### 2. PR詳細取得

#### コマンド

```bash
gh pr view <pr-number> --json <fields>
```

#### パラメータ

- `<pr-number>`: PR番号
- `--json <fields>`: 取得するフィールド（カンマ区切り）

#### 取得フィールド

```bash
gh pr view $PR_NUMBER --json mergeable,mergeStateStatus,isDraft
```

| フィールド | 型 | 値 | 説明 |
|-----------|-----|-----|------|
| `mergeable` | string | `MERGEABLE`, `CONFLICTING`, `UNKNOWN` | マージ可能性 |
| `mergeStateStatus` | string | `CLEAN`, `UNSTABLE`, `BLOCKED`, `DIRTY`, `UNKNOWN` | マージ状態 |
| `isDraft` | boolean | `true`, `false` | ドラフトPRかどうか |

#### 出力形式

```json
{
  "mergeable": "MERGEABLE",
  "mergeStateStatus": "CLEAN",
  "isDraft": false
}
```

#### 使用例

```bash
PR_DATA=$(gh pr view 123 --json mergeable,mergeStateStatus,isDraft)
MERGEABLE=$(echo "$PR_DATA" | jq -r '.mergeable')
MERGE_STATE=$(echo "$PR_DATA" | jq -r '.mergeStateStatus')
IS_DRAFT=$(echo "$PR_DATA" | jq -r '.isDraft')

if [ "$MERGEABLE" != "MERGEABLE" ]; then
  echo "PR is not mergeable"
  exit 0
fi

if [ "$IS_DRAFT" = "true" ]; then
  echo "PR is a draft"
  exit 0
fi
```

### 3. PRマージ

#### コマンド

```bash
gh pr merge <pr-number> --merge --auto
```

#### パラメータ

- `<pr-number>`: マージするPR番号
- `--merge`: Merge commitを使用（デフォルト：squash）
- `--auto`: 条件が満たされたら自動的にマージ

#### マージ方法の違い

| オプション | 説明 | コミット履歴 |
|-----------|------|-------------|
| `--merge` | Merge commit | すべてのコミットを保持 |
| `--squash` | Squash and merge | 1つのコミットにまとめる |
| `--rebase` | Rebase and merge | リニアな履歴 |

**自動マージでは**: `--merge`を使用（仕様要件）

#### 出力形式

**成功時**:
```
✓ Merged pull request #123 (feature-branch)
```

**失敗時**:
```
! Failed to merge pull request #123: <reason>
```

#### エラーケース

| エラー | 原因 | 対処 |
|-------|------|------|
| `pull request already merged` | 既にマージ済み | スキップ（正常終了） |
| `merge conflict` | マージ競合 | スキップ（正常終了） |
| `required status checks failed` | CIチェック失敗 | スキップ（正常終了） |
| `insufficient permissions` | 権限不足 | エラー終了 |

#### 使用例

```bash
echo "Auto-merging PR #$PR_NUMBER"
if gh pr merge $PR_NUMBER --merge --auto; then
  echo "✓ Successfully merged PR #$PR_NUMBER"
else
  EXIT_CODE=$?
  echo "✗ Failed to merge PR #$PR_NUMBER (exit code: $EXIT_CODE)"
  exit 1
fi
```

## データ型定義

### PRの状態

#### mergeable

```typescript
type Mergeable =
  | "MERGEABLE"     // マージ可能
  | "CONFLICTING"   // 競合あり
  | "UNKNOWN";      // 不明
```

#### mergeStateStatus

```typescript
type MergeStateStatus =
  | "CLEAN"         // すべてOK、マージ可能
  | "UNSTABLE"      // 一部チェック失敗だがマージ可能
  | "BLOCKED"       // ブロックされている
  | "DIRTY"         // 競合あり
  | "UNKNOWN";      // 不明
```

## レート制限

### GitHub API制限

- **認証済み（GitHub Actions）**: 1,000リクエスト/時
- **通常使用量**: PRごとに2-3リクエスト
  - PR一覧取得: 1回
  - PR詳細取得: 1回
  - PRマージ: 1回

**見積もり**: 1時間に300PR程度まで処理可能（実際には十分）

### リトライ戦略

レート制限エラーは通常発生しないため、リトライは実装しません。エラー時は失敗として扱います。

## セキュリティ

### トークンの扱い

```yaml
env:
  GH_TOKEN: ${{ github.token }}
```

- ワークフロー実行中のみ有効
- ログに出力されない
- 最小権限で動作

### ログ出力

機密情報をログに出力しないように注意：

```bash
# OK: PR番号、ブランチ名は公開情報
echo "Processing PR #$PR_NUMBER from branch $BRANCH_NAME"

# NG: トークンを出力しない
echo "Token: $GH_TOKEN"  # 絶対にしない
```

## エラーハンドリング

### 推奨パターン

```bash
# PR一覧取得
PR_NUMBER=$(gh pr list --head "$BRANCH" --json number --jq '.[0].number')
if [ -z "$PR_NUMBER" ]; then
  echo "No PR found for branch $BRANCH"
  exit 0  # 正常終了（スキップ）
fi

# PR詳細取得
if ! PR_DATA=$(gh pr view "$PR_NUMBER" --json mergeable,mergeStateStatus,isDraft); then
  echo "Failed to get PR details"
  exit 1  # エラー終了
fi

# PRマージ
if ! gh pr merge "$PR_NUMBER" --merge --auto; then
  echo "Failed to merge PR"
  exit 1  # エラー終了
fi
```

## テスト方法

### ローカルテスト

GitHub CLI を使用してローカルでテスト可能：

```bash
# 認証
gh auth login

# テスト
export GH_TOKEN=$(gh auth token)
gh pr list --head feature/test --json number
gh pr view 123 --json mergeable,mergeStateStatus,isDraft
```

### CI/CDテスト

実際のPRを作成してワークフローをトリガーし、動作を確認します。

## 参考資料

- [GitHub CLI: Manual](https://cli.github.com/manual/)
- [GitHub CLI: pr list](https://cli.github.com/manual/gh_pr_list)
- [GitHub CLI: pr view](https://cli.github.com/manual/gh_pr_view)
- [GitHub CLI: pr merge](https://cli.github.com/manual/gh_pr_merge)
