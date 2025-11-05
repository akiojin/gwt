# API Documentation

このドキュメントは、`@akiojin/claude-worktree`の主要な公開関数のAPIリファレンスです。

## Git Operations (`src/git.ts`)

### Branch Management

#### `getAllBranches(): Promise<BranchInfo[]>`

ローカルおよびリモートの全ブランチを取得します。

**戻り値:** `BranchInfo`オブジェクトの配列

```typescript
const branches = await git.getAllBranches();
// [{name: 'main', type: 'local', branchType: 'main', isCurrent: true}, ...]
```

#### `getLocalBranches(): Promise<BranchInfo[]>`

ローカルブランチのみを取得します。

**戻り値:** ローカルブランチの`BranchInfo`配列

#### `getRemoteBranches(): Promise<BranchInfo[]>`

リモートブランチのみを取得します。

**戻り値:** リモートブランチの`BranchInfo`配列

#### `createBranch(branchName: string, baseBranch?: string): Promise<void>`

新しいブランチを作成します。

**パラメータ:**

- `branchName`: 作成するブランチ名
- `baseBranch`: ベースとなるブランチ（デフォルト: 'main'）

**例外:** `GitError` - ブランチ作成失敗時

#### `branchExists(branchName: string): Promise<boolean>`

ブランチが存在するかチェックします。

**パラメータ:**

- `branchName`: チェックするブランチ名

**戻り値:** ブランチが存在する場合`true`

#### `deleteBranch(branchName: string, force?: boolean): Promise<void>`

ブランチを削除します。

**パラメータ:**

- `branchName`: 削除するブランチ名
- `force`: 強制削除フラグ（デフォルト: false）

**例外:** `GitError` - ブランチ削除失敗時

### Version Management

#### `getCurrentVersion(repoRoot: string): Promise<string>`

`package.json`から現在のバージョンを取得します。

**パラメータ:**

- `repoRoot`: リポジトリのルートパス

**戻り値:** セマンティックバージョン文字列（例: "1.2.3"）

#### `calculateNewVersion(currentVersion: string, versionBump: 'patch' | 'minor' | 'major'): string`

新しいバージョンを計算します。

**パラメータ:**

- `currentVersion`: 現在のバージョン
- `versionBump`: バージョンアップの種類

**戻り値:** 新しいバージョン文字列

**例:**

```typescript
calculateNewVersion("1.2.3", "minor"); // => '1.3.0'
```

#### `executeNpmVersionInWorktree(worktreePath: string, newVersion: string): Promise<void>`

ワークツリー内でバージョンを更新します。

**パラメータ:**

- `worktreePath`: ワークツリーのパス
- `newVersion`: 新しいバージョン

### Change Management

#### `hasUncommittedChanges(worktreePath: string): Promise<boolean>`

未コミット変更があるかチェックします。

**パラメータ:**

- `worktreePath`: チェックするワークツリーのパス

**戻り値:** 未コミット変更がある場合`true`

#### `commitChanges(worktreePath: string, message: string): Promise<void>`

変更をコミットします。

**パラメータ:**

- `worktreePath`: ワークツリーのパス
- `message`: コミットメッセージ

#### `stashChanges(worktreePath: string): Promise<void>`

変更をstashします。

**パラメータ:**

- `worktreePath`: ワークツリーのパス

#### `discardAllChanges(worktreePath: string): Promise<void>`

全ての変更を破棄します。

**パラメータ:**

- `worktreePath`: ワークツリーのパス

**警告:** この操作は元に戻せません

## Worktree Operations (`src/worktree.ts`)

### Worktree Management

#### `worktreeExists(branchName: string): Promise<string | null>`

指定ブランチのワークツリーが存在するかチェックします。

**パラメータ:**

- `branchName`: チェックするブランチ名

**戻り値:** ワークツリーが存在する場合そのパス、存在しない場合`null`

#### `generateWorktreePath(repoRoot: string, branchName: string): Promise<string>`

ブランチ名からワークツリーパスを生成します。

**パラメータ:**

- `repoRoot`: リポジトリのルートパス
- `branchName`: ブランチ名

**戻り値:** 生成されたワークツリーパス

#### `createWorktree(config: WorktreeConfig): Promise<void>`

新しいワークツリーを作成します。

**パラメータ:**

- `config.branchName`: ブランチ名
- `config.worktreePath`: ワークツリーのパス
- `config.repoRoot`: リポジトリのルートパス
- `config.isNewBranch`: 新規ブランチかどうか
- `config.baseBranch`: ベースブランチ

**例外:** `WorktreeError` - ワークツリー作成失敗時

#### `removeWorktree(worktreePath: string, force?: boolean): Promise<void>`

ワークツリーを削除します。

**パラメータ:**

- `worktreePath`: 削除するワークツリーのパス
- `force`: 強制削除フラグ（デフォルト: false）

**例外:** `WorktreeError` - ワークツリー削除失敗時

#### `listAdditionalWorktrees(): Promise<WorktreeInfo[]>`

追加のワークツリー一覧を取得します（メインリポジトリを除く）。

**戻り値:** `WorktreeInfo`オブジェクトの配列

## AI Tool Integration

### Claude Code (`src/claude.ts`)

#### `launchClaudeCode(worktreePath: string, args?: string[]): Promise<void>`

Claude Codeを起動します。

**パラメータ:**

- `worktreePath`: 作業ディレクトリ
- `args`: 追加引数（オプション）

#### `isClaudeCodeAvailable(): Promise<boolean>`

Claude Codeが利用可能かチェックします。

**戻り値:** 利用可能な場合`true`

### Codex CLI (`src/codex.ts`)

#### `launchCodexCLI(worktreePath: string, args?: string[]): Promise<void>`

Codex CLIを起動します。

**パラメータ:**

- `worktreePath`: 作業ディレクトリ
- `args`: 追加引数（オプション）

#### `isCodexAvailable(): Promise<boolean>`

Codex CLIが利用可能かチェックします。

**戻り値:** 利用可能な場合`true`

## GitHub Integration (`src/github.ts`)

### Pull Request Management

#### `getMergedPullRequests(): Promise<PullRequestInfo[]>`

マージ済みPRを取得します。

**戻り値:** `PullRequestInfo`オブジェクトの配列

**前提条件:** GitHub CLIが認証済みであること

#### `isGitHubCLIAvailable(): Promise<boolean>`

GitHub CLIが利用可能かチェックします。

**戻り値:** 利用可能な場合`true`

#### `checkGitHubAuth(): Promise<boolean>`

GitHub CLIの認証状態をチェックします。

**戻り値:** 認証済みの場合`true`

## Session Management (`src/config/index.ts`)

### Session Operations

#### `saveSession(session: SessionInfo): Promise<void>`

セッション情報を保存します。

**パラメータ:**

- `session`: 保存するセッション情報

#### `loadSession(): Promise<SessionInfo | null>`

最後のセッションを読み込みます。

**戻り値:** セッション情報、または存在しない場合`null`

#### `getAllSessions(): Promise<SessionInfo[]>`

全セッションを取得します。

**戻り値:** `SessionInfo`オブジェクトの配列

## Types

### BranchInfo

```typescript
interface BranchInfo {
  name: string;
  type: "local" | "remote";
  branchType: "main" | "develop" | "feature" | "hotfix" | "release" | "other";
  isCurrent: boolean;
}
```

### WorktreeConfig

```typescript
interface WorktreeConfig {
  branchName: string;
  worktreePath: string;
  repoRoot: string;
  isNewBranch: boolean;
  baseBranch: string;
}
```

### WorktreeInfo

```typescript
interface WorktreeInfo {
  path: string;
  branch: string;
  commit: string;
}
```

### SessionInfo

```typescript
interface SessionInfo {
  worktreePath: string;
  branchName: string;
  timestamp: number;
  aiTool: "claude-code" | "codex-cli";
}
```

## Error Handling

### GitError

Git操作の失敗時にスローされます。

```typescript
try {
  await git.createBranch("feature/test");
} catch (error) {
  if (error instanceof GitError) {
    console.error("Git operation failed:", error.message);
  }
}
```

### WorktreeError

ワークツリー操作の失敗時にスローされます。

```typescript
try {
  await worktree.createWorktree(config);
} catch (error) {
  if (error instanceof WorktreeError) {
    console.error("Worktree operation failed:", error.message);
  }
}
```

## CLI Usage

現在サポートされているコマンドライン引数:

- `-h, --help`: ヘルプを表示

**例:**

```bash
# 対話型ランチャーを起動
claude-worktree

# ヘルプを表示
claude-worktree --help
```
