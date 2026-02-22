# タスクリスト: gwt 起動時に前回のWindowを復元する

## Phase 1: 仕様・計画

- [x] T001 [共通] spec.md を作成 `specs/SPEC-8f9b2d01/spec.md`
- [x] T002 [共通] plan.md を作成 `specs/SPEC-8f9b2d01/plan.md`
- [x] T003 [共通] tasks.md を作成 `specs/SPEC-8f9b2d01/tasks.md`

## Phase 2: ステートとコマンド追加

- [x] T004 [US1] windowセッション永続化ユーティリティを追加/更新 `gwt-gui/src/lib/windowSessions.ts`
- [x] T005 [US1] ウィンドウ復元用Tauriコマンドを追加 `crates/gwt-tauri/src/commands/window.rs`
- [x] T006 [US1] App起動時にウィンドウセッションを読み取り復元する処理を追加 `gwt-gui/src/App.svelte`
- [x] T007 [US1] ウィンドウ間重複復元を避けるリーダーTTL制御を追加 `gwt-gui/src/App.svelte`

## Phase 3: 仕様保存整合

- [x] T008 [US3] 開閉時のセッション保存/削除をbest-effortで統合 `gwt-gui/src/App.svelte`
- [x] T009 [US2] `open_gwt_window` と `get_current_window_label` の結果差異吸収（label差分時更新）を実装 `gwt-gui/src/App.svelte`

## Phase 4: TDD と検証

- [x] T010 [US1] `windowSessions` のユニットテスト追加 `gwt-gui/src/lib/windowSessions.test.ts`
- [x] T011 [US2] Rustコマンド単体テスト（`normalize_window_label`) 継続 `crates/gwt-tauri/src/commands/window.rs`
- [x] T012 [共通] `cargo test -p gwt-tauri` を実施
- [x] T013 [共通] `pnpm -C gwt-gui check`/`pnpm -C gwt-gui test windowSessions.test.ts` を実施（環境に依存）

## Phase 5: 仕様横断

- [x] T014 [共通] 仕様一覧 `specs/specs.md` を更新し、Window復元を現行仕様として追記
- [x] T015 [共通] 既存仕様 `SPEC-4470704f` と整合、関連範囲外記述を更新
