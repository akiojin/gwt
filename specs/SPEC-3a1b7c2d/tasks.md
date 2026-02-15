# タスクリスト: GUI Session Summary のスクロールバック要約（実行中対応）

## 依存関係/並列性

- US1/US2/US3 は `crates/gwt-tauri/src/commands/sessions.rs` を共有するため直列
- UI 表示変更はバックエンドと並列可能
- テストは該当実装の後に実施

## Phase 1: セットアップ

- [x] T001 [共通] 既存の Session Summary フローとキャッシュ/イベントを確認 `crates/gwt-tauri/src/commands/sessions.rs`

## Phase 2: 基盤

- [x] T002 [共通] scrollback 要約用の pane 選定ヘルパと SummaryJob 型を追加 `crates/gwt-tauri/src/commands/sessions.rs`
- [x] T003 [共通] scrollback 取得と要約入力生成を追加 `crates/gwt-tauri/src/commands/sessions.rs`

## Phase 3: ストーリー 1

- [x] T004 [US1] session_id 未保存時に scrollback fallback を有効化 `crates/gwt-tauri/src/commands/sessions.rs`
- [x] T005 [US1] `session-summary-updated` で `pane:` 擬似ID を emit `crates/gwt-tauri/src/commands/sessions.rs`
- [x] T006 [P] [US1] `Live (pane summary)` 表示に切り替え `gwt-gui/src/lib/components/MainArea.svelte`
- [x] T007 [US1] scrollback fallback job 生成のユニットテスト追加 `crates/gwt-tauri/src/commands/sessions.rs`

## Phase 4: ストーリー 2

- [x] T008 [US2] 最終出力が最新の pane を選定 `crates/gwt-tauri/src/commands/sessions.rs`
- [x] T009 [US2] pane 選定ロジックのユニットテスト追加 `crates/gwt-tauri/src/commands/sessions.rs`

## Phase 5: ストーリー 3

- [x] T010 [US3] session_id 確定時は既存フローを維持（fallback しない） `crates/gwt-tauri/src/commands/sessions.rs`

## Phase 6: 仕上げ・横断

- [x] T011 [共通] `cargo test -p gwt-tauri sessions` を実行 `crates/gwt-tauri`

## Phase 7: 永続キャッシュ（US4/US6）

- [ ] T012 [US4] Session Summary の永続キャッシュ保存先/フォーマットを実装（atomic write） `crates/gwt-core` / `crates/gwt-tauri`
- [ ] T013 [US4] `get_branch_session_summary` 初回で永続キャッシュを lazy load して即表示に反映 `crates/gwt-tauri/src/commands/sessions.rs`
- [ ] T014 [US6] 何を要約しているか（source/識別子/入力更新時刻）を結果に含め、UIで表示 `crates/gwt-tauri` + `gwt-gui`
- [ ] T015 [US4] 永続キャッシュ即表示のユニットテスト追加 `crates/gwt-tauri/src/commands/sessions.rs`

## Phase 8: 更新制御（US5）

- [ ] T016 [US5] タブ無しは更新不要でキャッシュ表示のみ（自動更新しない） `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`
- [ ] T017 [US5] Liveフォーカス15秒 / Live非フォーカス60秒へ更新間隔を切替 `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`
- [ ] T018 [US5] スクロールバックに変更がない場合は更新しないことをテストで保証 `crates/gwt-tauri/src/commands/sessions.rs`
- [ ] T019 [US5] フロント側の更新間隔切替のテスト追加 `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`
