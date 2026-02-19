# タスクリスト: Version History 即時表示（永続キャッシュ + プリフェッチ）

## Phase 1: 永続キャッシュ基盤

- [ ] T001 [P] [US3] キャッシュファイルパス算出関数の実装 crates/gwt-tauri/src/commands/version_history.rs
- [ ] T002 [P] [US3] ディスクキャッシュ read/write 関数の実装 crates/gwt-tauri/src/commands/version_history.rs
- [ ] T003 [US3] ディスクキャッシュのユニットテスト crates/gwt-tauri/src/commands/version_history.rs
- [ ] T004 [US3] get_cached_version_history にディスクキャッシュフォールバック追加 crates/gwt-tauri/src/commands/version_history.rs
- [ ] T005 [US3] generate_and_cache_version_history にディスク書き出し追加 crates/gwt-tauri/src/commands/version_history.rs

## Phase 2: changelog 先行表示

- [ ] T006 [US4] get_project_version_history で "generating" レスポンスに changelog_markdown を含める crates/gwt-tauri/src/commands/version_history.rs
- [ ] T007 [US4] changelog 先行表示のユニットテスト crates/gwt-tauri/src/commands/version_history.rs
- [ ] T008 [US4] VersionHistoryPanel で generating 状態でも changelog を表示する gwt-gui/src/lib/components/VersionHistoryPanel.svelte
- [ ] T009 [US4] VersionHistoryPanel の changelog 先行表示テスト gwt-gui/src/lib/components/VersionHistoryPanel.test.ts

## Phase 3: 並列生成

- [ ] T010 [US5] AppState に AI 要約生成用セマフォ（最大3）を追加 crates/gwt-tauri/src/state.rs
- [ ] T011 [US5] generate_and_cache_version_history をセマフォ配下に変更 crates/gwt-tauri/src/commands/version_history.rs
- [ ] T012 [US5] フロントエンドの逐次生成ロジック（stepGenerate）を並列対応に変更 gwt-gui/src/lib/components/VersionHistoryPanel.svelte
- [ ] T013 [US5] フロントエンド並列生成のテスト gwt-gui/src/lib/components/VersionHistoryPanel.test.ts

## Phase 4: プリフェッチ

- [ ] T014 [US2] prefetch_version_history Tauri コマンドの実装 crates/gwt-tauri/src/commands/version_history.rs
- [ ] T015 [US2] プリフェッチのユニットテスト crates/gwt-tauri/src/commands/version_history.rs
- [ ] T016 [US2] プロジェクトオープン時にプリフェッチをバックグラウンド起動 crates/gwt-tauri/src/commands/project.rs
- [ ] T017 [US2] プリフェッチ統合テスト crates/gwt-tauri/src/commands/version_history.rs

## Phase 5: フロントエンド統合

- [ ] T018 [US1] VersionHistoryPanel の loadVersions を並列キャッシュ対応に改修 gwt-gui/src/lib/components/VersionHistoryPanel.svelte
- [ ] T019 [US1] キャッシュヒット時の即時表示テスト gwt-gui/src/lib/components/VersionHistoryPanel.test.ts

## Phase 6: 仕上げ

- [ ] T020 [共通] cargo clippy + cargo fmt + svelte-check gwt-gui/ crates/
- [ ] T021 [共通] 全テスト通過確認（cargo test + pnpm test）
