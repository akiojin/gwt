<!-- GWT_SPEC_ARTIFACT:doc:tasks.md -->
doc:tasks.md

# Tasks: パフォーマンスプロファイリング基盤 (#1705)

All tasks completed in commit `626c6ce3`. Test task T029 added post-analysis. Rev2 tasks added for startup-stability timing and deep trace.

## Phase 1: 基盤整備

- [x] T001: Settings に `profiling: bool` フィールド追加 (`crates/gwt-core/src/config/settings.rs`)
- [x] T002: ConfigToml / LocalConfigToml に `profiling` フィールド追加 + From impl 更新
- [x] T003: LogConfig に `profiling: bool` フィールド追加 (`crates/gwt-core/src/logging/logger.rs`)
- [x] T004: `tracing-chrome = "0.7"` をワークスペース依存に追加 (`Cargo.toml`, `crates/gwt-core/Cargo.toml`)
- [x] T005: `init_logger()` に Chrome Trace レイヤー追加 (`ChromeLayerBuilder` + `FlushGuard`)
- [x] T006: `ProfilingGuard` 構造体を追加しエクスポート
- [x] T007: `main.rs` で `Settings::load_global()` → `init_logger()` を呼び出し

## Phase 2: バックエンド計装 [P]

- [x] T008: 全 Tauri commands (28ファイル) に `#[instrument(skip_all, fields(command = "..."))]` を追加
- [x] T009: gwt-core `git/repository.rs` に `#[instrument]` を追加 (11関数)
- [x] T010: gwt-core `worktree/manager.rs` に `#[instrument]` を追加 (8関数)
- [x] T011: gwt-core `config/settings.rs` に `#[instrument]` を追加 (4関数)
- [x] T012: gwt-core `terminal/{pty,pane,manager}.rs` に `#[instrument]` を追加 (10関数)
- [x] T013: gwt-core `docker/{detector,manager}.rs` に `#[instrument]` を追加 (6関数)

## Phase 3: フリーズ検知 [P]

- [x] T014: `AppState` に `last_heartbeat: Mutex<Option<Instant>>` を追加 (`state.rs`)
- [x] T015: `heartbeat` Tauri コマンド追加 (`commands/system.rs`)
- [x] T016: `FrontendMetric` 構造体追加 + `report_frontend_metrics` コマンド追加
- [x] T017: watchdog background task 追加 (`app.rs` setup 側、2秒間隔・3秒超過で WARN)
- [x] T018: `heartbeat` / `report_frontend_metrics` を `invoke_handler` に登録

## Phase 4+6: フロントエンド計測 + UI [P]

- [x] T019: `profiling.svelte.ts` 新規作成 (Svelte 5 runes ストア)
- [x] T020: `tauriInvoke.ts` に `performance.now()` 計測追加
- [x] T021: `App.svelte` に profiling 初期化 / クリーンアップ追加
- [x] T022: `StatusBar.svelte` に `PROFILING` バッジ追加
- [x] T023: `commands/settings.rs` の `SettingsData` に `profiling` フィールド追加

## Phase 5: テスト計装 [P]

- [x] T024: `#[cfg(test)] init_test_tracing()` ヘルパー追加 (`logger.rs`)
- [x] T025: `logging/mod.rs` から `init_test_tracing` をエクスポート
- [x] T026: `vitest.setup.ts` 新規作成 (`GWT_TEST_PERF=1` で実行時間 + ヒープ計測)
- [x] T027: `vite.config.ts` に `setupFiles` 登録
- [x] T028: `playwright.config.ts` に `trace: 'on-first-retry'` 設定

## Post-Analysis: プロファイリング固有テスト

- [x] T029: `test_init_logger_with_profiling` — `profiling=true` で `init_logger()` を呼び出し、`profile.json` が生成されることを確認する単体テスト (`crates/gwt-core/src/logging/logger.rs` の `#[cfg(test)]` モジュール)

## Post-Implementation: Settings Developer tab

- [x] T030: Settings に `Developer` タブを追加し、`Enable Profiling` トグルを配置 (`gwt-gui/src/lib/components/SettingsPanel.svelte`)
- [x] T031: profiling トグルの保存とタブ表示を `SettingsPanel.test.ts` で検証

## Rev2: Startup stability timing + deep trace

- [x] T032: テスト: frontend startup token 管理と `Frontend Hydrated` 完了条件を unit test で固定する (`gwt-gui/src/lib/*`)
- [x] T033: `gwt-gui/src/App.svelte` から startup profiling helper を抽出し、`open_project` / cold-start restore 開始点と hydration 3 本の完了点を token で追跡する
- [x] T034: `gwt-gui/src/lib/profiling.svelte.ts` と `gwt-gui/src/lib/tauriInvoke.ts` の metric schema を拡張し、invoke RTT と startup/hydration measure を同居させる
- [x] T035: `crates/gwt-tauri/src/commands/system.rs` の `FrontendMetric` / `report_frontend_metrics` を rev2 schema に更新し、startup metric を構造化ログで残す
- [x] T036: `crates/gwt-tauri/src/commands/project.rs` に `open_project` subphase span と slow-path warn threshold を追加する
- [x] T037: `crates/gwt-tauri/src/commands/branches.rs`, `cleanup.rs`, `terminal.rs`, `version_history.rs`, `project_index.rs` の startup/hydration hot path に deep trace を追加する
- [x] T038: cold-start restore 経路 (`gwt-gui/src/lib/windowSessionRestore.ts`, `gwt-gui/src/App.svelte`) に restore-session startup metric を追加する
- [x] T039: テスト: stale token / project switch 時に古い startup metric が破棄されることを検証する
- [ ] T040: targeted verification で `profile.json` / logs から startup blocker を追えることを確認する

## Verification

- [x] `cargo check` — 成功
- [x] `cargo clippy --all-targets --all-features -- -D warnings` — 警告なし
- [x] `cargo test` — 583 passed (既存失敗25件は変更前と同一、回帰なし)
- [x] `svelte-check` — 0 errors, 1 warning (既存 `MergeDialog` 警告のみ)
- [x] `commitlint` — 通過
- [x] `pnpm test src/lib/startupProfiling.test.ts src/lib/tauriInvoke.test.ts src/lib/components/OpenProject.test.ts` — 43 passed
- [x] `pnpm exec svelte-check --tsconfig ./tsconfig.json` — 0 errors, 0 warnings
- [x] `cargo test -p gwt-tauri frontend_metric_accepts -- --nocapture` — 2 tests passed
- [x] `cargo check -p gwt-tauri` — success
- [x] `cargo clippy -p gwt-tauri --all-targets --all-features -- -D warnings` — success
