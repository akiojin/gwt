# 実装計画: Qwen CLIビルトインツール統合

**仕様ID**: `SPEC-afd20ca6` | **日付**: 2025-11-19 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-afd20ca6/spec.md` からの機能仕様

## 概要

既存のビルトインツール機構を活用し、Qwen CLIを第4のビルトインAIツールとして統合する。Claude Code、Codex、Geminiと同じパターンに従い、一貫性のあるユーザー体験を提供する。Qwen CLI固有の特性（--checkpointingによるセッション管理、起動時の継続・再開オプションなし）に対応する。

**主要な技術的アプローチ**:
- 既存パターンの踏襲: `src/gemini.ts` を参考に `src/qwen.ts` を実装
- ビルトインツール定義の拡張: `BUILTIN_TOOLS` 配列に `QWEN_CLI_TOOL` 追加
- エラーハンドリングの統合: `QwenError` クラスと `isRecoverableError` への追加
- テスト駆動開発: `tests/unit/qwen.test.ts` を先に作成（TDD）

## 技術コンテキスト

**言語/バージョン**: TypeScript 5.x（ES Module形式）
**主要な依存関係**:
- execa: プロセス実行
- chalk: ターミナル出力の色付け
- Ink: TUIコンポーネント（既存UIとの統合）
**ストレージ**: N/A（セッション管理はQwen CLI自体が~/.qwenに保存）
**テスト**: vitest（モックベース）、既存パターン（vi.mock, describe/it/expect）
**ターゲットプラットフォーム**: Node.js 18+、Linux/macOS/Windows
**プロジェクトタイプ**: 単一プロジェクト（CLIツール）
**パフォーマンス目標**: 起動時間±2秒以内（Claude Code、Gemini と同等）
**制約**:
- bunx実行環境が必要
- Qwen CLIパッケージ（@qwen-code/qwen-code）の可用性
- 既存のビルトインツール機構への非破壊的追加
**スケール/範囲**:
- 機能: 3つのユーザーストーリー（P1～P3）
- ファイル: 4ファイル追加/変更（qwen.ts, builtin-tools.ts, index.ts, qwen.test.ts）
- テスト: 10件以上のテストケース

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

### CLAUDE.md準拠チェック

#### ✅ シンプルさの極限追求
- **評価**: 合格
- **根拠**: 既存パターンの踏襲により複雑性を最小化。新規設計なし、Gemini実装の95%を再利用
- **実装方針**: src/gemini.ts を参考にコピー＆カスタマイズ（DRYより一貫性優先）

#### ✅ ユーザビリティと開発者体験の品質
- **評価**: 合格
- **根拠**: Claude Code、Gemini と同一のUX。選択→起動の流れは変更なし
- **実装方針**: displayName "Qwen" のみ変更、他のUI要素は既存のまま

#### ✅ Spec Kit TDD絶対遵守
- **評価**: 合格
- **根拠**: このplan.md作成時点で仕様（spec.md）承認済み。次フェーズでテスト先行
- **実装方針**:
  1. Phase 2でtasks.md生成
  2. tests/unit/qwen.test.ts作成（Red）
  3. ユーザー承認
  4. src/qwen.ts実装（Green）
  5. リファクタリング（必要に応じて）

#### ✅ 既存ファイル優先メンテナンス
- **評価**: 合格
- **根拠**:
  - 新規: src/qwen.ts, tests/unit/qwen.test.ts（必須）
  - 既存変更: src/config/builtin-tools.ts, src/index.ts（最小限の追加）
- **実装方針**: 既存ファイルへの変更は追加のみ（削除・大規模変更なし）

#### ✅ Conventional Commits遵守
- **評価**: 合格（実装後に検証）
- **実装方針**:
  - コミット前に `bunx commitlint --from HEAD~1 --to HEAD` 実行
  - `feat: Qwenをビルトインツールとして追加` 形式
  - 1コミット1タスク原則

**ゲート判定**: ✅ **合格** - すべての原則を満たしており、フェーズ0へ進行可能

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-afd20ca6/ または specs/feature/support-qwen/
├── spec.md              # 機能仕様（完了）
├── plan.md              # このファイル（/speckit.plan 出力）
├── research.md          # フェーズ0出力（次のステップ）
├── data-model.md        # フェーズ1出力（N/A - データモデルなし）
├── quickstart.md        # フェーズ1出力（開発者ガイド）
├── contracts/           # フェーズ1出力（N/A - API契約なし）
├── checklists/          # 品質チェックリスト
│   └── requirements.md  # 仕様品質チェックリスト（完了）
└── tasks.md             # フェーズ2出力（/speckit.tasks）
```

### ソースコード（リポジトリルート）

```text
src/
├── qwen.ts                      # 新規: Qwen CLI起動ロジック
├── config/
│   └── builtin-tools.ts         # 変更: QWEN_CLI_TOOL追加
├── index.ts                     # 変更: QwenError処理、分岐追加
├── claude.ts                    # 参考: 実装パターン
└── gemini.ts                    # 参考: 最も類似

tests/unit/
├── qwen.test.ts                 # 新規: Qwen起動テスト
├── claude.test.ts               # 参考: テストパターン
└── codex.test.ts                # 参考: モックパターン
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 既存のコードベースパターンを理解し、Qwen CLI固有の実装詳細を確認する

**出力**: `specs/feature/support-qwen/research.md`

### 調査項目

#### 1. 既存のコードベース分析

**現在の技術スタック**:
- 言語: TypeScript（ES Module、strict mode）
- ランタイム: Bun（packageManager: "bun@latest"）
- ビルド: tsc（TypeScript Compiler）
- テスト: vitest（vi.mock, describe/it/expect）
- プロセス実行: execa
- ターミナル出力: chalk
- UI: Ink（Reactベースのターミナル UI）

**既存のパターンとアーキテクチャ**:
- ビルトインツール定義: `src/config/builtin-tools.ts` の `CustomAITool` 型
- 起動関数: `async function launchXXX(worktreePath, options): Promise<void>`
- エラークラス: `class XXXError extends Error { name = "XXXError" }`
- テストモック: `vi.mock("execa")`, `vi.mock("fs")`
- 引数構築パターン: `args.push(...)` + `switch (options.mode)`

**統合ポイント**:
- `src/config/builtin-tools.ts`: `BUILTIN_TOOLS` 配列にツール定義追加
- `src/index.ts`: `handleAIToolWorkflow` 関数に分岐追加
- `src/index.ts`: `isRecoverableError` 関数にエラー追加

#### 2. 技術的決定

**決定1: 実装パターンの選択**
- **選択**: Gemini CLI実装（src/gemini.ts）を参考パターンとする
- **理由**:
  - Gemini も bunx 経由実行（Qwen と同じ）
  - ローカルコマンド検出パターンあり（`isGeminiCommandAvailable`）
  - セッション管理フラグあり（`-r latest`）
  - 最新の実装（2025-11時点）
- **代替案**: Claude Code パターン（より複雑、IS_SANDBOX対応など不要）

**決定2: テストモックの範囲**
- **選択**: execa、fs、utils/terminal のモック（既存パターン踏襲）
- **理由**:
  - 実際のQwen CLI実行なしでテスト可能
  - CI/CD環境での安定性
  - 既存テスト（claude.test.ts, codex.test.ts）と一貫性
- **代替案**: 実際のQwen CLI実行（遅い、外部依存、不安定）

**決定3: モード引数の扱い**
- **選択**: normal/continue/resume すべて空配列 `[]`
- **理由**: Qwen CLIには起動時の継続・再開オプションが存在しない（公式ドキュメント確認済み）
- **代替案**: `-i --prompt-interactive`（調査の結果、セッション再開ではなく対話継続用と判明）

#### 3. 制約と依存関係

**制約1: Qwen CLI公式仕様への依存**
- Qwen CLI（@qwen-code/qwen-code）のバージョン変更により動作が変わる可能性
- `--checkpointing` フラグの仕様が変わる可能性（低い）
- 緩和策: `--checkpointing` が無効でも基本機能（起動）は動作する設計

**制約2: bunx実行環境の必須性**
- ローカルに qwen コマンドがない場合、bunx が必須
- bunx がない環境では動作不可（既存のClaude Code、Codexと同じ）
- 緩和策: エラーメッセージで bunx インストールを案内

**制約3: 既存のビルトインツール機構の不変性**
- `CustomAITool` 型定義の変更は他のツールに影響
- `BUILTIN_TOOLS` の順序変更は既存ユーザーのUIに影響
- 緩和策: 既存ツールの後に追加（最小限の変更）

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: Qwen CLI統合の詳細設計を定義する

**出力**:
- `specs/feature/support-qwen/quickstart.md`（開発者ガイド）
- data-model.md: N/A（データモデルなし）
- contracts/: N/A（外部API契約なし）

### 1.1 データモデル設計

**該当なし**: この機能はデータ永続化を含まない。Qwen CLIのセッション管理は ~/.qwen/ に保存されるが、gwtは関与しない。

### 1.2 API契約設計

**該当なし**: この機能は外部APIを公開しない。内部関数（`launchQwenCLI`）は既存パターンと同じシグネチャ。

### 1.3 コンポーネント設計

#### src/qwen.ts

**エクスポート**:
- `export class QwenError extends Error`
- `export async function launchQwenCLI(worktreePath: string, options): Promise<void>`
- `export async function isQwenCLIAvailable(): Promise<boolean>`（将来の拡張用）

**内部関数**:
- `async function isQwenCommandAvailable(): Promise<boolean>` - ローカルqwenコマンド検出

**ロジックフロー**:
```
1. worktreePath存在確認（existsSync）
2. 起動メッセージ表示（chalk.blue）
3. 引数配列構築:
   - デフォルト: ["--checkpointing"]
   - skipPermissions時: ["--yolo"] 追加
   - extraArgs追加
4. ローカルqwenコマンド検出（which/where）
5. 分岐:
   - ローカルあり: execa("qwen", args, ...)
   - ローカルなし: execa("bunx", ["@qwen-code/qwen-code@latest", ...args], ...)
6. エラー時: QwenErrorでラップ、Windowsならトラブルシューティング表示
```

#### src/config/builtin-tools.ts

**追加内容**:
```typescript
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

// BUILTIN_TOOLSに追加
export const BUILTIN_TOOLS: CustomAITool[] = [
  CLAUDE_CODE_TOOL,
  CODEX_CLI_TOOL,
  GEMINI_CLI_TOOL,
  QWEN_CLI_TOOL, // 追加
];
```

#### src/index.ts

**変更箇所1: インポート**
```typescript
import { launchQwenCLI, QwenError } from "./qwen.js";
```

**変更箇所2: isRecoverableError関数**
```typescript
// 3箇所にQwenError追加
error instanceof QwenError ||
error.name === "QwenError" ||
name === "QwenError" ||
```

**変更箇所3: handleAIToolWorkflow関数**
```typescript
} else if (tool === "qwen-cli") {
  await launchQwenCLI(worktreePath, {
    mode: mode === "resume" ? "resume" : mode === "continue" ? "continue" : "normal",
    skipPermissions,
    envOverrides: sharedEnv,
  });
} else {
```

### 1.4 テスト設計

#### tests/unit/qwen.test.ts

**テストスイート構成** (claude.test.ts参考):

1. **基本起動テスト**
   - T001: 正常起動（bunx経由）
   - T002: ローカルqwenコマンド使用
   - T003: worktreeパス不在でエラー

2. **モード別起動テスト**
   - T004: normalモード（引数: ["--checkpointing"]）
   - T005: continueモード（引数同じ）
   - T006: resumeモード（引数同じ）

3. **権限スキップテスト**
   - T007: skipPermissions=true で --yolo 付与
   - T008: skipPermissions=false で --yolo なし

4. **エラーハンドリングテスト**
   - T009: bunx不在でENOENTエラー
   - T010: QwenError発生とcause保持
   - T011: Windowsプラットフォームでトラブルシューティング表示

5. **環境変数テスト**
   - T012: envOverridesのマージ
   - T013: extraArgsの追加

6. **ターミナル管理テスト**
   - T014: exitRawMode呼び出し
   - T015: childStdio.cleanup呼び出し

**モック設定** (既存パターン):
```typescript
vi.mock("execa", () => ({ execa: vi.fn() }));
vi.mock("fs", () => ({ existsSync: vi.fn(() => true) }));
vi.mock("../../src/utils/terminal", () => ({
  getTerminalStreams: vi.fn(() => mockTerminalStreams),
  createChildStdio: vi.fn(() => mockChildStdio),
}));
```

### 1.5 quickstart.md（開発者ガイド）

**内容**:
1. 前提条件（bunインストール、仕様承認、テスト先行）
2. TDDフロー（Red → Green → Refactor）
3. ファイル別実装手順
4. テスト実行方法（`bun run test tests/unit/qwen.test.ts`）
5. デバッグ方法
6. コミット前チェックリスト

## フェーズ2: タスク分解

**注**: このフェーズは `/speckit.tasks` コマンドで実行されます。`/speckit.plan` では `tasks.md` を作成しません。

**Phase 2の出力**: `specs/feature/support-qwen/tasks.md`

## 原則チェック（再評価）

*フェーズ1設計完了後の再チェック*

#### ✅ シンプルさの極限追求（再評価）
- **評価**: 合格
- **設計後の確認**:
  - 新規ファイル2件（qwen.ts, qwen.test.ts）
  - 既存ファイル変更2件（最小限の追加のみ）
  - 複雑な分岐なし、Geminiパターンの95%再利用
- **結論**: 設計はシンプルさの原則を維持

#### ✅ TDD絶対遵守（再評価）
- **評価**: 合格
- **設計後の確認**:
  - テスト設計完了（16件のテストケース定義済み）
  - 実装前にテストコード作成の手順明確化（quickstart.md）
  - Red-Green-Refactorサイクルを quickstart.md で義務化
- **結論**: TDD原則を満たす設計

**最終ゲート判定**: ✅ **合格** - フェーズ2（タスク分解）へ進行可能

## 次のステップ

1. `/speckit.tasks` コマンド実行 → `tasks.md` 生成
2. `tasks.md` 承認後、TDDフローに従って実装：
   - tests/unit/qwen.test.ts 作成（Red）
   - ユーザー承認
   - src/qwen.ts 実装（Green）
   - リファクタリング
3. ビルド確認（`bun run build`）
4. テスト実行（`bun run test`）
5. Conventional Commitsでコミット
6. プッシュ＆PR作成

---

**ドキュメント作成日**: 2025-11-19
**最終更新日**: 2025-11-19
**ステータス**: フェーズ1完了、フェーズ2待ち
