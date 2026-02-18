# タスクリスト: 設定画面のスクロールをタブ切り替えに変更

## Phase 1: テスト作成（TDD）

- [x] T001 [US1][US2] テストを更新: `<details>` セレクタをタブ操作に書き換え、タブ切り替えテスト・初期表示テストを追加 `gwt-gui/src/lib/components/SettingsPanel.test.ts`

## Phase 2: タブUI実装

- [x] T002 [US1][US2] `SettingsTabId` 型と `activeSettingsTab` state を追加し、タブバー HTML + `{#if}` 条件表示に置き換え `gwt-gui/src/lib/components/SettingsPanel.svelte`
- [x] T003 [US1][US3] 不要な `<details>` / divider 関連 CSS を削除し、`.settings-tabs` / `.settings-tab-btn` / `.settings-tab-content` スタイルを追加 `gwt-gui/src/lib/components/SettingsPanel.svelte`

## Phase 3: 検証

- [x] T004 [US1][US2][US3] vitest 実行で全テストがパスすることを確認 `gwt-gui/src/lib/components/SettingsPanel.test.ts`
