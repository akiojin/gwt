# タスクリスト: 設定画面フォントファミリー選択

## ストーリー間依存関係

- US1（即時プレビュー）を先に実装し、US2（保存・起動復元）で永続化を接続する
- US3（Closeロールバック）は US1 のプレビュー反映機構に依存する

## Phase 1: セットアップ

- [x] T001 [P] [US1] 既存設定画面と型定義を調査し、フォントファミリー追加ポイントを特定する `gwt-gui/src/lib/components/SettingsPanel.svelte` `gwt-gui/src/lib/types.ts`

## Phase 2: 基盤（設定モデル）

- [x] T002 [US2] `AppearanceSettings` に `ui_font_family` / `terminal_font_family` と既定値を追加する `crates/gwt-core/src/config/settings.rs`
- [x] T003 [US2] Tauri DTO `SettingsData` に新フィールドを追加し、from/to 変換を更新する `crates/gwt-tauri/src/commands/settings.rs`
- [x] T004 [US2] 設定正規化と後方互換のユニットテストを追加する `crates/gwt-core/src/config/settings.rs` `crates/gwt-tauri/src/commands/settings.rs`

## Phase 3: ストーリー 1（即時プレビュー）

- [x] T005 [US1] Settings Appearance タブに `UI Font Family` / `Terminal Font Family` のセレクトを追加する `gwt-gui/src/lib/components/SettingsPanel.svelte`
- [x] T006 [US1] 選択変更で CSS 変数と terminal event を即時反映する `gwt-gui/src/lib/components/SettingsPanel.svelte`
- [x] T007 [US1] terminal 側で `gwt-terminal-font-family` イベントを購読してフォント反映する `gwt-gui/src/lib/terminal/TerminalView.svelte`

## Phase 4: ストーリー 2（保存・起動復元）

- [x] T008 [US2] Save 時に新フォント項目を設定保存ペイロードへ含める `gwt-gui/src/lib/components/SettingsPanel.svelte`
- [x] T009 [US2] 起動時 `get_settings` からフォントファミリーを先行適用する `gwt-gui/src/main.ts`
- [x] T010 [US2] アプリ側設定更新イベントでフォントファミリーを適用する `gwt-gui/src/App.svelte`

## Phase 5: ストーリー 3（Closeロールバック）

- [x] T011 [US3] Settings Close 時に未保存フォント変更を保存済み値へ戻す `gwt-gui/src/lib/components/SettingsPanel.svelte`

## Phase 6: テスト（TDD）

- [x] T012 [US1] SettingsPanel のフォント選択・保存テストを追加し GREEN 化する `gwt-gui/src/lib/components/SettingsPanel.test.ts`
- [x] T013 [US1] TerminalView のフォント初期化・イベント反映テストを追加し GREEN 化する `gwt-gui/src/lib/terminal/TerminalView.test.ts`
- [x] T014 [US2] Playwright でフォント保存 E2E を追加する `gwt-gui/e2e/windows-shell-selection.spec.ts`
- [x] T015 [US3] Playwright で Close ロールバック E2E を追加する `gwt-gui/e2e/windows-shell-selection.spec.ts`
- [x] T016 [P] [共通] E2E モックの `get_settings` 既定レスポンスを新スキーマに更新する `gwt-gui/e2e/support/tauri-mock.ts`

## Phase 7: 仕上げ・横断

- [x] T017 [P] [共通] `spec.md` / `plan.md` / `tasks.md` / `tdd.md` を更新し、仕様・実装・テスト記録を同期する `specs/SPEC-7b36bdc7/spec.md` `specs/SPEC-7b36bdc7/plan.md` `specs/SPEC-7b36bdc7/tasks.md` `specs/SPEC-7b36bdc7/tdd.md`
- [x] T018 [共通] 関連テストを最終実行し、結果を `tdd.md` に追記する `specs/SPEC-7b36bdc7/tdd.md`
