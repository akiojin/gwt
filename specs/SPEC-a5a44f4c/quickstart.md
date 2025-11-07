# Quickstart: Releaseテスト安定化（保護ブランチ＆スピナー）

## 1. 準備

```bash
# Worktree ルートで SPECIFY_FEATURE をセット
export SPECIFY_FEATURE=SPEC-a5a44f4c

# Plan テンプレートを初期化（済みの場合はスキップ可）
SPECIFY_FEATURE=$SPECIFY_FEATURE ./.specify/scripts/bash/setup-plan.sh --json
```

- 依存インストール: `bun install`
- ビルド前提: `bun run build`（`vitest` 実行時に自動ビルドされるが、CI と揃えるなら事前ビルド推奨）

## 2. 影響テストの単体実行

```bash
# Integration / Acceptance
bunx vitest run src/ui/__tests__/integration/navigation.test.tsx
bunx vitest run src/ui/__tests__/acceptance/navigation.acceptance.test.tsx

# Protected branch component test
bunx vitest run src/ui/__tests__/components/App.protected-branch.test.tsx

# Spinner unit test
bunx vitest run tests/unit/worktree-spinner.test.ts
```

- すべて PASS した後に `bun run test` を実行すると release コマンドと同じ網羅率になる。

## 3. よくあるトラブル

| 症状 | 原因 | 対処 |
| --- | --- | --- |
| `mockIsProtectedBranchName is not defined` | `vi.hoisted` が抜けている | ファイル冒頭に `const { mockIsProtectedBranchName } = vi.hoisted(() => ({ ... }));` を追加し、その変数を `vi.mock` で返す |
| `Failed to get repository root` | `getRepositoryRoot` スタブを設定していない | `const getRepositoryRootSpy = vi.spyOn(gitModule, 'getRepositoryRoot').mockResolvedValue('/repo');` を `beforeEach` に追加 |
| `Cannot redefine property: execa` | `vi.spyOn` で ESM 名前空間を書き換えようとしている | `vi.mock('execa', () => ({ execa: execaMock }));` 方式へ切り替える |

## 4. Agent Context Update

Plan/Quickstart を更新後、以下を実行し CLAUDE.md 等へ最新スタックを同期する:

```bash
SPECIFY_FEATURE=$SPECIFY_FEATURE ./.specify/scripts/bash/update-agent-context.sh claude
```

## 5. Pull Request チェック

- テスト: `bun run test`
- Lint: `bun run lint`
- 変更ファイル: `git status -sb`
- Conventional Commit: `fix: ensure release tests pass on protected branches`
