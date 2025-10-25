# クイックスタートガイド: Docker/root環境でのClaude Code自動承認機能

**仕様ID**: SPEC-8efcbf19
**対象**: 開発者・レビュアー
**最終更新**: 2025-10-25

## 機能概要

Docker/root環境でClaude Codeの`--dangerously-skip-permissions`フラグを使用する際、IS_SANDBOX=1環境変数を自動設定してrootユーザー制限を回避します。

### ユースケース

- Docker環境でClaude Codeを実行する開発者
- コンテナ内でPermission promptなしでClaude Codeを使用したい開発者
- CI/CD環境でClaude Codeを自動化したいDevOpsエンジニア

### 主な変更点

| 項目 | 変更前 | 変更後 |
|------|--------|--------|
| Root環境でのskipPermissions | エラーで起動失敗 | IS_SANDBOX=1設定で正常起動 |
| 警告メッセージ | なし | Docker環境であることを通知 |
| 非root環境 | 既存の動作 | 変更なし（既存の動作維持） |

## セットアップ

### 前提条件

- Bun 1.0以上
- Docker環境（推奨）
- Linux/macOS（POSIXシステム）

### 開発環境の準備

```bash
# リポジトリをクローン
git clone <repository-url>
cd claude-worktree

# 依存関係をインストール
bun install

# ビルド
bun run build
```

## 実装箇所

### 修正対象ファイル

**src/claude.ts** - `launchClaudeCode`関数（17-134行目付近）

### 変更内容の概要

1. **rootユーザー検出ロジックの追加** (約5行)
   ```typescript
   let isRoot = false;
   try {
     isRoot = process.getuid && process.getuid() === 0;
   } catch {
     // Windows等では何もしない
   }
   ```

2. **環境変数設定の追加** (約3行)
   ```typescript
   env: isRoot && options.skipPermissions
     ? { ...process.env, IS_SANDBOX: '1' }
     : process.env
   ```

3. **警告メッセージの追加** (約3行)
   ```typescript
   if (isRoot && options.skipPermissions) {
     console.log(chalk.yellow("   ⚠️  Docker/サンドボックス環境として実行中（IS_SANDBOX=1）"));
   }
   ```

### 実装フロー

```
launchClaudeCode()開始
    ↓
rootユーザー検出
    ↓
skipPermissions=true?
    ├─ Yes + Root → IS_SANDBOX=1設定 + 警告表示
    └─ No/Non-root → 既存の動作
    ↓
execaでClaude Code起動
```

## 開発ワークフロー

### 1. ローカルでの開発

```bash
# ウォッチモードでビルド
bun run dev

# 別のターミナルでテスト実行
bun run test:watch
```

### 2. テストの実行

#### ユニットテスト

```bash
# すべてのテストを実行
bun run test

# カバレッジ付きで実行
bun run test:coverage

# 特定のテストファイルを実行
bun run test tests/unit/claude.test.ts
```

#### Docker環境でのテスト

```bash
# Dockerコンテナを起動（root環境）
docker run -it --rm -v $(pwd):/app -w /app node:22 bash

# コンテナ内でテスト
bun install
bun run build
bun run start
# → "Skip permission checks?"でYesを選択
# → "⚠️ Docker/サンドボックス環境として実行中" が表示されることを確認
```

### 3. 動作確認の手順

#### テストケース1: Root環境でskipPermissions=true

```bash
# Docker環境で実行
docker run -it --rm node:22 bash

# Claude Worktreeを実行
bunx @akiojin/claude-worktree
# → "Skip permission checks?"でYesを選択

# 期待結果:
# ✅ "⚠️ Docker/サンドボックス環境として実行中（IS_SANDBOX=1）"が表示される
# ✅ Claude Codeがエラーなく起動する
```

#### テストケース2: 非root環境でskipPermissions=true

```bash
# 非rootユーザーで実行
bunx @akiojin/claude-worktree
# → "Skip permission checks?"でYesを選択

# 期待結果:
# ✅ 警告メッセージは表示されない
# ✅ 既存の動作と同じ（IS_SANDBOX=1は設定されない）
```

#### テストケース3: Root環境でskipPermissions=false

```bash
# Docker環境で実行
docker run -it --rm node:22 bash
bunx @akiojin/claude-worktree
# → "Skip permission checks?"でNoを選択

# 期待結果:
# ✅ 警告メッセージは表示されない
# ✅ IS_SANDBOX=1は設定されない
# ✅ 通常のPermission prompt動作
```

## よくある操作

### デバッグモードでの実行

```bash
# 環境変数を手動で確認
NODE_DEBUG=* bunx . 2>&1 | grep IS_SANDBOX
```

### ログの確認

```bash
# Claude Codeの起動ログを確認
bunx . 2>&1 | tee claude-worktree.log
```

## トラブルシューティング

### 問題1: Windows環境で動作しない

**症状**: Windows環境でIS_SANDBOX=1が設定されない

**原因**: `process.getuid()`がWindows環境で利用不可

**解決策**:
- 設計上の制限です。Windows環境ではrootユーザー検出がスキップされ、既存の動作を維持します
- Docker Desktop for Windowsを使用してLinuxコンテナで実行してください

### 問題2: IS_SANDBOX=1を設定してもClaude Codeがエラーを返す

**症状**: `--dangerously-skip-permissions cannot be used with root/sudo privileges`エラーが表示される

**原因**: Claude Codeの将来バージョンでIS_SANDBOX=1がサポートされなくなった可能性

**解決策**:
1. Claude Codeのバージョンを確認:
   ```bash
   bunx @anthropic-ai/claude-code@latest --version
   ```
2. GitHub Issue #3490を確認して最新情報を取得
3. 必要に応じて、非rootユーザーでの実行に切り替え

### 問題3: 警告メッセージが表示されない

**症状**: rootユーザーでskipPermissions=trueでも警告が表示されない

**デバッグ手順**:
1. rootユーザーであることを確認:
   ```bash
   id -u  # 0であることを確認
   ```
2. TypeScriptのビルドを確認:
   ```bash
   bun run build
   ```
3. ログを確認:
   ```bash
   bunx . 2>&1 | grep "サンドボックス"
   ```

### 問題4: テストが失敗する

**症状**: `bun run test`でテストが失敗する

**解決策**:
```bash
# 依存関係を再インストール
rm -rf node_modules bun.lockb
bun install

# ビルドをクリーン
bun run clean
bun run build

# テストを再実行
bun run test
```

## 関連資料

- [機能仕様書](./spec.md)
- [実装計画](./plan.md)
- [調査レポート](./research.md)
- [Claude Code GitHub Issue #3490](https://github.com/anthropics/claude-code/issues/3490)
- [Node.js process.getuid() Documentation](https://nodejs.org/api/process.html#processgetuid)

## 次のステップ

1. ✅ フェーズ0完了: 調査と技術スタック決定
2. ✅ フェーズ1完了: 設計とクイックスタートガイド作成
3. ⏭️ `/speckit.tasks` を実行してタスクを生成
4. ⏭️ `/speckit.implement` で実装を開始
