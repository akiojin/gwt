# データモデル: UI移行 - Ink.js（React）ベースのCLIインターフェース

**SPEC ID**: SPEC-4c2ef107
**日付**: 2025-01-25
**関連**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md)

## 概要

このドキュメントは、Ink.js UI移行で使用する主要なエンティティとその関係を定義します。すべてのエンティティはTypeScriptの型として実装され、既存の`src/ui/types.ts`を拡張します。

## 主要エンティティ

### 1. Screen（画面）

アプリケーションの各画面を表す状態管理エンティティ。

#### 属性

| 属性 | 型 | 説明 | 必須 |
|------|-----|------|------|
| type | ScreenType | 画面の種類 | ✅ |
| state | ScreenState | 画面の状態 | ✅ |
| data | unknown | 画面固有のデータ | ❌ |

#### 型定義

```typescript
type ScreenType =
  | 'branch-list'
  | 'worktree-manager'
  | 'branch-creator'
  | 'pr-cleanup'
  | 'ai-tool-selector'
  | 'session-selector'
  | 'execution-mode-selector';

type ScreenState = 'active' | 'hidden';

interface Screen {
  type: ScreenType;
  state: ScreenState;
  data?: unknown;
}
```

#### 画面遷移ルール

```
branch-list (初期画面)
  ├─> worktree-manager (mキー)
  ├─> branch-creator (nキー)
  ├─> pr-cleanup (cキー)
  ├─> ai-tool-selector (ブランチ選択時)
  └─> session-selector (-rオプション時)

ai-tool-selector
  └─> execution-mode-selector

各サブ画面
  └─> branch-list (qキー、ESCキー)
```

#### 検証ルール

- 1つの画面のみが`active`状態
- 他の画面は`hidden`状態
- 不正な遷移は許可しない

### 2. BranchItem（ブランチアイテム）

表示用のブランチ情報。既存の`BranchInfo`を拡張して表示用プロパティを追加。

#### 属性

| 属性 | 型 | 説明 | 必須 |
|------|-----|------|------|
| name | string | ブランチ名 | ✅ |
| type | "local" \| "remote" | ブランチタイプ | ✅ |
| branchType | BranchType | ブランチの分類 | ✅ |
| isCurrent | boolean | 現在のブランチか | ✅ |
| icons | string[] | 表示用アイコン配列 | ✅ |
| worktreeStatus | WorktreeStatus | Worktree状態 | ❌ |
| hasChanges | boolean | 変更があるか | ✅ |
| label | string | 表示用ラベル | ✅ |
| value | string | 選択時の値 | ✅ |

#### 型定義

```typescript
type BranchType = "feature" | "hotfix" | "release" | "main" | "develop" | "other";
type WorktreeStatus = "active" | "inaccessible" | undefined;

interface BranchItem {
  // 既存のBranchInfoから継承
  name: string;
  type: "local" | "remote";
  branchType: BranchType;
  isCurrent: boolean;
  description?: string;

  // 表示用プロパティ（追加）
  icons: string[];
  worktreeStatus?: WorktreeStatus;
  hasChanges: boolean;
  label: string;
  value: string;
}
```

#### アイコンマッピング

```typescript
// icons配列の構成: [branchIcon, worktreeIcon?, changeIcon?, remoteIcon?]

// branchIcon（ブランチタイプ）
const branchIcons: Record<BranchType, string> = {
  main: '⚡',
  develop: '⚡',
  feature: '✨',
  hotfix: '🔥',
  release: '🚀',
  other: '📌'
};

// worktreeIcon（Worktree状態）
const worktreeIcons: Record<WorktreeStatus, string | undefined> = {
  active: '🟢',
  inaccessible: '🟠',
  undefined: undefined
};

// changeIcon（変更状態）
const changeIcons = {
  current: '⭐',
  hasChanges: '✏️',
  warning: '⚠️',
  none: undefined
};

// remoteIcon（リモートマーク）
const remoteIcon = '☁';
```

#### 検証ルール

- `name`は空文字列禁止
- `type`が"remote"の場合、通常"origin/"で始まる
- `isCurrent`が`true`の場合、`changeIcon`は'⭐'
- `label`は`icons + location + name`の形式

### 3. Statistics（統計情報）

リアルタイム更新される集計データ。

#### 属性

| 属性 | 型 | 説明 | 必須 |
|------|-----|------|------|
| localCount | number | ローカルブランチ数 | ✅ |
| remoteCount | number | リモートブランチ数 | ✅ |
| worktreeCount | number | Worktree数 | ✅ |
| changesCount | number | 変更のあるWorktree数 | ✅ |
| lastUpdated | Date | 最終更新日時 | ✅ |

#### 型定義

```typescript
interface Statistics {
  localCount: number;
  remoteCount: number;
  worktreeCount: number;
  changesCount: number;
  lastUpdated: Date;
}
```

#### 更新頻度

- **P1/P2実装時**: 画面表示時に1回計算（静的）
- **P3実装時**: バックグラウンドで定期更新（リアルタイム）
  - 更新間隔: 5秒（設定可能）
  - Git操作後は即座に更新

#### 検証ルール

- すべてのカウントは非負整数
- `worktreeCount`は`localCount`以下

### 4. Layout（レイアウト）

画面のレイアウト情報。ターミナルサイズに基づいて動的に計算。

#### 属性

| 属性 | 型 | 説明 | 必須 |
|------|-----|------|------|
| terminalHeight | number | ターミナルの高さ（行数） | ✅ |
| terminalWidth | number | ターミナルの幅（列数） | ✅ |
| headerLines | number | ヘッダーの行数 | ✅ |
| footerLines | number | フッターの行数 | ✅ |
| contentHeight | number | コンテンツ領域の高さ | ✅ |

#### 型定義

```typescript
interface Layout {
  terminalHeight: number;
  terminalWidth: number;
  headerLines: number;
  footerLines: number;
  contentHeight: number;
}
```

#### 計算式

```typescript
const calculateLayout = (): Layout => {
  const terminalHeight = process.stdout.rows || 24;
  const terminalWidth = process.stdout.columns || 80;

  const headerLines = 4; // タイトル + 区切り + 統計 + 空行
  const footerLines = 1; // アクション行

  const contentHeight = Math.max(
    5, // 最低5行
    terminalHeight - headerLines - footerLines
  );

  return {
    terminalHeight,
    terminalWidth,
    headerLines,
    footerLines,
    contentHeight
  };
};
```

#### 動的再計算

```typescript
useEffect(() => {
  const handleResize = () => {
    setLayout(calculateLayout());
  };

  process.stdout.on('resize', handleResize);
  return () => process.stdout.off('resize', handleResize);
}, []);
```

#### 検証ルール

- `terminalHeight` >= 10（最低高さ）
- `terminalWidth` >= 40（最低幅）
- `contentHeight` >= 5（最低コンテンツ高さ）

## エンティティ関係図

```
┌──────────────┐
│   Screen     │
│  (画面状態)   │
└──────┬───────┘
       │ manages
       ▼
┌──────────────┐     displays      ┌──────────────┐
│ BranchItem   │◄──────────────────│   Layout     │
│ (表示データ) │                    │ (レイアウト)  │
└──────┬───────┘                    └──────────────┘
       │ aggregates
       ▼
┌──────────────┐
│ Statistics   │
│ (統計情報)    │
└──────────────┘
```

## 既存エンティティとの関係

### 継承・拡張

```typescript
// 既存の型（src/ui/types.ts）
interface BranchInfo {
  name: string;
  type: "local" | "remote";
  branchType: BranchType;
  isCurrent: boolean;
  description?: string;
}

// 新規の型（拡張）
interface BranchItem extends BranchInfo {
  icons: string[];
  worktreeStatus?: WorktreeStatus;
  hasChanges: boolean;
  label: string;
  value: string;
}
```

### 変換関数

```typescript
// BranchInfo → BranchItem
const toBranchItem = async (
  branch: BranchInfo,
  worktree?: WorktreeInfo
): Promise<BranchItem> => {
  const icons = await generateIcons(branch, worktree);
  const hasChanges = worktree ? await getChangedFilesCount(worktree.path) > 0 : false;
  const label = formatBranchLabel(branch, icons);

  return {
    ...branch,
    icons,
    worktreeStatus: worktree?.isAccessible === false ? 'inaccessible' : worktree ? 'active' : undefined,
    hasChanges,
    label,
    value: branch.name
  };
};
```

## データフロー

```
Git/Worktree API
      ↓
BranchInfo[], WorktreeInfo[]
      ↓
[変換処理]
      ↓
BranchItem[], Statistics
      ↓
[Reactコンポーネント]
      ↓
Layout計算
      ↓
画面レンダリング
```

## パフォーマンス考慮事項

### メモ化

```typescript
// 高コストな計算結果をキャッシュ
const items = useMemo(() => {
  return branches.map(branch => toBranchItem(branch, worktreeMap.get(branch.name)));
}, [branches, worktreeMap]);
```

### 最適化戦略

1. **BranchItem生成**: `useMemo`でキャッシュ
2. **Statistics計算**: P3実装時は`useState`+`setInterval`でバックグラウンド更新
3. **Layout再計算**: `resize`イベント時のみ（過剰な再計算を防ぐ）

## 次のステップ

1. ✅ データモデル定義完了
2. ⏭️ quickstart.md作成
3. ⏭️ 型定義の実装（`src/ui/types.ts`拡張）
4. ⏭️ 変換関数の実装（TDDで）

---

**作成日**: 2025-01-25
**承認**: Phase 1完了後に実装開始
