# クイックスタートガイド: ブランチ作成・選択機能の改善

**仕様ID**: `SPEC-908f506d` | **日付**: 2025-10-29
**関連**: [spec.md](./spec.md) | [plan.md](./plan.md) | [data-model.md](./data-model.md)

## 概要

このガイドは、ブランチ作成・選択機能の改善を実装する開発者向けの簡潔な手順書です。

## 前提条件

- Bun 1.0+ がインストールされていること
- Gitリポジトリのクローンが完了していること
- 基本的なTypeScript/React（Ink.js）の知識

## セットアップ

### 1. 依存関係のインストール

```bash
cd /claude-worktree
bun install
```

### 2. ビルド

```bash
bun run build
```

### 3. ローカル実行

```bash
# 開発環境での実行
bun run start

# またはビルド済みの実行
bunx .
```

### 4. 開発モード（watch）

```bash
# TypeScriptのwatchモード
bun run dev
```

## テスト

### ユニットテスト実行

```bash
# すべてのテストを実行
bun test

# watchモード
bun run test:watch

# カバレッジ付き
bun run test:coverage

# UI付き
bun run test:ui
```

### テストファイルの配置

```text
src/ui/components/screens/BranchActionSelectorScreen.tsx
  ↓ テストファイル
src/ui/__tests__/components/screens/BranchActionSelectorScreen.test.tsx
```

## 実装手順

### Phase 1: カレントブランチ選択時のWorktree作成スキップ

**Priority: P1** | **Estimated: 1-2 hours**

#### 1.1 WorktreeOrchestratorの改修

**ファイル**: `src/services/WorktreeOrchestrator.ts`

**変更箇所**: `ensureWorktree` メソッド

```typescript
import { getCurrentBranch } from '../git.js';

async ensureWorktree(
  branch: string,
  repoRoot: string,
  options: EnsureWorktreeOptions = {},
): Promise<string> {
  const baseBranch = options.baseBranch ?? "main";
  const isNewBranch = options.isNewBranch ?? false;

  // カレントブランチチェックを追加
  const currentBranch = await getCurrentBranch();
  if (currentBranch === branch) {
    this.logger.info(`Current branch selected: ${branch}. Using repository root.`);
    return repoRoot;
  }

  // 既存のロジック（変更なし）
  const existingPath = await this.worktreeService.worktreeExists(branch);
  if (existingPath) {
    // ...
  }
  // ...
}
```

#### 1.2 テストの追加

**ファイル**: `src/services/__tests__/WorktreeOrchestrator.test.ts`

```typescript
test('returns repository root when current branch is selected', async () => {
  // getCurrentBranch() が 'main' を返すようにモック
  vi.mock('../git.js', () => ({
    getCurrentBranch: vi.fn().mockResolvedValue('main'),
  }));

  const orchestrator = new WorktreeOrchestrator(mockService, mockLogger);
  const result = await orchestrator.ensureWorktree('main', '/repo/root');

  expect(result).toBe('/repo/root');
  // Worktree作成が呼ばれていないことを確認
  expect(mockService.createWorktree).not.toHaveBeenCalled();
});
```

### Phase 2: ブランチアクション選択画面の追加

**Priority: P2** | **Estimated: 2-3 hours**

#### 2.1 型定義の更新

**ファイル**: `src/ui/types.ts`

```typescript
// ScreenType に追加
export type ScreenType =
  | 'branch-list'
  | 'branch-action-selector'  // 新規追加
  | 'branch-creator'
  // ... 既存の型

// 新規型定義
export type BranchAction = 'use-existing' | 'create-new';
```

#### 2.2 BranchActionSelectorScreen の作成

**ファイル**: `src/ui/components/screens/BranchActionSelectorScreen.tsx`

```typescript
import React from 'react';
import { Box, Text } from 'ink';
import { Select } from '../common/Select.js';
import type { SelectedBranchState, BranchAction } from '../../types.js';

export interface BranchActionSelectorScreenProps {
  branch: SelectedBranchState;
  onUseExisting: () => void;
  onCreateNew: () => void;
  onBack: () => void;
}

interface SelectItem<T> {
  label: string;
  value: T;
}

export function BranchActionSelectorScreen({
  branch,
  onUseExisting,
  onCreateNew,
  onBack,
}: BranchActionSelectorScreenProps) {
  const actionItems: SelectItem<BranchAction>[] = [
    { label: '既存ブランチで続行', value: 'use-existing' },
    { label: '新規ブランチを作成', value: 'create-new' },
  ];

  const handleSelect = (action: BranchAction) => {
    if (action === 'use-existing') {
      onUseExisting();
    } else {
      onCreateNew();
    }
  };

  const handleInput = (input: string) => {
    if (input === 'q') {
      onBack();
    }
  };

  return (
    <Box flexDirection="column">
      <Text bold>ブランチ: {branch.displayName}</Text>
      <Text dimColor>実行するアクションを選択してください</Text>
      <Box marginTop={1}>
        <Select
          items={actionItems}
          onSelect={handleSelect}
          renderIndicator={isSelected => (isSelected ? '› ' : '  ')}
        />
      </Box>
      <Box marginTop={1}>
        <Text dimColor>[q] 戻る</Text>
      </Box>
    </Box>
  );
}
```

#### 2.3 App.tsx の改修

**ファイル**: `src/ui/components/App.tsx`

**変更箇所1**: 状態管理の追加

```typescript
const [selectedBranch, setSelectedBranch] = useState<SelectedBranchState | null>(null);
const [baseBranchForCreation, setBaseBranchForCreation] = useState<string | undefined>(undefined);
```

**変更箇所2**: handleSelect の修正

```typescript
const handleSelect = useCallback(async (item: BranchItem) => {
  const selection: SelectedBranchState = item.type === 'remote'
    ? {
        name: toLocalBranchName(item.name),
        displayName: item.name,
        branchType: 'remote',
        remoteBranch: item.name,
      }
    : {
        name: item.name,
        displayName: item.name,
        branchType: 'local',
      };

  setSelectedBranch(selection);

  // カレントブランチ判定
  const currentBranch = await getCurrentBranch();
  if (currentBranch === selection.name) {
    // カレントブランチの場合、直接AIツール選択へ
    setSelectedTool(null);
    navigateTo('ai-tool-selector');
  } else {
    // カレントブランチでない場合、アクション選択画面へ
    navigateTo('branch-action-selector');
  }
}, [...]);
```

**変更箇所3**: ハンドラーの追加

```typescript
const handleBranchActionUseExisting = useCallback(() => {
  setSelectedTool(null);
  navigateTo('ai-tool-selector');
}, [navigateTo]);

const handleBranchActionCreate = useCallback(() => {
  const baseBranch = selectedBranch?.branchType === 'remote'
    ? selectedBranch.remoteBranch
    : selectedBranch?.name;
  setBaseBranchForCreation(baseBranch);
  navigateTo('branch-creator');
}, [selectedBranch, navigateTo]);
```

**変更箇所4**: renderScreen の更新

```typescript
function renderScreen(): React.ReactNode {
  switch (currentScreen) {
    // ... 既存のケース

    case 'branch-action-selector':
      return selectedBranch ? (
        <BranchActionSelectorScreen
          branch={selectedBranch}
          onUseExisting={handleBranchActionUseExisting}
          onCreateNew={handleBranchActionCreate}
          onBack={goBack}
        />
      ) : null;

    case 'branch-creator':
      return (
        <BranchCreatorScreen
          onBack={goBack}
          onCreate={handleCreate}
          baseBranch={baseBranchForCreation}  // 追加
        />
      );

    // ... 既存のケース
  }
}
```

### Phase 3: BranchCreatorScreen のベースブランチ対応

**Priority: P3** | **Estimated: 1 hour**

#### 3.1 Props の更新

**ファイル**: `src/ui/components/screens/BranchCreatorScreen.tsx`

```typescript
export interface BranchCreatorScreenProps {
  onBack: () => void;
  onCreate: (branchName: string) => Promise<void>;
  baseBranch?: string;  // 新規追加
}
```

#### 3.2 ベースブランチ決定ロジックの更新

```typescript
export function BranchCreatorScreen({
  onBack,
  onCreate,
  baseBranch,  // 追加
}: BranchCreatorScreenProps) {
  // ...

  const handleCreate = async () => {
    const fullBranchName = `${branchType}/${branchName}`;

    // ベースブランチの決定
    const effectiveBaseBranch = baseBranch ?? resolveBaseBranch();

    // Worktree作成（effectiveBaseBranchを使用）
    // ...
  };

  // ...
}
```

## デバッグ

### ログの確認

```bash
# Worktreeロジックのログを確認
bun run start 2>&1 | grep -i worktree

# カレントブランチ判定のログを確認
bun run start 2>&1 | grep -i "current branch"
```

### 開発ツール

```bash
# TypeScriptの型チェック
bun run type-check

# Lintチェック
bun run lint

# フォーマットチェック
bun run format:check
```

## トラブルシューティング

### よくある問題

#### 問題1: getCurrentBranch() が null を返す

**原因**: detached HEAD状態、またはGitが見つからない

**解決策**:
```bash
# 現在のブランチを確認
git branch --show-current

# detached HEADの場合、ブランチにチェックアウト
git checkout main
```

#### 問題2: Ink.jsコンポーネントが表示されない

**原因**: Reactのインポートが不足、またはPropsの型不一致

**解決策**:
```typescript
// Reactのインポートを確認
import React from 'react';

// Propsの型チェック
bun run type-check
```

#### 問題3: テストが失敗する

**原因**: モックが正しく設定されていない

**解決策**:
```typescript
// vi.mock() をファイルの先頭に配置
vi.mock('../git.js', () => ({
  getCurrentBranch: vi.fn().mockResolvedValue('main'),
}));
```

## ベストプラクティス

### コーディング規約

1. **既存のパターンを踏襲**
   - Selectコンポーネントの使い方
   - useScreenStateフックの使い方
   - エラーハンドリングの方法

2. **型安全性の確保**
   ```typescript
   // 良い例
   const action: BranchAction = 'use-existing';

   // 悪い例
   const action = 'use-existing';  // 型推論に頼らない
   ```

3. **テストファースト**
   - 実装前にテストシナリオを作成
   - Red → Green → Refactor のサイクル

### パフォーマンス

1. **useCallback の活用**
   ```typescript
   const handleSelect = useCallback((item: BranchItem) => {
     // ...
   }, [/* 依存配列 */]);
   ```

2. **不要な再レンダリングを避ける**
   - React.memoの適切な使用
   - 状態の適切な分離

## 次のステップ

1. ✅ Phase 0: 調査完了（research.md）
2. ✅ Phase 1: 設計完了（data-model.md、quickstart.md）
3. ⏭️ Phase 2: タスク生成（`/speckit.tasks`）
4. ⏭️ Phase 3: 実装開始（`/speckit.implement`）

## 参考資料

- [Ink.js ドキュメント](https://github.com/vadimdemedes/ink)
- [Vitest ドキュメント](https://vitest.dev/)
- [TypeScript ドキュメント](https://www.typescriptlang.org/docs/)
- プロジェクトの CLAUDE.md: 開発指針
- 仕様書: [spec.md](./spec.md)
