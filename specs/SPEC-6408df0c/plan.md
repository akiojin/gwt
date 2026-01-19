# 実装計画: HuskyでCIと同等のLintを実行

**仕様ID**: `SPEC-6408df0c` | **日付**: 2026-01-19 | **仕様書**: `/specs/SPEC-6408df0c/spec.md`
**入力**: `/specs/SPEC-6408df0c/spec.md` からの機能仕様

## 概要

Huskyのpre-pushフックでCIと同等のLint（clippy、fmt、markdownlint）を実行し、`bunx husky` で自動セットアップできるようにする。Lint失敗はpushを止め、英語で理由を表示する。フック内容は自動テストで検証する。

## 技術コンテキスト

**言語/バージョン**: Rust（stable）/ Node or Bun
**主要な依存関係**: Husky、markdownlint-cli
**ストレージ**: N/A
**テスト**: シェルスクリプト検証（フック内容）
**ターゲットプラットフォーム**: macOS/Linux/Windows
**プロジェクトタイプ**: 単一プロジェクト
**パフォーマンス目標**: push前に完了すること
**制約**: pre-pushで実行、CIと同一コマンド
**スケール/範囲**: リポジトリ全体

## 原則チェック

- シンプルさを優先し、既存の `.husky/` と `package.json` を修正して対応する
- 既存ファイル改修を優先し、新規ファイル追加は最小限
- 自動テストを追加してTDD原則を満たす

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-6408df0c/
├── plan.md
└── tasks.md
```

### ソースコード（リポジトリルート）

```text
.husky/
├── pre-push
└── pre-commit
package.json
scripts/
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 既存HuskyフックとCI Lintの一致を確認する

**出力**: `specs/SPEC-6408df0c/research.md`（省略可）

### 調査項目

1. **既存のコードベース分析**
   - `.husky/` 既存フック内容
   - CIのLintコマンド一覧

2. **技術的決定**
   - `bunx husky` を使用
   - pre-pushにLintを集約

3. **制約と依存関係**
   - Rust/Bun/Nodeの事前インストールが前提

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前にフック構成と検証方法を定義する

**出力**:
- `specs/SPEC-6408df0c/data-model.md`（N/A）
- `specs/SPEC-6408df0c/quickstart.md`（N/A）
- `specs/SPEC-6408df0c/contracts/`（N/A）

### 1.1 データモデル設計

**ファイル**: `data-model.md`（N/A）

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`（N/A）

### 1.3 契約/インターフェース

**ディレクトリ**: `contracts/`（N/A）

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書

**出力**: `specs/SPEC-6408df0c/tasks.md`

## 実装戦略

### 優先順位付け

1. **P1**: pre-pushでCI同等Lint
2. **P2**: Husky自動セットアップ
3. **P3**: 英語エラーメッセージ

### 独立したデリバリー

- ストーリー1を完了 → CI同等Lintがローカルでブロック
- ストーリー2を追加 → セットアップ漏れを防止
- ストーリー3を追加 → 失敗理由の明確化

## テスト戦略

- **ユニットテスト**: N/A
- **統合テスト**: フック内容の自動検証スクリプト
- **エンドツーエンドテスト**: `git push` の手動確認
- **パフォーマンステスト**: N/A

## リスクと緩和策

### 技術的リスク

1. **フックが長時間化**: pushが遅くなる
   - **緩和策**: フック内容をCIと同一に限定し、必要以上に増やさない

2. **環境不足**: Rust/Bun/Node未導入で失敗
   - **緩和策**: 失敗時に英語で理由を表示

### 依存関係リスク

1. **Husky導入失敗**: prepare/postinstall実行環境差
   - **緩和策**: `bunx husky` 前提でインストールを統一

## 次のステップ

1. ✅ フェーズ0完了: 調査と技術スタック決定
2. ✅ フェーズ1完了: フック構成と検証方針の定義
3. ⏭️ `/speckit.tasks` を実行してタスクを生成
4. ⏭️ `/speckit.implement` で実装を開始
