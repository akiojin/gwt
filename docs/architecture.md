# Architecture Documentation

`gwt`（Rust版）のアーキテクチャ設計ドキュメント。

## Overview

gwtはGit worktreeを活用した対話型ブランチ管理ツールです。Rustで単一バイナリ化され、CLI TUIとWeb UIの両方を提供します。Coding Agent（Claude Code/Codex CLI/Gemini CLI）との統合により、ブランチ単位の開発フローを高速化します。

## Architecture Diagram

```
┌──────────────────────────────┐
│         gwt-cli (TUI)         │
│   CLI/TUI/Flow Orchestration  │
└──────────────┬───────────────┘
               │ uses
┌──────────────▼───────────────┐
│           gwt-core            │
│ Git/Worktree/Config/Logging   │
│ Agent/Session/Error/Lock      │
└──────────────┬───────────────┘
               │
┌──────────────▼───────────────┐
│           gwt-web             │
│ Axum REST/WebSocket/PTY       │
└──────────────┬───────────────┘
               │ serves
┌──────────────▼───────────────┐
│        gwt-frontend           │
│        Leptos CSR UI          │
└──────────────────────────────┘
```

## Module Responsibilities

### 1. gwt-cli（`crates/gwt-cli`）

**責務:**

- コマンドライン引数のパースと起動モード制御
- RatatuiベースのTUI描画と入力処理
- gwt-coreを利用したGit/Worktree操作のオーケストレーション
- エラー表示とユーザー操作フローの管理

**主な画面:**

- ブランチ一覧
- Worktree作成ウィザード
- セッション/ログ/設定画面

### 2. gwt-core（`crates/gwt-core`）

**責務:**

- Git操作（gix + 外部gitフォールバック）
- Worktree管理（作成・削除・修復・ロック）
- 設定/セッション管理（TOML + 自動移行）
- ログ管理（JSON Lines）
- Coding Agent起動とセッション履歴
- エラーコードとメッセージ定義

**永続データ:**

- `~/.gwt/.gwt.toml`
- `~/.gwt/sessions/`
- `~/.gwt/profiles.yaml`
- `~/.gwt/logs/`

### 3. gwt-web（`crates/gwt-web`）

**責務:**

- AxumベースのREST API
- WebSocket/PTYによる端末ストリーム
- WASM/静的アセットの配信

**提供API:**

- Worktree/Branch一覧・作成・削除
- 設定取得・更新
- セッション履歴取得

### 4. gwt-frontend（`crates/gwt-frontend`）

**責務:**

- Leptos CSRによるWeb UI
- API呼び出しと画面状態の同期
- xterm.jsによる端末表示

**主な画面:**

- Worktree一覧
- Branch一覧
- Terminal
- Settings
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
Coding Agent Launch (claude.ts / codex.ts)
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
Coding Agent Launch
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

4. **Coding Agent Error**: Coding Agent 起動失敗
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
- ブランチ選択→ワークツリー作成→Coding Agent 起動
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
- `bunx -p @akiojin/gwt gwt`で実行可能

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
- **Release Please**: 自動リリース

## References

- [Git Worktree Documentation](https://git-scm.com/docs/git-worktree)
- [GitHub CLI Documentation](https://cli.github.com/)
- [Claude Code Documentation](https://docs.claude.com/en/docs/claude-code)
