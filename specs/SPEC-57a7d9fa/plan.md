# 実装計画: Worktreeディレクトリパスを`.git/worktree`から`.worktrees`に変更

**仕様ID**: `SPEC-57a7d9fa` | **日付**: 2025-10-31 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-57a7d9fa/spec.md` からの機能仕様

## 概要

claude-worktreeツールにおいて、新規Worktreeの作成先を`.git/worktree`から`.worktrees`に変更します。これにより、Worktreeがより分かりやすい場所に配置され、`.gitignore`への追加も容易になります。既存の`.git/worktree`配下のWorktreeには影響を与えず、後方互換性を維持します。

## 技術コンテキスト

**言語/バージョン**: TypeScript 5.8+, Bun 1.0+
**主要な依存関係**: execa (Gitコマンド実行), path (Node.js標準), chalk (CLI出力装飾), ink (React-based UI)
**ストレージ**: ファイルシステム（.gitignoreファイルの読み書き）
**テスト**: Vitest 2.1+
**ターゲットプラットフォーム**: macOS、Linux、Windows（Bun 1.0+対応プラットフォーム）
**プロジェクトタイプ**: 単一 - CLIツール
**パフォーマンス目標**: Worktree作成時間 < 5秒
**制約**:
- 既存の`generateWorktreePath`メソッドのシグネチャは変更しない
- 既存Worktreeに影響を与えない
- Git worktree標準機能のみ使用
**スケール/範囲**:
- 対象ファイル: 2ファイル（src/worktree.ts、tests/unit/worktree.test.ts）
- 対象関数: 1関数（generateWorktreePath）
- 追加機能: .gitignore更新ロジック

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

### CLAUDE.mdに基づく原則

- ✅ **シンプルさの追求**: パス文字列を1箇所変更するだけのシンプルな実装
- ✅ **ユーザビリティ**: `.worktrees`は`.git/worktree`より直感的な配置
- ✅ **TDD**: テストファイルを先に修正し、Red-Green-Refactorサイクルを実施
- ✅ **既存ファイル優先**: 新規ファイル作成なし、既存ファイルの修正のみ
- ✅ **エラー解消**: ビルドとテストが成功するまで完了としない

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-57a7d9fa/
├── spec.md              # 機能仕様書
├── plan.md              # このファイル（実装計画）
├── research.md          # 技術調査結果
├── data-model.md        # データモデル設計
├── quickstart.md        # 開発者向けガイド
└── checklists/
    └── requirements.md  # 仕様品質チェックリスト
```

### ソースコード（リポジトリルート）

```text
src/
├── worktree.ts          # 主要な変更対象（generateWorktreePath関数）
├── git.ts               # Gitignore更新用のヘルパー（必要に応じて）
└── ui/
    └── types.ts         # 型定義

tests/
└── unit/
    └── worktree.test.ts # テスト更新対象
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 既存の実装を理解し、変更が必要な箇所を特定する

**出力**: `specs/SPEC-57a7d9fa/research.md`

### 調査項目

1. **既存のコードベース分析**
   - `src/worktree.ts`の`generateWorktreePath`関数（130-137行目）
   - `tests/unit/worktree.test.ts`のテストケース（99-130行目）
   - `.gitignore`ファイルの現在の内容と構造
   - Git worktreeコマンドの動作確認

2. **技術的決定**
   - 決定1: `.gitignore`更新ロジックを`src/worktree.ts`に直接実装するか、別ファイルに分離するか
   - 決定2: `.gitignore`更新のタイミング（初回Worktree作成時のみ、または毎回チェック）
   - 決定3: 既存の`.gitignore`エントリーの重複チェック方法

3. **制約と依存関係**
   - 制約1: Node.js `fs`モジュールを使用して`.gitignore`を読み書き
   - 制約2: 既存のWorktreeService APIとの互換性維持
   - 制約3: エラーハンドリング（`.gitignore`が読み取り専用の場合など）

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:
- `specs/SPEC-57a7d9fa/data-model.md`
- `specs/SPEC-57a7d9fa/quickstart.md`

### 1.1 データモデル設計

**ファイル**: `data-model.md`

主要なエンティティとその関係を定義：
- Worktreeパス文字列の構造
- `.gitignore`エントリーの形式
- エラーメッセージの定義

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向けの簡潔なガイド：
- ローカル開発環境のセットアップ
- テストの実行方法（`bun run build && bun test`）
- デバッグ方法（`bun run start`）
- 変更後の動作確認手順

### 1.3 契約/インターフェース（該当する場合）

**ディレクトリ**: `contracts/`

該当なし - 内部実装の変更のため、外部APIの変更はありません。

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-57a7d9fa/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装：
1. **P1**: Worktreeパス生成ロジックの変更
   - `src/worktree.ts`の修正（1ファイル、1関数）
   - `tests/unit/worktree.test.ts`の期待値更新
2. **P2**: `.gitignore`更新機能
   - `.gitignore`読み書きロジックの実装
   - 重複チェック機能の実装
3. **P3**: 後方互換性の検証
   - 既存Worktreeが影響を受けないことをテストで確認

### テスト戦略

**TDD アプローチ**:
1. **Red**: テストを先に修正（`.worktrees`を期待値に）
2. **Green**: 実装を修正してテストを通す
3. **Refactor**: コードをクリーンアップ

**テストカバレッジ**:
- `generateWorktreePath`関数の単体テスト
- `.gitignore`更新ロジックの単体テスト
- エッジケース（`.gitignore`が存在しない、読み取り専用など）

### リスク管理

**潜在的リスク**:
1. 既存Worktreeへの影響 → 対策: テストで検証
2. `.gitignore`更新の失敗 → 対策: エラーハンドリングとロギング
3. パフォーマンス劣化 → 対策: ベンチマークテスト

## 成功基準

実装完了の定義:
- ✅ `bun run build`が成功
- ✅ `bun test`で全テストが通過
- ✅ 新規Worktreeが`.worktrees/`配下に作成される
- ✅ `.gitignore`に`.worktrees/`が追加される
- ✅ 既存Worktreeが影響を受けない

## 次のステップ

1. Phase 0を完了: `research.md`を作成
2. Phase 1を完了: `data-model.md`、`quickstart.md`を作成
3. `/speckit.tasks`でタスクリストを生成
4. `/speckit.implement`で実装を実行
