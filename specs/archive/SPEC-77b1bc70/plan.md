# 実装計画: リリースフロー要件の明文化とリリース開始時 main→develop 同期

**仕様ID**: `SPEC-77b1bc70` | **日付**: 2026-01-16 | **仕様書**: `specs/SPEC-77b1bc70/spec.md`
**入力**: `/specs/SPEC-77b1bc70/spec.md` からの機能仕様

## 概要

リリースフローの要件を仕様化し、prepare-release で main→develop を先行統合する。
同時に release.yml の back-merge（sync-develop）を廃止し、リリースガイドを削除する。

## 技術コンテキスト

**言語/バージョン**: YAML（GitHub Actions）、Markdown
**主要な依存関係**: GitHub Actions / gh CLI
**ストレージ**: N/A
**テスト**: 変更内容の検証スクリプト（簡易）
**ターゲットプラットフォーム**: GitHub Actions
**プロジェクトタイプ**: 単一リポジトリ
**パフォーマンス目標**: N/A
**制約**: GITHUB_TOKEN の push 権限が必要
**スケール/範囲**: リリースフロー（prepare-release / release）に限定

## 原則チェック

- シンプルさ: 既存 workflow を最小変更で拡張する
- テストファースト: 変更点を検証するスクリプトを先に作成
- 既存コード尊重: 既存 workflow と README を更新し、新規ファイルは検証用に限定
- 品質ゲート: markdownlint / clippy / rustfmt の既存CIに影響しないことを確認

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-77b1bc70/
├── plan.md
└── spec.md
```

### ソースコード（リポジトリルート）

```text
.github/workflows/
├── prepare-release.yml
└── release.yml

docs/
├── release-guide.md (削除)
└── release-guide.ja.md (削除)

README.md
README.ja.md
scripts/
└── check-release-flow.sh (新規・検証用)
```

## フェーズ0: 調査

- 既存の release フロー（prepare-release.yml / release.yml）と関連ドキュメントを確認
- release-guide 参照箇所（README）を特定

## フェーズ1: 設計

### 変更方針

1. prepare-release.yml
   - develop チェックアウト後に `git merge --no-ff origin/main` を実行
   - マージ成功時に develop を push
   - GITHUB_TOKEN を利用

2. release.yml
   - sync-develop ジョブを削除

3. ドキュメント
   - release-guide.* を削除
   - README 内の参照を仕様（spec）に切り替え

### テスト方針

- 変更内容を検証する簡易スクリプトを作成
  - prepare-release.yml に同期ステップがある
  - release.yml に sync-develop が無い
  - release-guide が削除され、README 参照も更新済み

## フェーズ2: タスク生成

- `/specs/SPEC-77b1bc70/tasks.md` を作成

## 実装戦略

1. 検証スクリプト追加（TDD準拠）
2. workflow 更新
3. ドキュメント削除・README更新
4. 検証スクリプト実行

## テスト戦略

- `scripts/check-release-flow.sh` を実行して要件適合を確認

## リスクと緩和策

### 技術的リスク

1. **GITHUB_TOKEN 権限不足**: develop push が失敗する
   - **緩和策**: リポジトリ設定の Workflow permissions を確認

2. **競合による失敗**: main→develop マージでコンフリクト
   - **緩和策**: prepare-release を失敗させ、手動解決後に再実行

## 次のステップ

1. tasks.md を作成
2. 変更を実装
3. 検証スクリプト実行
4. コミット & push
