# データモデル: Worktreeクリーンアップ選択機能

**仕様ID**: SPEC-55fe506f
**作成日**: 2025-11-10

## 概要

このドキュメントは、複数ブランチ選択機能で使用されるデータ構造とその関係性を定義します。

## エンティティ定義

### 1. 選択状態 (Selection State)

**型**: `Set<string>`

**説明**:
クリーンアップのために選択されたブランチ名のコレクション。セット型を使用することで、重複排除と高速検索（O(1)）を実現。

**属性**:
- 要素: ブランチ名 (string)

**操作**:
- `add(branchName)`: ブランチを選択に追加
- `delete(branchName)`: ブランチを選択から削除
- `has(branchName)`: ブランチが選択されているかチェック
- `clear()`: 全選択を解除
- `size`: 選択中のブランチ数

**検証ルール**:
- 保護ブランチ（main, master, develop）は追加不可
- 重複は自動的に排除される

**状態遷移**:
```
[空] --add()--> [1個選択] --add()--> [複数選択]
     <-clear()--          <-delete()--
                          <-clear()-------------
```

---

### 2. ブランチアイテム (Branch Item)

**型**: `BranchItem` (既存型を使用)

**説明**:
ブランチ一覧に表示される個別のブランチ情報。選択機能では主に`name`フィールドを使用し、警告判定には`hasUnpushedCommits`と`mergedPR`を使用。

**主要属性**:
- `name`: ブランチ名 (string)
- `hasUnpushedCommits`: プッシュされていないコミットの有無 (boolean | undefined)
- `mergedPR`: マージ済みPR情報 (`{ number: number; mergedAt: string }` | undefined)
- `worktree`: worktree情報 (`WorktreeInfo` | undefined)
- `branchType`: ブランチタイプ ("feature" | "hotfix" | "release" | "main" | "develop" | "other")

**選択可否の判定ロジック**:
```typescript
function isSelectable(branch: BranchItem): boolean {
  // 保護ブランチは選択不可
  if (PROTECTED_BRANCHES.includes(branch.name)) {
    return false;
  }
  return true;
}
```

**警告判定ロジック**:
```typescript
function shouldWarn(branch: BranchItem): boolean {
  // プッシュされていない、またはマージされていない場合は警告
  return branch.hasUnpushedCommits || !branch.mergedPR;
}
```

---

### 3. 選択マーカー (Selection Marker)

**型**: `string` (表示用)

**説明**:
ブランチ選択状態を視覚的に表現する記号。通常は白色の `*`、警告対象は赤色の `*` で表示。

**状態**:
- 未選択: ` ` (スペース)
- 選択済み（通常）: `*` (白色)
- 選択済み（警告）: `*` (赤色 - chalk.red)

**表示位置**:
```
> * ⚡ feature/branch-name    2025-11-10 15:30
│ │ │
│ │ └─ アイコン列
│ └─── 選択マーカー
└───── カーソル
```

**生成ロジック**:
```typescript
function getSelectionMarker(
  isSelected: boolean,
  shouldWarn: boolean
): string {
  if (!isSelected) {
    return ' ';
  }
  return shouldWarn ? chalk.red('*') : '*';
}
```

---

### 4. クリーンアップUIステート (Cleanup UI State)

**型**: `CleanupUIState` (既存型を使用)

**説明**:
クリーンアップ実行中の進捗状態を管理。選択機能では、選択数の表示と0個選択時の警告に拡張。

**主要属性**:
- `indicators`: ブランチごとの進捗インジケーター (`Record<string, CleanupIndicator>`)
- `footerMessage`: フッターメッセージ (`CleanupFooterMessage | null`)
- `inputLocked`: 入力ロック状態 (boolean)

**選択機能での拡張**:
```typescript
interface CleanupFooterMessage {
  text: string;
  color?: 'cyan' | 'green' | 'yellow' | 'red';
}

// 選択数表示
const selectionMessage: CleanupFooterMessage = {
  text: `選択中: ${selectedBranches.size}個のブランチ`,
  color: undefined // デフォルト色
};

// 0個選択時の警告
const noSelectionMessage: CleanupFooterMessage = {
  text: 'クリーンアップ対象が選択されていません',
  color: 'yellow'
};
```

---

### 5. コンポーネントProps拡張

#### SelectProps 拡張

**追加Props**:
```typescript
interface SelectProps<T> {
  // ... 既存のProps ...

  onSpace?: (item: T) => void;    // スペースキー押下時のコールバック
  onEscape?: () => void;           // ESCキー押下時のコールバック
}
```

**関係性**:
- `onSpace`: `toggleBranchSelection` 関数と接続
- `onEscape`: `clearBranchSelection` 関数と接続

---

#### BranchListScreenProps 拡張

**追加Props**:
```typescript
interface BranchListScreenProps {
  // ... 既存のProps ...

  selectedBranches?: Set<string>;           // 選択状態
  onToggleSelection?: (branchName: string) => void;  // 選択トグル
  onClearSelection?: () => void;            // 全選択解除
}
```

**関係性**:
- `selectedBranches`: App.tsx のステートから渡される
- `onToggleSelection`: App.tsx の `toggleBranchSelection` 関数
- `onClearSelection`: App.tsx の `clearBranchSelection` 関数

---

## データフロー

```
App.tsx (親)
  │
  ├─ selectedBranches: Set<string>     [ステート]
  ├─ toggleBranchSelection()           [ハンドラー]
  └─ clearBranchSelection()            [ハンドラー]
        │
        ↓ Props
  BranchListScreen
        │
        ├─ Select
        │    ├─ onSpace → toggleBranchSelection
        │    └─ onEscape → clearBranchSelection
        │
        └─ renderBranchRow
             ├─ selectedBranches.has(branchName) [選択状態チェック]
             ├─ shouldWarn(branch)               [警告判定]
             └─ getSelectionMarker()             [マーカー生成]
```

---

## 保護ブランチ定数

**定義** (worktree.ts:34):
```typescript
export const PROTECTED_BRANCHES = ["main", "master", "develop"];
```

**使用箇所**:
- `isProtectedBranchName()`: 正規化済みブランチ名で判定
- 選択可否の判定
- クリーンアップ対象の除外

---

## 状態遷移図

### 選択状態の遷移

```
┌─────────────┐
│ 未選択状態  │
│ size: 0     │
└──────┬──────┘
       │
       │ スペースキー (保護ブランチ以外)
       ↓
┌─────────────┐
│ 1個選択     │
│ size: 1     │
└──────┬──────┘
       │
       │ スペースキー (別のブランチ)
       ↓
┌─────────────┐
│ 複数選択    │
│ size: N     │
└──────┬──────┘
       │
       │ スペースキー (選択済みブランチ)
       ↓
┌─────────────┐
│ 1個削除     │
│ size: N-1   │
└──────┬──────┘
       │
       │ ESCキー
       ↓
┌─────────────┐
│ 全選択解除  │
│ size: 0     │
└─────────────┘
```

---

### クリーンアップ実行フロー

```
[選択状態]
    │
    ├─ size === 0 → [警告表示] → [処理中断]
    │
    └─ size > 0
         │
         └─ forEach(branchName)
              │
              ├─ isProtectedBranch? → [スキップ]
              │
              ├─ hasWorktree?
              │    ├─ Yes → removeWorktree() + deleteBranch()
              │    └─ No  → deleteBranch()
              │
              ├─ 成功 → [✅インジケーター]
              ├─ 失敗 → [❌インジケーター]
              └─ スキップ → [⏭️インジケーター]
```

---

## 検証ルール

### 選択時の検証

1. **保護ブランチの除外**
   - main, master, develop は選択不可
   - `isProtectedBranchName()` で判定

2. **重複の自動排除**
   - `Set` 型により自動的に処理

### クリーンアップ実行時の検証

1. **選択数のチェック**
   - `selectedBranches.size === 0` の場合は警告表示

2. **二重チェック**
   - 実行時にも再度保護ブランチを除外

3. **worktree存在確認**
   - worktreeが存在する場合のみ削除実行

---

## パフォーマンス考慮

### O(1) 操作

- `Set.has()`: 選択状態チェック
- `Set.add()`: 選択追加
- `Set.delete()`: 選択削除

### メモ化

- `branchItems`: `useMemo` でメモ化済み
- 選択状態変更時も既存のメモ化構造を維持

### 再レンダリング最適化

- `React.memo` による Select コンポーネントの最適化
- `arePropsEqual` でカスタム比較

---

## まとめ

このデータモデルは、既存の型定義を最大限活用しながら、最小限の変更で複数選択機能を実現します。主要な変更点は：

1. **新規ステート**: `Set<string>` 型の選択状態
2. **Props拡張**: Select と BranchListScreen にオプショナルPropsを追加
3. **表示ロジック**: 既存の `renderBranchRow` にマーカー表示を追加

すべての変更は後方互換性を維持し、既存の機能に影響を与えません。
