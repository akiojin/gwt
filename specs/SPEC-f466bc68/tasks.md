# タスクリスト: プロジェクトを開いたときに前回のエージェントタブを復元する

## Phase 1: 仕様・計画

- [x] T001 [共通] spec.md を作成・更新 `specs/SPEC-f466bc68/spec.md`
- [x] T002 [共通] plan.md を作成・更新 `specs/SPEC-f466bc68/plan.md`
- [x] T003 [共通] tasks.md を作成 `specs/SPEC-f466bc68/tasks.md`

## Phase 2: ストーリー 1（復元）

- [x] T004 [US1] 永続化/復元ユーティリティを追加 `gwt-gui/src/lib/agentTabsPersistence.ts`
- [x] T005 [US1] App にプロジェクト単位の保存/復元を実装 `gwt-gui/src/App.svelte`

## Phase 3: ストーリー 2（ターミナル初期表示）

- [x] T006 [US2] scrollback tail の先出し表示を追加 `gwt-gui/src/lib/terminal/TerminalView.svelte`

## Phase 4: テストと検証（TDD）

- [x] T007 [US1] 永続化/復元ロジックのテスト `gwt-gui/src/lib/agentTabsPersistence.test.ts`
- [x] T008 [US2] TerminalView のテスト `gwt-gui/src/lib/terminal/TerminalView.test.ts`
- [x] T009 [共通] `pnpm -C gwt-gui test` / `pnpm -C gwt-gui check` を実行

## Phase 5: 仕上げ・横断

- [x] T010 [共通] Spec Kit スクリプトの Bash 3.2 互換修正 `.specify/scripts/bash/common.sh`
