# 実装計画: Releaseテスト安定化（保護ブランチ＆スピナー）

**仕様ID**: `SPEC-a5a44f4c` | **日付**: 2025-11-07 | **仕様書**: [specs/SPEC-a5a44f4c/spec.md](./spec.md)
**入力**: `specs/SPEC-a5a44f4c/spec.md` からの機能仕様

## 概要

release ワークフローを阻害している 3 つのテスト失敗を解消する。Vitest の hoist 仕様に合わせて `isProtectedBranchName` 系モックを `vi.hoisted` へ移行し、保護ブランチ切替テストでは Git 依存をスタブして `switchToProtectedBranch` 呼び出しを保証する。さらに `tests/unit/worktree-spinner.test.ts` では `execa` を一括モックしてスピナーの start/stop を検証できるようにする。すべての変更はテストコードに限定し、アプリ本体の挙動を変えない。

## 技術コンテキスト

**言語/バージョン**: TypeScript 5.8.x (ESM) / React 19 / Ink 6 / Bun 1.0+  
**主要な依存関係**: Vitest 2.1.x、happy-dom 20.0.8、@testing-library/react 16.3.0、execa 9.6.0  
**ストレージ**: N/A（テストはメモリ内モックのみ）  
**テスト**: Vitest + @testing-library/react + happy-dom、PassThrough を使った Node.js ストリーム  
**ターゲットプラットフォーム**: macOS/Linux CLI (Ink) + Bun runtime  
**プロジェクトタイプ**: 単一 CLI アプリ（モノリポ構造）  
**パフォーマンス目標**: 追加テストを含めても `bun run test` が < 3 分、単体テストは < 60 秒  
**制約**: 本体ロジックを変更しない、`vi.mock` の hoist 制約に従う、bun コマンドで実行可能  
**スケール/範囲**: 対象ファイルは `src/ui/__tests__/*` と `tests/unit/worktree-spinner.test.ts` に限定

**Language/Version**: TypeScript 5.8.x / React 19 / Ink 6 / Bun 1.0+  
**Primary Dependencies**: Vitest 2.1.x, happy-dom 20.0.8, @testing-library/react 16.3.0, execa 9.6.0  
**Storage**: N/A  
**Tests**: Vitest run via `bunx vitest` (happy-dom environment)  
**Project Type**: CLI (single-package)

## 原則チェック

- **シンプルさ最優先**: プロダクションコードは触らず、テストの最小限修正のみとする → ✅
- **Worktree運用**: 既存 worktree ブランチ (hotfix-auto-release) で完結し、新規ブランチを作らない → ✅
- **Spec Kit順守**: specification→plan→tasks→implement の順番で進める（現在 plan フェーズ）→ ✅
- **CLI体験の品質維持**: テストのみ修正し UI/UX へ副作用を与えない → ✅

ゲート結果: PASS（上記4原則を満たすため Phase 0 へ進行可）。

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-a5a44f4c/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── testing-contract.md
└── tasks.md (後続フェーズ)
```

### ソースコード（リポジトリルート）

```text
src/
└── ui/
    └── __tests__/
        ├── integration/navigation.test.tsx
        ├── acceptance/navigation.acceptance.test.tsx
        ├── components/App.protected-branch.test.tsx
        └── ...
tests/
└── unit/worktree-spinner.test.ts
```

## フェーズ0: 調査（技術スタック選定）

**目的**: Vitest の hoist 仕様、Git 依存の切り離し、execa のモック方針を整理する。

**出力**: `specs/SPEC-a5a44f4c/research.md`

### 調査ハイライト

1. **Vitest hoist 対策**: `vi.hoisted` を使って共有モックを定義するのが最もシンプル。`beforeEach` で `mockReset` すればテスト間汚染を防げる。
2. **Git 依存切り離し**: `getRepositoryRoot` だけを `vi.spyOn` でスタブすれば `switchToProtectedBranch` まで処理が進み、`execa('git', …)` を発火させずに済む。
3. **execa モック戦略**: 名前空間インポートに spy すると property 再定義エラーが出るため、`vi.mock('execa', () => ({ execa: execaMock }))` で ESM を完全に差し替える。

決定と根拠の詳細は [research.md](./research.md) を参照。

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: テストダブルの責務と実装手順を文書化し、開発者が素早く再現できるようにする。

**出力**: `data-model.md`, `quickstart.md`, `contracts/testing-contract.md`

### 1.1 データモデル

`data-model.md` では以下を定義:

- ProtectedBranchMock / RepoRootStub / ExecaMockProcess の属性とリセット手順
- Vitest フック (`beforeEach`, `afterEach`) での状態管理

### 1.2 クイックスタート

`quickstart.md` で以下を提供:

- `SPECIFY_FEATURE` の設定→`setup-plan.sh` 実行手順
- 影響テスト3本の単体実行コマンド
- テスト毎の期待ログ例とトラブルシューティング（`Cannot redefine property` など）

### 1.3 契約/インターフェース

`contracts/testing-contract.md` で以下を整理:

- Protected branch テストの入出力（`switchToProtectedBranch` 引数、`navigateTo` 副作用）
- Spinner テストの入出力（`startSpinner` メッセージ、`stopSpinner` 呼び出し順）

### Agent Context Update

フェーズ1完了後に `.specify/scripts/bash/update-agent-context.sh claude` を実行し、CLAUDE.md へ最新の言語/依存情報を反映する（Plan 適用後に実施予定）。

## フェーズ2: タスク生成

- `/speckit.tasks` でタスクリストを生成し、P1→P2→P3 の順で実装する。
- 主要タスク例: 「integration/acceptance テストの hoisted 化」「App.protected-branch の Git スタブ追加」「worktree-spinner の execa モック化」

## 実装戦略

- **P1**: Integration & Acceptance テストの `vi.hoisted` 化とモックリセット。
- **P2**: `App.protected-branch.test.tsx` で Git 依存をスタブし、`switchToProtectedBranch` 呼び出しをアサート。
- **P3**: Spinner テストで `execa` を完全モックし、`PassThrough` を使ってストリーム完了をシミュレート。
- すべての変更後に対象テスト + `bun run test --runTestsByPath <files>` で回帰確認。

## テスト戦略

- **ユニットテスト**: `tests/unit/worktree-spinner.test.ts` でスピナー挙動を検証。`vi.mock` を使ったエンドツーエンド的ユニット。
- **統合テスト**: `src/ui/__tests__/integration/navigation.test.tsx`、`src/ui/__tests__/acceptance/navigation.acceptance.test.tsx`。
- **コンポーネントテスト**: `src/ui/__tests__/components/App.protected-branch.test.tsx` で保護ブランチフローを確認。
- **回帰テスト**: `bun run test` をフル実行し release パイプラインと同等の信頼性を得る。

## リスクと緩和策

1. **Vitest 仕様変更リスク**: `vi.hoisted` API が将来変わる可能性。→ 緩和: コメントで公式ドキュメント参照リンクを残し、他のテストでも同じパターンを共有。
2. **スタブ漏れリスク**: `getRepositoryRoot` をスタブし忘れると再び `git` 実行で失敗。→ 緩和: `beforeEach` でデフォルト `mockResolvedValue('/repo')` を設定し、`afterEach` で `mockImplementation(original)` へ戻すユーティリティを設置。
3. **ストリームタイミングリスク**: Spinner テストで `setTimeout` が未完了のままアサーションに到達。→ 緩和: `await Promise.resolve()` で microtask を flush し、`stopSpinner` 呼び出しを待つ。

### 依存関係リスク

- **happy-dom 差異**: バージョン差異で DOM API が変わる可能性。→ 緩和: package.json の依存を変更せず既存 version に合わせる。

## 次のステップ

1. ✅ フェーズ0完了: `research.md` 作成（本ドキュメントと同期）
2. ✅ フェーズ1完了: `data-model.md` / `quickstart.md` / `contracts/testing-contract.md` の草案を作成
3. ⏭️ `/speckit.tasks` を実行してタスクを生成
4. ⏭️ `/speckit.implement` で実装を開始
