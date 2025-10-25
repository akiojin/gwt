# クイックスタートガイド: リリースプロセス

**仕様ID**: `SPEC-23bb2eed` | **日付**: 2025-10-25 | **実装計画**: [plan.md](./plan.md)

## 概要

このガイドは、@akiojin/claude-worktree プロジェクトのリリースプロセスを説明します。semantic-release を使用した自動リリースワークフローにより、開発者は PR をマージするだけで自動的にバージョン管理、CHANGELOG 生成、npm 公開が実行されます。

## 前提条件

### 必須要件

- Git がインストールされている
- GitHub アカウントとリポジトリへのアクセス権限
- Conventional Commits の理解（コミットメッセージの規約）

### GitHub Secrets の設定（管理者のみ）

リリースワークフローには以下の GitHub Secrets が必要です：

| Secret 名 | 説明 | 設定場所 |
|-----------|------|---------|
| `GITHUB_TOKEN` | GitHub API アクセス用（自動提供） | 設定不要 |
| `NPM_TOKEN` | npm registry への公開用 | Settings → Secrets → Actions |

## リリースプロセス

### 標準的なリリースフロー

```text
1. 機能開発 → feature ブランチで作業
2. PR 作成 → main ブランチへの PR を作成
3. レビュー → コードレビューとテスト
4. PR マージ → main ブランチにマージ
5. 自動リリース → semantic-release が自動実行
   - コミット解析
   - バージョン決定
   - CHANGELOG 更新
   - npm 公開
   - GitHub Release 作成
```

### ステップ 1: 機能開発

feature ブランチで開発を行います：

```bash
# feature ブランチを作成
git checkout -b feature/add-new-feature

# 開発とコミット（Conventional Commits 形式）
git add .
git commit -m "feat: 新しい機能を追加"

# リモートにプッシュ
git push origin feature/add-new-feature
```

### ステップ 2: PR 作成とレビュー

1. GitHub で main ブランチへの PR を作成
2. コードレビューを受ける
3. テストが成功することを確認
4. レビュー承認後、PR をマージ

### ステップ 3: 自動リリース

main ブランチへのマージ後、GitHub Actions が自動的に以下を実行します：

```text
1. テスト実行 (bun run test)
2. ビルド (bun run build)
3. semantic-release 実行
   - コミットメッセージを解析
   - リリースタイプを決定（major/minor/patch）
   - バージョン番号を決定
   - CHANGELOG.md を更新
   - package.json を更新
   - Git タグを作成（例: v1.2.3）
   - npm に公開
   - GitHub Release を作成
```

### リリース実行の確認

リリースが成功したかを確認する方法：

```bash
# GitHub Actions の実行状態を確認
# https://github.com/akiojin/claude-worktree/actions

# npm registry で公開を確認
# https://www.npmjs.com/package/@akiojin/claude-worktree

# GitHub Releases で確認
# https://github.com/akiojin/claude-worktree/releases
```

## Conventional Commits

semantic-release はコミットメッセージからリリースタイプを自動決定します。

### コミットメッセージの形式

```text
<type>(<scope>): <subject>

<body>

<footer>
```

### 主要なコミットタイプ

| タイプ | 説明 | リリースタイプ | バージョン例 |
|--------|------|---------------|-------------|
| `feat:` | 新機能の追加 | minor | 1.0.0 → 1.1.0 |
| `fix:` | バグ修正 | patch | 1.0.0 → 1.0.1 |
| `BREAKING CHANGE:` | 破壊的変更 | major | 1.0.0 → 2.0.0 |
| `chore:` | 雑務（ビルド、設定など） | リリースなし | - |
| `docs:` | ドキュメント更新のみ | リリースなし | - |
| `style:` | コードスタイル修正 | リリースなし | - |
| `refactor:` | リファクタリング | リリースなし | - |
| `test:` | テスト追加・修正 | リリースなし | - |

### コミットメッセージの例

#### 新機能の追加（minor リリース）

```bash
git commit -m "feat: セッション管理機能を追加

ユーザーが以前のセッションを再開できるように
-c, --continue および -r, --resume オプションを実装"
```

**結果**: 1.0.0 → 1.1.0

#### バグ修正（patch リリース）

```bash
git commit -m "fix: Docker環境でのパスハンドリング修正

WSL2環境でのパス変換エラーを修正"
```

**結果**: 1.0.0 → 1.0.1

#### 破壊的変更（major リリース）

```bash
git commit -m "feat!: Bun 1.0+ を必須に変更

BREAKING CHANGE: npx 対応を廃止し、bunx のみサポート
ユーザーは Bun 1.0.0 以上をインストールする必要があります"
```

**結果**: 1.0.0 → 2.0.0

#### リリースなし

```bash
git commit -m "docs: README.md の誤字を修正"
```

**結果**: リリースされない

## よくある操作

### リリースを手動でトリガーする

通常は main ブランチへのマージで自動リリースされますが、手動で再実行する場合：

1. GitHub Actions のページに移動
2. "Release" ワークフローを選択
3. "Run workflow" ボタンをクリック
4. main ブランチを選択して実行

### リリースをスキップする

特定のマージでリリースを実行したくない場合、コミットメッセージに `[skip release]` を含めます：

```bash
git commit -m "chore: ビルド設定を更新 [skip release]"
```

**注**: リリース対象のコミット（feat:, fix: など）がない場合、semantic-release は自動的にリリースをスキップします。

### CHANGELOG を手動で確認

CHANGELOG.md はリリース時に自動更新されます：

```bash
# 最新の CHANGELOG を確認
cat CHANGELOG.md

# Git でコミット履歴を確認
git log --oneline
```

### npm パッケージの公開状態を確認

```bash
# npm registry の最新バージョンを確認
npm view @akiojin/claude-worktree version

# すべてのバージョンを確認
npm view @akiojin/claude-worktree versions
```

## トラブルシューティング

### リリースが実行されない

**症状**: PR をマージしたがリリースが作成されない

**原因と対処法**:

1. **リリース対象のコミットがない**
   - 確認: コミットメッセージが `feat:`, `fix:`, `BREAKING CHANGE:` を含むか
   - 対処: Conventional Commits 形式でコミット

2. **GitHub Actions が失敗している**
   - 確認: https://github.com/akiojin/claude-worktree/actions
   - 対処: エラーログを確認して修正

3. **テストまたはビルドが失敗している**
   - 確認: ローカルで `bun run test` と `bun run build` を実行
   - 対処: テストエラーを修正してから再マージ

### npm publish が失敗する

**症状**: semantic-release は成功するが npm 公開に失敗

**原因と対処法**:

1. **NPM_TOKEN が無効**
   - 確認: GitHub Settings → Secrets → NPM_TOKEN
   - 対処: npm で新しいトークンを生成して更新

2. **バージョンが既に存在**
   - 確認: `npm view @akiojin/claude-worktree versions`
   - 対処: 通常は発生しない（semantic-release が自動管理）

### GitHub Release 作成が失敗する

**症状**: npm 公開は成功するが GitHub Release が作成されない

**原因と対処法**:

1. **GITHUB_TOKEN の権限不足**
   - 確認: `.github/workflows/release.yml` の `permissions` セクション
   - 対処: `contents: write` 権限を確認

2. **タグが既に存在**
   - 確認: `git tag` でタグ一覧を確認
   - 対処: 重複タグを削除して再実行

## 開発ワークフロー

### 通常の開発フロー

```bash
# 1. 最新の main を取得
git checkout main
git pull origin main

# 2. feature ブランチを作成
git checkout -b feature/my-feature

# 3. 開発とコミット
git add .
git commit -m "feat: 新機能を追加"

# 4. リモートにプッシュ
git push origin feature/my-feature

# 5. GitHub で PR を作成

# 6. レビュー承認後、PR をマージ

# 7. 自動リリースを確認
# https://github.com/akiojin/claude-worktree/actions
```

### ホットフィックスフロー

緊急のバグ修正の場合：

```bash
# 1. hotfix ブランチを作成
git checkout -b hotfix/critical-bug main

# 2. バグ修正とコミット
git add .
git commit -m "fix: 重大なバグを修正"

# 3. PR を作成してマージ（レビュー優先度高）

# 4. 自動的に patch バージョンがリリース
# 例: 1.2.3 → 1.2.4
```

## 設定ファイル

### .releaserc.json

semantic-release の設定ファイル（プロジェクトルートに配置）：

```json
{
  "branches": ["main"],
  "tagFormat": "v${version}",
  "plugins": [
    "@semantic-release/commit-analyzer",
    "@semantic-release/release-notes-generator",
    [
      "@semantic-release/changelog",
      {
        "changelogFile": "CHANGELOG.md"
      }
    ],
    [
      "@semantic-release/npm",
      {
        "npmPublish": true
      }
    ],
    [
      "@semantic-release/git",
      {
        "assets": ["CHANGELOG.md", "package.json"],
        "message": "chore(release): ${nextRelease.version} [skip ci]\n\n${nextRelease.notes}"
      }
    ],
    "@semantic-release/github"
  ]
}
```

詳細は [data-model.md](./data-model.md) を参照。

### .github/workflows/release.yml

GitHub Actions ワークフロー（変更不要）：

```yaml
name: Release

on:
  push:
    branches: [main]

permissions:
  contents: write
  issues: write
  pull-requests: write
  id-token: write

jobs:
  release:
    name: Release
    runs-on: ubuntu-latest

    steps:
      - name: Checkout code
        uses: actions/checkout@v4
        with:
          fetch-depth: 0
          persist-credentials: false

      - name: Setup Bun
        uses: oven-sh/setup-bun@v2
        with:
          bun-version: latest

      - name: Install dependencies
        run: bun install

      - name: Run tests
        run: bun run test

      - name: Build
        run: bun run build

      - name: Semantic Release
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          NPM_TOKEN: ${{ secrets.NPM_TOKEN }}
        run: |
          bunx semantic-release
```

## 参考資料

- [semantic-release ドキュメント](https://semantic-release.gitbook.io/)
- [Conventional Commits 仕様](https://www.conventionalcommits.org/)
- [GitHub Actions ドキュメント](https://docs.github.com/en/actions)
- [npm パッケージ公開ガイド](https://docs.npmjs.com/cli/publish)

## まとめ

このリリースプロセスにより、以下のメリットが得られます：

- ✅ **完全自動化**: PR マージのみでリリース完了
- ✅ **バージョン管理**: Conventional Commits から自動決定
- ✅ **CHANGELOG**: 自動生成と更新
- ✅ **品質保証**: テストとビルドが必須
- ✅ **シンプル**: 手動操作不要、ヒューマンエラーなし

次のステップ: [tasks.md](./tasks.md)（`/speckit.tasks` コマンドで生成）で実装タスクを確認します。
