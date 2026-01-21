# 調査レポート: Worktreeクリーンアップ選択機能

**調査実施日**: 2025-11-10
**仕様ID**: SPEC-55fe506f

## 概要

既存のブランチ一覧画面に複数ブランチの選択・一括クリーンアップ機能を追加するため、現在の実装構造を調査しました。

## 調査結果

### 1. 既存のステート管理パターン

**App.tsx のステート管理** (App.tsx:81-93):
- 単一値: `useState<T | null>(null)`
- 複数値: `Record<string, T>` (cleanupIndicators)
- 配列: `string[]` (hiddenBranches)
- React Hooks による関数型ステート管理

### 2. BranchListScreen の構造

**Props構造** (BranchListScreen.tsx:49-64):
- `onSelect`: 単一ブランチ選択時のコールバック
- `onCleanupCommand`: 'c'キー押下時のコールバック
- `cleanupUI`: クリーンアップ進捗表示用のステート

**renderBranchRow の実装** (BranchListScreen.tsx:171-228):
- 選択状態に基づく背景色変更
- クリーンアップインジケーターの表示
- 固定幅レイアウトによる整列表示

### 3. Select コンポーネントのキー処理

**現在処理されているキー** (Select.tsx:130-169):
- `upArrow` / `k`: カーソルを上に移動
- `downArrow` / `j`: カーソルを下に移動
- `return`: 現在選択中のアイテムを確定

**拡張性**:
- useInput ブロック内に新規キー処理を追加可能
- その他のキーは親コンポーネントに伝播
- disabled 状態を参照して処理を分岐可能

### 4. 既存のクリーンアップロジック

**handleCleanupCommand の実行フロー** (App.tsx:488-636):
1. UI操作ロック (`cleanupInputLocked`)
2. 対象取得 (`getMergedPRWorktrees()`)
3. 逐次処理（スキップ条件チェック → worktree削除 → ブランチ削除）
4. インジケーター更新
5. 完了後処理 (hiddenBranches 追加、refresh)

**エラーハンドリング**:
- try-catch で個別ブランチの失敗を捕捉
- 失敗時は `❌` インジケーターを表示し、処理を継続

### 5. ブランチ情報の構造

**BranchItem 型** (types.ts:174-184):
- `hasUnpushedCommits`: プッシュされていないコミットの有無
- `mergedPR`: マージ済みPR情報
- `worktreeStatus`: worktreeの状態

**状態判定フィールド**:
- `hasUnpushedCommits`: Git コマンドベースで高信頼
- `mergedPR`: GitHub API ベースで高信頼
- `worktree.isAccessible`: パス存在確認

## 技術的決定事項

### 決定1: 選択状態の管理方法

**選択**: `Set<string>` (ブランチ名のセット)

**理由**:
- 重複排除が自動
- O(1)の高速検索 (`has()` メソッド)
- 追加・削除が簡単 (`add()`, `delete()`, `clear()`)
- メモリ効率が良い
- 既存パターンとの整合性

**代替案**:
- `Array<string>`: 検索がO(n)、重複チェックが必要
- `Map<string, boolean>`: 冗長、メモリ使用量増

### 決定2: マーカー表示の実装場所

**選択**: `renderBranchRow` 内に直接実装

**理由**:
- 既存パターンとの一貫性（クリーンアップインジケーターも同様）
- レイアウト制御の一元化
- リアルタイム性（ステート変更時に即座に反映）
- シンプルさ（新規コンポーネント作成による複雑化を回避）

**実装イメージ**:
```typescript
const isSelected = actualIndex === selectedIndex;
const isMarkedForCleanup = selectedBranches?.has(item.name) ?? false;
const marker = isMarkedForCleanup ? '*' : ' ';
const staticPrefix = `${arrow} ${marker} ${indicatorPrefix}`;
```

**代替案**:
- 別コンポーネント: 小規模な機能に対して過剰
- formatBranchItem内: ステート変更時に全ブランチ再フォーマット必要

### 決定3: 保護ブランチ判定

**選択**: 定数配列 (`PROTECTED_BRANCHES`) を継続利用

**理由**:
- 既存実装が存在 (worktree.ts:34)
- 十分なカバレッジ (`["main", "master", "develop"]`)
- シンプルさ
- 一貫性（既存のクリーンアップロジックと同じ判定基準）

**既存実装** (worktree.ts:34-41):
```typescript
export const PROTECTED_BRANCHES = ["main", "master", "develop"];

export function isProtectedBranchName(branchName: string): boolean {
  const normalized = branchName
    .replace(/^refs\/heads\//, "")
    .replace(/^origin\//, "");
  return PROTECTED_BRANCHES.includes(normalized);
}
```

**代替案**:
- 正規表現: 現状の要件で不要、メンテナンス複雑化
- 設定ファイル: 過剰機能、設定ファイル読み込みの複雑化

### 決定4: 警告対象ブランチの判定基準

**選択**: `hasUnpushedCommits` OR マージされていない (複合条件)

**理由**:
- 既存ロジックとの整合性 (handleCleanupCommand のスキップ条件)
- 安全性の向上（作業中のブランチを誤って削除するリスク低減）
- 明確な警告（選択不可理由をユーザーに明示）

**判定ロジック**:
```typescript
const shouldWarn = (branch: BranchItem): boolean => {
  // プッシュされていない、またはマージされていない場合は警告
  if (branch.hasUnpushedCommits || !branch.mergedPR) {
    return true;
  }
  return false; // 安全: クリーンアップ可能
};
```

**信頼性**:
- `hasUnpushedCommits`: Git コマンド直接実行で高信頼
- `mergedPR`: GitHub API ベースで高信頼（認証が必要）

**代替案**:
- `hasUnpushedCommits`のみ: マージ状態を考慮せず不十分
- `mergedPR`のみ: ローカルコミットの損失リスク

## 実装推奨事項

### 1. 新規ステート追加

```typescript
// App.tsx
const [selectedBranches, setSelectedBranches] = useState<Set<string>>(new Set());
```

### 2. 選択トグル関数

```typescript
const toggleBranchSelection = useCallback((branchName: string) => {
  setSelectedBranches(prev => {
    const next = new Set(prev);
    if (next.has(branchName)) {
      next.delete(branchName);
    } else {
      next.add(branchName);
    }
    return next;
  });
}, []);
```

### 3. 全選択解除関数

```typescript
const clearBranchSelection = useCallback(() => {
  setSelectedBranches(new Set());
}, []);
```

### 4. キーハンドリング追加 (Select.tsx)

```typescript
useInput((input, key) => {
  if (disabled) {
    return;
  }

  // ... 既存のキー処理 ...

  if (input === ' ' && onSpace) {
    const selectedItem = items[selectedIndex];
    if (selectedItem) {
      onSpace(selectedItem);
    }
  } else if (key.escape && onEscape) {
    onEscape();
  }
});
```

### 5. Props 拡張 (BranchListScreen.tsx)

```typescript
export interface BranchListScreenProps {
  // ... 既存のProps ...
  selectedBranches?: Set<string>;
  onToggleSelection?: (branchName: string) => void;
  onClearSelection?: () => void;
}
```

### 6. renderBranchRow の修正

```typescript
const isSelected = selectedBranches?.has(item.name) ?? false;
const isWarning = item.hasUnpushedCommits || !item.mergedPR;
const selectionMarker = isSelected
  ? (isWarning ? chalk.red('*') : '*')
  : ' ';
const arrow = isCursor ? '>' : ' ';
const staticPrefix = `${arrow}${selectionMarker} ${indicatorPrefix}`;
```

### 7. クリーンアップ実行の分岐

```typescript
const handleCleanupCommand = useCallback(async () => {
  if (selectedBranches.size === 0) {
    setCleanupFooterMessage({
      text: 'クリーンアップ対象が選択されていません',
      color: 'yellow'
    });
    return;
  }

  // 選択されたブランチのみをクリーンアップ
  const branchesToCleanup = Array.from(selectedBranches);
  // ... 既存のクリーンアップロジック ...
}, [selectedBranches]);
```

## パフォーマンス考慮事項

**React.memo 最適化** (Select.tsx:213-216):
- `arePropsEqual` でカスタム比較実装済み
- 選択状態変更時も効率的

**useMemo 活用** (App.tsx:172-211):
- `branchItems`: ブランチ情報の整形結果をメモ化
- `visibleBranches`: `hiddenBranches` 除外後の配列をメモ化

## 結論

既存実装は複数選択機能の追加に適した構造を持っています。主要な変更点:

1. **ステート追加**: `Set<string>` で選択状態管理
2. **キー処理追加**: スペースキーでの選択切替、ESCキーで全選択解除
3. **表示修正**: `renderBranchRow` にマーカー表示追加（赤色警告対応）
4. **ロジック拡張**: `handleCleanupCommand` で選択ブランチのクリーンアップ

既存のクリーンアップロジック、エラーハンドリング、UI更新機構はそのまま活用可能です。
