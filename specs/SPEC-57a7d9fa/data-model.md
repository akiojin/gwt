# データモデル: Worktreeディレクトリパス変更

**仕様ID**: `SPEC-57a7d9fa` | **日付**: 2025-10-31

## 概要

本ドキュメントは、Worktreeディレクトリパス変更に関連するデータ構造と関係性を定義します。

## 1. エンティティ定義

### 1.1 Worktreeパス

**概念**: Worktreeディレクトリへのファイルシステムパス

**構造**:
```
<repoRoot>/.worktrees/<sanitizedBranchName>
```

**例**:
- `/path/to/repo/.worktrees/feature-user-auth`
- `C:\projects\myrepo\.worktrees\bugfix-login`

**属性**:

| 属性名 | 型 | 説明 | 制約 |
|--------|------|------|------|
| repoRoot | string | リポジトリのルートディレクトリパス | 絶対パス |
| branchName | string | ブランチ名（元の名前） | Git有効なブランチ名 |
| sanitizedBranchName | string | サニタイズされたブランチ名 | ファイルシステム安全な名前 |
| worktreePath | string | 完全なWorktreeパス | 絶対パス |

**関係性**:
```
repoRoot + ".worktrees" + sanitizedBranchName = worktreePath
```

**検証ルール**:
- `repoRoot`は存在するディレクトリ
- `sanitizedBranchName`は次の文字を含まない: `/\:*?"<>|`
- `worktreePath`はまだ存在しないパス（新規作成の場合）

**状態遷移**:
1. 初期状態: パスが存在しない
2. Worktree作成: `git worktree add`によりディレクトリ作成
3. 最終状態: Worktreeディレクトリが存在し、Gitによって管理される

### 1.2 Gitignoreエントリー

**概念**: `.gitignore`ファイル内の行エントリー

**構造**:
```
.worktrees/
```

**属性**:

| 属性名 | 型 | 説明 | 制約 |
|--------|------|------|------|
| entry | string | Gitignoreパターン文字列 | `.worktrees/` |
| position | number | ファイル内の行位置 | >= 0 |
| exists | boolean | エントリーが既に存在するか | true/false |

**検証ルール**:
- `entry`は`.worktrees/`（末尾スラッシュ必須）
- ファイル内に重複エントリーが存在しない
- エントリーは独立した行として存在する

**状態遷移**:
1. 初期状態: `.gitignore`にエントリーが存在しない
2. 追加処理: `.gitignore`の末尾に`.worktrees/`を追加
3. 最終状態: `.gitignore`に`.worktrees/`エントリーが存在

### 1.3 WorktreeConfig

**概念**: Worktree作成に必要な設定情報

**既存の型定義**（src/ui/types.ts）:
```typescript
export interface WorktreeConfig {
  branchName: string;
  worktreePath: string;
  repoRoot: string;
  isNewBranch: boolean;
  baseBranch: string;
}
```

**変更点**: なし（既存の型定義をそのまま使用）

**使用箇所**:
- `generateWorktreePath`: `worktreePath`を生成
- `createWorktree`: Worktreeを作成

## 2. データフロー

### 2.1 Worktreeパス生成フロー

```
入力: repoRoot, branchName
  ↓
サニタイズ: branchName → sanitizedBranchName
  ↓
パス結合: repoRoot + ".worktrees" + sanitizedBranchName
  ↓
出力: worktreePath
```

**擬似コード**:
```typescript
function generateWorktreePath(repoRoot: string, branchName: string): string {
  const sanitizedBranchName = branchName.replace(/[\/\\:*?"<>|]/g, "-");
  const worktreeDir = path.join(repoRoot, ".worktrees");
  return path.join(worktreeDir, sanitizedBranchName);
}
```

### 2.2 Gitignore更新フロー

```
入力: repoRoot
  ↓
.gitignoreパスを構築
  ↓
.gitignoreの存在確認
  ├─ 存在しない → 新規作成
  └─ 存在する → 読み込み
       ↓
       エントリーの重複チェック
       ├─ 存在する → 何もしない
       └─ 存在しない → エントリーを追加
            ↓
            .gitignoreに書き込み
```

**擬似コード**:
```typescript
async function ensureGitignoreEntry(repoRoot: string): Promise<void> {
  const gitignorePath = path.join(repoRoot, '.gitignore');
  const entry = '.worktrees/';

  try {
    // ファイルの読み込み（存在しない場合は空文字列）
    const content = await fs.readFile(gitignorePath, 'utf-8').catch(() => '');

    // エントリーの重複チェック
    const lines = content.split('\n');
    if (!lines.includes(entry)) {
      // エントリーを追加
      await fs.appendFile(gitignorePath, `\n${entry}\n`);
    }
  } catch (error) {
    throw new WorktreeError('Failed to update .gitignore', error);
  }
}
```

## 3. エラーモデル

### 3.1 WorktreeError

**既存の定義**（src/worktree.ts）:
```typescript
export class WorktreeError extends Error {
  constructor(
    message: string,
    public cause?: unknown,
  ) {
    super(message);
    this.name = "WorktreeError";
  }
}
```

### 3.2 エラーメッセージ定義

| エラーコード | メッセージテンプレート | 原因 | 対処方法 |
|-------------|---------------------|------|---------|
| GITIGNORE_PERMISSION_DENIED | "Failed to update .gitignore: Permission denied" | `.gitignore`が読み取り専用 | ファイルのパーミッションを確認 |
| GITIGNORE_WRITE_FAILED | "Failed to write to .gitignore: {reason}" | ディスク容量不足など | ディスク容量を確認 |
| WORKTREE_PATH_EXISTS | "Worktree path already exists: {path}" | パスが既に存在 | 既存のWorktreeを削除 |
| WORKTREE_DIR_IS_FILE | ".worktrees exists but is a file, not a directory" | `.worktrees`がファイルとして存在 | `.worktrees`ファイルを削除 |

## 4. パフォーマンス特性

### 4.1 時間複雑度

| 操作 | 時間複雑度 | 説明 |
|------|-----------|------|
| generateWorktreePath | O(n) | nはブランチ名の長さ（サニタイズ処理） |
| ensureGitignoreEntry | O(m) | mは`.gitignore`の行数 |

### 4.2 空間複雑度

| 操作 | 空間複雑度 | 説明 |
|------|-----------|------|
| generateWorktreePath | O(n) | nはパス文字列の長さ |
| ensureGitignoreEntry | O(m) | mは`.gitignore`の内容サイズ |

## 5. 制約条件

### 5.1 システム制約

- **ファイルシステム制約**: パス長の最大値（Windows: 260文字、Unix: 4096文字）
- **パーミッション**: `.gitignore`への書き込み権限が必要
- **ディスク容量**: Worktreeディレクトリ作成に必要な容量

### 5.2 ビジネス制約

- **後方互換性**: 既存の`.git/worktree`配下のWorktreeは影響を受けない
- **一貫性**: 同じブランチ名に対して常に同じWorktreeパスを生成

## 6. テストデータ例

### 6.1 正常系

```typescript
// 入力
const repoRoot = "/home/user/projects/myrepo";
const branchName = "feature/user-authentication";

// 期待される出力
const expectedPath = "/home/user/projects/myrepo/.worktrees/feature-user-authentication";

// Gitignore
const expectedEntry = ".worktrees/";
```

### 6.2 エッジケース

```typescript
// 特殊文字を含むブランチ名
const branchName = "feature/user:auth*with?special<chars>";
const expectedSanitized = "feature-user-auth-with-special-chars-";

// Windowsパス
const repoRoot = "C:\\Users\\John\\Projects\\myrepo";
const expectedPath = "C:\\Users\\John\\Projects\\myrepo\\.worktrees\\feature-test";
```

## 7. 移行戦略

### 7.1 段階的移行

1. **Phase 1**: 新規Worktreeのみ`.worktrees`を使用
2. **Phase 2**: 既存Worktreeは手動移行（ドキュメント提供）
3. **Phase 3**: 自動移行ツール（将来の機能として検討）

### 7.2 共存期間

- 旧パス（`.git/worktree`）と新パス（`.worktrees`）が共存可能
- Gitは両方のパスのWorktreeを正しく管理
- ユーザーは必要に応じて移行可能

## 8. 監視とログ

### 8.1 ログレベル

| レベル | イベント | 例 |
|--------|---------|-----|
| INFO | Worktree作成成功 | "Created worktree at .worktrees/feature-test" |
| WARN | Gitignoreエントリー追加失敗（非致命的） | "Failed to update .gitignore, continuing anyway" |
| ERROR | Worktree作成失敗 | "Failed to create worktree: Permission denied" |

### 8.2 メトリクス

- Worktree作成時間（平均、P95、P99）
- Gitignore更新時間
- エラー率

## 9. まとめ

このデータモデルは、Worktreeディレクトリパス変更に必要な全てのエンティティ、関係性、制約条件を定義しています。実装時にはこのモデルに従って、一貫性のある設計を維持してください。
