# タスクリスト: シンプルターミナルタブ

## Phase 1: バックエンド基盤

- [ ] T001 [US1] [コア] PaneManager に `spawn_shell()` メソッド追加（launch_agent のブランチマッピングスキップ版） `crates/gwt-core/src/terminal/manager.rs`
- [ ] T002 [US1] [テスト] `spawn_shell()` のユニットテスト `crates/gwt-core/src/terminal/manager.rs`
- [ ] T003 [US1] [Tauri コマンド] `spawn_shell` コマンドの実装（$SHELL 解決・PTY 生成・I/O スレッド起動） `crates/gwt-tauri/src/commands/terminal.rs`
- [ ] T004 [US1] [登録] `spawn_shell` コマンドを Tauri コマンドレジストリに登録 `crates/gwt-tauri/src/commands/mod.rs`

## Phase 2: メニューとショートカット

- [ ] T005 [US1,US2] [メニュー] Tools > New Terminal メニュー項目の追加 `crates/gwt-tauri/src/menu.rs`
- [ ] T006 [US2] [ショートカット] Ctrl+` アクセラレータ設定（非対応時はフロントエンド keydown フォールバック） `crates/gwt-tauri/src/menu.rs` `gwt-gui/src/App.svelte`
- [ ] T007 [US1] [イベント] menu-action ハンドラへの "new-terminal" ディスパッチ追加 `crates/gwt-tauri/src/app.rs`

## Phase 3: OSC 7 パース

- [ ] T008 [US5] [テスト] OSC 7 パーサーのユニットテスト作成（TDD: RED） `crates/gwt-core/src/terminal/osc.rs`
- [ ] T009 [US5] [実装] OSC 7 パーサーの実装（TDD: GREEN）: バイトスキャン・URL デコード・バッファ分断対応 `crates/gwt-core/src/terminal/osc.rs`
- [ ] T010 [US5] [統合] stream_pty_output() への OSC 7 検出組み込み（agent_name=="terminal" のみ） `crates/gwt-tauri/src/commands/terminal.rs`
- [ ] T011 [US5] [イベント] `terminal-cwd-changed` Tauri イベントの定義・発行（重複抑制付き） `crates/gwt-tauri/src/commands/terminal.rs`

## Phase 4: フロントエンド - タブ管理

- [ ] T012 [US3] [型定義] Tab.type に "terminal" 追加、Tab.cwd フィールド追加 `gwt-gui/src/lib/types.ts`
- [ ] T013 [US1] [テスト] ターミナルタブ作成・表示のフロントエンドテスト（TDD: RED） `gwt-gui/src/lib/components/__tests__/`
- [ ] T014 [US1] [実装] App.svelte に new-terminal アクション・タブ作成ロジック追加 `gwt-gui/src/App.svelte`
- [ ] T015 [US3] [実装] MainArea.svelte にターミナルタブのレンダリング追加（グレードット・basename ラベル・ホバーでフルパスツールチップ） `gwt-gui/src/lib/components/MainArea.svelte`
- [ ] T016 [US5] [実装] terminal-cwd-changed イベントリスナー・ラベル更新ロジック `gwt-gui/src/App.svelte`

## Phase 5: ライフサイクル管理

- [ ] T017 [US4] [実装] シェル exit 時のタブ自動クローズ処理（既存 terminal-closed イベント活用） `gwt-gui/src/App.svelte`
- [ ] T018 [US4] [実装] プロジェクトクローズ時のターミナルタブ PTY kill・除去 `gwt-gui/src/App.svelte`
- [ ] T019 [US4] [テスト] ターミナルタブのクローズ挙動テスト `gwt-gui/src/lib/components/__tests__/`

## Phase 6: Window メニュー統合

- [ ] T020 [US6] [バックエンド] sync_window_agent_tabs をターミナルタブ対応に拡張 `crates/gwt-tauri/src/commands/window_tabs.rs`
- [ ] T021 [US6] [フロントエンド] タブ同期にターミナルタブを含める `gwt-gui/src/App.svelte`

## Phase 7: 永続化と復元

- [ ] T022 [US7] [テスト] ターミナルタブの永続化・復元テスト（TDD: RED） `gwt-gui/src/lib/__tests__/`
- [ ] T023 [US7] [実装] agentTabsPersistence にターミナルタブの保存・復元ロジック追加 `gwt-gui/src/lib/agentTabsPersistence.ts`
- [ ] T024 [US7] [実装] 復元時の spawn_shell 呼び出しと cwd 再設定 `gwt-gui/src/App.svelte`

## Phase 8: 仕上げ

- [ ] T025 [P] [共通] cargo clippy + cargo fmt + svelte-check による品質チェック
- [ ] T026 [P] [共通] 全テスト通過の確認（cargo test + vitest）
- [ ] T027 [P] [共通] specs.md にSPEC-f490dded を登録
