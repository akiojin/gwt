# 調査レポート: ブランチ作成・選択機能の改善

**仕様ID**: `SPEC-908f506d` | **日付**: 2025-10-29
**関連**: [spec.md](./spec.md) | [plan.md](./plan.md)

## 概要

このドキュメントは、ブランチ作成・選択機能の改善実装に向けて、既存のコードベースを調査し、技術的決定事項をまとめたものです。

## 1. 既存のコードベース分析

### 1.1 Ink.js画面遷移パターン（useScreenState）

**ファイル**: `src/ui/hooks/useScreenState.ts`

**パターン**:
```typescript
export function useScreenState(initialScreen: ScreenType) {
  const [screenStack, setScreenStack] = useState<ScreenType[]>([initialScreen]);

  const navigateTo = useCallback((screen: ScreenType) => {
    setScreenStack(prev => [...prev, screen]);
  }, []);

  const goBack = useCallback(() => {
    setScreenStack(prev => prev.length > 1 ? prev.slice(0, -1) : prev);
  }, []);

  return {
    currentScreen: screenStack[screenStack.length - 1],
    navigateTo,
    goBack,
  };
}
```

**特徴**:
- スタック型履歴管理
- `navigateTo()` で新しい画面をプッシュ
- `goBack()` で前の画面にポップ
- 最後の画面が常に表示される

**適用**:
- BranchActionSelectorScreenを新規追加する際も同じパターンを使用
- 既存の画面遷移ロジックに統合可能

### 1.2 既存のSelectコンポーネント

**ファイル**: `src/ui/components/common/Select.tsx`

**Props**:
```typescript
interface SelectProps<T> {
  items: SelectItem<T>[];
  onSelect: (item: T) => void;
  limit?: number;
  renderIndicator?: (isSelected: boolean) => string;
  renderItem?: (item: SelectItem<T>, isSelected: boolean) => string;
}

interface SelectItem<T> {
  label: string;
  value: T;
}
```

**特徴**:
- ジェネリック型で任意のデータを扱える
- カスタムレンダリング可能
- キーボード操作（↑↓ / j/k）
- ループなしカーソル（境界で停止）

**適用例（BranchActionSelectorScreen）**:
```typescript
const actionItems: SelectItem<BranchAction>[] = [
  { label: '既存ブランチで続行', value: 'use-existing' },
  { label: '新規ブランチを作成', value: 'create-new' },
];

<Select
  items={actionItems}
  onSelect={handleActionSelect}
  renderIndicator={isSelected => isSelected ? '› ' : '  '}
/>
```

### 1.3 BranchCreatorScreen の現在の実装

**ファイル**: `src/ui/components/screens/BranchCreatorScreen.tsx`

**現在のProps**:
```typescript
export interface BranchCreatorScreenProps {
  onBack: () => void;
  onCreate: (branchName: string) => Promise<void>;
}
```

**ステップ構成**:
1. ブランチタイプ選択（feature/hotfix/release）
2. ブランチ名入力
3. 作成実行

**ベースブランチの決定**:
- `resolveBaseBranch()` 関数で自動決定（main → master → develop → dev）
- ユーザーが指定する方法がない

**必要な改修**:
```typescript
export interface BranchCreatorScreenProps {
  onBack: () => void;
  onCreate: (branchName: string) => Promise<void>;
  baseBranch?: string;  // 追加：オプショナル
}
```

- `baseBranch` が指定されていればそれを使用
- 指定されていなければ `resolveBaseBranch()` で自動決定（既存の挙動を維持）

### 1.4 WorktreeOrchestrator の現在のフロー

**ファイル**: `src/services/WorktreeOrchestrator.ts`

**ensureWorktree メソッド**:
```typescript
async ensureWorktree(
  branch: string,
  repoRoot: string,
  options: EnsureWorktreeOptions = {},
): Promise<string> {
  const baseBranch = options.baseBranch ?? "main";
  const isNewBranch = options.isNewBranch ?? false;

  // 既存worktree検索
  const existingPath = await this.worktreeService.worktreeExists(branch);
  if (existingPath) {
    return existingPath;
  }

  // 新規worktree作成
  const worktreePath = await generateWorktreePath(repoRoot, branch);
  await this.worktreeService.createWorktree({
    branchName: branch,
    worktreePath,
    repoRoot,
    isNewBranch,
    baseBranch,
  });

  return worktreePath;
}
```

**問題点**:
- カレントブランチかどうかをチェックしていない
- 常にWorktree作成を試みる

**必要な改修**:
```typescript
async ensureWorktree(
  branch: string,
  repoRoot: string,
  options: EnsureWorktreeOptions = {},
): Promise<string> {
  // カレントブランチチェックを追加
  const currentBranch = await getCurrentBranch();
  if (currentBranch === branch) {
    // カレントブランチの場合、リポジトリルートを返す
    return repoRoot;
  }

  // 既存のロジック...
}
```

### 1.5 カレントブランチ判定の既存実装

**ファイル**: `src/git.ts`

**getCurrentBranch 関数**:
```typescript
async function getCurrentBranch(): Promise<string | null> {
  try {
    const { stdout } = await execa("git", ["branch", "--show-current"]);
    return stdout.trim() || null;
  } catch {
    return null;
  }
}
```

**特徴**:
- `git branch --show-current` を使用
- detached HEADの場合は空文字列→null
- エラー時はnull

**使用方法**:
- `getAllBranches()` 関数内で既に使用されている
- `isCurrent` フラグの設定に利用
- WorktreeOrchestratorでも同じ関数を使用可能

## 2. 技術的決定

### 決定1: BranchActionSelectorScreen の実装パターン

**選択肢**:
1. 新規カスタムコンポーネント
2. 既存のSelectコンポーネントを再利用

**決定**: 既存のSelectコンポーネントを再利用

**理由**:
- 既存のUIパターンと一貫性を保つ
- 開発時間の短縮
- テスト済みのコンポーネントを活用
- メンテナンス性の向上

**実装例**:
```typescript
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

  return (
    <Box flexDirection="column">
      <Text bold>ブランチ: {branch.displayName}</Text>
      <Text dimColor>実行するアクションを選択してください</Text>
      <Box marginTop={1}>
        <Select
          items={actionItems}
          onSelect={handleSelect}
          renderIndicator={isSelected => isSelected ? '› ' : '  '}
        />
      </Box>
    </Box>
  );
}
```

**代替案**:
- カスタムコンポーネント：より柔軟だが、開発コストが高い

### 決定2: カレントブランチ判定の実装場所

**選択肢**:
1. App.tsx の `handleSelect` 内で判定
2. WorktreeOrchestrator.ensureWorktree() 内で判定
3. 新規サービス層を追加

**決定**: WorktreeOrchestrator.ensureWorktree() 内で判定

**理由**:
- Worktree作成ロジックとカレントブランチ判定は密接に関連
- 単一責任の原則に従う（Worktree管理はOrchestratorの責任）
- 他の呼び出し箇所でも自動的に適用される
- テストが容易

**実装**:
```typescript
async ensureWorktree(
  branch: string,
  repoRoot: string,
  options: EnsureWorktreeOptions = {},
): Promise<string> {
  // カレントブランチチェック
  const currentBranch = await getCurrentBranch();
  if (currentBranch === branch) {
    this.logger.info(`Current branch selected: ${branch}. Using repository root.`);
    return repoRoot;
  }

  // 既存のロジック...
}
```

**代替案**:
- App.tsx内での判定：UI層が過度に複雑になる
- 新規サービス：過剰設計

### 決定3: ベースブランチの渡し方

**選択肢**:
1. App.tsx で状態管理（useState）
2. React Context で共有
3. グローバルステート管理（Redux/Zustand）

**決定**: App.tsx で状態管理（useState）

**理由**:
- シンプルで理解しやすい
- スコープが限定的（アプリ全体で共有する必要なし）
- 既存のパターンと一貫性（`selectedBranch` も同様に管理）
- 追加の依存関係不要

**実装**:
```typescript
// App.tsx
const [selectedBranch, setSelectedBranch] = useState<SelectedBranchState | null>(null);
const [baseBranchForCreation, setBaseBranchForCreation] = useState<string | undefined>(undefined);

const handleBranchActionCreate = useCallback(() => {
  // 選択したブランチをベースブランチとして設定
  setBaseBranchForCreation(selectedBranch?.name);
  navigateTo('branch-creator');
}, [selectedBranch, navigateTo]);

// BranchCreatorScreenへの渡し方
<BranchCreatorScreen
  onBack={goBack}
  onCreate={handleCreate}
  baseBranch={baseBranchForCreation}
/>
```

**代替案**:
- React Context：小規模なステートには過剰
- グローバルステート：このアプリには不要な複雑さ

## 3. 制約と依存関係

### 制約1: 既存のInk.js画面遷移システムとの統合

**制約内容**:
- `useScreenState` フックのスタック型履歴管理に従う
- `navigateTo` と `goBack` のみを使用
- 画面の状態は親コンポーネント（App.tsx）で管理

**対応策**:
- BranchActionSelectorScreenも同じパターンに従う
- `ScreenType` に `'branch-action-selector'` を追加
- `renderScreen()` 内で新しいケースを追加

### 制約2: 'n'キーの直接作成フローとの共存

**制約内容**:
- 'n'キーを押すと、直接BranchCreatorScreenに遷移する既存フロー
- この動作を変更してはならない

**対応策**:
- 'n'キーのハンドラー（`handleNewBranch`）は変更しない
- BranchActionSelectorScreenはEnterキー選択時のみ表示
- 2つのフローが独立して動作するように実装

**フロー図**:
```
BranchListScreen
  ├─ 'n'キー → BranchCreatorScreen (baseBranch=undefined)
  └─ Enter → BranchActionSelectorScreen
              ├─ '既存使用' → AIToolSelectorScreen
              └─ '新規作成' → BranchCreatorScreen (baseBranch=selectedBranch)
```

### 制約3: ブランチ切り替え禁止

**制約内容**:
- CLAUDE.mdの指針：「ブランチ切り替えは絶対禁止」
- Worktreeは起動したブランチで作業を完結させる設計

**対応策**:
- すべての操作は現在のブランチで完結
- 新規ブランチ作成もWorktreeで実行
- カレントブランチ選択時は、そのままルートディレクトリを使用

## 4. リスクと緩和策

### リスク1: detached HEAD状態の処理

**リスク内容**:
- `git branch --show-current` がdetached HEADでは空文字列を返す
- カレントブランチ判定が不正確になる

**緩和策**:
```typescript
const currentBranch = await getCurrentBranch();
if (!currentBranch) {
  // detached HEADの場合は通常フローへ
  // （アクション選択画面を表示）
}
```

### リスク2: リモートブランチをベースに新規作成

**リスク内容**:
- ユーザーがリモートブランチ（例: origin/feature-x）を選択して新規作成する場合
- ベースブランチの指定が正しく渡されるか

**緩和策**:
- `selectedBranch` には `remoteBranch` フィールドが含まれる
- リモートブランチの場合、完全な参照（例: origin/feature-x）をベースブランチとして使用
- `git worktree add` はリモート参照をサポート

**実装**:
```typescript
const handleBranchActionCreate = useCallback(() => {
  const baseBranch = selectedBranch?.branchType === 'remote'
    ? selectedBranch.remoteBranch
    : selectedBranch?.name;
  setBaseBranchForCreation(baseBranch);
  navigateTo('branch-creator');
}, [selectedBranch, navigateTo]);
```

### リスク3: 既存Worktreeとの衝突

**リスク内容**:
- カレントブランチのWorktreeが既に別の場所に存在する場合
- どちらを優先すべきか

**緩和策**:
```typescript
// カレントブランチ判定を最優先
const currentBranch = await getCurrentBranch();
if (currentBranch === branch) {
  return repoRoot;  // Worktree検索より優先
}

// 既存Worktree検索
const existingPath = await this.worktreeService.worktreeExists(branch);
if (existingPath) {
  return existingPath;
}
```

## 5. パフォーマンス考慮事項

### getCurrentBranch() の呼び出し頻度

**現状**:
- `getAllBranches()` で1回呼び出し（ブランチ一覧取得時）
- `ensureWorktree()` で追加呼び出し

**影響**:
- `git branch --show-current` は高速（< 10ms）
- パフォーマンスへの影響は無視できる

**最適化不要**:
- キャッシュは不要（ブランチ切り替え禁止のため、値は変わらない）

## 6. テスト計画

### ユニットテスト

**WorktreeOrchestrator**:
- カレントブランチを選択した場合、リポジトリルートを返す
- カレントブランチ以外を選択した場合、Worktreeパスを返す
- getCurrentBranch()がnullの場合、通常フローを実行

**BranchActionSelectorScreen**:
- 2つの選択肢が表示される
- '既存ブランチで続行'を選択すると`onUseExisting`が呼ばれる
- '新規ブランチを作成'を選択すると`onCreateNew`が呼ばれる

**BranchCreatorScreen**:
- `baseBranch`が指定されている場合、それを使用
- `baseBranch`が未指定の場合、`resolveBaseBranch()`を使用

### 統合テスト

**画面遷移フロー**:
1. BranchListScreen → カレントブランチ選択 → AIToolSelectorScreen（直接）
2. BranchListScreen → 他のブランチ選択 → BranchActionSelectorScreen → '既存使用' → AIToolSelectorScreen
3. BranchListScreen → 他のブランチ選択 → BranchActionSelectorScreen → '新規作成' → BranchCreatorScreen

### モックデータ

**テスト用ブランチ情報**:
```typescript
const mockBranches: BranchItem[] = [
  { name: 'main', type: 'local', isCurrent: true, hasWorktree: false },
  { name: 'feature/test', type: 'local', isCurrent: false, hasWorktree: true },
  { name: 'origin/feature/remote', type: 'remote', isCurrent: false, hasWorktree: false },
];
```

## 7. まとめ

### 実装の準備完了

以下の技術的決定により、実装の準備が整いました：

1. ✅ **BranchActionSelectorScreen**: 既存のSelectコンポーネントを再利用
2. ✅ **カレントブランチ判定**: WorktreeOrchestrator.ensureWorktree()内で実装
3. ✅ **ベースブランチ管理**: App.tsxでuseStateを使用

### 次のステップ

1. Phase 1に進む（data-model.md、quickstart.md作成）
2. タスク生成（/speckit.tasks）
3. 実装開始（/speckit.implement）

### 未解決の質問

なし - すべての技術的不確実性が解決されました。
