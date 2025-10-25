# 技術調査: AIツール(Claude Code / Codex CLI)のbunx移行

**仕様ID**: `SPEC-c0deba7e` | **日付**: 2025-10-25
**関連ドキュメント**: [spec.md](./spec.md) | [plan.md](./plan.md)

## 調査概要

このドキュメントは、Claude CodeとCodex CLIの起動方式をnpxからbunxへ移行するための技術調査結果をまとめています。既存のCodex CLI bunx実装をリファレンスとして、Claude Codeのbunx移行パターンを決定します。

## 調査項目1: 既存のCodex CLI bunx実装分析

### 現状確認

**ファイル**: `src/codex.ts`

**bunx起動パターン**:
```typescript
await execa('bunx', [CODEX_CLI_PACKAGE, ...args], {
  cwd: worktreePath,
  stdio: 'inherit',
  shell: true
});
```

**エラーハンドリング**:
```typescript
catch (error: any) {
  const errorMessage = error.code === 'ENOENT'
    ? 'bunx command not found. Please ensure Bun is installed so Codex CLI can run via bunx.'
    : `Failed to launch Codex CLI: ${error.message || 'Unknown error'}`;
  throw new CodexError(errorMessage, error);
}
```

**Windowsトラブルシューティングメッセージ**:
```typescript
if (platform() === 'win32') {
  console.error(chalk.red('\n💡 Windows troubleshooting tips:'));
  console.error(chalk.yellow('   1. Bun がインストールされ bunx が利用可能か確認'));
  console.error(chalk.yellow('   2. "bunx @openai/codex@latest -- --help" を実行してセットアップを確認'));
  console.error(chalk.yellow('   3. ターミナルやIDEを再起動して PATH を更新'));
}
```

### 重要な発見

1. **execaライブラリの使用**: Node.js子プロセス実行に`execa`を使用
2. **shell: trueオプション**: シェル経由での実行が有効
3. **stdio: 'inherit'**: 標準入出力を親プロセスに継承
4. **ENOENTエラー検出**: コマンド未検出時の明確なエラーメッセージ
5. **プラットフォーム固有のガイダンス**: Windows環境で追加のトラブルシューティング表示

## 調査項目2: Claude Code起動の現状確認

### 現状確認

**ファイル**: `src/claude.ts`

**npx起動パターン**:
```typescript
await execa('npx', ['--yes', CLAUDE_CLI_PACKAGE, ...args], {
  cwd: worktreePath,
  stdio: 'inherit',
  shell: true
});
```

**エラーハンドリング**:
```typescript
catch (error: any) {
  const errorMessage = error.code === 'ENOENT'
    ? 'npx command not found. Please ensure Node.js/npm is installed so Claude Code can run via npx.'
    : `Failed to launch Claude Code: ${error.message || 'Unknown error'}`;
  throw new ClaudeError(errorMessage, error);
}
```

**Windowsトラブルシューティングメッセージ**:
```typescript
if (platform() === 'win32') {
  console.error(chalk.red('\n💡 Windows troubleshooting tips:'));
  console.error(chalk.yellow('   1. Ensure Node.js/npm がインストールされ npx が利用可能か確認'));
  console.error(chalk.yellow('   2. "npx @anthropic-ai/claude-code@latest -- --version" を実行してセットアップを確認'));
  console.error(chalk.yellow('   3. ターミナルやIDEを再起動して PATH を更新'));
}
```

**実行モード処理**:
- **normal**: 引数なし（新規セッション）
- **continue**: `-c` オプション（前回のセッション継続）
- **resume**: カスタム会話選択 → `--resume <sessionId>`

**オプション引数パススルー**:
```typescript
if (options.extraArgs && options.extraArgs.length > 0) {
  args.push(...options.extraArgs);
}
```

### 重要な発見

1. **npxの`--yes`フラグ**: パッケージ自動インストール承認
2. **bunxとの違い**: bunxは`--yes`不要（デフォルトで自動承認）
3. **引数順序**: `npx --yes PACKAGE ...args` → `bunx PACKAGE ...args`
4. **エラーメッセージの類似性**: Codex CLIと同じパターンを使用可能

## 調査項目3: Bunパッケージの可用性確認

### @anthropics/claude-codeパッケージ

**調査結果**:
- **パッケージ名**: `@anthropic-ai/claude-code@latest`（src/claude.tsから確認）
- **bunx互換性**: npmパッケージはbunxで実行可能（公式サポート）
- **動作確認方法**: `bunx @anthropic-ai/claude-code@latest -- --version`
- **想定される問題**: 特になし（Bunはnpmパッケージをネイティブサポート）

### @openai/codexパッケージ

**調査結果**:
- **パッケージ名**: `@openai/codex@latest`
- **bunx互換性**: 既にsrc/codex.tsで実装済み、動作確認済み
- **実績**: プロジェクト内で既に使用中

### 決定: bunx移行は技術的に問題なし

両パッケージともbunx経由での実行が可能。npxとbunxの互換性が高く、コマンド置き換えのみで移行可能。

## 調査項目4: エラーメッセージのパターン統一

### Bunインストール手順の案内方法

**決定**: 以下のメッセージパターンを採用

**基本エラーメッセージ**（日本語）:
```
bunx command not found. Please ensure Bun is installed so Claude Code/Codex CLI can run via bunx.
```

**Bunインストール手順**:
```
Bun のインストール方法:
  macOS/Linux: curl -fsSL https://bun.sh/install | bash
  Windows: powershell -c "irm bun.sh/install.ps1|iex"

詳細: https://bun.sh/docs/installation
```

### Windows固有のトラブルシューティング

**決定**: プラットフォーム検出によるガイダンス追加

**Windowsガイダンス**:
```
💡 Windows troubleshooting tips:
   1. Bun がインストールされ bunx が利用可能か確認
   2. "bunx @anthropic-ai/claude-code@latest -- --version" を実行してセットアップを確認
   3. ターミナルやIDEを再起動して PATH を更新
   4. PowerShellの実行ポリシーを確認: Get-ExecutionPolicy
```

### PATH更新手順の説明

**決定**: インストール後のPATH更新手順を案内

**PATH更新ガイダンス**:
```
Bun インストール後:
  1. ターミナルを再起動
  2. `bunx --version` で動作確認
  3. 動作しない場合、シェル設定ファイル（~/.bashrc, ~/.zshrc等）を確認
  4. Windows: 環境変数PATHに Bun インストールディレクトリを追加
```

## 技術的決定のサマリー

### 決定1: bunx起動パターンの統一

**決定**: Codex CLIと同じパターンをClaude Codeに適用

**変更前**:
```typescript
await execa('npx', ['--yes', CLAUDE_CLI_PACKAGE, ...args], { ... });
```

**変更後**:
```typescript
await execa('bunx', [CLAUDE_CLI_PACKAGE, ...args], { ... });
```

**理由**:
- bunxは`--yes`フラグ不要（デフォルトで自動承認）
- Codex CLIと一貫したパターン
- シンプルな実装

### 決定2: エラーハンドリングの統一

**決定**: Codex CLIのエラーハンドリングパターンを再利用

**パターン**:
```typescript
try {
  await execa('bunx', [PACKAGE, ...args], { ... });
} catch (error: any) {
  const errorMessage = error.code === 'ENOENT'
    ? 'bunx command not found. Please ensure Bun is installed.'
    : `Failed to launch: ${error.message}`;

  if (platform() === 'win32') {
    // Windows固有のガイダンス
  }

  throw new Error(errorMessage, error);
}
```

**理由**:
- ENOENT検出による明確なエラーメッセージ
- プラットフォーム固有のガイダンス
- 既存パターンとの一貫性

### 決定3: UI表示文言の更新

**決定**: すべてのAIツール起動表示でbunx表記を使用

**対象ファイル**:
- `src/ui/prompts.ts` - AIツール選択メニュー
- `src/ui/display.ts` - ヘルプメッセージ（該当する場合）

**変更例**:
```
変更前: "Claude Code (npx @anthropic-ai/claude-code@latest)"
変更後: "Claude Code (bunx @anthropic-ai/claude-code@latest)"
```

### 決定4: ドキュメント更新範囲

**決定**: README、トラブルシューティング、APIドキュメントを更新

**対象ファイル**:
1. `README.md` - インストール手順、使用例
2. `README.ja.md` - 日本語版
3. `docs/troubleshooting.md` - bunx前提のトラブルシューティング
4. `docs/api.md` - API例（該当する場合）

**更新内容**:
- `npx`表記を`bunx`に置き換え
- Bunインストール手順を追加
- Windows固有のトラブルシューティング強化

## 制約と仮定の検証

### 制約: Bun 1.0.0以上

**検証結果**: ✅ 妥当
- bunx機能はBun 1.0.0以降で安定
- package.jsonのenginesフィールドで`"bun": ">=1.0.0"`を指定済み

### 仮定: bunxパッケージ実行のサポート

**検証結果**: ✅ 検証済み
- Bunはnpmパッケージをネイティブサポート
- `@anthropic-ai/claude-code`と`@openai/codex`の両方でbunx実行可能

### 仮定: 既存機能の権限設定

**検証結果**: ✅ 影響なし
- bunxはnpxと同等の権限で実行
- `--dangerously-skip-permissions`等のオプションはそのまま使用可能

## リスク評価

### 技術的リスク: 低

- 既存のCodex CLI bunx実装が動作実績あり
- execaライブラリでのコマンド置き換えのみ
- 大規模な変更不要

### ユーザー影響リスク: 中

- Bun未導入ユーザーへの影響
- 緩和策: 明確なエラーメッセージとインストール手順

### 依存関係リスク: 低

- Bun公式配布チャネルは安定
- npmパッケージは継続提供

## 次のステップ

1. ✅ 調査完了: bunx移行パターン確定
2. ⏭️ data-model.md生成: ランタイムエンティティ定義
3. ⏭️ quickstart.md生成: 開発者向け移行ガイド
4. ⏭️ tasks.md生成: 実装タスクリスト作成
5. ⏭️ 実装開始: Claude Code bunx移行
