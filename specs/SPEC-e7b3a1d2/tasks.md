# タスクリスト: ウィンドウ・タブ切り替えショートカット

## Phase 1: Accelerator 検証 + タブ切り替え

- [x] T001 [P] [US1] Tauri accelerator 検証（CmdOrCtrl+Shift+[/] / CmdOrCtrl+Backquote の動作確認） crates/gwt-tauri/src/menu.rs
- [x] T002 [US3] Window メニューに Previous Tab / Next Tab 項目追加（accelerator 付き） crates/gwt-tauri/src/menu.rs
- [x] T003 [US1] menu-action イベントに previous-tab / next-tab アクション追加 crates/gwt-tauri/src/app.rs
- [x] T004 [US1,US2] タブ切り替えユニットテスト作成（TDD: テストファースト — RED 確認後に実装） gwt-gui/src/lib/tabNavigation.test.ts
- [x] T005 [US1] タブ切り替えロジック実装（tabNavigation.ts に分離、表示順ベース、ラップなし、Summary 含む） gwt-gui/src/lib/tabNavigation.ts
- [x] T005a [US1] App.svelte の handleMenuAction に previous-tab / next-tab アクション統合 gwt-gui/src/App.svelte

## Phase 2: ウィンドウ切り替え + MRU 管理

- [x] T006 [US4] AppState に MRU リスト（window_focus_history）追加 crates/gwt-tauri/src/state.rs
- [x] T007 [US4] MRU リストのユニットテスト作成（TDD: テストファースト） crates/gwt-tauri/src/state.rs
- [x] T008 [US4] WindowEvent::Focused(true) で MRU リスト更新 crates/gwt-tauri/src/app.rs
- [x] T009 [US4,US5] Window メニューに Next Window / Previous Window 項目追加（accelerator 付き） crates/gwt-tauri/src/menu.rs
- [x] T010 [US4,US5] ウィンドウ切り替えロジック実装（MRU 順、非表示ウィンドウ show+focus 含む） crates/gwt-tauri/src/app.rs

## Phase 3: macOS 標準 Window メニュー項目

- [x] T011 [US6] macOS 向け Minimize (Cmd+M) / Zoom メニュー項目追加 crates/gwt-tauri/src/menu.rs
- [x] T012 [US6] Bring All to Front メニュー項目追加 crates/gwt-tauri/src/menu.rs
- [x] T013 [US6] Minimize / Zoom / Bring All to Front のアクションハンドリング crates/gwt-tauri/src/app.rs

## Phase 4: README 更新 + 仕上げ

- [x] T014 [US7] README.md にキーボードショートカット包括一覧セクション追加 README.md
- [x] T015 [US7] README.ja.md にキーボードショートカット包括一覧セクション追加 README.ja.md
- [x] T016 [P] [共通] Window メニュー構造の整合性検証（ナビゲーション → タブ一覧 → Minimize/Zoom → ウィンドウ一覧 → Bring All to Front）
