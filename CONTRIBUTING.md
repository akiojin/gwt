# Contributing to gwt

`@akiojin/gwt`へのコントリビューションをご検討いただきありがとうございます！

## Development Setup

### Prerequisites

- Bun 1.3.1+（推奨: 最新版）
- Node.js 18+（任意、Node製開発ツール利用時）
- Git 2.25+
- GitHub CLI (オプション、PRクリーンアップ機能のテストに必要)

### Installation

1. リポジトリをフォーク

2. クローン

```bash
git clone https://github.com/YOUR_USERNAME/gwt.git
cd gwt
```

1. 依存関係をインストール

```bash
bun install
```

1. ビルド

```bash
bun run build
```

1. ローカルで実行

```bash
bunx .
```

## Project Structure

```
gwt/
├── src/                  # ソースコード
│   ├── index.ts          # メインエントリーポイント
│   ├── git.ts            # Git操作
│   ├── worktree.ts       # ワークツリー管理
│   ├── github.ts         # GitHub統合
│   ├── claude.ts         # Claude Code統合
│   ├── codex.ts          # Codex CLI統合
│   ├── config/           # 設定・セッション管理
│   └── ui/               # UI components
├── tests/                # テストコード
│   ├── unit/             # ユニットテスト
│   ├── integration/      # 統合テスト
│   ├── e2e/              # E2Eテスト
│   ├── fixtures/         # テストフィクスチャ
│   └── helpers/          # テストヘルパー
├── docs/                 # ドキュメント
└── specs/                # 仕様書
```

## Development Workflow

### 1. ブランチを作成

> **エージェント向け注意**: この手順は人間の開発者が手動で実行することを前提としています。エージェントは、ユーザーから明確で具体的な指示がない限り、ブランチの作成や削除を実行してはいけません。

```bash
git checkout -b feature/your-feature-name
```

ブランチ命名規則:

- `feature/` - 新機能
- `fix/` - バグ修正
- `docs/` - ドキュメント更新
- `refactor/` - リファクタリング
- `test/` - テスト追加

### 2. 開発

コード品質ツール:

- ESLint: `bun run lint`
- Prettier: `bun run format`
- TypeScript: `bun run type-check`

### 3. テスト

```bash
# 全テスト実行
bun test

# ウォッチモード
bun test:watch

# カバレッジレポート
bun test:coverage

# 特定のテストファイルのみ
bun test tests/unit/git.test.ts
```

テストカバレッジ目標: 80%以上

### 4. コミット

このプロジェクトは[Conventional Commits](https://www.conventionalcommits.org/)を使用します。

コミットメッセージ形式:

```
<type>(<scope>): <subject>

<body>

<footer>
```

**Types:**

- `feat`: 新機能
- `fix`: バグ修正
- `docs`: ドキュメント更新
- `test`: テスト追加・修正
- `refactor`: リファクタリング
- `chore`: ビルド・ツール更新

**例:**

```bash
git commit -m "feat(worktree): add support for custom worktree paths"
git commit -m "fix(git): handle branch names with special characters"
git commit -m "docs: update API documentation"
```

commitlintが自動的にメッセージを検証します。

### 5. プッシュしてPRを作成

```bash
git push origin feature/your-feature-name
```

GitHubでPull Requestを作成してください。

## Pull Request Guidelines

### PR Checklist

- [ ] テストを追加/更新した
- [ ] 全てのテストがパスする (`bun test`)
- [ ] ドキュメントを更新した（必要な場合）
- [ ] コミットメッセージが規約に従っている
- [ ] コードがlintエラーを含まない
- [ ] カバレッジが80%以上

### PR Description Template

```markdown
## 概要

<!-- 変更の概要を記述 -->

## 変更内容

## <!-- 主な変更点をリスト化 -->

-

## 関連Issue

<!-- Closes #123 -->

## テスト

<!-- テストの内容や手順 -->

## スクリーンショット（該当する場合）

<!-- 実行例や出力例 -->
```

## Coding Standards

### TypeScript

- **Strict Mode**: 有効
- **Type Safety**: `any`の使用は最小限に
- **Naming Conventions**:
  - `camelCase` for variables and functions
  - `PascalCase` for classes and interfaces
  - `UPPER_CASE` for constants

### Code Style

- **Indentation**: 2 spaces
- **Line Length**: 100文字以内（推奨）
- **Semicolons**: 必須
- **Quotes**: シングルクォート

### Error Handling

- カスタムエラークラスを使用 (`GitError`, `WorktreeError`)
- エラーメッセージは明確で有用に
- 適切なエラーコンテキストを提供

```typescript
throw new GitError(`Failed to create branch ${branchName}`, originalError);
```

### Documentation

- 公開関数にはJSDocコメントを追加
- 複雑なロジックにはインラインコメント
- READMEとAPIドキュメントを最新に保つ

```typescript
/**
 * Create a new Git branch
 * @param branchName - Name of the branch to create
 * @param baseBranch - Base branch (default: 'main')
 * @throws {GitError} If branch creation fails
 */
export async function createBranch(
  branchName: string,
  baseBranch = "main",
): Promise<void> {
  // implementation
}
```

## Testing Guidelines

### Test Structure

```typescript
describe("Module/Feature", () => {
  beforeEach(() => {
    // Setup
  });

  afterEach(() => {
    // Cleanup
  });

  describe("Function Name", () => {
    it("should do something specific", () => {
      // Arrange
      // Act
      // Assert
    });
  });
});
```

### Mock Strategy

- `execa`モックを使用してGitコマンドをシミュレーション
- `node:fs`モックでファイルシステムを隔離
- テストフィクスチャを活用

### Test Coverage

- 新機能には必ずテストを追加
- バグ修正には再現テストを追加
- エッジケースもカバー

## Issue Reporting

### Bug Report Template

```markdown
## 環境

- OS:
- Node.js バージョン（利用している場合）:
- Bun バージョン:
- Claude Worktree バージョン:

## 再現手順

1.
2.
3.

## 期待される動作

## 実際の動作

## エラーメッセージ/ログ
```

### Feature Request Template

```markdown
## 機能の概要

## ユースケース

## 提案する実装方法（あれば）

## 代替案（あれば）
```

## Communication

### Channels

- **GitHub Issues**: バグ報告・機能リクエスト
- **GitHub Discussions**: 質問・アイデア議論
- **Pull Requests**: コードレビュー

### Language

- Issue・PR: 日本語・英語どちらでも可
- コード・コメント: 英語推奨
- ドキュメント: 日本語

## Release Process

リリースは自動化されています:

1. PRがmainブランチにマージ
2. GitHub Actionsがテスト実行
3. Semantic Releaseがバージョン決定
4. npmに自動公開
5. CHANGELOG.md自動更新

## Questions?

分からないことがあれば、遠慮なくIssueやDiscussionで質問してください！

## License

コントリビューションはプロジェクトと同じライセンス（MIT）の下で提供されます。

---

再度、コントリビューションありがとうございます！ 🎉
