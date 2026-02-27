# タスクリスト: From Issue でブランチ名に prefix が二重表示される

## Phase 1: セットアップ

- [x] T001 [US1] 仕様書を作成し Issue #1288 の受け入れ条件を定義する `specs/SPEC-1288/spec.md`
- [x] T002 [US1] 実装計画とタスクを作成する `specs/SPEC-1288/plan.md`, `specs/SPEC-1288/tasks.md`

## Phase 2: ストーリー 1

- [x] T003 [US1] From Issue の branch 表示が suffix のみになるテストを追加する `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T004 [US1] `AgentLaunchForm.svelte` で表示を suffix-only にし launch 時に full branch name を組み立てる `gwt-gui/src/lib/components/AgentLaunchForm.svelte`

## Phase 3: 仕上げ・横断

- [x] T005 [共通] Frontend テストと型チェックを実行して回帰がないことを確認する `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
