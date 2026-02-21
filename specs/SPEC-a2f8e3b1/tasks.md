# タスクリスト: Launch Agent From Issue ブランチプレフィックスAI判定

## Phase 1: バックエンド - AI プレフィックス判定

- [x] T001 [US2] [TDD] `parse_classify_response()` の単体テストを先に書く `crates/gwt-core/src/ai/issue_classify.rs`
- [x] T002 [US2] [実装] `issue_classify.rs` モジュールを新規作成（プロンプト定数 + `classify_issue_prefix()` + `parse_classify_response()`） `crates/gwt-core/src/ai/issue_classify.rs`
- [x] T003 [US2] [実装] `ai/mod.rs` にモジュール登録と公開エクスポートを追加 `crates/gwt-core/src/ai/mod.rs`
- [x] T004 [US2] [実装] `classify_issue_branch_prefix` Tauri コマンドを追加 `crates/gwt-tauri/src/commands/issue.rs`
- [x] T005 [US2] [実装] Tauri コマンドのルーティング登録 `crates/gwt-tauri/src/app.rs`（invoke_handler に追加）

## Phase 2: フロントエンド - 判定フロー統合

- [x] T006 [US1,US2] [実装] `determinePrefixForIssue()` 関数を追加（ラベル優先・AIフォールバック） `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T007 [US3] [実装] AI 判定中の状態管理変数を追加（`prefixClassifying`, `classifyRequestId`, `prefixCache`） `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T008 [US6] [実装] Issue 選択時のキャンセルとキャッシュロジック実装 `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T009 [US3] [実装] プレフィックスドロップダウンに空欄+スピナーの判定中状態を追加 `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T010 [US5] [実装] AI 失敗/不正レスポンス時のドロップダウン未選択状態の実装 `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T011 [US7] [実装] プレフィル `$effect` を新しい判定フローに統合 `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T012 [US4,FR-014] [実装] `BranchPrefix` 型に空文字列（`""`）を許容し、Launch ボタンのバリデーションで空プレフィックスをディスエーブル `gwt-gui/src/lib/components/AgentLaunchForm.svelte`

## Phase 3: フロントエンド - テスト

- [x] T013 [US1] [TDD] ラベル判定の優先順位テスト（bug→bugfix, hotfix→hotfix, bug+hotfix→hotfix） `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T014 [US2] [TDD] AI 判定呼び出しの発動条件テスト `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T015 [US3] [TDD] AI 判定中のスピナー表示テスト `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T016 [US5] [TDD] AI 失敗時のドロップダウン未選択状態テスト `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T017 [US6] [TDD] Issue 連続切り替え時のリクエスト棄却テスト `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- [x] T018 [US6] [TDD] キャッシュ再利用テスト `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`

## Phase 4: 検証

- [x] T019 [共通] [検証] `cargo test` 全テスト通過確認（既存 `test_merge_empty_os_env` 失敗は本変更と無関係）
- [x] T020 [共通] [検証] `cargo clippy --all-targets --all-features -- -D warnings` 警告なし確認
- [x] T021 [共通] [検証] `cd gwt-gui && pnpm test` 全テスト通過確認（511 tests, 42 files）
- [x] T022 [共通] [検証] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` 型チェック通過確認
