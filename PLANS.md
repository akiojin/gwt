# PLANS

## Warning文言タイポ修正

- [x] T401: `src/index.ts` の自動インストール警告文を修正

## Issue #444: Worktree作成時のstaleディレクトリ自動回復

- [x] 仕様追記: `specs/SPEC-d2f4762a/spec.md` にUS6/FR/エッジケースを追加
- [x] 実装計画追記: `specs/SPEC-d2f4762a/plan.md` に自己回復方針とToDoを追加
- [x] タスク追記: `specs/SPEC-d2f4762a/tasks.md` にTDD/実装タスクを追加
- [ ] T301: `tests/integration/branch-creation.test.ts` でstaleディレクトリ削除→再作成のテストを追加
- [ ] T302: `tests/integration/branch-creation.test.ts` で判定不能ディレクトリは削除せずエラーになるテストを追加
- [ ] T303: `src/worktree.ts` にstale判定/削除の前処理を追加
- [ ] T304: `src/worktree.ts` に判定不能時のエラーメッセージを追加
- [ ] T201: 既存の統合チェック（format/markdownlint/lint）を実行
