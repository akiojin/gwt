# 調査レポート: Docker/root環境でのClaude Code自動承認機能

**仕様ID**: SPEC-8efcbf19
**作成日**: 2025-10-25
**目的**: 技術スタック選定と既存コードパターンの理解

## 既存コードベース分析

### 技術スタック

**言語・ランタイム**:
- TypeScript 5.8.3
- Bun 1.0+ (package manager & runtime)
- Node.js 互換 (process API使用)

**主要な依存関係**:
- `execa@9.6.0`: プロセス実行ライブラリ（環境変数設定サポート）
- `chalk@5.4.1`: ターミナル出力の色付け
- `@inquirer/prompts@6.0.1`: 対話型プロンプト

**テストフレームワーク**:
- `vitest@2.1.8`: ユニット・統合・E2Eテスト
- `@vitest/coverage-v8@2.1.8`: カバレッジ計測

### 既存の実装パターン

#### 1. launchClaudeCode関数（src/claude.ts:17-134）

**現在の実装**:
```typescript
export async function launchClaudeCode(
  worktreePath: string,
  options: {
    skipPermissions?: boolean;
    mode?: "normal" | "continue" | "resume";
    extraArgs?: string[];
  } = {},
): Promise<void>
```

**主要な処理フロー**:
1. worktreePath存在確認
2. コンソールメッセージ表示（chalk使用）
3. モード別のargsビルド（normal/continue/resume）
4. skipPermissions=true時に`--dangerously-skip-permissions`追加
5. execaでClaude Code起動

**環境変数設定なし**: 現在の実装ではexecaに環境変数を渡していない

#### 2. エラーハンドリングパターン

**ClaudeErrorクラス** (src/claude.ts:7-14):
```typescript
export class ClaudeError extends Error {
  constructor(
    message: string,
    public cause?: unknown,
  ) {
    super(message);
    this.name = "ClaudeError";
  }
}
```

**エラーハンドリング** (src/claude.ts:111-133):
- try-catchで例外捕捉
- error.codeで特定のエラータイプを判定
- プラットフォーム別のトラブルシューティングヒント表示

#### 3. メッセージ表示パターン

**chalkの使用パターン**:
- `chalk.blue()`: 情報メッセージ（例: "🚀 Launching Claude Code..."）
- `chalk.yellow()`: 警告メッセージ（例: "⚠️ Skipping permissions check"）
- `chalk.red()`: エラーメッセージ
- `chalk.gray()`: 補助情報

## 技術的決定

### 決定1: rootユーザー検出方法

**選択**: `process.getuid() === 0`

**理由**:
- POSIX標準APIで広くサポートされている
- Node.js/Bunで直接利用可能
- UID 0はUNIX/Linuxでrootユーザーを示す標準

**代替案**:
- `process.env.USER === 'root'`: 環境変数は信頼性が低い（偽装可能）
- `os.userInfo().username === 'root'`: 追加の依存関係が必要

**実装方法**:
```typescript
// Try-catchでprocess.getuid()の非存在をハンドリング
try {
  const isRoot = process.getuid && process.getuid() === 0;
  if (isRoot && options.skipPermissions) {
    // IS_SANDBOX=1を設定
  }
} catch {
  // Windowsなど非POSIXシステムでは何もしない
}
```

### 決定2: 環境変数設定方法

**選択**: execaの`env`オプション

**理由**:
- execaは既に使用中の依存関係
- 環境変数の安全な設定をサポート
- 既存の環境変数をマージ可能

**実装方法**:
```typescript
await execa("bunx", [CLAUDE_CLI_PACKAGE, ...args], {
  cwd: worktreePath,
  stdio: "inherit",
  shell: true,
  env: isRootAndSkipPermissions
    ? { ...process.env, IS_SANDBOX: '1' }
    : process.env,
});
```

**代替案**:
- `process.env.IS_SANDBOX = '1'`: グローバルに設定してしまう（副作用あり）
- `child_process.spawn()`: execaより低レベル

### 決定3: 警告メッセージ表示

**選択**: chalkの既存パターンに従う

**警告メッセージ案**:
```typescript
console.log(chalk.yellow("   ⚠️  Docker/サンドボックス環境として実行中（IS_SANDBOX=1）"));
```

**理由**:
- 既存のUI/UXパターンとの一貫性
- ユーザーがセキュリティリスクを認識できる
- 絵文字とインデントで視認性向上

### 決定4: エラーハンドリング

**選択**: try-catchでprocess.getuid()の非存在をハンドリング

**理由**:
- Windowsなど非POSIXシステムでは`process.getuid()`が存在しない
- 例外発生時は既存の動作を維持（フォールバック）
- 後方互換性を保証

**実装方法**:
```typescript
let isRoot = false;
try {
  isRoot = process.getuid && process.getuid() === 0;
} catch {
  // process.getuid()が利用できない環境では、isRoot=falseのまま
}
```

## 制約と依存関係

### 制約

1. **POSIXシステムのみ対応**
   - Windows環境では`process.getuid()`が利用不可
   - 影響: Windows環境ではrootユーザー検出がスキップされ、IS_SANDBOX=1は設定されない
   - 許容理由: Docker環境はLinux/macOSが主流、Windowsでの影響は限定的

2. **IS_SANDBOX=1は非公式環境変数**
   - Claude Code側で公式ドキュメント化されていない
   - コミュニティ発見（GitHub Issue #3490）
   - 影響: 将来のClaude Codeバージョンで動作しなくなる可能性
   - 緩和策: 警告メッセージでユーザーに通知、ドキュメント化

3. **既存の動作を非破壊的に拡張**
   - 非rootユーザーでの動作は変更しない
   - skipPermissions=false時は変更しない
   - 影響: 既存ユーザーへの影響を最小化

### 依存関係

1. **Claude Code CLI（@anthropic-ai/claude-code）**
   - IS_SANDBOX=1環境変数のサポート
   - 検証状況: コミュニティメンバーによる動作確認済み（GitHub Issue #3490）
   - リスク: 公式サポートではないため、将来変更される可能性あり

2. **Node.js process API**
   - `process.getuid()`: POSIX準拠システムでのみ利用可能
   - `process.env`: 環境変数アクセス

3. **execa@9.6+**
   - 環境変数設定機能（`env`オプション）
   - 既に依存関係として含まれている

## ベストプラクティスと参考資料

### セキュリティベストプラクティス

1. **明示的なユーザー確認**
   - skipPermissions=trueは既にユーザーが選択済み
   - 警告メッセージで追加の注意喚起

2. **フォールバック動作**
   - process.getuid()が利用できない場合は既存の動作を維持
   - エラーを投げず、サイレントにフォールバック

3. **ドキュメント化**
   - README.mdにDocker環境での使用方法を記載
   - セキュリティリスクと制限事項を明記

### テスト戦略

1. **ユニットテスト**
   - rootユーザー検出ロジックのモック
   - 環境変数設定の検証
   - 警告メッセージ表示の検証

2. **統合テスト**
   - Docker環境でのE2Eテスト
   - root/非root両環境でのテスト

### 参考資料

- [Claude Code GitHub Issue #3490](https://github.com/anthropics/claude-code/issues/3490) - IS_SANDBOX=1環境変数の発見
- [Node.js process.getuid() Documentation](https://nodejs.org/api/process.html#processgetuid)
- [execa Environment Variables](https://github.com/sindresorhus/execa#env)
- [SPEC-c0deba7e](../SPEC-c0deba7e/spec.md) - AIツールのbunx移行（関連仕様）

## 次のステップ

✅ **フェーズ0完了**: 技術スタック決定と既存コードパターン理解

次のフェーズ:
- **フェーズ1**: データモデル設計（N/A）とクイックスタートガイド作成
- **フェーズ2**: タスク生成（`/speckit.tasks`）
- **実装**: コード修正とテスト追加
