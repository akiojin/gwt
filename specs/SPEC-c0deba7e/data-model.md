# データモデル: AIツール(Claude Code / Codex CLI)のbunx移行

**仕様ID**: `SPEC-c0deba7e` | **日付**: 2025-10-25
**関連ドキュメント**: [spec.md](./spec.md) | [plan.md](./plan.md) | [research.md](./research.md)

## 概要

この機能は主にコマンド実行ロジックの変更であり、永続化データを含みません。このドキュメントでは、bunx起動処理で使用されるランタイムエンティティとその関係を定義します。

## ランタイムエンティティ

### 1. LaunchCommand

**目的**: AIツール起動コマンドの構成を表現

**定義箇所**: `src/claude.ts`, `src/codex.ts`（関数パラメータ）

```typescript
interface LaunchOptions {
  mode?: 'normal' | 'continue' | 'resume';
  skipPermissions?: boolean;         // Claude Codeのみ
  bypassApprovals?: boolean;         // Codex CLIのみ
  extraArgs?: string[];
}
```

**フィールド説明**:

| フィールド | 型 | 必須 | 説明 | デフォルト値 |
|-----------|------|------|------|------------|
| `mode` | 'normal' \| 'continue' \| 'resume' | - | 実行モード | 'normal' |
| `skipPermissions` | boolean | - | 権限チェックスキップ（Claude Codeのみ） | false |
| `bypassApprovals` | boolean | - | 承認バイパス（Codex CLIのみ） | false |
| `extraArgs` | string[] | - | 追加の引数 | [] |

**実行モードの説明**:
- **normal**: 新規セッションを開始
- **continue**: 前回のセッションを継続（Claude Code: `-c`, Codex CLI: `resume --last`）
- **resume**: セッションを選択して再開（Claude Code: カスタム選択, Codex CLI: `resume`）

**bunxコマンドへの変換ロジック**:

```typescript
// Claude Code
const command = 'bunx';
const packageName = '@anthropic-ai/claude-code@latest';
const args: string[] = [];

switch (mode) {
  case 'continue':
    args.push('-c');
    break;
  case 'resume':
    args.push('--resume', sessionId);
    break;
  case 'normal':
  default:
    // 引数なし
}

if (skipPermissions) {
  args.push('--dangerously-skip-permissions');
}

if (extraArgs) {
  args.push(...extraArgs);
}

// 最終コマンド: bunx @anthropic-ai/claude-code@latest [args]
```

```typescript
// Codex CLI
const command = 'bunx';
const packageName = '@openai/codex@latest';
const args: string[] = [];

switch (mode) {
  case 'continue':
    args.push('resume', '--last');
    break;
  case 'resume':
    args.push('resume');
    break;
  case 'normal':
  default:
    // 引数なし
}

if (bypassApprovals) {
  args.push('--yolo');
}

if (extraArgs) {
  args.push(...extraArgs);
}

args.push('-c', 'web_search_request=true');

// 最終コマンド: bunx @openai/codex@latest [args] -c web_search_request=true
```

---

### 2. BunxExecutionContext

**目的**: bunx実行時のコンテキスト情報

**定義箇所**: `src/claude.ts`, `src/codex.ts`（execa関数パラメータ）

```typescript
interface BunxExecutionContext {
  cwd: string;           // 作業ディレクトリ
  stdio: 'inherit';      // 標準入出力の継承
  shell: boolean;        // シェル経由実行
}
```

**フィールド説明**:

| フィールド | 型 | 値 | 説明 |
|-----------|------|------|------|
| `cwd` | string | worktreePath | ワークツリーの絶対パス |
| `stdio` | string | 'inherit' | 親プロセスの標準入出力を継承 |
| `shell` | boolean | true | シェル経由でコマンドを実行 |

**使用例**:
```typescript
await execa('bunx', [packageName, ...args], {
  cwd: worktreePath,
  stdio: 'inherit',
  shell: true
});
```

---

### 3. ErrorGuidance

**目的**: bunx未導入環境でのエラーメッセージとガイダンス

**定義箇所**: `src/claude.ts`, `src/codex.ts`（エラーハンドリング）

```typescript
interface ErrorGuidance {
  platform: 'win32' | 'darwin' | 'linux';
  errorCode: string;
  errorMessage: string;
  installInstructions: string[];
  troubleshootingTips: string[];
}
```

**フィールド説明**:

| フィールド | 型 | 説明 | 例 |
|-----------|------|------|------|
| `platform` | string | OS種別 | 'win32', 'darwin', 'linux' |
| `errorCode` | string | エラーコード | 'ENOENT' |
| `errorMessage` | string | エラーメッセージ | 'bunx command not found' |
| `installInstructions` | string[] | インストール手順 | ['curl -fsSL https://bun.sh/install \| bash'] |
| `troubleshootingTips` | string[] | トラブルシューティングヒント | ['ターミナルを再起動', 'PATH を確認'] |

**エラー検出と処理ロジック**:

```typescript
try {
  await execa('bunx', [packageName, ...args], context);
} catch (error: any) {
  const platform = platform();
  const errorCode = error.code;

  // エラーメッセージ生成
  const errorMessage = errorCode === 'ENOENT'
    ? 'bunx command not found. Please ensure Bun is installed.'
    : `Failed to launch: ${error.message || 'Unknown error'}`;

  // Windows固有のガイダンス
  if (platform === 'win32') {
    console.error('💡 Windows troubleshooting tips:');
    console.error('   1. Bun がインストールされ bunx が利用可能か確認');
    console.error('   2. "bunx <package>@latest -- --help" で動作確認');
    console.error('   3. ターミナルを再起動して PATH を更新');
    console.error('   4. PowerShell実行ポリシーを確認: Get-ExecutionPolicy');
  }

  throw new Error(errorMessage, error);
}
```

**プラットフォーム別インストール手順**:

| プラットフォーム | インストールコマンド |
|----------------|-------------------|
| macOS / Linux | `curl -fsSL https://bun.sh/install \| bash` |
| Windows | `powershell -c "irm bun.sh/install.ps1\|iex"` |

---

### 4. AIToolDescriptor

**目的**: UI表示用のAIツール情報

**定義箇所**: `src/ui/prompts.ts`（AIツール選択メニュー）

```typescript
interface AIToolDescriptor {
  name: string;           // ツール名
  displayName: string;    // 表示名
  packageName: string;    // bunxパッケージ名
  description: string;    // 説明文
  command: string;        // 起動コマンド例
}
```

**フィールド説明**:

| フィールド | 型 | Claude Code例 | Codex CLI例 |
|-----------|------|--------------|------------|
| `name` | string | 'claude' | 'codex' |
| `displayName` | string | 'Claude Code' | 'Codex CLI' |
| `packageName` | string | '@anthropic-ai/claude-code@latest' | '@openai/codex@latest' |
| `description` | string | 'Anthropic Claude AI' | 'OpenAI Codex' |
| `command` | string | 'bunx @anthropic-ai/claude-code@latest' | 'bunx @openai/codex@latest' |

**UI表示での使用**:

```typescript
const aiTools: AIToolDescriptor[] = [
  {
    name: 'claude',
    displayName: 'Claude Code',
    packageName: '@anthropic-ai/claude-code@latest',
    description: 'Anthropic Claude AI (bunx @anthropic-ai/claude-code@latest)',
    command: 'bunx @anthropic-ai/claude-code@latest'
  },
  {
    name: 'codex',
    displayName: 'Codex CLI',
    packageName: '@openai/codex@latest',
    description: 'OpenAI Codex (bunx @openai/codex@latest)',
    command: 'bunx @openai/codex@latest'
  }
];
```

---

## エンティティ間の関係

### 関係図

```text
LaunchOptions
    ↓ (入力)
BunxExecutionContext
    ↓ (実行)
execa('bunx', [packageName, ...args], context)
    ↓ (エラー時)
ErrorGuidance
    ↓ (UI表示)
AIToolDescriptor
```

### データフロー

#### 正常系フロー

```text
1. User selects AI tool
    ↓
2. LaunchOptions 生成
    mode: 'normal' | 'continue' | 'resume'
    extraArgs: []
    ↓
3. bunxコマンド構築
    command: 'bunx'
    args: [packageName, ...modeArgs, ...extraArgs]
    ↓
4. BunxExecutionContext 設定
    cwd: worktreePath
    stdio: 'inherit'
    shell: true
    ↓
5. execa実行
    bunx @anthropic-ai/claude-code@latest [args]
    ↓
6. AIツール起動成功
```

#### エラーフロー（bunx未検出）

```text
1. execa実行
    bunx @anthropic-ai/claude-code@latest [args]
    ↓
2. エラー発生（ENOENT）
    error.code === 'ENOENT'
    ↓
3. ErrorGuidance 生成
    platform: process.platform()
    errorCode: 'ENOENT'
    errorMessage: 'bunx command not found'
    ↓
4. プラットフォーム検出
    if (platform === 'win32') → Windows固有ガイダンス
    else → 汎用ガイダンス
    ↓
5. エラーメッセージ表示
    - bunx未検出メッセージ
    - Bunインストール手順
    - トラブルシューティングヒント
```

---

## 変更前後の比較

### Claude Code起動コマンドの変更

| 項目 | 変更前（npx） | 変更後（bunx） |
|------|-------------|--------------|
| コマンド | `npx` | `bunx` |
| フラグ | `--yes` | なし |
| パッケージ | `@anthropic-ai/claude-code@latest` | 同じ |
| 引数パススルー | `...args` | 同じ |
| エラー検出 | `ENOENT` → 'npx command not found' | `ENOENT` → 'bunx command not found' |

**変更例**:
```typescript
// 変更前
await execa('npx', ['--yes', CLAUDE_CLI_PACKAGE, ...args], { ... });

// 変更後
await execa('bunx', [CLAUDE_CLI_PACKAGE, ...args], { ... });
```

### Codex CLI起動コマンド

| 項目 | 現状（bunx） | 変更 |
|------|------------|------|
| コマンド | `bunx` | 変更なし ✅ |
| パッケージ | `@openai/codex@latest` | 変更なし ✅ |
| エラー検出 | `ENOENT` → 'bunx command not found' | 変更なし ✅ |

---

## まとめ

この機能のデータモデルは以下の4つのランタイムエンティティで構成されます：

1. **LaunchOptions**: AIツール起動オプション
2. **BunxExecutionContext**: bunx実行コンテキスト
3. **ErrorGuidance**: エラーメッセージとガイダンス
4. **AIToolDescriptor**: UI表示用ツール情報

すべてのエンティティはTypeScript型定義として実装され、永続化は行いません。bunx移行により、npxの`--yes`フラグが不要になり、コマンド構造がシンプルになります。
