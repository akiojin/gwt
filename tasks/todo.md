# TODO: Issue #1288 From Issue Branch Name Display

## 背景
Issue #1288「From Issue からブランチ作成時にブランチ名の表示が不自然（prefix が重複して見える）」を、`SPEC-1288` と TDD で修正する。

## 実装ステップ
- [x] `specs/SPEC-1288/spec.md`, `plan.md`, `tasks.md` を作成して要件を定義
- [x] `AgentLaunchForm.svelte` の From Issue 表示を suffix-only（`issue-<number>`）に修正
- [x] Launch 時の branch 名組み立てを full name（`{prefix}issue-{number}`）で固定
- [x] `AgentLaunchForm.test.ts` に表示と payload の回帰テストを追加
- [x] テスト/チェック実行（対象 test + svelte-check）

## 検証結果
- [x] `cd gwt-gui && pnpm test src/lib/components/AgentLaunchForm.test.ts`（pass: 39 tests）
- [x] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json`（0 errors / 1 warning: 既存 `MergeDialog.svelte`）
