# タスクリスト: PR Status Preview（GUI）

## Phase 1: バックエンド — データモデル + GraphQL取得

- [x] T001 [US1] [型定義] `PrStatusInfo` / `WorkflowRunStatus` / `ReviewInfo` 等のRust構造体を定義 `crates/gwt-core/src/git/pullrequest.rs`
- [x] T002 [US1] [テスト] T001で定義した構造体のシリアライズ/デシリアライズテスト `crates/gwt-core/src/git/pullrequest.rs`
- [x] T003 [US1] [実装] 軽量GraphQLクエリビルダー（ブランチ名リスト→PRステータス+各Workflowの最新1 Run一括取得） `crates/gwt-core/src/git/graphql.rs`
- [x] T004 [US1] [テスト] 軽量GraphQL JSONレスポンスのパーステスト（正常/空/エラー/部分欠損） `crates/gwt-core/src/git/graphql.rs`
- [x] T005 [US2] [実装] 詳細GraphQLクエリ（選択PR単体のレビューコメント + inline comments + 変更サマリー取得） `crates/gwt-core/src/git/graphql.rs`
- [x] T006 [US2] [テスト] 詳細GraphQLレスポンスのパーステスト `crates/gwt-core/src/git/graphql.rs`
- [x] T007 [US1] [実装] `PrCache` をポーリング対応に拡張（リフレッシュ/レートリミット時キャッシュ維持） `crates/gwt-core/src/git/pullrequest.rs`
- [x] T008 [US1] [テスト] `PrCache` リフレッシュとレートリミットフォールバックのテスト `crates/gwt-core/src/git/pullrequest.rs`

## Phase 2: バックエンド — Tauriコマンド

- [x] T009 [US1] [実装] `fetch_pr_status` Tauriコマンド（全ブランチ分のPR/CI一括取得） `crates/gwt-tauri/src/commands/`
- [x] T010 [US2] [実装] `fetch_pr_detail` Tauriコマンド（特定PRの詳細取得） `crates/gwt-tauri/src/commands/`
- [x] T011 [US3] [実装] `fetch_ci_log` Tauriコマンド（`gh run view <run_id> --log`実行） `crates/gwt-tauri/src/commands/`
- [x] T012 [US1] [テスト] Tauriコマンド群のユニットテスト `crates/gwt-tauri/src/commands/`

## Phase 3: フロントエンド — 型定義 + ポーリング基盤

- [x] T013 [US1] [型定義] TypeScript型定義追加（`PrStatusInfo`, `WorkflowRunInfo`, `ReviewInfo`, `ReviewComment`, `PrChangeSummary`） `gwt-gui/src/lib/types.ts`
- [x] T014 [US4] [実装] ポーリングモジュール（30秒間隔、visibilitychange連動、フォアグラウンド復帰即時リフレッシュ） `gwt-gui/src/lib/prPolling.ts`
- [x] T015 [US4] [テスト] ポーリングのライフサイクルテスト（開始/停止/フォーカス復帰） `gwt-gui/src/lib/prPolling.test.ts`

## Phase 4: フロントエンド — Worktreeツリー展開

- [x] T016 [US1] [実装] Sidebar.svelteのブランチ一覧をツリー化（トグルアイコン追加、展開/折りたたみ） `gwt-gui/src/lib/components/Sidebar.svelte`
- [x] T017 [US1] [実装] PRステータスバッジ表示（`#42 Open` / `No PR` / `GitHub not connected`）Local+Allフィルター対応、Remoteはバッジのみ `gwt-gui/src/lib/components/Sidebar.svelte`
- [x] T018 [US1] [実装] ツリー展開時の各Workflowの最新1 Run表示（workflow名 + ステータスアイコン＋色）LocalおよびAllフィルターのみ `gwt-gui/src/lib/components/Sidebar.svelte`
- [x] T019 [US1] [テスト] ツリー展開/折りたたみのインタラクションテスト `gwt-gui/src/lib/components/Sidebar.test.ts`
- [x] T020 [US3] [実装] Workflow Runクリックでxterm.jsターミナルタブを開く `gwt-gui/src/lib/components/Sidebar.svelte`

## Phase 5: フロントエンド — Session Summary PRサブタブ

- [x] T021 [US2] [実装] PrStatusSection.svelte 新規作成（メタデータ表示） `gwt-gui/src/lib/components/PrStatusSection.svelte`
- [x] T022 [US2] [実装] レビュー情報サブセクション（レビューアー承認状態 + inline コメント） `gwt-gui/src/lib/components/PrStatusSection.svelte`
- [x] T023 [US2] [実装] コードスニペットのシンタックスハイライト表示 `gwt-gui/src/lib/components/PrStatusSection.svelte`
- [x] T024 [US2] [実装] 変更サマリーサブセクション（ファイル一覧、追加/削除行数、コミット一覧） `gwt-gui/src/lib/components/PrStatusSection.svelte`
- [x] T025 [US2] [実装] WorktreeSummaryPanel.svelteにサブタブUI（「Summary」「PR」）を追加し、PRタブでPrStatusSectionを表示 `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`
- [x] T026 [US2] [テスト] PrStatusSectionのレンダリングテスト + サブタブ切替えテスト `gwt-gui/src/lib/components/PrStatusSection.test.ts`

## Phase 6: 統合 + グレースフルデグレード

- [x] T027 [US1] [実装] グレースフルデグレード統合（GhCliStatus連携、「GitHub not connected」表示） `gwt-gui/src/lib/components/Sidebar.svelte`
- [x] T028 [US4] [実装] ポーリングをSidebarとWorktreeSummaryPanelに接続 `gwt-gui/src/lib/components/Sidebar.svelte`
- [x] T029 [P] [共通] [検証] `cargo clippy` + `cargo fmt` + `svelte-check` + 全テスト通過確認
- [x] T030 [P] [共通] [検証] CLAUDE.md のアイコンガイドライン更新反映確認
