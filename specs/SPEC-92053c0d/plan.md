# 実装計画: commitlint を npm ci 無しで実行可能にする

**仕様ID**: `SPEC-92053c0d` | **日付**: 2026-02-03 | **仕様書**: `specs/SPEC-92053c0d/spec.md`
**入力**: `/specs/SPEC-92053c0d/spec.md` からの機能仕様

## 概要

commitlint 設定が `@commitlint/config-conventional` 未導入でも読み込めるようにフォールバック設定を追加し、`bunx commitlint` が npm ci 無しで動作することを保証する。合わせてフォールバックを検証する軽量テストを追加する。

## 技術コンテキスト

**言語/バージョン**: Node.js 18+
**主要な依存関係**: commitlint（CLI/設定）
**ストレージ**: N/A
**テスト**: Node スクリプトによる設定ロード検証
**ターゲットプラットフォーム**: ローカル開発環境/CI
**プロジェクトタイプ**: 単一（Rust + JS 設定）
**パフォーマンス目標**: 即時読み込み（数百 ms）
**制約**: 既存の commitlint ルールを変更しない
**スケール/範囲**: 1 リポジトリ内設定のみ

## 原則チェック

- シンプルさ優先（設定ファイル内で完結）
- 開発者体験の改善（npm ci 前提を除去）

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-92053c0d/
├── plan.md
├── spec.md
└── tasks.md
```

### ソースコード（リポジトリルート）

```text
commitlint.config.cjs
scripts/
```

## 実装戦略

1. フォールバック設定を commitlint.config.cjs に追加
2. 依存未導入を模擬するテストスクリプトを追加
3. テスト実行でフォールバックを確認

## テスト戦略

- **ユニットテスト**: commitlint 設定ロードを検証する Node スクリプト
- **統合テスト**: `bunx commitlint --from HEAD~1 --to HEAD` 実行（任意）

## リスクと緩和策

1. **parserPreset が未解決で失敗する**
   - **緩和策**: 依存未導入時は parserPreset を設定しない

## 次のステップ

1. ✅ 仕様更新
2. ⏭️ tasks.md 生成
3. ⏭️ テスト追加 → 実装
