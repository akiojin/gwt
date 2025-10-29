# 調査レポート: カスタムAIツール対応機能

**日付**: 2025-10-28
**仕様ID**: SPEC-30f6d724

## 調査目的

既存のコードベースを分析し、カスタムAIツール対応機能を実装するための技術的決定を行う。

## 1. 既存コードベース分析

### 1.1 設定管理パターン (`src/config/index.ts`)

**現在の実装**:

- **設定ファイルパス優先順位**:
  1. `./claude-worktree.json` (プロジェクトローカル)
  2. `~/.config/claude-worktree/config.json` (推奨)
  3. `~/.claude-worktree.json` (レガシー)

- **設定読み込みパターン**:
  ```typescript
  // 複数パスを試行し、最初に見つかったものを使用
  for (const configPath of configPaths) {
    try {
      const content = await readFile(configPath, "utf-8");
      const userConfig = JSON.parse(content);
      return { ...DEFAULT_CONFIG, ...userConfig };
    } catch (error) {
      // 次のパスを試す
    }
  }
  ```

- **環境変数による上書き**: 設定ファイルがない場合、環境変数からも読み込み

**適用可能なパターン**:
- カスタムツール設定も同様の優先順位パターンを使用
- ただし、ツール設定は専用ファイル `~/.claude-worktree/tools.json` を使用（設定を分離）

### 1.2 AIツール起動ロジック (`src/claude.ts`, `src/codex.ts`)

**Claude Code実装パターン**:

```typescript
export async function launchClaudeCode(
  worktreePath: string,
  options: {
    mode?: "normal" | "continue" | "resume";
    skipPermissions?: boolean;
    extraArgs?: string[];
  } = {}
): Promise<void> {
  const args: string[] = [];

  // モード別引数
  switch (options.mode) {
    case "continue": args.push("-c"); break;
    case "resume": args.push("-r"); break;
  }

  // 権限スキップ
  if (options.skipPermissions) {
    args.push("--yes");
  }

  // 追加引数
  if (options.extraArgs) {
    args.push(...options.extraArgs);
  }

  // bunx経由で実行
  await execa("bunx", ["@anthropic-ai/claude-code@latest", ...args], {
    cwd: worktreePath,
    stdio: "inherit",
  });
}
```

**Codex CLI実装パターン**:

```typescript
// デフォルト引数を常に付与
const DEFAULT_CODEX_ARGS = ["--auto-approve", "--verbose"];

export async function launchCodexCLI(
  worktreePath: string,
  options: { /* ... */ } = {}
): Promise<void> {
  const args: string[] = [...DEFAULT_CODEX_ARGS];

  // モード別引数（サブコマンド形式）
  switch (options.mode) {
    case "continue": args.push("resume", "--last"); break;
    case "resume": args.push("resume"); break;
  }

  // bunx経由で実行
  await execa("bunx", ["@openai/codex@latest", ...args], {
    cwd: worktreePath,
    stdio: "inherit",
  });
}
```

**共通パターンの抽出**:
1. 引数配列の構築（デフォルト + モード + 権限 + 追加）
2. execaでbunx実行
3. stdio: "inherit"で標準入出力を継承

**適用可能な設計**:
- 汎用的な `launchCustomAITool()` 関数を作成
- 引数結合ロジックを共通化
- 実行タイプ（path/bunx/command）別の分岐

### 1.3 UIコンポーネント構造 (`src/ui/components/screens/AIToolSelectorScreen.tsx`)

**現在の実装**:

```typescript
export type AITool = 'claude-code' | 'codex-cli';

const toolItems: AIToolItem[] = [
  {
    label: 'Claude Code',
    value: 'claude-code',
    description: 'Official Claude CLI tool',
  },
  {
    label: 'Codex CLI',
    value: 'codex-cli',
    description: 'Alternative AI coding assistant',
  },
];
```

**問題点**: ハードコードされたツールリスト

**適用可能な設計**:
- `getAllTools()` 関数からツールリストを動的取得
- `AITool` 型をstring型に変更（カスタムIDに対応）
- ビルトインツールもCustomAITool形式で定義

### 1.4 セッション管理 (`SessionData`)

**現在の実装**:

```typescript
export interface SessionData {
  lastWorktreePath: string | null;
  lastBranch: string | null;
  timestamp: number;
  repositoryRoot: string;
}
```

**適用可能な設計**:
- オプショナルフィールド `lastUsedTool?: string` を追加
- セッションファイルパス: `~/.config/claude-worktree/sessions/{repo}_hash.json`
- 24時間の有効期限は維持

### 1.5 プロセス実行パターン (execa)

**使用例**:

```typescript
await execa("bunx", ["package@latest", ...args], {
  cwd: worktreePath,
  stdio: "inherit",
  env: { /* カスタム環境変数 */ }
});
```

**適用可能な設計**:
- `env` オプションでカスタム環境変数を設定
- `stdio: "inherit"` で標準入出力を継承（ユーザーとの対話を保つ）

## 2. 技術的決定

### 2.1 設定ファイル形式

**決定**: JSON

**理由**:
- 既存の`config.json`と統一
- TypeScriptの型チェックが容易
- ユーザーにとって馴染み深い

**代替案**:
- YAML: より読みやすいが、追加の依存関係が必要
- TOML: bunエコシステムで人気だが、学習コストがある

### 2.2 設定ファイルパス

**決定**: `~/.claude-worktree/tools.json`

**理由**:
- 既存の `~/.claude-worktree.json` と同じディレクトリ（ユーザーの混乱を防ぐ）
- `~/.config/claude-worktree/config.json` とは分離（役割の明確化）
- ツール設定は独立して管理すべき

**代替案**:
- `~/.config/claude-worktree/tools.json`: より標準的だが、ユーザーが見つけにくい
- `~/.claude-worktree/tools.json`: 新しいディレクトリだが、既存パターンと異なる

### 2.3 型定義とバリデーション

**決定**: TypeScriptインターフェース + 手動検証

**理由**:
- 依存関係の最小化（Zod不使用）
- シンプルな検証ロジック（必須フィールド、enum値、重複チェック）
- 十分な型安全性

**代替案**:
- Zod: より強力な検証だが、追加の依存関係
- JSON Schema: 標準的だが、実行時検証のオーバーヘッド

**検証項目**:
1. 必須フィールド存在チェック（id, displayName, type, command, modeArgs）
2. type値検証（'path' | 'bunx' | 'command'のいずれか）
3. id重複チェック
4. commandの存在確認（type='path'の場合）

### 2.4 コマンド解決方法

**決定**: which/whereコマンド経由

**理由**:
- セキュリティ（PATH環境変数から安全に解決）
- クロスプラットフォーム対応（whichはUnix/Linux、whereはWindows）
- 実行前に存在確認できる

**実装例**:
```typescript
async function resolveCommand(commandName: string): Promise<string> {
  const whichCommand = process.platform === 'win32' ? 'where' : 'which';
  const { stdout } = await execa(whichCommand, [commandName]);
  return stdout.trim().split('\n')[0]; // 最初のパスを使用
}
```

**代替案**:
- 直接PATH検索: セキュリティリスク
- ハードコードパス: 柔軟性が低い

### 2.5 ビルトインツールとの統合

**決定**: ビルトインツールもCustomAITool形式で定義

**理由**:
- コードの一貫性
- `getAllTools()` で統一的に扱える
- 将来的にビルトインツールの設定をカスタマイズ可能

**実装**:
```typescript
const BUILTIN_TOOLS: CustomAITool[] = [
  {
    id: 'claude-code',
    displayName: 'Claude Code',
    type: 'bunx',
    command: '@anthropic-ai/claude-code@latest',
    modeArgs: {
      normal: [],
      continue: ['-c'],
      resume: ['-r'],
    },
    permissionSkipArgs: ['--yes'],
  },
  {
    id: 'codex-cli',
    displayName: 'Codex CLI',
    type: 'bunx',
    command: '@openai/codex@latest',
    defaultArgs: ['--auto-approve', '--verbose'],
    modeArgs: {
      normal: [],
      continue: ['resume', '--last'],
      resume: ['resume'],
    },
  },
];
```

## 3. 制約と依存関係

### 3.1 技術的制約

1. **Bun実行環境**: Bun 1.0+ が必須
2. **execa v9.6.0**: 既存の依存関係、変更なし
3. **ファイルシステムAPI**: Node.js `fs/promises` 使用
4. **プロセス起動**: bunx経由での実行が推奨

### 3.2 互換性制約

1. **既存ツールとの後方互換性**: 100%維持
2. **SessionData拡張**: オプショナルフィールドで後方互換
3. **UI変更**: 既存のキーバインドやレイアウトを維持

### 3.3 セキュリティ制約

1. **パス検証**: type='path'の場合、絶対パスのみ許可
2. **コマンド解決**: which/where経由で安全に解決
3. **環境変数**: ログ出力時にマスキング（機密情報保護）

## 4. 実装方針

### 4.1 新規ファイル

1. **`src/config/tools.ts`**: カスタムツール設定管理
   - `loadToolsConfig()`: 設定読み込み
   - `validateToolConfig()`: 検証
   - `getToolById()`: ID検索
   - `getAllTools()`: ビルトイン + カスタムの統合

2. **`src/launcher.ts`**: 汎用AIツール起動
   - `launchCustomAITool()`: type別実行
   - `buildArgs()`: 引数結合ロジック
   - `resolveCommand()`: コマンド解決

### 4.2 既存ファイル変更

1. **`src/config/index.ts`**:
   - `SessionData`に`lastUsedTool?: string`追加

2. **`src/claude.ts`, `src/codex.ts`**:
   - `launchCustomAITool()`を内部で使用（リファクタリング）

3. **`src/ui/components/screens/AIToolSelectorScreen.tsx`**:
   - `getAllTools()`からツールリスト取得
   - `AITool`型を`string`に変更

4. **`src/index.ts`**:
   - `handleAIToolWorkflow()`をカスタムツール対応に拡張

## 5. 次のステップ

1. ✅ 調査完了
2. ⏭️ Phase 1: `data-model.md`, `quickstart.md`, `contracts/types.ts` 作成
3. ⏭️ `/speckit.tasks` でタスク生成
4. ⏭️ TDD実施（テスト作成 → 実装）
