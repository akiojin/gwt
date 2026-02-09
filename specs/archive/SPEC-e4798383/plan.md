# 実装計画: GitHub Issue連携の空Issue自動スキップ

**仕様ID**: `SPEC-e4798383` | **日付**: 2026-01-26 | **仕様書**: specs/SPEC-e4798383/spec.md
**入力**: `/specs/SPEC-e4798383/spec.md` からの機能仕様

## 概要

Issue一覧が0件の場合に、Issue選択ステップを自動的にスキップしてブランチ名入力へ遷移する。ウィザード状態の更新とテスト追加で、ユーザー操作なしにスムーズに進行させる。

## 技術コンテキスト

**言語/バージョン**: Rust 2021 Edition (stable)
**主要な依存関係**: ratatui, crossterm, gix, tracing
**ストレージ**: N/A
**テスト**: cargo test
**ターゲットプラットフォーム**: ローカルCLI (macOS/Linux/Windows)
**プロジェクトタイプ**: 単一プロジェクト（ワークスペース構成）
**パフォーマンス目標**: Issue一覧取得後の遷移は即時
**制約**: 既存ウィザードフローの互換性維持
**スケール/範囲**: Issue一覧 0件時のみの挙動変更

## 原則チェック

- シンプルな実装でユーザー体験を改善
- 既存フローへの影響を最小化

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-e4798383/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
└── tasks.md
```

### ソースコード（リポジトリルート）

```text
crates/
├── gwt-cli/src/tui/screens/wizard.rs
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 既存のIssue取得・ウィザード遷移の実装位置を確認

**出力**: `specs/SPEC-e4798383/research.md`

## フェーズ1: 設計（アーキテクチャと契約）

**出力**:
- `specs/SPEC-e4798383/data-model.md`
- `specs/SPEC-e4798383/quickstart.md`

## フェーズ2: タスク生成

**出力**: `specs/SPEC-e4798383/tasks.md`

## 実装戦略

- Issue一覧取得後の処理を集約して、0件時の自動スキップを実装
- 既存のIssue選択ロジックと整合性を保つ

## テスト戦略

- ウィザード状態に対するユニットテストを追加

## リスクと緩和策

1. **遷移条件の二重管理**: IssueSelectとBranchNameInputの遷移が重複
   - **緩和策**: Issue一覧処理を単一ヘルパーへ集約

## 次のステップ

1. ✅ 仕様更新
2. ✅ 実装計画
3. ⏭️ タスク更新
4. ⏭️ 実装
