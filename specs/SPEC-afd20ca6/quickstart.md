# Qwen CLI統合 - 開発者クイックスタートガイド

**仕様ID**: `SPEC-afd20ca6`
**最終更新**: 2025-11-19

## 前提条件

### 必須環境
- ✅ Bun インストール済み（`bun --version`）
- ✅ 仕様書（spec.md）承認済み
- ✅ 実装計画（plan.md）承認済み
- ✅ CLAUDE.md のTDD原則理解

### 開発哲学
> **TDD絶対遵守**: テスト（Red）→ ユーザー承認 → 実装（Green）→ リファクタリング

## TDDフロー

### Phase 1: Red（テスト失敗）

#### ステップ1: テストファイル作成
```bash
# tests/unit/qwen.test.tsを作成
# 内容: 全16テストケース（plan.mdのテスト設計参照）
```

**テスト構成** (plan.md 1.4節参照):
- 基本起動テスト: T001～T003
- モード別起動テスト: T004～T006
- 権限スキップテスト: T007～T008
- エラーハンドリングテスト: T009～T011
- 環境変数テスト: T012～T013
- ターミナル管理テスト: T014～T015

#### ステップ2: テスト実行（Red確認）
```bash
bun run test tests/unit/qwen.test.ts
# 期待: すべてのテストが失敗（qwen.ts未実装のため）
```

#### ステップ3: ユーザー承認
```text
✋ **ここで停止**: ユーザーにテストコードをレビュー依頼
- テストケースが仕様（spec.md）を満たしているか？
- テストケースが実装を強制していないか？（実装の詳細ではなく動作をテスト）
- 承認後、Phase 2へ進む
```

### Phase 2: Green（テスト合格）

#### ステップ4: 実装ファイル作成
```bash
# 1. src/qwen.ts を作成（Gemini実装を参考）
# 2. src/config/builtin-tools.ts に QWEN_CLI_TOOL 追加
# 3. src/index.ts に QwenError処理と分岐追加
```

**実装ガイドライン**:
- `src/gemini.ts` を95%コピー＆カスタマイズ
- カスタマイズ箇所:
  - パッケージ名: `@google/gemini-cli` → `@qwen-code/qwen-code`
  - コマンド名: `gemini` → `qwen`
  - デフォルト引数: `[]` → `["--checkpointing"]`
  - 権限スキップ: `"-y"` → `"--yolo"`
  - モード引数: すべて `[]`（Qwenに継続・再開オプションなし）

#### ステップ5: テスト実行（Green確認）
```bash
bun run test tests/unit/qwen.test.ts
# 期待: すべてのテストが合格
```

#### ステップ6: ビルド確認
```bash
bun run build
# 期待: エラーなし、dist/qwen.js生成
```

### Phase 3: Refactor（リファクタリング）

#### ステップ7: コード品質チェック
```bash
# 型チェック
bun run type-check

# リント
bun run lint src/qwen.ts

# フォーマット
bun run format src/qwen.ts tests/unit/qwen.test.ts
```

#### ステップ8: 全テスト実行
```bash
# 既存テストが壊れていないか確認
bun run test
# 期待: すべてのテスト（既存+新規）が合格
```

## ファイル別実装ガイド

### 1. tests/unit/qwen.test.ts

**参考ファイル**: `tests/unit/claude.test.ts`

**テンプレート構造**:
```typescript
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// モック設定
vi.mock("execa", () => ({ execa: vi.fn() }));
vi.mock("fs", () => ({ existsSync: vi.fn(() => true) }));
vi.mock("../../src/utils/terminal", () => ({
  getTerminalStreams: vi.fn(() => mockTerminalStreams),
  createChildStdio: vi.fn(() => mockChildStdio),
}));

import { launchQwenCLI } from "../../src/qwen.js";
import { execa } from "execa";

const mockExeca = execa as ReturnType<typeof vi.fn>;

describe("launchQwenCLI", () => {
  // T001～T015のテストケース
});
```

**重要ポイント**:
- モックは実装の詳細ではなく、インターフェースをテスト
- `expect(mockExeca).toHaveBeenCalledWith(...)` で引数検証
- エラーケースも網羅（ENOENTエラー、QwenError）

### 2. src/qwen.ts

**参考ファイル**: `src/gemini.ts`

**実装チェックリスト**:
- [ ] QwenError クラス定義（name = "QwenError"）
- [ ] launchQwenCLI 関数（async, 戻り値 Promise<void>）
- [ ] isQwenCommandAvailable 内部関数（which/where）
- [ ] isQwenCLIAvailable エクスポート関数（将来用）
- [ ] worktreePath 存在確認（existsSync）
- [ ] 引数構築ロジック（--checkpointing, --yolo）
- [ ] ローカル/bunx 分岐
- [ ] エラーハンドリング（QwenErrorでラップ）
- [ ] Windows トラブルシューティング
- [ ] ターミナル管理（exitRawMode, childStdio.cleanup）

**コピー元（Gemini）との差分**:
```diff
- const GEMINI_CLI_PACKAGE = "@google/gemini-cli@latest";
+ const QWEN_CLI_PACKAGE = "@qwen-code/qwen-code@latest";

- export class GeminiError extends Error {
+ export class QwenError extends Error {

- export async function launchGeminiCLI(
+ export async function launchQwenCLI(

- const args: string[] = [];
+ const args: string[] = ["--checkpointing"];

- if (options.skipPermissions) { args.push("-y"); }
+ if (options.skipPermissions) { args.push("--yolo"); }

- case "continue": args.push("-r", "latest"); break;
- case "resume": args.push("-r", "latest"); break;
+ case "continue": /* no args */ break;
+ case "resume": /* no args */ break;

- const hasLocalGemini = await isGeminiCommandAvailable();
+ const hasLocalQwen = await isQwenCommandAvailable();

- if (hasLocalGemini) { await execa("gemini", args, ...); }
+ if (hasLocalQwen) { await execa("qwen", args, ...); }

- await execa("bunx", [GEMINI_CLI_PACKAGE, ...args], ...);
+ await execa("bunx", [QWEN_CLI_PACKAGE, ...args], ...);
```

### 3. src/config/builtin-tools.ts

**変更内容**:
```typescript
// ファイル末尾に追加
export const QWEN_CLI_TOOL: CustomAITool = {
  id: "qwen-cli",
  displayName: "Qwen",
  type: "bunx",
  command: "@qwen-code/qwen-code@latest",
  defaultArgs: ["--checkpointing"],
  modeArgs: {
    normal: [],
    continue: [],
    resume: [],
  },
  permissionSkipArgs: ["--yolo"],
};

// BUILTIN_TOOLS配列に追加
export const BUILTIN_TOOLS: CustomAITool[] = [
  CLAUDE_CODE_TOOL,
  CODEX_CLI_TOOL,
  GEMINI_CLI_TOOL,
  QWEN_CLI_TOOL, // ← 追加
];
```

### 4. src/index.ts

**変更1: インポート追加** (ファイル冒頭)
```typescript
import { launchQwenCLI, QwenError } from "./qwen.js";
```

**変更2: isRecoverableError関数** (3箇所)
```typescript
// 箇所1: instanceof チェック
if (
  error instanceof GitError ||
  error instanceof WorktreeError ||
  error instanceof CodexError ||
  error instanceof GeminiError ||
  error instanceof QwenError || // ← 追加
  error instanceof DependencyInstallError
) {
  return true;
}

// 箇所2: error.name チェック
if (error instanceof Error) {
  return (
    error.name === "GitError" ||
    error.name === "WorktreeError" ||
    error.name === "CodexError" ||
    error.name === "GeminiError" ||
    error.name === "QwenError" || // ← 追加
    error.name === "DependencyInstallError"
  );
}

// 箇所3: name変数チェック
const name = (error as { name?: string }).name;
return (
  name === "GitError" ||
  name === "WorktreeError" ||
  name === "CodexError" ||
  name === "GeminiError" ||
  name === "QwenError" || // ← 追加
  name === "DependencyInstallError"
);
```

**変更3: handleAIToolWorkflow関数** (gemini-cli分岐の後)
```typescript
} else if (tool === "gemini-cli") {
  await launchGeminiCLI(worktreePath, {
    mode: mode === "resume" ? "resume" : mode === "continue" ? "continue" : "normal",
    skipPermissions,
    envOverrides: sharedEnv,
  });
} else if (tool === "qwen-cli") { // ← 追加開始
  await launchQwenCLI(worktreePath, {
    mode: mode === "resume" ? "resume" : mode === "continue" ? "continue" : "normal",
    skipPermissions,
    envOverrides: sharedEnv,
  });
} // ← 追加終了
else {
  // Custom tool
  printInfo(`Launching custom tool: ${toolConfig.displayName}`);
```

## デバッグ方法

### テスト単体実行
```bash
# 特定のテストスイートのみ実行
bun run test tests/unit/qwen.test.ts

# 特定のテストケースのみ実行（describeまたはitの名前でフィルタ）
bun run test tests/unit/qwen.test.ts -t "T001"

# watchモード（ファイル変更時に自動実行）
bun run test:watch tests/unit/qwen.test.ts
```

### モックのデバッグ
```typescript
// テスト内でモック呼び出し確認
console.log("execa calls:", mockExeca.mock.calls);

// モックがどの引数で呼ばれたか詳細表示
console.log("First call args:", mockExeca.mock.calls[0]);
```

### 実際のQwen CLI起動テスト（手動）
```bash
# ローカルに qwen がある場合
cd /path/to/worktree
qwen --checkpointing

# bunx経由
cd /path/to/worktree
bunx @qwen-code/qwen-code@latest --checkpointing
```

## コミット前チェックリスト

### 1. テスト
- [ ] `bun run test` - すべてのテスト合格
- [ ] `bun run test:coverage` - カバレッジ確認（qwen.ts 80%以上）

### 2. 品質
- [ ] `bun run type-check` - 型エラーなし
- [ ] `bun run lint` - リントエラーなし
- [ ] `bun run format` - フォーマット適用済み

### 3. ビルド
- [ ] `bun run build` - ビルド成功
- [ ] `dist/qwen.js` 生成確認

### 4. commitlint検証
```bash
# コミットメッセージ検証（コミット後）
bunx commitlint --from HEAD~1 --to HEAD

# 期待: ✔ No problems found
```

### 5. コミットメッセージ例
```
feat: Qwenをビルトインツールとして追加

Qwen CLIをビルトインAIツールとして統合。
- src/qwen.ts を新規作成（起動ロジック、エラーハンドリング）
- src/config/builtin-tools.ts に QWEN_CLI_TOOL 追加
- src/index.ts にQwenError処理と分岐ロジックを追加
- tests/unit/qwen.test.ts を追加（16テストケース）

主な特徴:
- --checkpointing フラグでセッション管理を有効化
- /chat コマンドで対話中にセッション保存・再開可能
- --yolo フラグで権限スキップモード対応

SPEC-afd20ca6

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
```

## トラブルシューティング

### Q1: テストが失敗する
```bash
# モックがリセットされているか確認
# beforeEach で vi.clearAllMocks() が呼ばれているか？

# モックの戻り値が正しく設定されているか確認
mockExeca.mockResolvedValue({ stdout: "", stderr: "" });
```

### Q2: 型エラーが出る
```bash
# execa のモック型が正しいか確認
const mockExeca = execa as ReturnType<typeof vi.fn>;

# tsconfig.json の設定確認
# - strict: true
# - esModuleInterop: true
```

### Q3: ビルドは成功するがテストが失敗
```bash
# dist/ をクリーンアップ
bun run clean
bun run build
bun run test
```

### Q4: commitlint エラー
```bash
# コミットメッセージの形式確認:
# - 最初の行: "type: subject" (subject 100文字以内)
# - type は feat|fix|docs|chore|test|refactor のいずれか
# - subject は小文字で開始（日本語OK）

# 例:
# ✓ feat: Qwenを追加
# ✗ Add Qwen (typeなし)
# ✗ feat:Qwenを追加 (コロンの後にスペースなし)
```

## 参考リソース

- **仕様書**: [spec.md](./spec.md)
- **実装計画**: [plan.md](./plan.md)
- **Gemini実装**: `src/gemini.ts` (最も類似)
- **Claude実装**: `src/claude.ts` (詳細なエラーハンドリング例)
- **Geminiテスト**: `tests/unit/gemini.test.ts` (作成予定時の参考)
- **Claudeテスト**: `tests/unit/claude.test.ts` (テストパターン参考)

## 次のステップ

✅ このquickstart.mdを読んだら、`/speckit.tasks` を実行してタスク分解（tasks.md）を生成します。

tasks.md承認後、TDDフローに従って実装を開始します。
