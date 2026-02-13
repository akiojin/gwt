# タスクリスト: Aboutダイアログにバージョン表示 + タイトルにプロジェクトパス表示（GUI）

## 依存関係

- US1 と US2 は独立。共通の検証は最終フェーズで実施する

## Phase 1: セットアップ

- [x] T001 [P] [US1] [準備タスク] 仕様と受け入れ条件の確認 specs/SPEC-ab9b7d08/spec.md

## Phase 2: 基盤

- [x] T002 [US1] [基盤実装] タイトル更新の呼び出し経路を確認 gwt-gui/src/App.svelte

## Phase 3: ストーリー 1

- [x] T003 [US1] [テスト] タイトルフォーマットの期待値更新 gwt-gui/src/lib/windowTitle.test.ts
- [x] T004 [US1] [実装] タイトルフォーマットを `<projectPath>` に変更 gwt-gui/src/lib/windowTitle.ts
- [x] T005 [US1] [実装] Tauri のタイトル更新権限を有効化 crates/gwt-tauri/capabilities/default.json

## Phase 4: ストーリー 2

- [x] T006 [US2] [テスト] About バージョン表示のフォーマット検証 gwt-gui/src/lib/windowTitle.test.ts
- [x] T007 [US2] [実装] About ダイアログに Version 表示 gwt-gui/src/App.svelte

## Phase 5: 仕上げ・横断

- [ ] T008 [P] [共通] [検証] `pnpm -C gwt-gui test` 実行 gwt-gui/package.json
- [ ] T009 [P] [共通] [検証] `pnpm -C gwt-gui check` 実行 gwt-gui/package.json
