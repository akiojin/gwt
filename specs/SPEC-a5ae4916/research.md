# 調査レポート: ブランチ一覧の表示順序改善

**仕様ID**: `SPEC-a5ae4916` | **日付**: 2025-10-25
**目的**: 既存のソートロジックを理解し、最適な実装アプローチを決定する

## 既存のコードベース分析

### 1. 現在のソートアルゴリズム

**場所**: `src/ui/table.ts` (80-86行目)

```typescript
const sortedBranches = [...filteredBranches].sort((a, b) => {
  if (a.isCurrent && !b.isCurrent) return -1;
  if (!a.isCurrent && b.isCurrent) return 1;
  if (a.branchType === "main" && b.branchType !== "main") return -1;
  if (a.branchType !== "main" && b.branchType === "main") return 1;
  return a.name.localeCompare(b.name);
});
```

**現在のソート優先順位**:
1. 現在のブランチ（`isCurrent`）
2. mainブランチ（`branchType === "main"`）
3. 名前順（`localeCompare`）

### 2. データ構造

**BranchInfo型** (`src/ui/types.ts`):
```typescript
interface BranchInfo {
  name: string;
  type: "local" | "remote";
  isCurrent: boolean;
  branchType: "main" | "develop" | "feature" | "hotfix" | "release" | "other";
}
```

**WorktreeInfo型** (`src/worktree.ts`):
```typescript
interface WorktreeInfo {
  path: string;
  branch: string;
  head: string;
  isAccessible?: boolean;
}
```

**worktreeMap生成** (`src/ui/table.ts` 64-66行目):
```typescript
const worktreeMap = new Map(
  worktrees.filter((w) => w.path !== process.cwd()).map((w) => [w.branch, w]),
);
```

### 3. 既存のテストパターン

**場所**: `tests/unit/ui/table.test.ts`

**既存テストの構造**:
- Vitestを使用したユニットテスト
- モックデータを使用したテストケース
- 期待値との比較によるアサーション

## 技術的決定

### 決定1: 新しいソート優先順位

**決定内容**: 以下の優先順位でソートする

1. 現在のブランチ（`isCurrent === true`）
2. mainブランチ（`branchType === "main"`）
3. developブランチ（`branchType === "develop"`）
4. **worktree付きブランチ**（新規）
5. **ローカルブランチ**（新規）
6. 名前順（`localeCompare`）

**根拠**:
- 現在のブランチとmainブランチは既存の優先順位を維持
- develop ブランチは Git フローなどで頻用されるため main の直後に固定
- worktree付きブランチは作業中である可能性が高く、頻繁にアクセスする
- ローカルブランチは即時作業可能で、リモートオンリーより優先度が高い
- release/hotfix ブランチは worktree が無い限り一般ルールに従わせ、例外を増やさない

### 決定2: worktreeの有無判定方法

**決定内容**: `worktreeMap.has(branchName)` を使用

```typescript
const aHasWorktree = worktreeMap.has(a.name);
const bHasWorktree = worktreeMap.has(b.name);
```

**根拠**:
- `Map.has()`はO(1)の時間計算量で高速
- 既存のworktreeMap構造を活用し、新しいデータ構造は不要
- 型安全で、存在しないブランチは`false`を返す

### 決定3: ローカルブランチ判定方法

**決定内容**: `branch.type === "local"` を使用

```typescript
const aIsLocal = a.type === "local";
const bIsLocal = b.type === "local";
```

**根拠**:
- BranchInfo型に既に`type`プロパティが存在
- "local"と"remote"の2値で明確に判定可能
- 追加のデータ取得やAPIコール不要

### 決定4: 実装アプローチ

**決定内容**: 既存のソート関数に条件を追加する（リファクタリングなし）

```typescript
const sortedBranches = [...filteredBranches].sort((a, b) => {
  // 1. 現在のブランチを最優先
  if (a.isCurrent && !b.isCurrent) return -1;
  if (!a.isCurrent && b.isCurrent) return 1;

  // 2. mainブランチを優先
  if (a.branchType === "main" && b.branchType !== "main") return -1;
  if (a.branchType !== "main" && b.branchType === "main") return 1;

  // 3. developブランチを優先（Clarifyで決定）
  if (a.branchType === "develop" && b.branchType !== "develop") return -1;
  if (a.branchType !== "develop" && b.branchType === "develop") return 1;

  // 4. worktreeがあるブランチを優先（新規）
  const aHasWorktree = worktreeMap.has(a.name);
  const bHasWorktree = worktreeMap.has(b.name);
  if (aHasWorktree && !bHasWorktree) return -1;
  if (!aHasWorktree && bHasWorktree) return 1;

  // 5. ローカルブランチを優先（新規）
  const aIsLocal = a.type === "local";
  const bIsLocal = b.type === "local";
  if (aIsLocal && !bIsLocal) return -1;
  if (!aIsLocal && bIsLocal) return 1;

  // 6. 名前順
  return a.name.localeCompare(b.name);
});
```

**根拠**:
- 既存のパターンに従い、可読性を維持
- 最小限の変更で実装可能
- テストが容易で、各条件を個別に検証できる

## 制約と依存関係

### 制約1: 既存の型定義を変更しない

**詳細**: BranchInfo、WorktreeInfo型は変更不可

**影響**:
- 新しいプロパティを追加できない
- 既存のプロパティのみを使用してソート

**対処**:
- 既存のプロパティ（`type`, `isCurrent`, `branchType`）とworktreeMapを組み合わせて判定

### 制約2: 既存のworktreeMap生成ロジックに依存

**詳細**: worktreeMapは`createBranchTable`関数内で生成される

**影響**:
- worktreeMapの構造（`Map<string, WorktreeInfo>`）に依存
- worktreeMapが正しく生成されることを前提とする

**対処**:
- worktreeMapの型を信頼し、`Map.has()`で安全に判定

### 制約3: Array.sort()の安定ソート

**詳細**: TypeScriptのArray.sort()は安定ソート（ES2019以降）

**影響**:
- 同じ優先度のブランチは元の順序を維持
- 名前順でソートした後も、他の条件で同順位なら名前順を保持

**対処**:
- 安定ソートを前提とした実装（追加の考慮不要）

## パフォーマンス分析

### 時間計算量

**現在のソート**: O(n log n)
**新しいソート**: O(n log n)

**追加の操作**:
- `worktreeMap.has()`: O(1)
- `branch.type === "local"`: O(1)

**結論**: 計算量は変わらず、O(n log n)を維持

### 空間計算量

**追加のメモリ**: なし（既存のworktreeMapを使用）

**結論**: メモリ使用量は変わらない

### 実測パフォーマンス目標

- 10個のブランチ: <1ms
- 50個のブランチ: <5ms
- 100個のブランチ: <10ms
- 300個のブランチ: <30ms

## 代替案の検討

### 代替案1: ブランチをグループ化して表示

**アプローチ**: ブランチを複数のグループに分けて表示

**利点**:
- 視覚的な分離が明確
- グループごとの見出しで分かりやすい

**欠点**:
- UI構造の大幅な変更が必要
- 既存のselect promptの構造に適合しない
- 実装コストが高い

**結論**: 採用しない（範囲外）

### 代替案2: 設定ファイルでソート順をカスタマイズ

**アプローチ**: ユーザーがソート順を設定ファイルで変更可能にする

**利点**:
- ユーザーの好みに合わせたカスタマイズが可能

**欠点**:
- 設定ファイルの実装と管理が必要
- デフォルト設定を決定する必要がある
- ユーザーにとって学習コストが高い

**結論**: 採用しない（範囲外）

### 代替案3: キャッシュを使用してパフォーマンス向上

**アプローチ**: ソート結果をキャッシュして再利用

**利点**:
- 繰り返しのソートが高速化

**欠点**:
- キャッシュの無効化タイミングの管理が複雑
- 現在のソートは十分高速（<10ms）で最適化不要

**結論**: 採用しない（過剰最適化）

## ベストプラクティス

### TypeScriptのベストプラクティス

1. **型安全性を維持**
   - 型ガードを使用せず、型システムを信頼
   - optional chainingは不要（型が保証されている）

2. **可読性を優先**
   - 各ソート条件を明確に記述
   - コメントで各ステップを説明

3. **テスタビリティ**
   - 各条件を個別にテスト可能な構造
   - モックデータで再現可能

### パフォーマンスのベストプラクティス

1. **不要な計算を避ける**
   - ソート関数内で複雑な計算をしない
   - O(1)の操作のみを使用

2. **安定ソートを活用**
   - 既存の順序を信頼し、追加のソートは不要

## 次のステップ

1. ✅ 既存コードの理解完了
2. ✅ 実装アプローチの決定完了
3. ⏭️ Phase 1: データモデルと設計ドキュメントの作成
4. ⏭️ Phase 2: タスク生成（`/speckit.tasks`）
5. ⏭️ 実装開始（`/speckit.implement`）
