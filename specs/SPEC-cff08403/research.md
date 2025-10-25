# 技術調査: PR自動マージ機能

**仕様ID**: `SPEC-cff08403` | **日付**: 2025-10-25 | **フェーズ**: Phase 0

## 調査概要

PR自動マージ機能の実装に必要な技術選択と既存コードベースとの統合方法を調査しました。

## 1. 既存のコードベース分析

### 1.1 既存CIワークフローの構造

#### Test ワークフロー (`.github/workflows/test.yml`)

**トリガー**:
- `push`: main、developブランチ
- `pull_request`: main、developブランチ

**ジョブ構成**:
1. `test`: Bunを使用した複数Node.jsバージョンでのテスト（18.x、20.x、22.x）
   - type-check、lint、test、coverage
2. `test-node`: Node.jsでのテスト
3. `build`: プロジェクトのビルド検証

#### Lint ワークフロー (`.github/workflows/lint.yml`)

**トリガー**:
- `push`: main、developブランチ
- `pull_request`: main、developブランチ

**ジョブ構成**:
1. `lint`: ESLint、Prettier、TypeScript type check
2. `commitlint`: コミットメッセージのlint（PRのみ）
3. `markdownlint`: Markdownファイルのlint

### 1.2 workflow_runトリガーパターンの決定

**選択**: `workflow_run`トリガー

**理由**:
- 他のワークフローの完了を検知できる
- PRコンテキストを取得可能
- 複数ワークフローの完了を個別に処理可能

**設定**:
```yaml
on:
  workflow_run:
    workflows: ["Test", "Lint"]
    types:
      - completed
```

### 1.3 ブランチ保護ルールとの統合

**想定**:
- ブランチ保護ルールが設定されている場合でも、必要な権限があれば自動マージ可能
- 保護ルールの要件（レビュー承認、ステータスチェック）は別途設定が必要

**対応**:
- 自動マージワークフローは保護ルールを回避せず、遵守する
- 必要な権限: `contents: write`, `pull-requests: write`

## 2. 技術的決定

### 2.1 GitHub CLI vs GitHub API

**決定**: GitHub CLI（gh）を主要ツールとして使用

**理由**:
- シンプルなコマンド構文
- 認証がGitHub Actionsトークンで自動処理
- PR情報取得とマージ操作が一貫したインターフェース
- JSON出力でのデータ取得が容易

**代替案**:
- GitHub API (REST/GraphQL): より細かい制御が可能だが、複雑性が増す

**使用例**:
```bash
# PR情報取得
gh pr view $PR_NUMBER --json mergeable,mergeStateStatus

# PRマージ
gh pr merge $PR_NUMBER --merge --auto
```

### 2.2 マージ状態の確認方法

**決定**: `mergeable`と`mergeStateStatus`の両方を確認

**詳細**:
- `mergeable`: PR自体がマージ可能か（競合なし）
  - `MERGEABLE`: マージ可能
  - `CONFLICTING`: 競合あり
  - `UNKNOWN`: 状態不明
- `mergeStateStatus`: マージ条件の全体的な状態
  - `CLEAN`: すべてのチェック成功、マージ可能
  - `UNSTABLE`: 一部チェック失敗だが設定によりマージ可能
  - `BLOCKED`: ブロックされている
  - `DIRTY`: 競合あり

**実装ロジック**:
```yaml
if [ "$MERGEABLE" != "MERGEABLE" ]; then
  exit 0  # スキップ
fi

if [ "$MERGE_STATE" != "CLEAN" ] && [ "$MERGE_STATE" != "UNSTABLE" ]; then
  exit 0  # スキップ
fi
```

### 2.3 PRの特定方法

**決定**: `workflow_run`イベントの`head_branch`を使用

**手順**:
1. `workflow_run.head_branch`からブランチ名を取得
2. `gh pr list --head <branch>`でPR番号を取得
3. PR番号が見つからない場合は正常終了（既にクローズまたはマージ済み）

**実装**:
```bash
PR_NUMBER=$(gh pr list --head ${{ github.event.workflow_run.head_branch }} --json number --jq '.[0].number')
```

### 2.4 エラーハンドリング戦略

**決定**: 段階的なチェックと明示的なログ出力

**レベル分け**:
1. **情報レベル**: PR未発見、既にマージ済み → 正常終了（exit 0）
2. **スキップレベル**: CI失敗、競合あり、ドラフトPR → スキップログを出力して正常終了
3. **エラーレベル**: API障害、権限不足 → エラーログを出力して失敗（exit 1）

**ログ出力方針**:
- すべての判定理由を明確にログに記録
- GitHub Actionsのステップ名で処理内容を明示
- `echo`コマンドでワークフローログに詳細を出力

## 3. 制約と依存関係への対応

### 3.1 GitHub APIレート制限

**制約**:
- GitHub Actionsからの呼び出しは比較的高いレート制限（1000リクエスト/時）
- `workflow_run`トリガーはPRごとに1回のみ実行

**対応**:
- 最小限のAPI呼び出し（PR情報取得1回、マージ1回）
- レート制限エラーは通常発生しないが、発生時は適切なエラーメッセージを表示

### 3.2 複数ワークフローの完了待機

**課題**: Test、Lintの両方が完了してからマージする必要がある

**解決策**:
- `workflow_run`は各ワークフロー完了時に個別にトリガー
- 最後に完了したワークフローで、すべてのワークフローの状態を確認
- GitHub APIの`mergeStateStatus`が全体の状態を反映するため、追加の待機ロジックは不要

**実装**:
```yaml
on:
  workflow_run:
    workflows: ["Test", "Lint"]  # 両方監視
    types:
      - completed
```

トリガーされた時点で：
1. 先に完了したワークフロー → PR状態確認 → まだ他のワークフロー実行中 → スキップ
2. 最後に完了したワークフロー → PR状態確認 → すべて成功 → マージ

### 3.3 ドラフトPRの除外ロジック

**決定**: GitHub CLIの`--json isDraft`で確認

**実装**:
```bash
IS_DRAFT=$(gh pr view $PR_NUMBER --json isDraft --jq '.isDraft')
if [ "$IS_DRAFT" = "true" ]; then
  echo "Draft PR, skipping auto-merge"
  exit 0
fi
```

**代替案**: PRリストで`--state open`でフィルタするが、明示的な確認の方が安全

## 4. 実装上の考慮事項

### 4.1 マージ方法の指定

**決定**: `--merge`フラグでMerge commitを明示

**コマンド**:
```bash
gh pr merge $PR_NUMBER --merge --auto
```

**`--auto`フラグの使用**:
- すべての必須チェックが完了するまで待機
- 条件が満たされた時点で自動的にマージ
- GitHub側で最終的な安全性確認が行われる

### 4.2 セキュリティと権限

**必要な権限**:
```yaml
permissions:
  contents: write        # ブランチへの書き込み
  pull-requests: write   # PRのマージ
```

**セキュリティ対策**:
- `workflow_run.conclusion == 'success'`を条件に追加
- `workflow_run.event == 'pull_request'`を確認
- ログに機密情報を含めない

### 4.3 テスト戦略

**ローカルテスト**: 不可（GitHub Actions環境が必要）

**統合テスト**:
1. テスト用ブランチでPR作成
2. CIが成功する変更をプッシュ
3. 自動マージを確認
4. 失敗ケースも同様にテスト

## 5. 決定事項サマリー

| 項目 | 決定内容 | 理由 |
|------|----------|------|
| トリガー | `workflow_run` | 複数ワークフローの完了検知に最適 |
| ツール | GitHub CLI（gh） | シンプル、認証が容易 |
| マージ条件 | `mergeable == MERGEABLE` && `mergeStateStatus in [CLEAN, UNSTABLE]` | 安全性と柔軟性のバランス |
| エラー処理 | 段階的チェックとログ出力 | デバッグ性と安全性 |
| 権限 | `contents: write`, `pull-requests: write` | 最小限の必要権限 |
| マージ方法 | `--merge --auto` | Merge commit、GitHub側で最終確認 |

## 6. 次のステップ

Phase 0完了。次はPhase 1（設計）に進みます：
- data-model.md: PRとCIワークフローの状態モデル
- quickstart.md: 開発者向けガイド
- contracts/: GitHub Actionsイベントペイロード仕様
