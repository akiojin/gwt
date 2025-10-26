# クイックスタート: Ink.js UI開発ガイド

**SPEC ID**: SPEC-4c2ef107
**日付**: 2025-01-25
**対象**: Ink.js UI移行プロジェクトの開発者

## 概要

このガイドは、Ink.js UIの開発を開始するための最小限の手順を提供します。TDD（テスト駆動開発）に従い、bunを使用して開発します。

## 前提条件

- bun がインストールされていること
- Git リポジトリがクローンされていること
- TypeScript の基本的な知識
- React の基本的な知識（hooksなど）

## セットアップ手順

### 1. 依存関係のインストール

```bash
# リポジトリルートで実行
cd /path/to/claude-worktree

# 依存関係をインストール
bun install

# Ink関連の依存関係を追加（初回のみ）
bun add ink react ink-select-input ink-text-input
bun add -D @types/react ink-testing-library jsdom @testing-library/jest-dom
```

### 2. 開発環境の確認

```bash
# ビルドが通ることを確認
bun run build

# テストが実行できることを確認
bun test

# アプリケーションが起動することを確認
bunx .
```

### 3. Vitest設定の確認

`vitest.config.ts`に以下が設定されているか確認：

```typescript
import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['./vitest.setup.ts'],
  },
});
```

`vitest.setup.ts`を作成（存在しない場合）:

```typescript
import '@testing-library/jest-dom';
```

## 開発ワークフロー

### TDD サイクル

Ink.js UIの開発はすべてTDD（テスト駆動開発）で行います。

```
1. 🔴 Red: テストを書く（失敗する）
     ↓
2. 🟢 Green: 最小限の実装で テストを通す
     ↓
3. 🔵 Refactor: リファクタリング
     ↓
   （繰り返し）
```

### 具体的な手順

#### Phase 1: テストを書く（Red）

```bash
# テストファイルを作成
touch src/ui/components/common/__tests__/Select.test.tsx
```

```typescript
// src/ui/components/common/__tests__/Select.test.tsx
import { describe, it, expect, vi } from 'vitest';
import { render } from 'ink-testing-library';
import { Select } from '../Select';

describe('Select', () => {
  it('should render items', () => {
    const items = [
      { label: 'Option 1', value: '1' },
      { label: 'Option 2', value: '2' },
    ];

    const { lastFrame } = render(
      <Select items={items} onSelect={vi.fn()} />
    );

    expect(lastFrame()).toContain('Option 1');
    expect(lastFrame()).toContain('Option 2');
  });

  it('should call onSelect when item is selected', () => {
    const onSelect = vi.fn();
    const items = [{ label: 'Option 1', value: '1' }];

    const { stdin } = render(
      <Select items={items} onSelect={onSelect} />
    );

    stdin.write('\r'); // Enter key
    expect(onSelect).toHaveBeenCalledWith('1');
  });
});
```

```bash
# テスト実行（失敗することを確認）
bun test Select.test.tsx
```

#### Phase 2: 実装する（Green）

```typescript
// src/ui/components/common/Select.tsx
import React from 'react';
import SelectInput from 'ink-select-input';

interface SelectProps {
  items: Array<{ label: string; value: string }>;
  onSelect: (value: string) => void;
  limit?: number;
}

export const Select: React.FC<SelectProps> = ({ items, onSelect, limit }) => {
  return (
    <SelectInput
      items={items}
      limit={limit}
      onSelect={(item) => onSelect(item.value)}
    />
  );
};
```

```bash
# テスト実行（成功することを確認）
bun test Select.test.tsx
```

#### Phase 3: リファクタリング（Refactor）

```typescript
// 型定義を分離
export interface SelectItem {
  label: string;
  value: string;
}

export interface SelectProps {
  items: SelectItem[];
  onSelect: (value: string) => void;
  limit?: number;
}

// コンポーネントはそのまま
export const Select: React.FC<SelectProps> = ({ items, onSelect, limit }) => {
  return (
    <SelectInput
      items={items}
      limit={limit}
      onSelect={(item) => onSelect(item.value)}
    />
  );
};
```

```bash
# テストが引き続き通ることを確認
bun test Select.test.tsx
```

## よくある操作

### 新規コンポーネントの作成

#### 1. テストファイル作成

```bash
# 例: Header コンポーネント
mkdir -p src/ui/components/parts/__tests__
touch src/ui/components/parts/__tests__/Header.test.tsx
```

#### 2. テスト作成（受け入れ条件から）

spec.mdの受け入れシナリオを参照してテストケースを作成。

```typescript
// src/ui/components/parts/__tests__/Header.test.tsx
import { describe, it, expect } from 'vitest';
import { render } from 'ink-testing-library';
import { Header } from '../Header';

describe('Header', () => {
  it('should display title and version', () => {
    const { lastFrame } = render(<Header version="1.3.0" />);

    expect(lastFrame()).toContain('Worktree Manager v1.3.0');
  });

  it('should display separator line', () => {
    const { lastFrame } = render(<Header version="1.0.0" />);

    expect(lastFrame()).toContain('━');
  });
});
```

#### 3. コンポーネント実装

```typescript
// src/ui/components/parts/Header.tsx
import React from 'react';
import { Box, Text } from 'ink';

interface HeaderProps {
  version: string;
}

export const Header: React.FC<HeaderProps> = ({ version }) => {
  return (
    <Box flexDirection="column" flexShrink={0}>
      <Text bold color="blue">
        Worktree Manager v{version}
      </Text>
      <Text>{'━'.repeat(process.stdout.columns || 80)}</Text>
    </Box>
  );
};
```

#### 4. テスト実行

```bash
bun test Header.test.tsx
```

### テストの実行

```bash
# すべてのテストを実行
bun test

# 特定のファイルのみ
bun test Header.test.tsx

# watchモード（開発中に便利）
bun test --watch

# カバレッジ確認
bun test --coverage
```

### 既存コードの移行

#### 1. 既存コードをlegacyに移動

```bash
# 例: prompts.tsの移行
git mv src/ui/prompts.ts src/ui/legacy/prompts.ts
```

#### 2. 新規Inkコンポーネントの作成（TDDで）

```bash
# テストファイル作成
touch src/ui/components/screens/__tests__/BranchListScreen.test.tsx

# テスト作成 → 実装 → リファクタリング
```

#### 3. エントリーポイントの更新

```typescript
// src/index.ts
const USE_INK_UI = process.env.USE_INK_UI === 'true';

if (USE_INK_UI) {
  const { render } = await import('ink');
  const { App } = await import('./ui/components/App.js');
  render(<App />);
} else {
  // 既存UI（レガシー）
  // ...
}
```

#### 4. 動作確認

```bash
# 新UIで実行
USE_INK_UI=true bunx .

# 既存UIで実行
bunx .
```

## トラブルシューティング

### Inkのレンダリング問題

#### 問題: 画面が正しく表示されない

**原因**: Boxの`flexDirection`や`flexGrow`の設定ミス

**解決策**:

```typescript
// 正しい例
<Box flexDirection="column" height="100%">
  <Box flexShrink={0}>{/* Header */}</Box>
  <Box flexGrow={1}>{/* Content */}</Box>
  <Box flexShrink={0}>{/* Footer */}</Box>
</Box>
```

#### 問題: スクロールが動作しない

**原因**: `limit`プロパティが設定されていない

**解決策**:

```typescript
const visibleRows = Math.max(5, process.stdout.rows - HEADER_LINES - FOOTER_LINES);

<SelectInput
  items={items}
  limit={visibleRows}  // ここを設定
  onSelect={onSelect}
/>
```

### bunとの互換性問題

#### 問題: Inkがbunで動作しない

**解決策**:
1. Ink v5.0.0で試す
2. ダメならInk v4.xにダウングレード
3. それでもダメならresearch.mdのPlan Cを参照

#### 問題: import エラー

**原因**: `.js`拡張子の不足

**解決策**:

```typescript
// 間違い
import { App } from './ui/components/App';

// 正しい（bunは.js拡張子必須）
import { App } from './ui/components/App.js';
```

### テスト失敗時の対処

#### 問題: ink-testing-libraryでエラー

**原因**: jsdom環境が設定されていない

**解決策**: `vitest.config.ts`を確認

```typescript
export default defineConfig({
  test: {
    environment: 'jsdom',  // ここを確認
    setupFiles: ['./vitest.setup.ts'],
  },
});
```

#### 問題: モックが動作しない

**解決策**:

```typescript
import { vi } from 'vitest';

// 関数のモック
const onSelect = vi.fn();

// アサーション
expect(onSelect).toHaveBeenCalledWith('expected-value');
```

## ベストプラクティス

### 1. コンポーネントの分割

```typescript
// Good: 責任が明確
<BranchListScreen>
  <Header />
  <Stats />
  <ScrollableList />
  <Footer />
</BranchListScreen>

// Bad: 1つのコンポーネントにすべて
<BranchListScreen>
  {/* すべてのロジックがここに */}
</BranchListScreen>
```

### 2. Propsの型定義

```typescript
// Good: 明確な型定義
interface BranchListScreenProps {
  branches: BranchItem[];
  worktrees: WorktreeInfo[];
  onSelect: (branchName: string) => void;
}

// Bad: any使用
interface Props {
  data: any;
  callback: any;
}
```

### 3. カスタムフックの活用

```typescript
// 再利用可能なロジックはカスタムフックに
const useTerminalSize = () => {
  const [size, setSize] = useState({
    rows: process.stdout.rows || 24,
    columns: process.stdout.columns || 80
  });

  useEffect(() => {
    const handleResize = () => {
      setSize({
        rows: process.stdout.rows || 24,
        columns: process.stdout.columns || 80
      });
    };

    process.stdout.on('resize', handleResize);
    return () => process.stdout.off('resize', handleResize);
  }, []);

  return size;
};
```

### 4. メモ化の適切な使用

```typescript
// 高コストな計算のみメモ化
const items = useMemo(() => {
  return branches.map(b => formatBranch(b));  // 高コスト
}, [branches]);

// シンプルな計算は不要
const count = branches.length;  // useMemoは不要
```

## 次のステップ

1. ✅ 開発環境セットアップ完了
2. ⏭️ Phase 0.1実施: Ink + bun動作確認
3. ⏭️ 共通コンポーネント実装（TDDで）
4. ⏭️ ブランチ一覧画面実装（P1）

## 参考リソース

- [Ink.js公式ドキュメント](https://github.com/vadimdemedes/ink)
- [ink-select-input](https://github.com/vadimdemedes/ink-select-input)
- [ink-testing-library](https://github.com/vadimdemedes/ink-testing-library)
- [Vitest公式ドキュメント](https://vitest.dev/)
- [プロジェクトのspec.md](./spec.md)（機能仕様）
- [プロジェクトのplan.md](./plan.md)（実装計画）
- [プロジェクトのresearch.md](./research.md)（調査結果）

---

**作成日**: 2025-01-25
**最終更新**: 2025-01-25
