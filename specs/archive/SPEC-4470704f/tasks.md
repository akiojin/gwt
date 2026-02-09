---
description: "gwt GUI マルチウィンドウ + Native Windowメニュー 実装タスク"
---

# タスク: gwt GUI マルチウィンドウ + Native Windowメニュー

**入力**: `specs/archive/SPEC-4470704f/`（`spec.md`, `plan.md`）
**前提条件**: `specs/archive/SPEC-4470704f/spec.md`, `specs/archive/SPEC-4470704f/plan.md`
**テスト**: 本仕様は回帰防止のため Rust ユニットテストを含める

## フェーズ1: ユーザーストーリー1 - マルチウィンドウ（1プロジェクト=1ウィンドウ）(P1)

- [ ] **T001** [US1] `crates/gwt-tauri/src/state.rs` をウィンドウ単位の project 状態管理へ変更し、ユニットテストを追加
- [ ] **T002** [US1] `crates/gwt-tauri/src/commands/project.rs` をウィンドウ単位で `open_project` / `close_project` できるよう更新
- [ ] **T003** [US1] `crates/gwt-tauri/src/commands/terminal.rs` の project 解決をウィンドウ単位へ移行（他ウィンドウと干渉しない）
- [ ] **T004** [US1] `crates/gwt-tauri/src/commands/settings.rs` の project 解決をウィンドウ単位へ移行
- [ ] **T005** [US1] `crates/gwt-tauri/capabilities/default.json` を更新し、追加ウィンドウ（`project-*`）でも必要権限が有効になるようにする

## フェーズ2: ユーザーストーリー2 - Windowメニューでウィンドウ（プロジェクト）切替 (P1)

- [ ] **T101** [US2] `crates/gwt-tauri/src/menu.rs` を追加し、ネイティブメニュー（File/Edit/View/Window/Settings/Help）を構築できるようにする
- [ ] **T102** [US2] `crates/gwt-tauri/src/app.rs` にネイティブメニューのセットと `on_menu_event` 処理を追加
- [ ] **T103** [US2] `File > New Window` の実装（新規ウィンドウ生成、show+focus）
- [ ] **T104** [US2] `Window` メニューの動的生成（プロジェクトが開かれているウィンドウ一覧、同名の区別、選択で show+focus）
- [ ] **T105** [US2] メニュー再構築トリガ（プロジェクト open/close、フォーカス変化、ウィンドウ破棄）を追加

## フェーズ3: ユーザーストーリー3 - Nativeメニューバー統合（in-app menubar廃止）(P2)

- [ ] **T201** [US3] `gwt-gui/src/App.svelte` から `MenuBar` を削除し、`menu-action` イベントを listen して既存の処理へ接続
- [ ] **T202** [US3] `gwt-gui/src/lib/components/MenuBar.svelte` を削除（未参照化でも可だが、本仕様では削除を優先）

## フェーズ4: 統合と検証

- [ ] **T401** [統合] `cargo test -p gwt-tauri` を実行してテストを通す
- [ ] **T402** [統合] `cd gwt-gui && npm run check` を実行して静的チェックを通す
- [ ] **T403** [統合] 手動受け入れ（`File > New Window`、2プロジェクト同時、`Window` メニュー切替、hide復帰、干渉なし）を確認

