# タスクリスト: PR Status Preview（GUI）

## Phase 1: バックエンド — データモデル + GraphQL取得

- [ ] T001 [US1] [型定義] `PrStatusInfo` / `WorkflowRunStatus` / `ReviewInfo` 等のRust構造体を定義 `crates/gwt-core/src/git/pullrequest.rs`
- [ ] T002 [US1] [テスト] T001で定義した構造体のシリアライズ/デシリアライズテスト `crates/gwt-core/src/git/pullrequest.rs`
- [ ] T003 [US1] [実装] GraphQLクエリビルダー（ブランチ名リスト→PR+CheckSuites+Reviews一括取得） `crates/gwt-core/src/git/graphql.rs`
- [ ] T004 [US1] [テスト] GraphQL JSONレスポンスのパーステスト（正常/空/エラー/部分欠損） `crates/gwt-core/src/git/graphql.rs`
- [ ] T005 [US2] [実装] PR詳細取得（レビューコメント + inline comments + 変更サマリー）のGraphQLクエリ `crates/gwt-core/src/git/graphql.rs`
- [ ] T006 [US2] [テスト] PR詳細GraphQLレスポンスのパーステスト `crates/gwt-core/src/git/graphql.rs`
- [ ] T007 [US1] [実装] `PrCache` をポーリング対応に拡張（リフレッシュ/レートリミット時キャッシュ維持） `crates/gwt-core/src/git/pullrequest.rs`
- [ ] T008 [US1] [テスト] `PrCache` リフレッシュとレートリミットフォールバックのテスト `crates/gwt-core/src/git/pullrequest.rs`

## Phase 2: バックエンド — Tauriコマンド

- [ ] T009 [US1] [実装] `fetch_pr_status` Tauriコマンド（全ブランチ分のPR/CI一括取得） `crates/gwt-tauri/src/commands/`
- [ ] T010 [US2] [実装] `fetch_pr_detail` Tauriコマンド（特定PRの詳細取得） `crates/gwt-tauri/src/commands/`
- [ ] T011 [US3] [実装] `fetch_ci_log` Tauriコマンド（`gh run view <run_id> --log`実行） `crates/gwt-tauri/src/commands/`
- [ ] T012 [US1] [テスト] Tauriコマンド群のユニットテスト `crates/gwt-tauri/src/commands/`

## Phase 3: フロントエンド — 型定義 + ポーリング基盤

- [ ] T013 [US1] [型定義] TypeScript型定義追加（`PrStatusInfo`, `WorkflowRunInfo`, `ReviewInfo`, `ReviewComment`, `PrChangeSummary`） `gwt-gui/src/lib/types.ts`
- [ ] T014 [US4] [実装] ポーリングモジュール（30秒間隔、visibilitychange連動、フォアグラウンド復帰即時リフレッシュ） `gwt-gui/src/lib/prPolling.ts`
- [ ] T015 [US4] [テスト] ポーリングのライフサイクルテスト（開始/停止/フォーカス復帰） `gwt-gui/src/lib/prPolling.test.ts`

## Phase 4: フロントエンド — Worktreeツリー展開

- [ ] T016 [US1] [実装] Sidebar.svelteのブランチ一覧をツリー化（トグルアイコン追加、展開/折りたたみ） `gwt-gui/src/lib/components/Sidebar.svelte`
- [ ] T017 [US1] [実装] PRステータスバッジ表示（`#42 Open` / `No PR` / `GitHub not connected`） `gwt-gui/src/lib/components/Sidebar.svelte`
- [ ] T018 [US1] [実装] ツリー展開時のWorkflow Run一覧表示（workflow名 + ステータスアイコン＋色） `gwt-gui/src/lib/components/Sidebar.svelte`
- [ ] T019 [US1] [テスト] ツリー展開/折りたたみのインタラクションテスト `gwt-gui/src/lib/components/Sidebar.test.ts`
- [ ] T020 [US3] [実装] Workflow Runクリックでxterm.jsターミナルタブを開く `gwt-gui/src/lib/components/Sidebar.svelte`

## Phase 5: フロントエンド — Session Summary PRセクション

- [ ] T021 [US2] [実装] PrStatusSection.svelte 新規作成（メタデータ表示） `gwt-gui/src/lib/components/PrStatusSection.svelte`
- [ ] T022 [US2] [実装] レビュー情報サブセクション（レビューアー承認状態 + inline コメント） `gwt-gui/src/lib/components/PrStatusSection.svelte`
- [ ] T023 [US2] [実装] コードスニペットのシンタックスハイライト表示 `gwt-gui/src/lib/components/PrStatusSection.svelte`
- [ ] T024 [US2] [実装] 変更サマリーサブセクション（ファイル一覧、追加/削除行数、コミット一覧） `gwt-gui/src/lib/components/PrStatusSection.svelte`
- [ ] T025 [US2] [実装] WorktreeSummaryPanel.svelteにPR Statusセクションを統合 `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`
- [ ] T026 [US2] [テスト] PrStatusSectionのレンダリングテスト（各パターン） `gwt-gui/src/lib/components/PrStatusSection.test.ts`

## Phase 6: 統合 + グレースフルデグレード

- [ ] T027 [US1] [実装] グレースフルデグレード統合（GhCliStatus連携、「GitHub not connected」表示） `gwt-gui/src/lib/components/Sidebar.svelte`
- [ ] T028 [US4] [実装] ポーリングをSidebarとWorktreeSummaryPanelに接続 `gwt-gui/src/lib/components/Sidebar.svelte`
- [ ] T029 [P] [共通] [検証] `cargo clippy` + `cargo fmt` + `svelte-check` + 全テスト通過確認
- [ ] T030 [P] [共通] [検証] CLAUDE.md のアイコンガイドライン更新反映確認
