# クイックスタートガイド: Worktreeディレクトリパス変更

**仕様ID**: `SPEC-57a7d9fa` | **日付**: 2025-10-31

## 概要

このガイドは、Worktreeディレクトリパス変更機能の開発を開始するための手順を提供します。

## 前提条件

- Bun 1.0+がインストールされていること
- Git 2.5+がインストールされていること
- このリポジトリがクローンされていること

```bash
# Bunのインストール確認
bun --version  # 1.0.0以上

# Gitのインストール確認
git --version  # 2.5.0以上
```

## セットアップ手順

### 1. 依存関係のインストール

```bash
cd /path/to/claude-worktree
bun install
```

### 2. ビルド

```bash
bun run build
```

出力: `dist/`ディレクトリにコンパイル済みJavaScriptファイルが生成されます。

### 3. テストの実行

```bash
# ビルド＆テストを一度に実行
bun run build && bun test

# または、ウォッチモードでテスト
bun run test:watch
```

## 開発ワークフロー

### TDDサイクル

このプロジェクトはTDD（Test-Driven Development）を採用しています。

```bash
# 1. Red: テストを先に書く/修正する
# tests/unit/worktree.test.ts を編集

# 2. テストを実行（失敗することを確認）
bun test

# 3. Green: 実装を修正してテストを通す
# src/worktree.ts を編集

# 4. テストを再実行（成功することを確認）
bun test

# 5. Refactor: コードをクリーンアップ
# src/worktree.ts をリファクタリング

# 6. テストを再実行（まだ成功することを確認）
bun test
```

### 開発サーバー（ウォッチモード）

```bash
# TypeScriptの自動再コンパイル
bun run dev
```

別のターミナルで:

```bash
# CLIツールをローカルで実行
bun run start
```

## よくある操作

### ローカルでCLIツールをテスト

```bash
# オプション1: ビルド済みバージョンを実行
bun run start

# オプション2: 直接実行
bunx .

# オプション3: 開発モードで実行（自動再ビルド）
bun run dev
# 別のターミナルで
bun run start
```

### 特定のテストを実行

```bash
# generateWorktreePathのテストのみ実行
bun test worktree.test.ts

# 特定のテストケースを実行
bun test --grep "should generate worktree path"
```

### コードカバレッジを確認

```bash
bun run test:coverage
```

出力: `coverage/`ディレクトリにHTMLレポートが生成されます。

### コードの品質チェック

```bash
# リント
bun run lint

# 型チェック
bun run type-check

# フォーマットチェック
bun run format:check

# フォーマット自動修正
bun run format
```

## 主要ファイルの編集

### src/worktree.ts

Worktreeパス生成ロジックの変更:

```typescript
// 変更前（135行目）
const worktreeDir = path.join(repoRoot, ".git", "worktree");

// 変更後
const worktreeDir = path.join(repoRoot, ".worktrees");
```

### tests/unit/worktree.test.ts

テストの期待値を更新:

```typescript
// 変更前（106行目）
expect(path).toBe("/path/to/repo/.git/worktree/feature-user-auth");

// 変更後
expect(path).toBe("/path/to/repo/.worktrees/feature-user-auth");
```

## デバッグ方法

### デバッグログの有効化

環境変数を設定:

```bash
export DEBUG=claude-worktree
bun run start
```

### Vitestデバッガーの使用

```bash
# UIモードでテストを実行
bun run test:ui
```

ブラウザが開き、テスト結果を視覚的に確認できます。

### VSCodeでのデバッグ

`.vscode/launch.json`を作成:

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "type": "node",
      "request": "launch",
      "name": "Debug Tests",
      "runtimeExecutable": "bun",
      "runtimeArgs": ["test"],
      "console": "integratedTerminal"
    }
  ]
}
```

## 変更後の動作確認

### 手動テスト手順

1. **ビルドとテストの成功を確認**:
   ```bash
   bun run build && bun test
   ```

2. **実際のリポジトリで動作確認**:
   ```bash
   # テスト用リポジトリを作成
   cd /tmp
   git init test-repo
   cd test-repo
   git commit --allow-empty -m "Initial commit"

   # claude-worktreeツールを実行
   /path/to/claude-worktree/dist/index.js

   # 新しいブランチを作成してWorktreeを確認
   # → .worktreesディレクトリが作成されることを確認
   ls -la .worktrees/
   ```

3. **.gitignoreの更新を確認**:
   ```bash
   cat .gitignore | grep ".worktrees"
   # → ".worktrees/" が含まれることを確認
   ```

4. **既存Worktreeが影響を受けないことを確認**:
   ```bash
   # .git/worktree配下にWorktreeが存在する場合
   cd .git/worktree/some-branch
   git status  # → 正常に動作することを確認
   ```

## トラブルシューティング

### ビルドエラー

**症状**: `bun run build`が失敗する

**原因**: TypeScriptの型エラー

**対処法**:
```bash
# 型エラーの詳細を確認
bun run type-check

# エラー箇所を修正
# src/worktree.ts を編集

# 再ビルド
bun run build
```

### テスト失敗

**症状**: `bun test`が失敗する

**原因**: 期待値が古い

**対処法**:
```bash
# 失敗したテストの詳細を確認
bun test --reporter=verbose

# テストファイルを修正
# tests/unit/worktree.test.ts を編集

# 再テスト
bun test
```

### .gitignore更新エラー

**症状**: `.gitignore`が更新されない

**原因**: ファイルが読み取り専用

**対処法**:
```bash
# パーミッションを確認
ls -l .gitignore

# 書き込み権限を追加
chmod u+w .gitignore

# 再実行
bun run start
```

## パフォーマンスのベンチマーク

### ビルド時間

```bash
time bun run build
# 期待値: < 5秒
```

### テスト実行時間

```bash
time bun test
# 期待値: < 10秒
```

### Worktree作成時間

```bash
# 実際のリポジトリで測定
time /path/to/claude-worktree/dist/index.js
# 期待値: < 5秒
```

## 次のステップ

1. ✅ セットアップ完了
2. 次: TDDサイクルで実装開始
3. 次: 変更後の動作確認
4. 次: プルリクエストの作成

## 参考リンク

- [Bun公式ドキュメント](https://bun.sh/docs)
- [Vitest公式ドキュメント](https://vitest.dev/)
- [Git worktree公式ドキュメント](https://git-scm.com/docs/git-worktree)
- [プロジェクトREADME](../../README.md)
- [実装計画](./plan.md)
- [技術調査](./research.md)
- [データモデル](./data-model.md)
