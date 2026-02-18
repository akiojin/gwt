# 実装計画: ブランチ選択時のdivergence/FF失敗ハンドリング（起動継続）

**仕様ID**: `SPEC-40c7b4f1` | **日付**: 2026-01-05 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-40c7b4f1/spec.md` からの機能仕様

## 概要

CLIとWeb UIの両方で、fast-forward pull 失敗や divergence（ahead/behind が両方 > 0）でも起動をブロックしない。警告表示は維持し、起動継続のみ保証する。divergence検知失敗時も警告して継続する。

## 技術コンテキスト

**言語/バージョン**: TypeScript（Bun/Node実行）
**主要な依存関係**: Ink（CLI UI）、React（Web UI）、execa、Vitest
**ストレージ**: N/A
**テスト**: Vitest + Testing Library
**ターゲットプラットフォーム**: ローカルCLI（macOS/Windows/Linux）
**プロジェクトタイプ**: 単一リポジトリ（CLI + Web UI）
**パフォーマンス目標**: N/A
**制約**: CLIユーザー向け出力は英語のみ、divergence判定ロジックは変更しない
**スケール/範囲**: 既存フローの分岐修正

## 原則チェック

- 実装はシンプルに保ち、ブロック条件のみを変更する
- 既存の警告文・UI文言は必要最小限の調整に留める
- 仕様ドキュメントにソースコードを記載しない

## プロジェクト構造

### ドキュメント（この機能）

- `specs/SPEC-40c7b4f1/spec.md`
- `specs/SPEC-40c7b4f1/plan.md`
- `specs/SPEC-40c7b4f1/tasks.md`

### ソースコード（リポジトリルート）

- `src/index.ts`（CLI起動フロー）
- `tests/unit/index.protected-workflow.test.ts`（CLIフローのユニットテスト）
- `src/web/client/src/pages/BranchDetailPage.tsx`（分岐状態の算出）
- `src/web/client/src/components/branch-detail/ToolLauncher.tsx`（起動UI）
- `tests/unit/web/client/pages/BranchDetailPage.test.tsx`（Web UIテスト）

## フェーズ0: 調査（技術スタック選定）

**目的**: 既存のdivergence検知と起動ブロック箇所を特定する

**出力**: 追加の調査ドキュメントは作成しない（コード確認のみ）

### 調査項目

1. CLIのdivergenceブロック条件（`src/index.ts`）
2. Web UIの起動ボタン無効化条件（`BranchDetailPage` / `ToolLauncher`）
3. 既存テストのブロック前提（CLI/Web UI）

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 影響範囲とテスト方針を確定する

**出力**: 設計ドキュメントは追加しない（変更範囲が限定的）

## フェーズ2: タスク生成

**次のステップ**: `tasks.md` を作成する

**入力**: このプラン + 仕様書

**出力**: `specs/SPEC-40c7b4f1/tasks.md`

## 実装戦略

### 優先順位付け

1. **P1**: CLI起動ブロック解除（divergenceでも起動を継続）
2. **P1**: Web UI起動ボタンのブロック解除
3. **P2**: divergence検知失敗時の継続処理
4. **P2**: 既存テストの更新

### 独立したデリバリー

- CLIの非ブロック化とテスト更新だけでも独立して価値を提供
- Web UIの非ブロック化と警告文調整を追加で提供

## テスト戦略

- **ユニットテスト**: CLI起動フローのdivergence時挙動、Web UIの起動ボタン状態
- **統合テスト**: なし（既存のユニットテスト更新で十分）

## リスクと緩和策

### 技術的リスク

1. **警告の意味が弱くなる**: ブロック解除で注意喚起が減る
   - **緩和策**: 警告表示は維持し、UI文言で注意喚起を継続

2. **既存テストの意図変更**: ブロック前提のテストが破綻
   - **緩和策**: 期待値を「警告あり/起動継続」に変更

## 次のステップ

1. ✅ 仕様確定
2. ✅ 実装計画作成
3. ⏭️ `tasks.md` 更新
4. ⏭️ 実装開始
