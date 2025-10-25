# クイックスタート: AIツール(Claude Code / Codex CLI)のbunx移行

**仕様ID**: `SPEC-c0deba7e` | **日付**: 2025-10-25
**関連ドキュメント**: [spec.md](./spec.md) | [plan.md](./plan.md) | [research.md](./research.md) | [data-model.md](./data-model.md)

## 概要

このガイドは、Claude CodeとCodex CLIの起動方式をnpxからbunxへ移行する作業を5分で理解するための開発者向けクイックスタートです。

## 前提条件

- TypeScript 5.8+の知識
- Gitの基本操作
- Bun 1.0+がローカル環境にインストール済み

## Bunインストール（未導入の場合）

### macOS / Linux

```bash
curl -fsSL https://bun.sh/install | bash
```

### Windows

```powershell
powershell -c "irm bun.sh/install.ps1|iex"
```

### 動作確認

```bash
bunx --version
# 出力例: 1.0.0
```

## 移行の背景

**現状**:
- **Claude Code**: npx経由で起動
- **Codex CLI**: bunx経由で起動（既に対応済み）

**問題点**:
- コードベース内でnpxとbunxが混在
- Bunを標準とするワークフローでの一貫性が欠如

**目標**:
- 両AIツールをbunx経由で起動に統一
- UI/ドキュメントの表記を統一

## 変更差分の確認

### Claude Code起動コマンド

**変更前** (`src/claude.ts:86`):
```typescript
await execa('npx', ['--yes', CLAUDE_CLI_PACKAGE, ...args], {
  cwd: worktreePath,
  stdio: 'inherit',
  shell: true
});
```

**変更後**:
```typescript
await execa('bunx', [CLAUDE_CLI_PACKAGE, ...args], {
  cwd: worktreePath,
  stdio: 'inherit',
  shell: true
});
```

**主な変更点**:
1. `'npx'` → `'bunx'`
2. `'--yes'`フラグを削除（bunxはデフォルトで自動承認）
3. パッケージ名と引数は変更なし

### エラーメッセージ

**変更前** (`src/claude.ts:92-94`):
```typescript
const errorMessage = error.code === 'ENOENT'
  ? 'npx command not found. Please ensure Node.js/npm is installed so Claude Code can run via npx.'
  : `Failed to launch Claude Code: ${error.message || 'Unknown error'}`;
```

**変更後**:
```typescript
const errorMessage = error.code === 'ENOENT'
  ? 'bunx command not found. Please ensure Bun is installed so Claude Code can run via bunx.'
  : `Failed to launch Claude Code: ${error.message || 'Unknown error'}`;
```

**主な変更点**:
1. `'npx'` → `'bunx'`
2. `'Node.js/npm'` → `'Bun'`

### Windowsトラブルシューティング

**変更前** (`src/claude.ts:96-100`):
```typescript
if (platform() === 'win32') {
  console.error(chalk.red('\n💡 Windows troubleshooting tips:'));
  console.error(chalk.yellow('   1. Ensure Node.js/npm がインストールされ npx が利用可能か確認'));
  console.error(chalk.yellow('   2. "npx @anthropic-ai/claude-code@latest -- --version" を実行してセットアップを確認'));
  console.error(chalk.yellow('   3. ターミナルやIDEを再起動して PATH を更新'));
}
```

**変更後**:
```typescript
if (platform() === 'win32') {
  console.error(chalk.red('\n💡 Windows troubleshooting tips:'));
  console.error(chalk.yellow('   1. Bun がインストールされ bunx が利用可能か確認'));
  console.error(chalk.yellow('   2. "bunx @anthropic-ai/claude-code@latest -- --version" を実行してセットアップを確認'));
  console.error(chalk.yellow('   3. ターミナルやIDEを再起動して PATH を更新'));
}
```

**主な変更点**:
1. `'Node.js/npm'` → `'Bun'`
2. `'npx'` → `'bunx'`

## ローカル開発環境での動作確認

### 1. ソースコード変更

```bash
# src/claude.tsを編集
# - 86行目: npx → bunx
# - 87行目: '--yes' を削除
# - 93行目: エラーメッセージ更新
# - 98-100行目: Windowsガイダンス更新
```

### 2. TypeScriptビルド

```bash
bun run build
```

### 3. 動作確認

#### 正常系テスト（Bun導入済み環境）

```bash
# テスト環境でbunx経由でClaude Codeを起動
bunx @anthropic-ai/claude-code@latest -- --version
```

**期待結果**: Claude Codeのバージョン情報が表示される

#### 異常系テスト（bunx未検出シミュレーション）

```bash
# PATHからBunを一時的に除外
export PATH_BACKUP=$PATH
export PATH=$(echo $PATH | sed 's|:.*bun.*||g')

# 起動試行
bun run start
# または
bunx .
```

**期待結果**: 以下のエラーメッセージが表示される
```
bunx command not found. Please ensure Bun is installed so Claude Code can run via bunx.

💡 Windows troubleshooting tips:
   1. Bun がインストールされ bunx が利用可能か確認
   2. "bunx @anthropic-ai/claude-code@latest -- --version" を実行してセットアップを確認
   3. ターミナルやIDEを再起動して PATH を更新
```

```bash
# PATH復元
export PATH=$PATH_BACKUP
```

## UI表示文言の更新

### AIツール選択メニュー

**ファイル**: `src/ui/prompts.ts`（該当箇所を確認）

**変更前**:
```
Claude Code (npx @anthropic-ai/claude-code@latest)
```

**変更後**:
```
Claude Code (bunx @anthropic-ai/claude-code@latest)
```

### 確認方法

```bash
# UI表示でbunx表記を確認
bun run start
# → AIツール選択メニューで確認
```

## ドキュメント更新

### 対象ファイル

1. `README.md` - 英語版
2. `README.ja.md` - 日本語版
3. `docs/troubleshooting.md` - トラブルシューティング

### 更新内容

#### README.md / README.ja.md

**変更箇所**:
- インストール手順セクション
- 使用例セクション
- トラブルシューティングリンク

**検索キーワード**: `npx`

```bash
# npx表記の残存確認
grep -r "npx" README.md README.ja.md
```

**変更例**:
```markdown
## Before
<!-- Using npx to run Claude Code -->
npx @anthropic-ai/claude-code@latest

## After
<!-- Using bunx to run Claude Code -->
bunx @anthropic-ai/claude-code@latest
```

#### docs/troubleshooting.md

**追加セクション**: bunx固有のトラブルシューティング

```markdown
## bunxが見つからない

**症状**: `bunx command not found`エラーが表示される

**解決方法**:
1. Bunがインストールされているか確認:
   ```bash
   bun --version
   ```

2. Bunがインストールされていない場合:
   - macOS/Linux: `curl -fsSL https://bun.sh/install | bash`
   - Windows: `powershell -c "irm bun.sh/install.ps1|iex"`

3. インストール後、ターミナルを再起動

4. PATHが正しく設定されているか確認:
   ```bash
   echo $PATH | grep bun
   ```

5. Windows固有: PowerShell実行ポリシーを確認:
   ```powershell
   Get-ExecutionPolicy
   # RemoteSigned または Unrestricted が推奨
   ```
```

## テスト実行

### ユニットテスト

```bash
bun run test
```

**対象テスト**:
- `tests/unit/claude.test.ts` - Claude Code起動ロジック
- `tests/unit/codex.test.ts` - Codex CLI起動ロジック（既存）

### 統合テスト

```bash
bun run test:integration
```

**対象テスト**:
- `tests/integration/ai-tool-launch.test.ts` - bunx経由の起動確認

### カバレッジ確認

```bash
bun run test:coverage
```

**目標**: 80%以上のカバレッジ

## よくある質問

### Q1: npx対応は残しますか？

**A**: いいえ。bunxへ完全移行し、npx対応は廃止します。これにより、コードベースの一貫性を保ちます。

### Q2: 既存ユーザーへの影響は？

**A**: Bun未導入ユーザーは、明確なエラーメッセージとインストール手順でガイダンスを受けます。

### Q3: Codex CLIは変更しますか？

**A**: いいえ。Codex CLIは既にbunx対応済みのため、変更不要です。

### Q4: Windows環境での注意点は？

**A**: PowerShellの実行ポリシーがRestrictedの場合、Bunインストールに失敗する可能性があります。`Set-ExecutionPolicy RemoteSigned`で変更してください。

### Q5: Node.js/npmは不要になりますか？

**A**: ランタイムとしては不要です。Bun 1.0+ を必須とし、CLIは Bun 上で動作します。ただし、開発者がNode製ツール（例: ドキュメント生成、補助スクリプト）を利用する場合は、任意でNode.js 18+を併用できます。

## トラブルシューティング

### bunx起動が遅い

**症状**: bunx経由の起動がnpxより遅い

**解決方法**:
1. Bunのバージョンを確認（1.0.0以上推奨）
2. キャッシュをクリア: `bun pm cache rm`
3. ネットワーク接続を確認

### Windows環境でPATHが認識されない

**症状**: Bunインストール後もbunxが見つからない

**解決方法**:
1. システム環境変数を開く
2. PATH変数にBunインストールディレクトリを追加
   - デフォルト: `%USERPROFILE%\.bun\bin`
3. ターミナルを完全に再起動（IDEごと）

### Bunインストールが失敗する

**症状**: インストールスクリプトがエラーを返す

**解決方法**:
1. PowerShell実行ポリシーを確認: `Get-ExecutionPolicy`
2. 必要に応じて変更: `Set-ExecutionPolicy RemoteSigned -Scope CurrentUser`
3. インストールを再試行

## 次のステップ

1. ✅ クイックスタートガイド確認完了
2. ⏭️ [tasks.md](./tasks.md)で実装タスクを確認
3. ⏭️ ローカル環境でbunx動作確認
4. ⏭️ 実装開始

## 参考資料

- [Bun公式ドキュメント](https://bun.sh/docs)
- [bunxコマンドリファレンス](https://bun.sh/docs/cli/bunx)
- [Bunインストールガイド](https://bun.sh/docs/installation)
- [execa APIドキュメント](https://github.com/sindresorhus/execa)
