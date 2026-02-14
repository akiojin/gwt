# タスクリスト: 全操作時の断続フリーズ抑止（System Monitor 負荷制御）

## Phase 1: 仕様反映

- [x] T001 `spec.md` を更新し、全操作フリーズの原因仮説と受け入れ条件を定義する `specs/SPEC-923b33fc/spec.md`
- [x] T002 `plan.md` を更新し、TDD前提の実装方針を確定する `specs/SPEC-923b33fc/plan.md`

## Phase 2: TDD（RED → GREEN）

- [x] T010 [US1] [RED] `systemMonitor` のポーリング間隔（5秒）検証テストを追加し、現状失敗を確認する `gwt-gui/src/lib/systemMonitor.svelte.test.ts`
- [x] T011 [US1] [RED] in-flight 重複呼び出し抑止テストを追加し、現状失敗を確認する `gwt-gui/src/lib/systemMonitor.svelte.test.ts`
- [x] T012 [US2] [RED] visibility 復帰時のウォームアップ再実行防止テストを追加し、現状失敗を確認する `gwt-gui/src/lib/systemMonitor.svelte.test.ts`
- [x] T013 [US1,US2] [GREEN] `createSystemMonitor` を 5秒間隔 + 単発タイマー + in-flight 抑止 + ウォームアップ1回化で実装する `gwt-gui/src/lib/systemMonitor.svelte.ts`
- [x] T014 [US1,US2] [GREEN] `pnpm -s vitest run src/lib/systemMonitor.svelte.test.ts` を通過させる `gwt-gui`

## Phase 3: 仕上げ

- [x] T020 [共通] 仕様・実装・テストの整合性を最終確認し、タスク完了状態を更新する `specs/SPEC-923b33fc/tasks.md`
