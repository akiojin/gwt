# クイックスタートガイド: ブランチ一覧の表示順序改善

**仕様ID**: `SPEC-a5ae4916` | **日付**: 2025-10-25

## 概要

このガイドは、ブランチ一覧の表示順序改善機能を実装するための手順を説明します。

## 前提条件

- Bun >= 1.0.0がインストールされている
- TypeScript 5.8+の知識がある
- Vitestでのテスト経験がある
- Gitとgit worktreeの基本的な知識がある

## セットアップ

### 1. リポジトリのクローンと依存関係のインストール

```bash
# リポジトリのクローン（既にクローン済みの場合はスキップ）
git clone <repository-url>
cd claude-worktree

# 依存関係のインストール
bun install
```

### 2. ブランチの確認

```bash
# 現在のブランチを確認
git branch

# SPEC-a5ae4916ブランチに切り替え（既に作成済み）
git checkout SPEC-a5ae4916
```

### 3. ビルドとテスト

```bash
# TypeScriptのビルド
bun run build

# テストの実行
bun run test

# カバレッジ付きテスト
bun run test:coverage
```

## 開発ワークフロー

### フェーズ1: テストの作成（TDD）

#### 1.1 既存のテストファイルを確認

```bash
# 既存のテストファイルを確認
cat tests/unit/ui/table.test.ts
```

#### 1.2 新しいテストケースを追加

**テスト対象**: `src/ui/table.ts`の`createBranchTable`関数

**テストケース**:
1. Worktree付きブランチがworktreeなしブランチより上に表示される
2. ローカルブランチがリモートオンリーブランチより上に表示される（最新コミット時刻が同一の場合）
3. 現在のブランチ → main → develop の順が維持される
4. release/hotfix ブランチは worktree が無い場合に一般ルールへ従う
5. worktree有無が同じブランチ群は最新コミット時刻の降順で並ぶ
6. 最新コミット時刻も同一の場合は名前順でソートされる

#### 1.3 テストの実行（失敗することを確認）

```bash
# テストを実行（Redフェーズ）
bun run test tests/unit/ui/table.test.ts
```

### フェーズ2: 実装

#### 2.1 ソートロジックの修正

**ファイル**: `src/ui/table.ts` (80-86行目)

**変更前**:
```typescript
const sortedBranches = [...filteredBranches].sort((a, b) => {
  if (a.isCurrent && !b.isCurrent) return -1;
  if (!a.isCurrent && b.isCurrent) return 1;
  if (a.branchType === "main" && b.branchType !== "main") return -1;
  if (a.branchType !== "main" && b.branchType === "main") return 1;
  return a.name.localeCompare(b.name);
});
```

**変更後**:
```typescript
const sortedBranches = [...filteredBranches].sort((a, b) => {
  // 1. 現在のブランチを最優先
  if (a.isCurrent && !b.isCurrent) return -1;
  if (!a.isCurrent && b.isCurrent) return 1;

  // 2. mainブランチを優先
  if (a.branchType === "main" && b.branchType !== "main") return -1;
  if (a.branchType !== "main" && b.branchType === "main") return 1;

  // 3. developブランチを優先
  if (a.branchType === "develop" && b.branchType !== "develop") return -1;
  if (a.branchType !== "develop" && b.branchType === "develop") return 1;

  // 4. worktreeがあるブランチを優先
  const aHasWorktree = worktreeMap.has(a.name);
  const bHasWorktree = worktreeMap.has(b.name);
  if (aHasWorktree && !bHasWorktree) return -1;
  if (!aHasWorktree && bHasWorktree) return 1;

  // 5. 最新コミット時刻が新しいブランチを優先
  const aCommit = a.latestCommitTimestamp ?? 0;
  const bCommit = b.latestCommitTimestamp ?? 0;
  if (aCommit !== bCommit) return bCommit - aCommit;

  // 6. ローカルブランチを優先
  const aIsLocal = a.type === "local";
  const bIsLocal = b.type === "local";
  if (aIsLocal && !bIsLocal) return -1;
  if (!aIsLocal && bIsLocal) return 1;

  // 7. 名前順
  return a.name.localeCompare(b.name);
});
```

#### 2.2 テストの実行（合格することを確認）

```bash
# テストを実行（Greenフェーズ）
bun run test tests/unit/ui/table.test.ts
```

#### 2.3 ブランチ一覧 UI の更新

**ファイル**: `src/ui/components/common/Select.tsx`, `src/ui/components/screens/BranchListScreen.tsx`

- `Select` コンポーネントに `renderItem` を追加し、ターミナル幅を考慮したカスタム描画を可能にする
- 各ブランチ行に「YYYY-MM-DD HH:mm」形式の時刻を右寄せで表示する
- 選択中の行は背景色をシアンに変更し、非選択行では既存の配色を維持する
- 既存のインジケータアイコンは保持しつつ、`cleanupUI` の配色指定を尊重する

#### 2.4 UI テストの実行（Greenフェーズ）

```bash
bun test src/ui/__tests__/components/screens/BranchListScreen.test.tsx
```

#### 2.5 リファクタリング

- コードの可読性を向上
- 不要なコメントを削除
- 変数名を明確にする

### フェーズ3: 検証

#### 3.1 すべてのテストを実行

```bash
# すべてのユニットテストを実行
bun run test

# カバレッジを確認
bun run test:coverage

# Lint でフォーマットと静的解析を確認
bun run lint
```

#### 3.2 型チェック

```bash
# TypeScriptの型チェック
bun run type-check
```

#### 3.3 Lintとフォーマット

```bash
# ESLintでコードをチェック
bun run lint

# Prettierでフォーマットをチェック
bun run format:check

# 自動フォーマット（必要に応じて）
bun run format

# CLIヘルプを表示して新しいソート順が反映されたバイナリを確認
bun run build && bun run start -- --help
```

## よくある操作

### 開発サーバーの起動

```bash
# ウォッチモードでビルド
bun run dev
```

### CLIの実行

```bash
# ビルド後に実行
bun run build
bun run start

# または直接実行
bunx .
```

### テストのデバッグ

```bash
# 特定のテストファイルのみ実行
bun run test tests/unit/ui/table.test.ts

# ウォッチモードでテスト
bun run test:watch

# UIモードでテスト
bun run test:ui
```

## トラブルシューティング

### 問題1: テストが失敗する

**症状**: テストが期待通りに動作しない

**解決策**:
1. ビルドが最新か確認: `bun run build`
2. `node_modules`を削除して再インストール: `rm -rf node_modules && bun install`
3. テストデータが正しいか確認

### 問題2: 型エラーが発生する

**症状**: TypeScriptの型エラーが発生

**解決策**:
1. `tsconfig.json`を確認
2. 型定義ファイル（`src/ui/types.ts`）を確認
3. `bun run type-check`で詳細なエラーを確認

### 問題3: ソートが期待通りに動作しない

**症状**: ブランチの表示順序が期待と異なる

**解決策**:
1. `worktreeMap`が正しく生成されているか確認
2. `BranchInfo`のプロパティ（`type`, `isCurrent`, `branchType`）が正しいか確認
3. デバッグ用のログを追加して確認

```typescript
console.log('Branch:', a.name, {
  isCurrent: a.isCurrent,
  branchType: a.branchType,
  type: a.type,
  hasWorktree: worktreeMap.has(a.name)
});
```

## 実装チェックリスト

### テストフェーズ

- [ ] 既存のテストが合格することを確認
- [ ] Worktree優先表示のテストを作成
- [ ] ローカルブランチ優先表示のテストを作成
- [ ] 複合条件のテストを作成
- [ ] エッジケースのテストを作成
- [ ] すべてのテストが失敗することを確認（Redフェーズ）

### 実装フェーズ

- [ ] `src/ui/table.ts`のソートロジックを修正
- [ ] Worktree判定ロジックを追加
- [ ] ローカルブランチ判定ロジックを追加
- [ ] コメントを追加して可読性を向上
- [ ] すべてのテストが合格することを確認（Greenフェーズ）

### 検証フェーズ

- [ ] すべてのユニットテストが合格
- [ ] カバレッジが十分か確認（80%以上推奨）
- [ ] 型チェックが合格
- [ ] Lintエラーがない
- [ ] フォーマットが正しい
- [ ] 手動テストで動作確認

### コミットフェーズ

- [ ] コミットメッセージを作成
- [ ] 変更をコミット
- [ ] プッシュして PR を作成（該当する場合）

## 参考資料

- [機能仕様](./spec.md)
- [実装計画](./plan.md)
- [調査レポート](./research.md)
- [データモデル](./data-model.md)
- [TypeScript公式ドキュメント](https://www.typescriptlang.org/)
- [Vitest公式ドキュメント](https://vitest.dev/)
- [Bun公式ドキュメント](https://bun.sh/)

## 次のステップ

1. ✅ セットアップ完了
2. ⏭️ テストの作成（TDD）
3. ⏭️ ソートロジックの実装
4. ⏭️ 検証とリファクタリング
5. ⏭️ `/speckit.tasks`でタスクを生成
6. ⏭️ `/speckit.implement`で実装を開始
