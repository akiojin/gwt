# Architecture Documentation

`@akiojin/gwt`のアーキテクチャ設計ドキュメント。

## Overview

Claude WorktreeはGit worktreeを活用した対話型ブランチ管理CLIツールです。Claude CodeまたはCodex CLIと統合し、効率的な開発ワークフローを実現します。

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────┐
│                      CLI Interface                      │
│                     (src/index.ts)                      │
└──────────────────┬──────────────────────────────────────┘
                   │
        ┌──────────┴──────────┬──────────────┬────────────┐
        │                     │              │            │
┌───────▼────────┐  ┌────────▼────────┐  ┌─▼──────┐  ┌──▼───────┐
│  Git Module    │  │ Worktree Module │  │ GitHub │  │  AI Tool │
│  (src/git.ts)  │  │(src/worktree.ts)│  │ Module │  │Integration│
└───────┬────────┘  └────────┬────────┘  └─┬──────┘  └──┬───────┘
        │                    │              │            │
┌───────▼────────┐  ┌────────▼────────┐  ┌─▼──────┐  ┌──▼───────┐
│ Branch Mgmt    │  │  WT Creation    │  │PR Mgmt │  │ Claude   │
│ Version Mgmt   │  │  WT Removal     │  │PR List │  │  Codex   │
│ Change Mgmt    │  │  WT List        │  └────────┘  └──────────┘
└────────────────┘  └─────────────────┘
        │                    │
        └──────────┬─────────┘
                   │
        ┌──────────▼──────────┐
        │  Session Management │
        │ (src/config/index.ts)│
        └─────────────────────┘
                   │
        ┌──────────▼──────────┐
        │    UI Components    │
        │    (src/ui/)        │
        │  - Table Display    │
        │  - Formatting       │
        └─────────────────────┘
```

## Module Responsibilities

### 1. CLI Interface (`src/index.ts`)

**責務:**

- コマンドライン引数のパース
- ユーザー入力の受付（inquirer）
- 各モジュールのオーケストレーション
- エラーハンドリングと終了処理

**主要な関数:**

- `main()`: エントリーポイント
- `handleSelection()`: ユーザー選択のルーティング
- `handleBranchSelection()`: ブランチ選択フロー
- `handleCreateNewBranch()`: 新規ブランチ作成フロー
- `handleManageWorktrees()`: ワークツリー管理フロー
- `handleCleanupMergedPRs()`: PRクリーンアップフロー

**依存関係:**

- `git.ts` - Git操作
- `worktree.ts` - ワークツリー管理
- `github.ts` - GitHub統合
- `claude.ts` / `codex.ts` - AIツール統合
- `config/index.ts` - セッション管理

### 2. Git Module (`src/git.ts`)

**責務:**

- Gitコマンドの実行と抽象化
- ブランチ管理（作成、削除、一覧取得）
- バージョン管理（package.jsonの読み書き）
- 変更管理（コミット、stash、破棄）

**主要な関数:**

- `getAllBranches()`: 全ブランチ取得
- `createBranch()`: ブランチ作成
- `deleteBranch()`: ブランチ削除
- `getCurrentVersion()`: 現在のバージョン取得
- `calculateNewVersion()`: 新バージョン計算
- `hasUncommittedChanges()`: 未コミット変更チェック
- `commitChanges()`: 変更をコミット

**技術スタック:**

- `execa` - Gitコマンド実行
- `node:fs` - ファイルシステム操作

### 3. Worktree Module (`src/worktree.ts`)

**責務:**

- Gitワークツリーの作成と削除
- ワークツリー一覧の取得
- ワークツリーパスの生成

**主要な関数:**

- `worktreeExists()`: ワークツリー存在チェック
- `createWorktree()`: ワークツリー作成
- `removeWorktree()`: ワークツリー削除
- `listAdditionalWorktrees()`: ワークツリー一覧
- `generateWorktreePath()`: パス生成

**技術スタック:**

- `execa` - Git worktreeコマンド実行
- `node:path` - パス操作

### 4. GitHub Module (`src/github.ts`)

**責務:**

- GitHub CLIとの統合
- マージ済みPRの取得
- PR情報の解析

**主要な関数:**

- `getMergedPullRequests()`: マージ済みPR取得
- `isGitHubCLIAvailable()`: GitHub CLI可用性チェック
- `checkGitHubAuth()`: 認証状態チェック

**技術スタック:**

- `execa` - `gh`コマンド実行

### 5. AI Tool Integration

#### Claude Code (`src/claude.ts`)

**責務:**

- Claude Codeの起動と管理
- 実行モードの設定

**主要な関数:**

- `launchClaudeCode()`: Claude Code起動
- `isClaudeCodeAvailable()`: 可用性チェック

#### Codex CLI (`src/codex.ts`)

**責務:**

- Codex CLIの起動と管理
- 実行モードの設定

**主要な関数:**

- `launchCodexCLI()`: Codex CLI起動
- `isCodexAvailable()`: 可用性チェック

**技術スタック:**

- `execa` - AIツールプロセス起動
- `node:child_process` - プロセス管理

### 6. Session Management (`src/config/index.ts`)

**責務:**

- セッション情報の永続化
- セッション履歴の管理
- 継続/再開機能のサポート

**主要な関数:**

- `saveSession()`: セッション保存
- `loadSession()`: 最新セッション読み込み
- `getAllSessions()`: 全セッション取得

**データ保存:**

- `~/.config/gwt/sessions.json`

**技術スタック:**

- `node:fs` - ファイル操作
- `node:os` - ホームディレクトリ取得

### 7. UI Components (`src/ui/`)

**責務:**

- ブランチテーブルの生成
- フォーマット済み出力
- ユーザーインタラクション

**主要な関数:**

- `createBranchTable()`: ブランチ選択テーブル生成
- `formatDisplay()`: 出力フォーマット

**技術スタック:**

- `inquirer` - 対話型プロンプト
- `chalk` - カラー出力

## Data Flow

### 1. Branch Selection Flow

```
User Input
    ↓
CLI (index.ts)
    ↓
git.getAllBranches() → Branch List
    ↓
worktree.listAdditionalWorktrees() → Existing Worktrees
    ↓
ui.createBranchTable() → Formatted Table
    ↓
inquirer.select() → User Selection
    ↓
worktree.createWorktree() → New Worktree
    ↓
AI Tool Launch (claude.ts / codex.ts)
    ↓
config.saveSession() → Session Persistence
```

### 2. Branch Creation Flow

```
User Input (Branch Type, Name, Base)
    ↓
CLI (handleCreateNewBranch)
    ↓
git.branchExists() → Check Existence
    ↓
git.createBranch() → Create Branch
    ↓
worktree.generateWorktreePath() → Path Generation
    ↓
worktree.createWorktree() → Worktree Creation
    ↓
(For Release) git.calculateNewVersion() → Version Bump
    ↓
(For Release) git.executeNpmVersionInWorktree() → Update package.json
    ↓
AI Tool Launch
    ↓
Session Save
```

### 3. PR Cleanup Flow

```
User Input
    ↓
CLI (handleCleanupMergedPRs)
    ↓
github.checkGitHubAuth() → Auth Check
    ↓
github.getMergedPullRequests() → PR List
    ↓
worktree.listAdditionalWorktrees() → Worktree List
    ↓
Match PRs with Worktrees → Cleanup Targets
    ↓
inquirer.checkbox() → User Selection
    ↓
git.hasUncommittedChanges() → Change Check
    ↓
(If changes) Commit / Stash / Discard
    ↓
worktree.removeWorktree() → Remove Worktree
    ↓
git.deleteBranch() → Delete Branch
```

## Error Handling Strategy

### Error Types

1. **GitError**: Git操作の失敗
   - ブランチ作成失敗
   - ブランチ削除失敗
   - コミット失敗

2. **WorktreeError**: ワークツリー操作の失敗
   - ワークツリー作成失敗
   - ワークツリー削除失敗

3. **GitHub CLI Error**: GitHub統合の失敗
   - 認証エラー
   - PR取得失敗

4. **AI Tool Error**: AIツール起動失敗
   - ツールが見つからない
   - プロセス起動失敗

### Error Recovery

- **自動リトライ**: なし（明示的な再実行が必要）
- **ロールバック**: なし（手動クリーンアップ）
- **エラーメッセージ**: 詳細なエラー情報をユーザーに提示

### Exit Handlers

`src/utils.ts`の`setupExitHandlers()`により以下を処理:

- SIGINT (Ctrl+C)
- SIGTERM
- Uncaught exceptions
- Unhandled rejections

## Configuration

### Session Storage

**場所:** `~/.config/gwt/sessions.json`

**形式:**

```json
{
  "sessions": [
    {
      "worktreePath": "/path/to/worktree",
      "branchName": "feature/test",
      "timestamp": 1234567890,
      "aiTool": "claude-code"
    }
  ]
}
```

## Testing Strategy

### Unit Tests (`tests/unit/`)

- 各モジュールの関数を独立してテスト
- モックを使用してGitコマンドをシミュレーション
- カバレッジ目標: 80%以上

### Integration Tests (`tests/integration/`)

- 複数モジュール間の連携をテスト
- Git操作とワークツリー操作の統合
- セッション管理の統合

### E2E Tests (`tests/e2e/`)

- 完全なユーザーワークフローをテスト
- ブランチ選択→ワークツリー作成→AIツール起動
- エラーリカバリーシナリオ

**Testing Framework:** Vitest

**Mock Strategy:**

- `execa`: Gitコマンドのモック
- `node:fs`: ファイルシステムのモック
- `inquirer`: ユーザー入力のモック

## Performance Considerations

### Bottlenecks

1. **Git Operations**: 特にリモートブランチ取得
2. **GitHub API Calls**: PR一覧取得
3. **File I/O**: セッション保存/読み込み

### Optimization

- **並列実行**: 独立したGitコマンドは並列実行可能
- **キャッシング**: ブランチ一覧のキャッシュ（現状未実装）
- **Lazy Loading**: 必要な時のみデータ取得

## Security Considerations

### Credentials

- GitHub認証は`gh`コマンドに委譲
- セッション情報に機密データは保存しない

### Command Injection Prevention

- `execa`を使用してコマンド実行を安全化
- ユーザー入力はサニタイズ

### File System Safety

- ワークツリーパスは`.git/worktree/`配下に制限
- パストラバーサル対策を実施

## Deployment

### Distribution

- npm registry経由で配布
- `bunx @akiojin/gwt`で実行可能

### Requirements

- Bun 1.0.0+
- Git 2.25+
- GitHub CLI (PRクリーンアップ機能使用時)
- Claude Code または Codex CLI

## Future Enhancements

### Planned Features

1. **ブランチ一覧のキャッシング**
2. **カスタムワークツリーパステンプレート**
3. **複数リポジトリ対応**
4. **プラグインシステム**
5. **WebUI (オプション)**

### Technical Debt

1. `src/index.ts`のリファクタリング（1000行超）
2. サービス層の導入
3. リポジトリ層の導入

## Maintenance

### Code Quality Tools

- **ESLint**: コード品質チェック
- **Prettier**: フォーマット
- **commitlint**: コミットメッセージ検証
- **markdownlint**: ドキュメント品質

### CI/CD

- **GitHub Actions**: テスト自動化
- **Codecov**: カバレッジレポート
- **Semantic Release**: 自動リリース

## References

- [Git Worktree Documentation](https://git-scm.com/docs/git-worktree)
- [GitHub CLI Documentation](https://cli.github.com/)
- [Claude Code Documentation](https://docs.claude.com/en/docs/claude-code)
