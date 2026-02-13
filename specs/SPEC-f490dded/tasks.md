# タスクリスト: シンプルターミナルタブ

## Phase 1: バックエンド基盤

- [ ] T001 [P] [US1] [型定義] AgentColor enum の確認・terminal 用マッピング設計 `crates/gwt-core/src/terminal/mod.rs`
- [ ] T002 [US1] [Tauri コマンド] `spawn_shell` コマンドの実装 `crates/gwt-tauri/src/commands/terminal.rs`
- [ ] T003 [US1] [テスト] `spawn_shell` の統合テスト `crates/gwt-tauri/tests/`

## Phase 2: メニューとショートカット

- [ ] T004 [US1,US2] [メニュー] Tools > New Terminal メニュー項目の追加 `crates/gwt-tauri/src/menu.rs`
- [ ] T005 [US2] [ショートカット] Ctrl+` アクセラレータの設定（フォールバック検討含む） `crates/gwt-tauri/src/menu.rs`
- [ ] T006 [US1] [イベント] menu-action ハンドラへの "new-terminal" ディスパッチ追加 `crates/gwt-tauri/src/app.rs`

## Phase 3: OSC 7 パース

- [ ] T007 [US5] [テスト] OSC 7 パーサーのユニットテスト作成（TDD: RED） `crates/gwt-core/src/terminal/osc.rs`
- [ ] T008 [US5] [実装] OSC 7 パーサーの実装（TDD: GREEN） `crates/gwt-core/src/terminal/osc.rs`
- [ ] T009 [US5] [統合] PTY リーダーループへの OSC 7 検出組み込み `crates/gwt-tauri/src/commands/terminal.rs`
- [ ] T010 [US5] [イベント] `terminal-cwd-changed` Tauri イベントの定義・発行 `crates/gwt-tauri/src/commands/terminal.rs`

## Phase 4: フロントエンド - タブ管理

- [ ] T011 [US3] [型定義] Tab.type に "terminal" 追加、Tab.cwd フィールド追加 `gwt-gui/src/lib/types.ts`
- [ ] T012 [US1] [テスト] ターミナルタブ作成・表示のフロントエンドテスト（TDD: RED） `gwt-gui/src/lib/components/__tests__/`
- [ ] T013 [US1] [実装] App.svelte に new-terminal アクション・タブ作成ロジック追加 `gwt-gui/src/App.svelte`
- [ ] T014 [US3] [実装] MainArea.svelte にターミナルタブのレンダリング追加（グレードット・basename ラベル・ホバーでフルパスツールチップ） `gwt-gui/src/lib/components/MainArea.svelte`
- [ ] T015 [US5] [実装] terminal-cwd-changed イベントリスナー・ラベル更新ロジック `gwt-gui/src/App.svelte`

## Phase 5: ライフサイクル管理

- [ ] T016 [US4] [実装] シェル exit 時のタブ自動クローズ処理 `gwt-gui/src/App.svelte`
- [ ] T017 [US4] [実装] プロジェクトクローズ時のターミナルタブ PTY kill・除去 `gwt-gui/src/App.svelte`
- [ ] T018 [US4] [テスト] ターミナルタブのクローズ挙動テスト `gwt-gui/src/lib/components/__tests__/`

## Phase 6: Window メニュー統合

- [ ] T019 [US6] [バックエンド] sync_window_agent_tabs をターミナルタブ対応に拡張 `crates/gwt-tauri/src/commands/window_tabs.rs`
- [ ] T020 [US6] [フロントエンド] タブ同期にターミナルタブを含める `gwt-gui/src/App.svelte`

## Phase 7: 永続化と復元

- [ ] T021 [US7] [テスト] ターミナルタブの永続化・復元テスト（TDD: RED） `gwt-gui/src/lib/__tests__/`
- [ ] T022 [US7] [実装] agentTabsPersistence にターミナルタブの保存・復元ロジック追加 `gwt-gui/src/lib/agentTabsPersistence.ts`
- [ ] T023 [US7] [実装] 復元時の spawn_shell 呼び出しと cwd 再設定 `gwt-gui/src/App.svelte`

## Phase 8: 仕上げ

- [ ] T024 [P] [共通] cargo clippy + cargo fmt + svelte-check による品質チェック
- [ ] T025 [P] [共通] 全テスト通過の確認（cargo test + vitest）
