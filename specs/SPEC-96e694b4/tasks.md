# タスク: Codex CLI gpt-5.2-codex 対応

**仕様ID**: `SPEC-96e694b4`
**ポリシー**: CLAUDE.md の TDD ルールに基づき、必ず RED→GREEN→リグレッションチェックの順に進める。

## フェーズ1: RED

- [x] **T3001** `src/cli/ui/utils/modelOptions.test.ts` と `src/cli/ui/__tests__/components/ModelSelectorScreen.initial.test.tsx` の期待値を gpt-5.2-codex に更新する。
- [x] **T3002** Codex の既定モデルを参照しているテストデータを gpt-5.2-codex に更新する（`src/cli/ui/__tests__/components/screens/BranchQuickStartScreen.test.tsx`、`src/cli/ui/__tests__/utils/branchFormatter.test.ts`、`tests/unit/index.*.test.ts`）。
- [x] **T3003** `tests/unit/codex.test.ts` にデフォルトモデルの期待値を追加・更新する。

## フェーズ2: GREEN

- [x] **T3004** `src/cli/ui/utils/modelOptions.ts` に gpt-5.2-codex を追加し、Extra high を選択可能にする。
- [x] **T3005** `src/codex.ts` と `src/shared/aiToolConstants.ts` のデフォルトモデルを gpt-5.2-codex に更新する。

## フェーズ3: リグレッションチェック

- [x] **T3006** 関連ユニットテストを実行し、モデル更新に起因する失敗がないことを確認する。
- [x] **T3007** `.specify/scripts/bash/update-specs-index.sh` を実行して仕様一覧を更新する。
