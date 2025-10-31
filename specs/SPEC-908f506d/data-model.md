# データモデル設計: ブランチ作成・選択機能の改善

**仕様ID**: `SPEC-908f506d` | **日付**: 2025-10-29
**関連**: [spec.md](./spec.md) | [plan.md](./plan.md) | [research.md](./research.md)

## 概要

このドキュメントでは、ブランチ作成・選択機能の改善に必要なデータモデルとインターフェースを定義します。

## 主要エンティティ

### 1. BranchItem

ブランチ一覧に表示されるブランチの情報を表します。

```typescript
interface BranchItem {
  name: string;           // ブランチ名（例: "feature/new-ui", "origin/develop"）
  type: 'local' | 'remote';  // ブランチタイプ
  isCurrent: boolean;     // カレントブランチかどうか
  hasWorktree: boolean;   // Worktreeが存在するか
  displayName?: string;   // 表示用の名前（オプション）
}
```

**フィールド詳細**:

- `name`: Gitブランチ名。ローカルブランチは "feature/test"、リモートブランチは "origin/feature/test" の形式
- `type`: ローカルブランチかリモートブランチかを区別
- `isCurrent`: `git branch --show-current` の結果と一致する場合 true
- `hasWorktree`: `git worktree list` で検出された場合 true
- `displayName`: UI表示用の名前（remoteブランチの場合、"origin/" を除いた名前など）

**使用箇所**:
- BranchListScreen: ブランチ一覧の表示
- BranchActionSelectorScreen: 選択されたブランチ情報の表示

**検証ルール**:
- `name` は必須かつ空文字列不可
- `type` は 'local' または 'remote' のみ
- `isCurrent` は true の場合、`type` は 'local' でなければならない

### 2. SelectedBranchState

ユーザーが選択したブランチの状態を保持します。

```typescript
interface SelectedBranchState {
  name: string;                    // ブランチ名
  displayName: string;             // 表示用の名前
  branchType: 'local' | 'remote';  // ブランチタイプ
  remoteBranch?: string;           // リモートブランチの完全参照（remoteタイプの場合）
}
```

**フィールド詳細**:

- `name`: ブランチ名（remoteの場合は "origin/" を除いたローカル名）
- `displayName`: UI表示用の名前
- `branchType`: ブランチタイプ
- `remoteBranch`: リモートブランチの完全参照（例: "origin/feature/test"）。ローカルブランチの場合は undefined

**使用箇所**:
- App.tsx: ブランチ選択状態の管理
- BranchActionSelectorScreen: 選択されたブランチ情報の表示
- WorktreeOrchestrator: Worktree作成時のブランチ指定

**状態遷移**:
```
初期状態: null
  ↓ ユーザーがブランチ選択
選択状態: SelectedBranchState
  ↓ AIツール起動完了
初期状態: null（リセット）
```

### 3. BranchAction

ブランチ選択後のアクションを表します。

```typescript
type BranchAction = 'use-existing' | 'create-new';
```

**値の意味**:
- `'use-existing'`: 選択したブランチをそのまま使用
- `'create-new'`: 選択したブランチをベースに新規ブランチを作成

**使用箇所**:
- BranchActionSelectorScreen: ユーザーの選択を処理

**状態フロー**:
```
BranchActionSelectorScreen表示
  ↓ ユーザーが選択
'use-existing' → AIToolSelectorScreen へ遷移
'create-new'   → BranchCreatorScreen へ遷移
```

### 4. ScreenType（拡張）

画面タイプを表す型定義に新しい画面を追加します。

```typescript
type ScreenType =
  | 'branch-list'
  | 'branch-action-selector'  // 新規追加
  | 'branch-creator'
  | 'worktree-manager'
  | 'ai-tool-selector'
  | 'execution-mode-selector'
  | 'permission-selector'
  | 'merged-prs-cleanup';
```

**新規追加**:
- `'branch-action-selector'`: ブランチ選択後のアクション選択画面

**使用箇所**:
- App.tsx: `useScreenState` フックで画面管理
- 画面遷移ロジック: `navigateTo()` の引数として使用

## コンポーネントインターフェース

### BranchActionSelectorScreen Props

新規作成する画面コンポーネントのProps定義です。

```typescript
interface BranchActionSelectorScreenProps {
  branch: SelectedBranchState;   // 選択されたブランチ情報
  onUseExisting: () => void;     // 既存ブランチで続行
  onCreateNew: () => void;       // 新規ブランチを作成
  onBack: () => void;            // 前の画面に戻る
}
```

**フィールド詳細**:

- `branch`: 選択されたブランチの情報（表示用）
- `onUseExisting`: 「既存ブランチで続行」が選択された時のコールバック
- `onCreateNew`: 「新規ブランチを作成」が選択された時のコールバック
- `onBack`: 'q' キーまたはESCで前の画面に戻るコールバック

**使用例**:
```typescript
<BranchActionSelectorScreen
  branch={selectedBranch}
  onUseExisting={() => navigateTo('ai-tool-selector')}
  onCreateNew={handleBranchActionCreate}
  onBack={goBack}
/>
```

### BranchCreatorScreen Props（拡張）

既存のコンポーネントに `baseBranch` を追加します。

```typescript
interface BranchCreatorScreenProps {
  onBack: () => void;
  onCreate: (branchName: string) => Promise<void>;
  baseBranch?: string;  // 新規追加：オプショナル
}
```

**新規フィールド**:

- `baseBranch`: ベースブランチ名（指定された場合、このブランチから新規ブランチを作成）
  - undefined の場合、従来通り `resolveBaseBranch()` で自動決定
  - 指定された場合、そのブランチを使用

**使用例**:
```typescript
// 'n' キーからの直接作成（従来通り）
<BranchCreatorScreen
  onBack={goBack}
  onCreate={handleCreate}
  // baseBranch は未指定（自動決定）
/>

// アクション選択からの作成（新機能）
<BranchCreatorScreen
  onBack={goBack}
  onCreate={handleCreate}
  baseBranch={baseBranchForCreation}  // 選択したブランチ
/>
```

## サービス層インターフェース

### EnsureWorktreeOptions（変更なし）

WorktreeOrchestratorのオプションは既存のまま使用します。

```typescript
interface EnsureWorktreeOptions {
  baseBranch?: string;
  isNewBranch?: boolean;
}
```

カレントブランチ判定は `ensureWorktree()` メソッド内で自動的に行われるため、新しいオプションは不要です。

## データフロー

### フロー1: カレントブランチを選択

```
1. BranchListScreen
   ↓ ユーザーがカレントブランチ (isCurrent=true) を選択
2. handleSelect() 実行
   → selectedBranch = { name: 'main', branchType: 'local', ... }
   ↓ カレントブランチ判定
3. navigateTo('ai-tool-selector')  // 直接遷移
   ↓
4. ensureWorktree('main', repoRoot)
   → getCurrentBranch() === 'main' → return repoRoot
   ↓
5. AIツール起動（ルートディレクトリ）
```

**データの流れ**:
- BranchItem (isCurrent=true) → SelectedBranchState → ensureWorktree → repoRoot

### フロー2: 他のブランチを選択（既存使用）

```
1. BranchListScreen
   ↓ ユーザーが他のブランチ (isCurrent=false) を選択
2. handleSelect() 実行
   → selectedBranch = { name: 'feature/test', branchType: 'local', ... }
   ↓ カレントブランチでないため
3. navigateTo('branch-action-selector')
   ↓
4. BranchActionSelectorScreen 表示
   ↓ ユーザーが「既存ブランチで続行」を選択
5. onUseExisting() → navigateTo('ai-tool-selector')
   ↓
6. ensureWorktree('feature/test', repoRoot)
   → Worktree作成または再利用
   ↓
7. AIツール起動（Worktreeディレクトリ）
```

**データの流れ**:
- BranchItem (isCurrent=false) → SelectedBranchState → BranchAction('use-existing') → ensureWorktree → worktreePath

### フロー3: 他のブランチを選択（新規作成）

```
1. BranchListScreen
   ↓ ユーザーが他のブランチ (develop) を選択
2. handleSelect() 実行
   → selectedBranch = { name: 'develop', branchType: 'local', ... }
   ↓
3. navigateTo('branch-action-selector')
   ↓
4. BranchActionSelectorScreen 表示
   ↓ ユーザーが「新規ブランチを作成」を選択
5. onCreateNew() → baseBranchForCreation = 'develop'
   → navigateTo('branch-creator')
   ↓
6. BranchCreatorScreen 表示（baseBranch='develop'）
   ↓ ユーザーがブランチ名を入力（例: 'feature/new-ui'）
7. onCreate('feature/new-ui') 実行
   → ensureWorktree('feature/new-ui', repoRoot, { baseBranch: 'develop', isNewBranch: true })
   ↓
8. 新規Worktree作成
   ↓
9. AIツール起動（新規Worktreeディレクトリ）
```

**データの流れ**:
- BranchItem → SelectedBranchState → BranchAction('create-new') → baseBranchForCreation → BranchCreatorScreen(baseBranch) → ensureWorktree → worktreePath

## 検証ルール

### BranchItem検証

```typescript
function validateBranchItem(item: BranchItem): boolean {
  // 名前は必須
  if (!item.name || item.name.trim() === '') return false;

  // typeは'local'または'remote'のみ
  if (item.type !== 'local' && item.type !== 'remote') return false;

  // isCurrentがtrueの場合、typeは'local'でなければならない
  if (item.isCurrent && item.type !== 'local') return false;

  return true;
}
```

### SelectedBranchState検証

```typescript
function validateSelectedBranchState(state: SelectedBranchState): boolean {
  // 名前は必須
  if (!state.name || state.name.trim() === '') return false;

  // branchTypeは'local'または'remote'のみ
  if (state.branchType !== 'local' && state.branchType !== 'remote') return false;

  // remoteタイプの場合、remoteBranchは必須
  if (state.branchType === 'remote' && !state.remoteBranch) return false;

  return true;
}
```

## エラーハンドリング

### getCurrentBranch() が null を返す場合

**ケース**: detached HEAD状態、またはGitコマンドエラー

**処理**:
```typescript
const currentBranch = await getCurrentBranch();
if (!currentBranch) {
  // カレントブランチが判定できない場合、通常フローへ
  // （アクション選択画面を表示）
}
```

### ensureWorktree() が失敗した場合

**ケース**: Worktree作成エラー、パーミッションエラー

**処理**:
- エラーメッセージを表示
- ブランチ一覧画面に戻る
- ログにエラー詳細を記録

## パフォーマンス考慮事項

### データ取得の最適化

- **getCurrentBranch()**: 高速（< 10ms）、キャッシュ不要
- **getAllBranches()**: 初回のみ取得、リフレッシュ時に再取得
- **worktreeExists()**: 高速（< 50ms）、既存Worktree検索

### メモリ使用量

- **BranchItem配列**: 通常100ブランチ以下（< 10KB）
- **SelectedBranchState**: 単一オブジェクト（< 1KB）
- 影響は無視できる

## まとめ

このデータモデル設計により、以下が実現されます：

1. ✅ カレントブランチの判定と特別な処理
2. ✅ ブランチ選択後のアクション選択
3. ✅ 選択したブランチをベースに新規ブランチ作成
4. ✅ 型安全性の確保（TypeScript）
5. ✅ 既存コードとの整合性

次のステップ: quickstart.md の作成
