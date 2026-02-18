# タスクリスト: Sidebar Branch Switch Non-Blocking

## Phase 1: セットアップ

- [x] T001 [P] [US1] [準備タスク] 既存の分岐切替時データ取得フローを確認する `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`

## Phase 2: 基盤

- [x] T002 [US1] [基盤実装] WorktreeSummaryPanel の一括取得 effect を Summary中心へ分離する `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`
- [x] T003 [US1] [基盤実装] タブ遅延取得 + branch単位キャッシュ + staleガードを実装する `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`

## Phase 3: ストーリー 1

- [x] T004 [US1] [テスト] ブランチ切替時に重いタブ取得が自動発火しない RED/GREEN を追加する `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`
- [x] T005 [US1] [実装] 選択描画優先の1フレーム遅延を導入する `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`

## Phase 4: ストーリー 2

- [x] T006 [US2] [テスト] Issue/PR/Docker のタブ表示時遅延取得を検証する `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`
- [x] T007 [US2] [実装] `fetch_latest_branch_pr` backend 短TTLキャッシュを導入する `crates/gwt-tauri/src/commands/pullrequest.rs`

## Phase 5: 仕上げ・横断

- [x] T008 [P] [共通] [検証] `pnpm -C gwt-gui test src/lib/components/WorktreeSummaryPanel.test.ts` を実行する
- [x] T009 [P] [共通] [検証] `cargo test -p gwt-tauri pullrequest` を実行する
