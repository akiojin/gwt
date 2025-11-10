# クイックスタートガイド: Worktreeクリーンアップ選択機能

**仕様ID**: SPEC-55fe506f
**作成日**: 2025-11-10

## 概要

このガイドは、複数ブランチ選択機能の開発を始めるための手順を提供します。

## セットアップ

### 1. 依存関係のインストール

```bash
bun install
```

### 2. ビルド

```bash
bun run build
```

### 3. テストの実行

```bash
# 全テストを実行
bun run test

# ウォッチモードで実行
bun run test:watch

# カバレッジ付きで実行
bun run test:coverage
```

## 開発ワークフロー

### TDD（テスト駆動開発）の実践

このプロジェクトはTDDを採用しています。実装の前に必ずテストを作成してください。

#### 1. テストファイルの作成

```bash
# テストファイルの場所
tests/ui/components/common/Select.test.tsx
tests/ui/components/screens/BranchListScreen.test.tsx
tests/ui/components/App.test.tsx
```

#### 2. Red-Green-Refactorサイクル

```bash
# 1. Red: テストを書く（失敗する）
bun run test:watch

# 2. Green: 最小限の実装でテストを通す
# コードを書く

# 3. Refactor: リファクタリング
# コードを改善する
```

### テスト実行例

```bash
# 特定のテストファイルのみ実行
bun run test tests/ui/components/common/Select.test.tsx

# パターンマッチで実行
bun run test Select
```

## よくある操作

### 1. 新規ステートの追加

**App.tsx**:
```typescript
const [selectedBranches, setSelectedBranches] = useState<Set<string>>(new Set());
```

### 2. コールバック関数の作成

**App.tsx**:
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

const clearBranchSelection = useCallback(() => {
  setSelectedBranches(new Set());
}, []);
```

### 3. Props の渡し方

**App.tsx → BranchListScreen**:
```typescript
<BranchListScreen
  branches={visibleBranches}
  stats={stats}
  onSelect={handleBranchSelect}
  onCleanupCommand={handleCleanupCommand}
  selectedBranches={selectedBranches}           // 追加
  onToggleSelection={toggleBranchSelection}     // 追加
  onClearSelection={clearBranchSelection}       // 追加
  // ... 他のProps ...
/>
```

**BranchListScreen.tsx → Select**:
```typescript
<Select
  items={branches}
  onSelect={onSelect}
  onSpace={onToggleSelection ? (item) => onToggleSelection(item.name) : undefined}  // 追加
  onEscape={onClearSelection}                   // 追加
  limit={limit}
  disabled={Boolean(cleanupUI?.inputLocked)}
  renderItem={renderBranchRow}
/>
```

### 4. キーハンドリングの追加

**Select.tsx**:
```typescript
useInput((input, key) => {
  if (disabled) {
    return;
  }

  // 既存のキー処理...

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

### 5. マーカー表示の実装

**BranchListScreen.tsx の renderBranchRow**:
```typescript
const isSelected = selectedBranches?.has(item.name) ?? false;
const isWarning = item.hasUnpushedCommits || !item.mergedPR;
const selectionMarker = isSelected
  ? (isWarning ? chalk.red('*') : '*')
  : ' ';
const arrow = isCursor ? '>' : ' ';
const staticPrefix = `${arrow}${selectionMarker} ${indicatorPrefix}`;
```

## デバッグ方法

### 1. コンソールログ

```typescript
console.log('selectedBranches:', Array.from(selectedBranches));
console.log('isSelected:', selectedBranches.has(branchName));
```

### 2. Ink Devtools

Inkの公式devtoolsは現在利用不可のため、代わりに以下の方法を使用：

```typescript
// デバッグ用のステート監視
useEffect(() => {
  console.log('Selection changed:', Array.from(selectedBranches));
}, [selectedBranches]);
```

### 3. テストでのデバッグ

```typescript
import { render } from 'ink-testing-library';

it('should toggle selection', () => {
  const { lastFrame, stdin } = render(<MyComponent />);

  // キー入力をシミュレート
  stdin.write(' ');

  // 出力を確認
  console.log(lastFrame());

  // アサーション
  expect(lastFrame()).toContain('*');
});
```

## トラブルシューティング

### 問題1: テストが失敗する

**症状**: `TypeError: Cannot read property 'has' of undefined`

**原因**: `selectedBranches` がundefinedの可能性

**解決策**:
```typescript
const isSelected = selectedBranches?.has(item.name) ?? false;
```

---

### 問題2: キー入力が反応しない

**症状**: スペースキーを押しても選択されない

**原因**: `onSpace` が正しく渡されていない可能性

**解決策**:
1. Propsの型定義を確認
2. コンソールログで `onSpace` が関数として渡されているか確認
3. `disabled` 状態を確認

---

### 問題3: マーカーが表示されない

**症状**: `*` マーカーが画面に表示されない

**原因**: レイアウト計算の問題

**解決策**:
1. `staticPrefix` の文字列を確認
2. `stringWidth()` の計算が正しいか確認
3. ターミナルの幅が十分か確認

---

### 問題4: ビルドエラー

**症状**: `Property 'onSpace' does not exist on type 'SelectProps'`

**原因**: 型定義が更新されていない

**解決策**:
```bash
# TypeScriptの型チェック
bun run type-check

# ビルドのクリーン
bun run clean
bun run build
```

---

## 開発Tips

### 1. 型安全性の確保

```typescript
// Optional Propsは常に `?:` を使用
interface BranchListScreenProps {
  selectedBranches?: Set<string>;
  onToggleSelection?: (branchName: string) => void;
}

// 使用時は `?.` または `??` でnullチェック
const isSelected = selectedBranches?.has(item.name) ?? false;
```

### 2. パフォーマンスの最適化

```typescript
// useCallback でメモ化
const toggleBranchSelection = useCallback((branchName: string) => {
  // ...
}, []); // 依存配列を最小限に

// useMemo でメモ化
const visibleBranches = useMemo(() => {
  return branches.filter(b => !hiddenBranches.includes(b.name));
}, [branches, hiddenBranches]);
```

### 3. コミットメッセージ

Conventional Commitsに従ってください：

```bash
# 機能追加
git commit -m "feat: ブランチ選択機能のスペースキー対応"

# バグ修正
git commit -m "fix: 選択マーカーの表示位置を修正"

# テスト追加
git commit -m "test: Select コンポーネントのスペースキーテストを追加"
```

### 4. コードレビュー前のチェックリスト

- [ ] テストがすべてパスする
- [ ] 型チェックがパスする (`bun run type-check`)
- [ ] Lintエラーがない (`bun run lint`)
- [ ] フォーマットが整っている (`bun run format`)
- [ ] コミットメッセージが Conventional Commits に従っている

## 参考リソース

- [Ink Documentation](https://github.com/vadimdemedes/ink)
- [Vitest Documentation](https://vitest.dev/)
- [React Testing Library](https://testing-library.com/docs/react-testing-library/intro/)
- [Conventional Commits](https://www.conventionalcommits.org/)

## 次のステップ

1. [spec.md](./spec.md) を読んで要件を理解
2. [research.md](./research.md) を読んで技術的決定を確認
3. [data-model.md](./data-model.md) を読んでデータ構造を理解
4. `/speckit.tasks` を実行してタスクリストを生成
5. テストを書いて実装を開始
