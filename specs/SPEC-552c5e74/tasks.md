# タスクリスト: Launch Agent のデフォルト設定保持（前回成功起動値）

## 依存関係

- US1 は単独で実装・検証可能（MVP）。
- US2 は US1 の保存基盤を前提に実装する。
- US3 は US1/US2 の完了後に回帰テストとして検証する。

## Phase 1: セットアップ

- [x] T001 [P] [US1] 仕様成果物を確定（spec/plan/tasks の整合確認） `specs/SPEC-552c5e74/spec.md`

## Phase 2: 基盤

- [x] T002 [US1] Launch defaults 永続化ユーティリティを追加 `gwt-gui/src/lib/agentLaunchDefaults.ts`
- [x] T003 [P] [US1] Launch defaults ユーティリティ単体テストを追加（TDD: RED） `gwt-gui/src/lib/agentLaunchDefaults.test.ts`

## Phase 3: ストーリー 1（成功起動時のデフォルト保持）

- [x] T004 [US1] AgentLaunchForm テストに「成功起動時のみ保存」を追加（TDD: RED） `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T005 [US1] AgentLaunchForm へ defaults 読込/適用ロジックを実装 `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T006 [US1] AgentLaunchForm へ「Launch 成功時のみ保存」ロジックを実装 `gwt-gui/src/lib/components/AgentLaunchForm.svelte`

## Phase 4: ストーリー 2（非成功操作は保存しない）

- [x] T007 [US2] Close/Launch失敗時にデフォルト更新されないテストを追加（TDD: RED） `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T008 [US2] 失敗・キャンセル経路で保存を行わないガードを実装 `gwt-gui/src/lib/components/AgentLaunchForm.svelte`

## Phase 5: ストーリー 3（不正データフォールバック）

- [x] T009 [US3] 不正保存値フォールバックのテストを追加（TDD: RED） `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T010 [US3] 無効 agent/runtime/version の補正ロジックを実装 `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T011 [US3] 保存対象外フィールド（New Branch 入力）が復元されないことをテスト追加 `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`

## Phase 6: 仕上げ・横断

- [x] T012 [P] [共通] Launch defaults 関連テストを実行して回帰確認 `gwt-gui/src/lib/agentLaunchDefaults.test.ts`
- [x] T013 [共通] AgentLaunchForm 既存テスト（GLM含む）を再実行して互換性確認 `gwt-gui/src/lib/components/AgentLaunchForm.glm.test.ts`
