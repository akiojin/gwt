# タスクリスト: メニュー無反応の受信経路修復と可観測性強化

## Phase 1: セットアップ

- [x] T001 [US1] 仕様/計画を確定する `specs/SPEC-9c1e4d2f/spec.md`

## Phase 2: TDD (RED)

- [x] T002 [US1] `setupMenuActionListener` の失敗系テストを追加する `gwt-gui/src/lib/menuAction.test.ts`

## Phase 3: 実装 (GREEN)

- [x] T003 [US1] メニューリスナー初期化失敗を文脈付きで再送出する `gwt-gui/src/lib/menuAction.ts`
- [x] T004 [US1] Appでメニュー初期化失敗を可視化する `gwt-gui/src/App.svelte`
- [x] T005 [US2] Rustメニューイベント受信/送信ログを追加する `crates/gwt-tauri/src/app.rs`
- [x] T008 [US2] Event ACL を明示的に許可する `crates/gwt-tauri/capabilities/default.json`
- [x] T009 [US2] capabilities 設定のユニットテストを追加する `crates/gwt-tauri/src/app.rs`

## Phase 4: 検証

- [x] T006 [P] [共通] フロント対象テストを実行する `gwt-gui/src/lib/menuAction.test.ts`
- [x] T007 [P] [共通] Rust対象テストを実行する `crates/gwt-tauri/src/app.rs`
