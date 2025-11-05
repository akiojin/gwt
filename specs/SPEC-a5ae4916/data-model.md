# データモデル: ブランチ一覧の表示順序改善

**仕様ID**: `SPEC-a5ae4916` | **日付**: 2025-10-25

## 概要

この機能は既存のデータモデルを使用し、新しいエンティティやプロパティは追加しません。既存の`BranchInfo`と`WorktreeInfo`型、および`worktreeMap`を活用してソート処理を実装します。

## 既存のエンティティ

### BranchInfo

**場所**: `src/ui/types.ts`

**説明**: ブランチの情報を表現するエンティティ

**属性**:

| 属性名 | 型 | 説明 | ソートでの使用 |
|--------|-----|------|---------------|
| `name` | `string` | ブランチ名 | 名前順ソートで使用 |
| `type` | `"local" \| "remote"` | ブランチのタイプ | ローカルブランチ優先判定で使用 |
| `isCurrent` | `boolean` | 現在のブランチかどうか | 最優先表示の判定で使用 |
| `branchType` | `"main" \| "develop" \| "feature" \| "hotfix" \| "release" \| "other"` | ブランチの種類 | main / develop 優先判定で使用 |
| `latestCommitTimestamp` | `number \| undefined` | 最新コミットのUNIXタイムスタンプ | worktree有無が同じグループ内での最新順ソートに使用 |

**ソートでの使用例**:

```typescript
// 現在のブランチ判定
if (a.isCurrent && !b.isCurrent) return -1;
if (!a.isCurrent && b.isCurrent) return 1;

// mainブランチ判定
if (a.branchType === "main" && b.branchType !== "main") return -1;
if (a.branchType !== "main" && b.branchType === "main") return 1;

// developブランチ判定（Clarify結果）
if (a.branchType === "develop" && b.branchType !== "develop") return -1;
if (a.branchType !== "develop" && b.branchType === "develop") return 1;

// 最新コミット時刻で降順ソート（新機能）
const aCommit = a.latestCommitTimestamp ?? 0;
const bCommit = b.latestCommitTimestamp ?? 0;
if (aCommit !== bCommit) return bCommit - aCommit;

// ローカルブランチ判定（Clarify結果）
const aIsLocal = a.type === "local";
const bIsLocal = b.type === "local";
if (aIsLocal && !bIsLocal) return -1;
if (!aIsLocal && bIsLocal) return 1;

// 名前順ソート
return a.name.localeCompare(b.name);
```

### WorktreeInfo

**場所**: `src/worktree.ts`

**説明**: worktreeの情報を表現するエンティティ

**属性**:

| 属性名 | 型 | 説明 | ソートでの使用 |
|--------|-----|------|---------------|
| `path` | `string` | worktreeのパス | 使用しない |
| `branch` | `string` | 関連するブランチ名 | worktreeMap のキーとして使用 |
| `head` | `string` | HEADのコミットハッシュ | 使用しない |
| `isAccessible` | `boolean \| undefined` | アクセス可能かどうか | 使用しない |

**ソートでの使用例**:

worktreeInfoは直接使用せず、worktreeMapを介して判定：

```typescript
const worktreeMap = new Map<string, WorktreeInfo>([
  ["feature/user-auth", { path: "/path/to/worktree", branch: "feature/user-auth", ... }],
  ["feature/new-ui", { path: "/path/to/worktree", branch: "feature/new-ui", ... }]
]);

// worktreeの有無判定（新機能）
const aHasWorktree = worktreeMap.has(a.name);
const bHasWorktree = worktreeMap.has(b.name);
if (aHasWorktree && !bHasWorktree) return -1;
```

## データ構造

### worktreeMap

**型**: `Map<string, WorktreeInfo>`

**説明**: ブランチ名をキーとしてworktree情報を格納するマップ

**生成方法** (`src/ui/table.ts`):

```typescript
const worktreeMap = new Map(
  worktrees
    .filter((w) => w.path !== process.cwd())
    .map((w) => [w.branch, w])
);
```

**使用方法**:

```typescript
// ブランチにworktreeが存在するかチェック
const hasWorktree = worktreeMap.has(branchName);

// worktree情報を取得
const worktree = worktreeMap.get(branchName);
```

**ソートでの使用**:

```typescript
// O(1)の時間で worktree の有無を判定
const aHasWorktree = worktreeMap.has(a.name);
const bHasWorktree = worktreeMap.has(b.name);

if (aHasWorktree && !bHasWorktree) return -1;  // aを優先
if (!aHasWorktree && bHasWorktree) return 1;   // bを優先
```

## ソート処理のデータフロー

### 1. データ取得

```typescript
// ブランチ情報とworktree情報を並行取得
const [branches, worktrees] = await Promise.all([
  getAllBranches(),           // BranchInfo[]
  listAdditionalWorktrees(),  // WorktreeInfo[]
]);
```

### 2. worktreeMapの生成

```typescript
const worktreeMap = new Map(
  worktrees
    .filter((w) => w.path !== process.cwd())
    .map((w) => [w.branch, w])
);
```

### 3. ソート処理

```typescript
const sortedBranches = [...filteredBranches].sort((a, b) => {
  // BranchInfoのプロパティとworktreeMapを使用してソート

  // 1. isCurrent プロパティで判定
  if (a.isCurrent !== b.isCurrent) { /* ... */ }

  // 2. branchType プロパティで判定
  if (a.branchType === "main" || b.branchType === "main") { /* ... */ }

  // 3. worktreeMapで判定（新機能）
  const aHasWorktree = worktreeMap.has(a.name);
  const bHasWorktree = worktreeMap.has(b.name);
  if (aHasWorktree !== bHasWorktree) { /* ... */ }

  // 4. 最新コミットタイムスタンプで降順判定（新機能）
  const aCommit = a.latestCommitTimestamp ?? 0;
  const bCommit = b.latestCommitTimestamp ?? 0;
  if (aCommit !== bCommit) { /* ... */ }

  // 5. type プロパティで判定（Clarify結果）
  const aIsLocal = a.type === "local";
  const bIsLocal = b.type === "local";
  if (aIsLocal !== bIsLocal) { /* ... */ }

  // 6. name プロパティで判定
  return a.name.localeCompare(b.name);
});
```

### 4. 表示用データの生成

```typescript
const choices = await createBranchTable(branches, worktrees);
// → Array<{ name: string; value: string; description?: string }>
```

## データの制約

### 型制約

1. **BranchInfo**
   - `type`は`"local"`または`"remote"`のいずれか
   - `isCurrent`はbooleanで必須
   - `branchType`は定義された6つの値のいずれか

2. **WorktreeInfo**
   - `branch`は一意のブランチ名
   - 同じブランチ名のworktreeは複数存在しない

3. **worktreeMap**
   - キーはブランチ名（string）
   - 値はWorktreeInfo
   - 重複するキーは存在しない

### ビジネスルール

1. **現在のブランチの一意性**
   - `isCurrent === true`のブランチは最大1つ
   - 複数ある場合、最初のものが優先される

2. **main / develop の存在**
   - `branchType === "main"` のブランチは通常 1 つ
   - `branchType === "develop"` は任意だが存在すれば main の直後に並ぶ
   - いずれかが存在しない場合は該当条件をスキップする

3. **ローカルとリモートの関係**
   - 同じブランチ名がローカルとリモート両方に存在する場合、ローカルのみ表示
   - 重複排除ロジックは既存の実装に依存

4. **release / hotfix の扱い**
   - `branchType === "release"` や `"hotfix"` は worktree が無い場合、一般ルール（worktree有無 → ローカル/リモート → 名前順）に従う

## パフォーマンス特性

### 時間計算量

| 操作 | 計算量 | 説明 |
|------|--------|------|
| worktreeMap生成 | O(m) | m = worktreeの数 |
| worktreeMap.has() | O(1) | Map構造の検索 |
| ブランチのソート | O(n log n) | n = ブランチの数 |
| 全体 | O(n log n + m) | 通常 m < n なので O(n log n) |

### 空間計算量

| データ構造 | 計算量 | 説明 |
|------------|--------|------|
| branches配列 | O(n) | n = ブランチの数 |
| worktrees配列 | O(m) | m = worktreeの数 |
| worktreeMap | O(m) | 追加のメモリ使用 |
| 全体 | O(n + m) | 通常 m < n なので O(n) |

## 検証ルール

### ソート前の検証

1. **branches配列の検証**
   - 配列が空でない（0個の場合は空配列を返す）
   - 各要素がBranchInfo型に準拠

2. **worktrees配列の検証**
   - 各要素がWorktreeInfo型に準拠
   - `branch`プロパティが存在

### ソート後の検証

1. **順序の検証**
   - 現在のブランチが最上部（存在する場合）
   - mainブランチが現在のブランチの次（存在する場合）
   - worktree付きブランチがworktreeなしブランチより上
   - ローカルブランチがリモートブランチより上
   - 同条件のブランチは名前順

2. **要素数の検証**
   - ソート前後で配列の長さが同じ
   - 要素が失われていない
   - 重複が発生していない

## まとめ

この機能は既存のデータモデルを活用し、新しいエンティティやプロパティを追加せずに実装できます。主な変更点は`createBranchTable`関数内のソートロジックのみで、データ構造には一切変更がありません。

これにより、以下の利点があります：

1. **後方互換性**: 既存の型定義やインターフェースを変更しない
2. **シンプルさ**: 新しいデータ構造の学習コスト不要
3. **パフォーマンス**: 追加のデータ取得やAPIコール不要
4. **保守性**: 既存のコードベースとの整合性を維持
