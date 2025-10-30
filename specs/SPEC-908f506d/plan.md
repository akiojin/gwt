# 実装計画: ブランチ作成・選択機能の改善

**仕様ID**: `SPEC-908f506d` | **日付**: 2025-10-29 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-908f506d/spec.md` からの機能仕様

## 概要

ブランチ一覧でブランチを選択した際の動作を改善する機能です。主に2つの改善を含みます：

1. **カレントブランチ選択時のWorktree作成スキップ**（不具合修正）：ルートディレクトリのカレントブランチを選択した場合、Worktreeを作成せずそのままAIツールを起動
2. **ブランチ選択後のアクション選択**（新機能）：ブランチ選択後に「既存ブランチで続行」か「新規ブランチを作成」を選択可能に

技術的アプローチ：
- Ink.js (React-based TUI) を使用した新規画面の追加
- WorktreeOrchestratorにカレントブランチ判定ロジックを追加
- BranchCreatorScreenにベースブランチパラメータを追加
- App.tsxの画面遷移フローを改修

## 技術コンテキスト

**言語/バージョン**: TypeScript 5.8+ (ES2022 target)
**主要な依存関係**:
- Ink.js 6.3.1 (React 19.2.0)
- execa 9.6.0 (Gitコマンド実行)
- chalk 5.4.1 (色付け)
**ストレージ**: N/A（Gitリポジトリを直接操作）
**テスト**: Vitest 2.1.8 + Ink Testing Library 4.0.0
**ターゲットプラットフォーム**: CLI（Bun 1.0+、Node.js 18+互換）
**プロジェクトタイプ**: 単一CLIプロジェクト
**パフォーマンス目標**:
- カレントブランチ選択から起動まで < 1秒
- アクション選択画面表示まで < 1秒
**制約**:
- 既存のWorktree管理システムと統合
- 'n'キーの直接新規作成フローは変更しない
- ブランチ切り替え禁止（Worktree設計思想）
**スケール/範囲**:
- 中規模リポジトリ（100+ ブランチ）対応
- 同時Worktree数: 制限なし（Git仕様に依存）

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

constitution.mdが未定義のため、CLAUDE.mdの開発指針を適用：

- ✅ **シンプルさの追求**: 新規画面は2択のシンプルな選択UI
- ✅ **ユーザビリティ優先**: 開発者体験を向上させる機能
- ✅ **既存コード尊重**: 既存のUIコンポーネントとパターンを再利用
- ✅ **Spec Kit準拠**: 仕様書作成→計画→タスク→実装のフローに従う
- ✅ **並列化**: P1/P2/P3の優先度で段階的に実装可能

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-908f506d/
├── plan.md              # このファイル（/speckit.plan コマンド出力）
├── spec.md              # 機能仕様（完了）
├── checklists/          # 品質チェックリスト（完了）
│   └── requirements.md
├── research.md          # フェーズ0出力（次のステップ）
├── data-model.md        # フェーズ1出力
├── quickstart.md        # フェーズ1出力
└── tasks.md             # フェーズ2出力（/speckit.tasks コマンド）
```

### ソースコード（リポジトリルート）

```text
src/
├── index.ts                   # メインエントリーポイント
├── git.ts                     # Git操作（getCurrentBranch）
├── worktree.ts                # Worktree操作
├── services/
│   └── WorktreeOrchestrator.ts  # Worktree管理（カレントブランチ判定追加）
├── ui/
│   ├── types.ts                 # 型定義（ScreenType追加）
│   ├── components/
│   │   ├── App.tsx              # メインアプリ（画面遷移フロー改修）
│   │   ├── common/
│   │   │   └── Select.tsx       # 共通選択コンポーネント（再利用）
│   │   └── screens/
│   │       ├── BranchListScreen.tsx        # ブランチ一覧
│   │       ├── BranchActionSelectorScreen.tsx  # 新規：アクション選択
│   │       ├── BranchCreatorScreen.tsx     # ブランチ作成（改修）
│   │       └── AIToolSelectorScreen.tsx    # AIツール選択
│   └── __tests__/
│       └── components/screens/
│           ├── BranchActionSelectorScreen.test.tsx  # 新規テスト
│           └── BranchCreatorScreen.test.tsx         # 既存テスト更新
tests/
├── integration/
│   └── branch-selection-flow.test.ts  # 統合テスト
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 要件に基づいて技術スタックを決定し、既存のコードパターンを理解する

**出力**: `specs/SPEC-908f506d/research.md`

### 調査項目

1. **既存のコードベース分析**
   - 現在のInk.js画面遷移パターン（useScreenState）
   - 既存のSelectコンポーネントの使用方法
   - BranchCreatorScreenの現在の実装
   - WorktreeOrchestratorの現在のフロー
   - カレントブランチ判定の既存実装（git.ts）

2. **技術的決定**
   - **決定1**: BranchActionSelectorScreenの実装パターン
     - 既存のSelectコンポーネントを再利用
     - 2択のシンプルなUI（既存使用/新規作成）
   - **決定2**: カレントブランチ判定の実装場所
     - WorktreeOrchestrator.ensureWorktree()にロジック追加
     - git.getCurrentBranch()を使用
   - **決定3**: ベースブランチの渡し方
     - App.tsxで状態管理（baseBranchForCreation）
     - BranchCreatorScreenのPropsに追加

3. **制約と依存関係**
   - **制約1**: 既存のInk.js画面遷移システムとの統合
   - **制約2**: 'n'キーの直接作成フローとの共存
   - **制約3**: ブランチ切り替え禁止（現在のブランチで作業完結）

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:
- `specs/SPEC-908f506d/data-model.md`
- `specs/SPEC-908f506d/quickstart.md`
- `specs/SPEC-908f506d/contracts/` （N/A: CLIのためAPI契約なし）

### 1.1 データモデル設計

**ファイル**: `data-model.md`

主要なエンティティとその関係を定義：
- **BranchItem**: ブランチ情報（name, type, isCurrent, hasWorktree）
- **SelectedBranchState**: 選択されたブランチ状態（name, branchType, remoteBranch）
- **BranchAction**: アクション種別（'use-existing' | 'create-new'）
- **ScreenType**: 画面タイプ（'branch-action-selector'を追加）

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向けの簡潔なガイド：
- 開発環境セットアップ（Bun、依存インストール）
- ローカル実行方法（`bun run start`）
- テスト実行方法（`bun test`）
- 新規画面の追加手順
- 既存コンポーネントの再利用パターン

### 1.3 契約/インターフェース（該当する場合）

**ディレクトリ**: N/A

CLIアプリケーションのため、外部API契約は不要。内部インターフェースはTypeScriptの型定義で管理。

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-908f506d/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装：
1. **P1 - カレントブランチでの直接作業**: WorktreeOrchestratorにカレントブランチ判定を追加
2. **P2 - 既存ブランチでの作業継続**: BranchActionSelectorScreen作成、App.tsx画面遷移改修
3. **P3 - 選択ブランチをベースに新規ブランチ作成**: BranchCreatorScreen改修、ベースブランチ連携

### 独立したデリバリー

各ユーザーストーリーは独立して実装・テスト・デプロイ可能：
- **P1完了** → カレントブランチ選択が正常に動作するMVP
- **P2追加** → アクション選択機能が使えるMVP
- **P3追加** → 完全な機能（選択ブランチからの新規作成）

### テスト駆動開発

各機能の実装前にテストを作成：
1. テストシナリオ作成（Red）
2. 最小限の実装（Green）
3. リファクタリング（Refactor）

## テスト戦略

### ユニットテスト

- **WorktreeOrchestrator**: カレントブランチ判定ロジック
- **BranchActionSelectorScreen**: コンポーネントの表示・選択
- **BranchCreatorScreen**: ベースブランチパラメータの処理

### 統合テスト

- **画面遷移フロー**: BranchListScreen → BranchActionSelectorScreen → AIToolSelectorScreen
- **カレントブランチフロー**: BranchListScreen → 直接AIToolSelectorScreen
- **新規作成フロー**: BranchActionSelectorScreen → BranchCreatorScreen

### エンドツーエンドテスト

- **シナリオ1**: カレントブランチを選択してAIツール起動
- **シナリオ2**: 他のブランチを選択→既存使用→AIツール起動
- **シナリオ3**: 他のブランチを選択→新規作成→ブランチ作成→AIツール起動

### Ink Testing Library使用

Ink.jsアプリケーションのテストには `ink-testing-library` を使用：
```typescript
import { render } from 'ink-testing-library';
import { BranchActionSelectorScreen } from '../BranchActionSelectorScreen';

test('displays action choices', () => {
  const { lastFrame } = render(<BranchActionSelectorScreen ... />);
  expect(lastFrame()).toContain('既存ブランチで続行');
  expect(lastFrame()).toContain('新規ブランチを作成');
});
```

## リスクと緩和策

### 技術的リスク

1. **既存のWorktree検出ロジックとの衝突**
   - **リスク**: カレントブランチのWorktreeが既に存在する場合の処理
   - **緩和策**: `worktreeExists()` で既存Worktreeを確認し、カレントブランチ判定を優先

2. **画面遷移フローの複雑化**
   - **リスク**: 新しい画面追加により、既存の画面遷移ロジックが複雑化
   - **緩和策**: useScreenState フックのスタック型履歴管理を活用、明確な遷移ルールを定義

3. **'n'キー直接作成フローとの共存**
   - **リスク**: 2つの新規作成フロー（'n'キーと選択後）が混乱を招く
   - **緩和策**: 'n'キーフローは変更せず、Enterキー選択時のみ新フローを適用

### 依存関係リスク

1. **Ink.jsのバージョン互換性**
   - **リスク**: Ink.js 6.3.1の特定機能に依存
   - **緩和策**: 既存のSelectコンポーネントパターンを踏襲、標準的なReact Hooksを使用

2. **Gitコマンドの挙動**
   - **リスク**: `git branch --show-current` が環境によって動作しない
   - **緩和策**: エラーハンドリングを実装、null時は通常フローにフォールバック

## 次のステップ

1. ⏭️ フェーズ0: 調査と技術スタック決定（research.md作成）
2. ⏭️ フェーズ1: 設計とアーキテクチャ定義（data-model.md、quickstart.md作成）
3. ⏭️ `/speckit.tasks` を実行してタスクを生成
4. ⏭️ `/speckit.implement` で実装を開始
