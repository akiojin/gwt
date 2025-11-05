# データモデル: Claude Code / Codex CLI 対応の対話型Gitワークツリーマネージャー

**仕様ID**: `SPEC-473b3d47` | **日付**: 2025-10-24
**関連ドキュメント**: [spec.md](./spec.md) | [plan.md](./plan.md)

## 概要

このドキュメントは、`@akiojin/claude-worktree`で使用されるデータモデルとエンティティを定義します。すべてのエンティティはTypeScript型定義として実装され、`src/ui/types.ts`および各モジュールで定義されています。

## エンティティ定義

### 1. BranchInfo

**目的**: Gitブランチの情報を表現

**定義箇所**: `src/ui/types.ts`

```typescript
interface BranchInfo {
  name: string;                    // ブランチ名（例: main, feature/new-feature, origin/develop）
  type: 'local' | 'remote';        // ローカルまたはリモートブランチ
  branchType: 'feature' | 'hotfix' | 'release' | 'other';  // ブランチタイプ
  isCurrent: boolean;              // 現在のブランチか（ローカルのみ）
}
```

**フィールド説明**:

| フィールド | 型 | 必須 | 説明 | 検証ルール |
|-----------|------|------|------|------------|
| `name` | string | ✓ | ブランチ名 | 空文字列不可、Git命名規則に準拠 |
| `type` | 'local' \| 'remote' | ✓ | ブランチタイプ | 'local'または'remote'のみ |
| `branchType` | enum | ✓ | Git Flowのブランチ分類 | feature/hotfix/release/otherのいずれか |
| `isCurrent` | boolean | ✓ | 現在のブランチか | ローカルブランチのみtrue可能 |

**ブランチタイプの決定ロジック**:

```typescript
function getBranchType(branchName: string): BranchInfo['branchType'] {
  if (branchName.startsWith('feature/')) return 'feature';
  if (branchName.startsWith('hotfix/')) return 'hotfix';
  if (branchName.startsWith('release/')) return 'release';
  return 'other';
}
```

**使用例**:
- ブランチ一覧表示
- ブランチ選択メニュー
- ワークツリー作成のソースブランチ

---

### 2. WorktreeInfo

**目的**: Gitワークツリーのメタデータを表現

**定義箇所**: `src/worktree.ts`

```typescript
interface WorktreeInfo {
  branch: string;                  // 関連ブランチ名
  path: string;                    // ワークツリーの絶対パス
  isAccessible?: boolean;          // アクセス可能性（オプション、デフォルトtrue）
}
```

**フィールド説明**:

| フィールド | 型 | 必須 | 説明 | 検証ルール |
|-----------|------|------|------|------------|
| `branch` | string | ✓ | 関連ブランチ名 | 空文字列不可、既存ブランチと一致 |
| `path` | string | ✓ | ワークツリーの絶対パス | 絶対パスである必要がある |
| `isAccessible` | boolean | - | ファイルシステムからアクセス可能か | true（デフォルト）/false/undefined |

**アクセス可能性の判定**:
- `isAccessible === false`: 別の環境で作成されたワークツリー（パスが存在しない）
- `isAccessible === undefined` または `true`: 通常のアクセス可能なワークツリー

**使用例**:
- ワークツリー一覧表示
- ワークツリー管理メニュー
- PRクリーンアップ対象の検出

---

### 3. WorktreeConfig

**目的**: 新規ワークツリー作成の設定を表現

**定義箇所**: `src/ui/types.ts`

```typescript
interface WorktreeConfig {
  branchName: string;              // 作成するブランチ名
  worktreePath: string;            // ワークツリーを作成するパス
  repoRoot: string;                // リポジトリのルートパス
  isNewBranch: boolean;            // 新規ブランチを作成するか
  baseBranch: string;              // ベースブランチ名
}
```

**フィールド説明**:

| フィールド | 型 | 必須 | 説明 | 検証ルール |
|-----------|------|------|------|------------|
| `branchName` | string | ✓ | 作成するブランチ名 | `isNewBranch`がtrueの場合、存在しない名前 |
| `worktreePath` | string | ✓ | ワークツリーパス | 絶対パス、存在しないディレクトリ |
| `repoRoot` | string | ✓ | リポジトリルート | 有効なGitリポジトリパス |
| `isNewBranch` | boolean | ✓ | 新規ブランチ作成フラグ | - |
| `baseBranch` | string | ✓ | ベースブランチ | 存在するブランチ名 |

**検証ロジック**:
- `isNewBranch === true` → `branchName`は既存ブランチと重複不可
- `isNewBranch === false` → `branchName`は既存ブランチと一致必須
- `baseBranch`は常に存在するブランチ名

**使用例**:
- ブランチ作成フロー
- リモートブランチからのローカルブランチ作成
- ワークツリー自動作成

---

### 4. SessionData

**目的**: ユーザーセッション情報の永続化

**定義箇所**: `src/config/index.ts`

```typescript
interface SessionData {
  lastWorktreePath: string | null;  // 最後に使用したワークツリーパス
  lastBranch: string | null;        // 最後に使用したブランチ名
  timestamp: number;                // セッションのタイムスタンプ（Unix time）
  repositoryRoot: string;           // リポジトリのルートパス
}
```

**フィールド説明**:

| フィールド | 型 | 必須 | 説明 | 検証ルール |
|-----------|------|------|------|------------|
| `lastWorktreePath` | string \| null | ✓ | 最後のワークツリーパス | nullまたは絶対パス |
| `lastBranch` | string \| null | ✓ | 最後のブランチ名 | nullまたはブランチ名 |
| `timestamp` | number | ✓ | タイムスタンプ | Unix time（ミリ秒） |
| `repositoryRoot` | string | ✓ | リポジトリルート | 絶対パス |

**状態遷移**:

```text
1. 初期状態
   lastWorktreePath: null
   lastBranch: null
   ↓
2. ワークツリー起動時
   lastWorktreePath: "/path/to/worktree"
   lastBranch: "feature/branch-name"
   timestamp: Date.now()
   ↓
3. セッション継続（-c）
   値を読み取り、同じワークツリーを開く
```

**永続化**:
- ファイルパス: `{repoRoot}/.config/claude-worktree-session.json`
- フォーマット: JSON
- 保存タイミング: AIツール起動直前

**使用例**:
- `-c`オプションによるセッション継続
- `-r`オプションによるセッション選択
- 最近使用したワークツリーの追跡

---

### 5. CleanupTarget

**目的**: マージ済みPRのクリーンアップ対象を表現

**定義箇所**: `src/ui/types.ts`

```typescript
interface CleanupTarget {
  worktreePath: string | null;  // ワークツリーパス（nullの場合はローカルブランチのみ）
  branch: string;               // ブランチ名
  pullRequest: MergedPullRequest | null; // 紐付くPR情報（存在しない場合はnull）
  hasUncommittedChanges: boolean; // 未コミット変更の有無
  hasUnpushedCommits: boolean;    // 未プッシュコミットの有無
  cleanupType: 'worktree-and-branch' | 'branch-only'; // クリーンアップタイプ
  hasRemoteBranch?: boolean;     // リモートブランチの有無
  isAccessible?: boolean;        // ワークツリーパス参照可否（falseの場合は後続処理でスキップ）
  invalidReason?: string;        // 無効理由（ログ用）
  reasons?: ('merged-pr' | 'no-diff-with-base')[]; // 検出理由
}
```

**フィールド説明**:

| フィールド | 型 | 必須 | 説明 | 検証ルール |
|-----------|------|------|------|------------|
| `worktreePath` | string / null | ✓ | ワークツリーパス（ブランチのみの場合はnull） | - |
| `branch` | string | ✓ | ブランチ名 | 空文字列不可 |
| `pullRequest` | object / null | ✓ | 紐付くマージ済みPR情報 | PRがない場合はnull |
| `hasUncommittedChanges` | boolean | ✓ | 未コミット変更の有無 | - |
| `hasUnpushedCommits` | boolean | ✓ | 未プッシュコミットの有無 | - |
| `cleanupType` | enum | ✓ | クリーンアップタイプ | 'worktree-and-branch'または'branch-only' |
| `hasRemoteBranch` | boolean | - | リモートブランチの存在可否 | - |
| `isAccessible` | boolean | - | ワークツリーのパス解決可否 | falseの場合は削除処理をスキップ |
| `invalidReason` | string | - | 無効理由の説明 | - |
| `reasons` | string[] | - | 検出理由（'merged-pr', 'no-diff-with-base'） | - |

**cleanupTypeの決定ロジック**:

```typescript
if (worktreePath) {
  cleanupType = 'worktree-and-branch';
} else {
  cleanupType = 'branch-only';
}
```

**状態遷移**:

```text
1. PR検出
   GitHub APIからマージ済みPRを取得
   ↓
2. クリーンアップ対象生成
   ローカルブランチとワークツリーを照合
   ↓
3. ユーザー選択
   削除対象を複数選択
   ↓
4. クリーンアップ実行
   ワークツリー削除 → ローカルブランチ削除 → リモートブランチ削除（オプション）
```

**使用例**:
- マージ済みPRの自動検出
- クリーンアップ対象の一覧表示
- 一括削除処理

---

### 6. MergedPullRequest

**目的**: GitHub APIから取得したマージ済みPR情報

**定義箇所**: `src/github.ts`

```typescript
interface MergedPullRequest {
  number: number;           // PR番号
  title: string;            // PRタイトル
  headRefName: string;      // ヘッドブランチ名
  url: string;              // PR URL
  state: string;            // 状態（"MERGED"）
  mergedAt: string;         // マージ日時（ISO 8601）
}
```

**フィールド説明**:

| フィールド | 型 | 必須 | 説明 |
|-----------|------|------|------|
| `number` | number | ✓ | PR番号 |
| `title` | string | ✓ | PRタイトル |
| `headRefName` | string | ✓ | ヘッドブランチ名（例: feature/new-feature） |
| `url` | string | ✓ | PR URL |
| `state` | string | ✓ | PR状態（マージ済みは"MERGED"） |
| `mergedAt` | string | ✓ | マージ日時 |

**取得元**: GitHub CLI (`gh pr list --state merged --json ...`)

**使用例**:
- CleanupTargetへの変換
- マージ済みPR一覧の表示

---

## エンティティ間の関係

### 関係図

```text
BranchInfo
    ↓ (1対0..1)
WorktreeInfo
    ↓ (1対0..1)
SessionData

BranchInfo
    ↓ (1対0..1)
MergedPullRequest
    ↓ (変換)
CleanupTarget
```

### 詳細な関係

1. **BranchInfo → WorktreeInfo**
   - 1つのブランチは0個または1個のワークツリーを持つ
   - 逆に、1つのワークツリーは必ず1つのブランチと紐づく

2. **WorktreeInfo → SessionData**
   - 最後に使用したワークツリーがSessionDataに記録される
   - 1つのリポジトリにつき1つのSessionDataが存在

3. **BranchInfo → MergedPullRequest → CleanupTarget**
   - ブランチがマージ済みPRに関連付けられる場合、CleanupTargetが生成される
   - CleanupTargetはブランチとワークツリーの両方を含む場合と、ブランチのみの場合がある

## データフロー

### 1. ブランチ選択 → ワークツリー作成

```text
User selects BranchInfo
    ↓
Check if WorktreeInfo exists
    ↓ (No)
Generate WorktreeConfig
    ↓
Create worktree with git command
    ↓
Save SessionData
    ↓
Launch AI Tool
```

### 2. セッション継続

```text
User runs with -c option
    ↓
Load SessionData
    ↓
Validate WorktreeInfo still exists
    ↓ (Yes)
Launch AI Tool in lastWorktreePath
```

### 3. PRクリーンアップ

```text
Fetch MergedPullRequest[] from GitHub
    ↓
For each MergedPullRequest:
    Find matching BranchInfo (by headRefName)
    ↓
    Find matching WorktreeInfo (by branch)
    ↓
    Create CleanupTarget (auto-cleanup candidate)
    ↓
Mark targets as pending (⏳) in UI state
    ↓
Execute cleanup sequentially (no manual selection)
    ↓
Update UI state per target (⠋→✅/⏭️/❌)
    ↓
Skip targets with pending changes or inaccessible paths
    ↓
Hold final result icons for 3 seconds, then restore cursor and prune removed branches
```

## 実装ファイルマッピング

| エンティティ | 定義ファイル | 使用箇所 |
|-------------|-------------|---------|
| `BranchInfo` | `src/ui/types.ts` | `src/git.ts`, `src/index.ts`, `src/ui/table.ts` |
| `WorktreeInfo` | `src/worktree.ts` | `src/worktree.ts`, `src/index.ts` |
| `WorktreeConfig` | `src/ui/types.ts` | `src/worktree.ts`, `src/index.ts` |
| `SessionData` | `src/config/index.ts` | `src/config/index.ts`, `src/index.ts` |
| `CleanupTarget` | `src/ui/types.ts` | `src/worktree.ts`, `src/index.ts`, `src/ui/display.ts` |
| `MergedPullRequest` | `src/github.ts` | `src/github.ts`, `src/worktree.ts` |

## 検証ルール詳細

### ブランチ名の検証

```typescript
// Git命名規則に準拠
function isValidBranchName(name: string): boolean {
  return name.length > 0 &&
         !name.includes('..') &&
         !name.startsWith('/') &&
         !name.endsWith('/');
}
```

### パスの検証

```typescript
// 絶対パスの検証
function isAbsolutePath(path: string): boolean {
  return path.startsWith('/') || // Unix
         /^[a-zA-Z]:\\/.test(path); // Windows
}
```

### ワークツリー存在確認

```typescript
async function worktreeExists(branchName: string): Promise<string | null> {
  const worktrees = await listWorktrees();
  const found = worktrees.find(w => w.branch === branchName);
  return found ? found.path : null;
}
```

## まとめ

このデータモデルは、以下の主要な責務を果たします：

1. **Gitブランチとワークツリーの管理**: BranchInfo, WorktreeInfo, WorktreeConfig
2. **セッションの永続化**: SessionData
3. **PRクリーンアップ**: MergedPullRequest, CleanupTarget

すべてのエンティティはTypeScriptの型システムにより厳密に型付けされ、実行時のバリデーションとコンパイル時の型チェックの両方で保護されています。
