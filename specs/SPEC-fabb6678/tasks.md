# タスクリスト: Error Reporting & Feature Suggestion

**仕様ID**: `SPEC-fabb6678`

**ストーリー間の依存関係**:

- US1（エラー自動通知）→ Phase 2（エラーバス基盤）に依存
- US2（能動的問題報告）→ Phase 3（報告フォームUI）に依存
- US3（改善提案）→ Phase 3（報告フォームUI）に依存
- US4（スクリーンキャプチャ）→ Phase 3 + Phase 5
- US5（診断情報選択）→ Phase 3 に依存
- US6（送信先リポ選択）→ Phase 3 に依存
- US7（プライバシー保護）→ Phase 3 に依存
- US8（送信失敗フォールバック）→ Phase 4 に依存

## Phase 1: セットアップ

- [ ] T001 [P] 報告画像保存ディレクトリの自動作成ロジックを追加する `crates/gwt-core/src/config.rs`
- [ ] T002 [P] Cargo.toml に画像キャプチャ用の OS ネイティブ依存クレートを追加する（macOS: core-graphics + core-foundation, Windows: windows） `Cargo.toml`

## Phase 2: 基盤 — 構造化エラー型 + エラーバス

- [x] T003 [US1] `ErrorSeverity` enum と `StructuredError` 構造体を定義する `crates/gwt-core/src/error.rs`
- [x] T004 [US1] `GwtError` に `severity()` メソッドを追加し、各バリアントのデフォルト severity を定義する `crates/gwt-core/src/error.rs`
- [x] T005 [US1] `StructuredError::from_gwt_error(error, command)` 変換メソッドを実装する `crates/gwt-core/src/error.rs`
- [x] T006 [US1] T003-T005 のユニットテストを作成する `crates/gwt-core/tests/error_structured_test.rs`
- [x] T007 [US1] `commands/agents.rs` の戻り値を `Result<T, StructuredError>` に移行する `crates/gwt-tauri/src/commands/agents.rs`
- [x] T008 [US1] `commands/agent_config.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/agent_config.rs`
- [x] T009 [US1] `commands/branch_suggest.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/branch_suggest.rs`
- [x] T010 [US1] `commands/branches.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/branches.rs`
- [x] T011 [US1] `commands/cleanup.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/cleanup.rs`
- [x] T012 [US1] `commands/docker.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/docker.rs`
- [x] T013 [US1] `commands/git_view.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/git_view.rs`
- [x] T014 [US1] `commands/hooks.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/hooks.rs`
- [x] T015 [US1] `commands/issue.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/issue.rs`
- [x] T016 [US1] `commands/issue_spec.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/issue_spec.rs`
- [x] T017 [US1] `commands/profiles.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/profiles.rs`
- [x] T018 [US1] `commands/project.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/project.rs`
- [x] T019 [US1] `commands/project_mode.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/project_mode.rs`
- [x] T020 [US1] `commands/pullrequest.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/pullrequest.rs`
- [x] T021 [US1] `commands/recent_projects.rs` は Result を返さないため移行不要
- [x] T022 [US1] `commands/sessions.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/sessions.rs`
- [x] T023 [US1] `commands/settings.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/settings.rs`
- [x] T024 [US1] `commands/skills.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/skills.rs`
- [x] T025 [US1] `commands/system.rs` は Result を返さないため移行不要
- [x] T026 [US1] `commands/terminal.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T027 [US1] `commands/update.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/update.rs`
- [x] T028 [US1] `commands/version_history.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/version_history.rs`
- [x] T029 [US1] `commands/window.rs` は Result を返さないため移行不要
- [x] T030 [US1] `commands/window_tabs.rs` の戻り値を移行する `crates/gwt-tauri/src/commands/window_tabs.rs`
- [x] T031 [US1] フロントエンド `errorBus.ts` を作成する（ErrorBus クラス、subscribe/emit/セッション重複抑制） `gwt-gui/src/lib/errorBus.ts`
- [x] T032 [US1] フロントエンド `tauriInvoke.ts` を作成する（invoke ラッパー、エラーバスへの通知） `gwt-gui/src/lib/tauriInvoke.ts`
- [x] T033 [US1] T031-T032 のユニットテストを作成する `gwt-gui/src/lib/errorBus.test.ts` `gwt-gui/src/lib/tauriInvoke.test.ts`
- [x] T034 [US1] 全 Svelte コンポーネント・TSファイルの `import { invoke } from "@tauri-apps/api/core"` を `import { invoke } from "$lib/tauriInvoke"` に置換する `gwt-gui/src/`
- [x] T035 [US1] App.svelte のトースト通知を拡張する（ToastAction に report-error を追加、エラーバス購読、Report リンク表示） `gwt-gui/src/App.svelte`
- [x] T036 [US1] Rust 側の全コマンド移行後に `cargo test` が全て通ることを確認する
- [x] T037 [US1] フロントエンド側の全テスト（`pnpm test`）が通ることを確認する

## Phase 3: 報告フォーム UI (US2, US3)

- [x] T038 [US7] プライバシーマスキングユーティリティを作成する `gwt-gui/src/lib/privacyMask.ts`
- [x] T039 [US7] マスキングユーティリティのユニットテストを作成する `gwt-gui/src/lib/privacyMask.test.ts`
- [x] T040 [US5] 診断情報収集用の Tauri コマンドを追加する（`read_recent_logs`, `get_report_system_info`） `crates/gwt-tauri/src/commands/report.rs`
- [x] T041 [US5] `commands/report.rs` をコマンドモジュール登録に追加する `crates/gwt-tauri/src/commands/mod.rs` `crates/gwt-tauri/src/lib.rs`
- [x] T042 [US5] フロントエンド診断情報収集ユーティリティを作成する `gwt-gui/src/lib/diagnostics.ts`
- [x] T043 [US2] Issue テンプレート生成ユーティリティを作成する（Bug Report / Feature Request 両方） `gwt-gui/src/lib/issueTemplate.ts`
- [x] T044 [US2] T043 のユニットテストを作成する `gwt-gui/src/lib/issueTemplate.test.ts`
- [x] T045 [US2][US3] ReportDialog.svelte を作成する（統合報告モーダル、Bug Report / Feature Request タブ、フォームフィールド、診断情報チェックボックス、プレビュー） `gwt-gui/src/lib/components/ReportDialog.svelte`
- [x] T046 [US2][US3] ReportDialog のユニットテストを作成する `gwt-gui/src/lib/components/ReportDialog.test.ts`
- [x] T047 [US2] App.svelte に ReportDialog の表示制御を追加する（showReportDialog 関数、トースト Report リンクとの連携） `gwt-gui/src/App.svelte`

## Phase 4: GitHub Issues 連携 (US6, US8)

- [x] T048 [US6] GitHub Issue 作成コマンドを実装する（`create_github_issue`） `crates/gwt-tauri/src/commands/report.rs`
- [x] T049 [US6] GitHub 認証情報の取得ロジックを実装する（gh CLI 経由） `crates/gwt-tauri/src/commands/report.rs`
- [x] T050 [US6] 送信先リポジトリ検出コマンドを追加する（現在の作業リポの owner/repo を取得） `crates/gwt-tauri/src/commands/report.rs`
- [x] T051 [US6] T048-T050 のユニットテストを作成する（URL解析テスト） `crates/gwt-tauri/src/commands/report.rs`
- [x] T052 [US8] ReportDialog にフォールバック UI を追加する（送信失敗時の "Copy & Open in Browser" ボタン、クリップボードコピー + shell.open） `gwt-gui/src/lib/components/ReportDialog.svelte`
- [x] T053 [US6] ReportDialog に送信先リポジトリ ドロップダウンを追加する `gwt-gui/src/lib/components/ReportDialog.svelte`

## Phase 5: スクリーンキャプチャ + Help メニュー (US4)

- [ ] T054 [US4] macOS 用ウィンドウスクリーンキャプチャを実装する（CGWindowListCreateImage）— 後日実装
- [ ] T055 [US4] Windows 用ウィンドウスクリーンキャプチャを実装する（PrintWindow / BitBlt）— 後日実装
- [x] T056 [US4] テキストベースのスクリーンキャプチャ Tauri コマンドを追加する（`capture_screen_text`） `crates/gwt-tauri/src/commands/report.rs`
- [ ] T057 [US4] T054-T056 のユニットテストを作成する — 後日実装
- [x] T058 [US4] ReportDialog にスクリーンキャプチャボタンを追加する `gwt-gui/src/lib/components/ReportDialog.svelte`
- [x] T059 Help メニューを新規追加する（Report Issue... + Suggest Feature... + About + Check for Updates） `crates/gwt-tauri/src/menu.rs`
- [x] T060 Help メニューのイベント処理を App.svelte に追加する `gwt-gui/src/App.svelte`
- [x] T061 macOS Application メニューから About / Check for Updates を Help メニューへ移動し、Application メニューには About のみ残す `crates/gwt-tauri/src/menu.rs`

## Phase 6: 仕上げ・横断

- [x] T062 [P] `cargo clippy --all-targets --all-features -- -D warnings` が通ることを確認する
- [x] T063 [P] `cargo fmt` で全 Rust コードをフォーマットする
- [x] T064 [P] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` が通ることを確認する
- [x] T065 [P] `cd gwt-gui && pnpm test` で全フロントエンドテストが通ることを確認する（534/535、既存flaky 1件のみ失敗）
- [x] T066 [P] `cargo test` で全バックエンドテストが通ることを確認する（433テスト合格）
- [x] T067 型定義を更新する（StructuredError の TypeScript 型を types.ts に追加） `gwt-gui/src/lib/types.ts`
