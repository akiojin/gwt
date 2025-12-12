# 実装計画: bugfixブランチタイプのサポート追加

**仕様ID**: `SPEC-1defd8fd` | **日付**: 2025-01-18 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-1defd8fd/spec.md` からの機能仕様

**注**: この計画は既存実装の事後ドキュメント化として作成されています。

## 概要

既存のfeature/hotfix/releaseブランチタイプに加えて、bugfixブランチタイプを追加する機能。通常のバグ修正（緊急ではない）を適切に分類し、🐛アイコンで視覚的に識別できるようにする。bugfix/とbug/の両プレフィックスをサポートし、既存ブランチタイプと同等の扱いを実現する。

## 技術コンテキスト

**言語/バージョン**: TypeScript 5.8.x / React 19 / Ink 6 / Bun 1.0+
**主要な依存関係**: Ink 6（CLI UI）、Vitest 2.1.x（テスト）、happy-dom 20.0.8、@testing-library/react 16.3.0
**ストレージ**: N/A（ブランチメタデータのみ、Gitリポジトリ内）
**テスト**: Vitest 2.1.x + Testing Library（UIコンポーネントテスト）
**ターゲットプラットフォーム**: Node.js 18+ / Bun 1.0+（CLI実行環境）
**プロジェクトタイプ**: 単一（CLIツール）
**パフォーマンス目標**: ブランチタイプ判定は即座（<1ms）、UI応答は即座（<100ms）
**制約**: 既存のBranchType型システムとの互換性維持、TypeScript型安全性の保持
**スケール/範囲**: 数百〜数千ブランチのリポジトリに対応

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

**注**: 本プロジェクトにはconstitution.mdがテンプレート状態のため、具体的な原則チェックは省略。ただし、以下の一般的な設計原則を遵守:

- ✅ **既存コードとの一貫性**: getBranchType関数のパターンマッチング順序を維持
- ✅ **型安全性**: TypeScript型システムで静的検証可能
- ✅ **テスタビリティ**: すべての機能要件がユニットテストでカバー可能
- ✅ **シンプルさ**: 既存パターンの自然な拡張として実装

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-1defd8fd/
├── spec.md              # 機能仕様
├── plan.md              # このファイル（実装計画）
├── tasks.md             # タスクリスト（/speckit.tasksで生成）
└── checklists/
    └── requirements.md  # 仕様品質チェックリスト
```

### ソースコード（リポジトリルート）

```text
src/
├── cli/ui/
│   ├── types.ts                              # BranchType型定義
│   ├── utils/branchFormatter.ts              # ブランチアイコン定義
│   ├── components/screens/
│   │   └── BranchCreatorScreen.tsx           # ブランチ作成UI
│   └── __tests__/
│       ├── utils/branchFormatter.test.ts     # フォーマッターテスト
│       └── components/screens/
│           └── BranchCreatorScreen.test.tsx  # UI テスト
├── config/constants.ts                       # BRANCH_TYPES/BRANCH_PREFIXES定数
├── git.ts                                    # getBranchType関数
└── services/git.service.ts                   # サービス層のgetBranchType

tests/
└── fixtures/branches.ts                      # テストフィクスチャ
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 既存の技術スタックとパターンを理解する

**出力**: （既存実装のため、事後ドキュメント化）

### 調査項目

1. **既存のコードベース分析**
   - 現在の技術スタック: TypeScript 5.8.x、React 19、Ink 6、Bun 1.0+
   - 既存のパターン:
     - BranchType型: ユニオン型で定義（"feature" | "hotfix" | ...）
     - BRANCH_TYPES定数: as constで厳密な型推論
     - getBranchType関数: パターンマッチング（startsWith）で判定
     - branchIcons: Record<BranchType, string>でアイコンマッピング
   - 統合ポイント:
     - src/cli/ui/types.ts: UI層の型定義
     - src/config/constants.ts: アプリケーション全体の定数
     - src/git.ts, src/services/git.service.ts: ブランチタイプ判定ロジック

2. **技術的決定**
   - **決定1**: BranchType型にbugfixを追加（既存のユニオン型パターンを踏襲）
   - **決定2**: bugfix/とbug/の両プレフィックスをサポート（柔軟性と後方互換性のため）
   - **決定3**: 🐛アイコンを使用（bugfixの普遍的シンボルとして直感的）
   - **決定4**: 既存テストフレームワーク（Vitest + Testing Library）を使用

3. **制約と依存関係**
   - **制約1**: 既存のBranchType型定義を破壊しない（型安全性維持）
   - **制約2**: getBranchType関数のパターンマッチング順序を維持（一貫性）
   - **制約3**: 既存テストが全てパスする（後方互換性）

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する（既存実装の事後ドキュメント化）

**出力**: （以下に記述）

### 1.1 データモデル設計

**ファイル**: （インラインで記述、別ファイル不要）

#### BranchType型

```typescript
type BranchType =
  | "feature"
  | "bugfix"   // ← 追加
  | "hotfix"
  | "release"
  | "main"
  | "develop"
  | "other";
```

#### BranchInfo インターフェース

```typescript
interface BranchInfo {
  name: string;
  type: "local" | "remote";
  branchType: BranchType;  // bugfixを含む
  isCurrent: boolean;
  // ... その他のプロパティ
}
```

#### 定数

```typescript
BRANCH_TYPES = {
  FEATURE: "feature",
  BUGFIX: "bugfix",     // ← 追加
  HOTFIX: "hotfix",
  RELEASE: "release",
  // ...
} as const;

BRANCH_PREFIXES = {
  FEATURE: "feature/",
  BUGFIX: "bugfix/",    // ← 追加
  HOTFIX: "hotfix/",
  RELEASE: "release/",
} as const;
```

#### アイコンマッピング

```typescript
branchIcons: Record<BranchType, string> = {
  feature: "✨",
  bugfix: "🐛",         // ← 追加
  hotfix: "🔥",
  release: "🚀",
  // ...
};
```

### 1.2 クイックスタートガイド

**ファイル**: （インラインで記述、別ファイル不要）

#### 開発者向けガイド

**bugfixブランチを作成する:**

1. CLIツールを起動: `gwt`
2. "Create new branch"を選択
3. ブランチタイプで"bugfix"を選択
4. ブランチ名（プレフィックスなし）を入力: 例 "null-pointer-exception"
5. bugfix/null-pointer-exceptionブランチが作成される

**既存のbug/プレフィックスブランチ:**

- bug/プレフィックスのブランチも自動的にbugfixタイプとして認識される
- 🐛アイコンで表示される

**テスト実行:**

```bash
bun run test
```

**ビルド:**

```bash
bun run build
```

### 1.3 契約/インターフェース（該当する場合）

**ディレクトリ**: （該当なし - 内部型定義のみ）

この機能は外部APIを公開しないため、契約ファイルは不要。内部TypeScript型定義が契約として機能する。

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書

**出力**: `specs/SPEC-1defd8fd/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装（既に完了）:

1. **P1 - ストーリー1**: 通常のバグ修正用ブランチを作成できる
   - 型定義追加
   - 定数追加
   - UI選択肢追加
   - アイコン追加

2. **P2 - ストーリー2**: bug/プレフィックスもbugfixとして認識される
   - getBranchType関数に判定ロジック追加

3. **P1 - ストーリー3**: 既存のfeature/hotfix/releaseと同等に扱われる
   - 既存機能との統合確認
   - テスト追加

### 独立したデリバリー

各ユーザーストーリーは独立して実装可能だが、本実装では一括で実施:
- 型システムの制約により、P1ストーリー1は他の変更と同時に実装する必要がある
- すべての変更を1コミット（ca915a0）で実施

## テスト戦略

### ユニットテスト

- **branchFormatter.test.ts**: bugfixブランチのフォーマット（アイコン表示）をテスト
- **BranchCreatorScreen.test.tsx**: UI選択肢にbugfixが含まれることをテスト
- **getBranchType関数**: bugfix/とbug/プレフィックスの判定をテスト（間接的）

### 統合テスト

- 既存の統合テスト（tests/integration/）がbugfixブランチでもパスすることを確認

### テストフィクスチャ

- `tests/fixtures/branches.ts`にbugfix/null-pointer-exceptionサンプルを追加

### カバレッジ目標

- 新規コード100%カバレッジ（型定義、定数、判定ロジック、UI、アイコン）
- 既存テストが全てパス（後方互換性）

## リスクと緩和策

### 技術的リスク

1. **型定義の破壊的変更**
   - **緩和策**: ユニオン型への追加のみ（既存値に影響なし）、TypeScriptコンパイラで静的検証

2. **getBranchType関数のパターンマッチング順序**
   - **緩和策**: 既存の順序ロジックを維持（main→develop→feature→bugfix→hotfix→release→other）

3. **UI表示の一貫性**
   - **緩和策**: 既存のRecord<BranchType, string>パターンを踏襲、型システムで網羅性を保証

### 依存関係リスク

1. **TypeScript型システム**
   - **緩和策**: ビルド時に型チェック実行、エラー時は即座に修正

2. **React/Inkバージョン**
   - **緩和策**: 既存の安定版（React 19, Ink 6）を使用、破壊的変更なし

## 次のステップ

1. ✅ フェーズ0完了: 既存コードベース分析と技術スタック確認
2. ✅ フェーズ1完了: 設計とアーキテクチャ定義（事後ドキュメント化）
3. ⏭️ `/speckit.tasks` を実行してタスクを生成（既存実装の文書化）
4. ✅ 実装完了: コミットca915a0で全変更を実施
5. ⏭️ ドキュメント更新: README.md/README.ja.mdにbugfixブランチタイプを追加
6. ⏭️ `/speckit.analyze` で品質検証を実行
