# Tasks: SPEC-1785 — SPECs画面からのAgent起動

## Phase 1: SpecItem拡張 + metadata連携 (US-4)

- [ ] T001: [TDD] SpecItem に branches フィールド追加、load_specs() 拡張
  - `crates/gwt-tui/src/screens/specs.rs`
  - SpecItem に `pub branches: Vec<String>` 追加
  - load_specs() で metadata.json の `"branches"` 配列を読み込み（フィールドなし → 空Vec）
  - テスト: load_specs_from_tempdir 拡張 — branches あり/なしのケース
  - テスト: 既存テストの sample_specs() に branches フィールド追加

- [ ] T002: [TDD] metadata.json ブランチ書き戻しユーティリティ
  - `crates/gwt-tui/src/screens/specs.rs` (または app.rs のヘルパー)
  - `save_spec_branch(repo_root, spec_dir_name, branch_name)` — metadata.json を読み込み、branches 配列にブランチ名を追記（重複回避）、書き戻し
  - テスト: tempdir で metadata.json 書き込み → 読み戻し → branches 含むか確認
  - テスト: 同じブランチ名を2回追加しても重複しないこと

## Phase 2: キーバインド + LaunchAgentメッセージ (US-1, US-2)

- [ ] T003: [TDD] SpecsMessage::LaunchAgent とキーバインド
  - `crates/gwt-tui/src/screens/specs.rs`
  - SpecsMessage に LaunchAgent バリアント追加
  - handle_key(): 一覧モードで Shift+Enter → LaunchAgent
  - handle_key(): 詳細モードで Shift+Enter → LaunchAgent
  - update(): LaunchAgent は状態変更なし（app.rs でインターセプト）
  - テスト: handle_key_shift_enter_returns_launch_agent
  - テスト: handle_key_detail_shift_enter_returns_launch_agent
  - テスト: handle_key_shift_enter_in_search_mode_returns_none

- [ ] T004: ヘッダーにキーバインドヒント追加
  - `crates/gwt-tui/src/screens/specs.rs`
- 一覧ヘッダー: `SPECs ({count})  [/] Search  [Shift+Enter] Launch`
- 詳細ヘッダー: `{spec_id} - {title}  [Shift+Enter] Launch  [Esc] Back`
  - テスト: render smoke テストでヒント文字列の存在確認

## Phase 3: Phase確認ダイアログ (US-3)

- [ ] T005: [TDD] Phase確認状態とUI
  - `crates/gwt-tui/src/screens/specs.rs`
  - SpecsState に `confirm_launch: bool` 追加
  - SpecsMessage に `ConfirmLaunch` / `CancelLaunch` バリアント追加
  - handle_key(): confirm_launch モードで Y/Enter → ConfirmLaunch、N/Esc → CancelLaunch
  - update(): ConfirmLaunch → confirm_launch = false（app.rs でインターセプト後 Wizard 起動）
  - update(): CancelLaunch → confirm_launch = false
  - render(): confirm_launch 時に確認メッセージオーバーレイ
  - テスト: draft phaseのSPECでconfirm_launch状態遷移
  - テスト: handle_key_confirm_y_returns_confirm_launch
  - テスト: handle_key_confirm_esc_returns_cancel_launch

## Phase 4: ブランチ選択ダイアログ (US-4) [P]

- [ ] T006: [TDD] ブランチ選択UI
  - `crates/gwt-tui/src/screens/specs.rs`
  - SpecsState に `branch_select_mode: bool`, `branch_candidates: Vec<String>`, `branch_selected: usize` 追加
  - SpecsMessage に `SelectBranch` / `CancelBranchSelect` / `BranchSelectPrev` / `BranchSelectNext` 追加
  - handle_key(): branch_select_mode で j/k → Prev/Next、Enter → SelectBranch、Esc → Cancel
  - render(): branch_select_mode 時にブランチ候補リストオーバーレイ（末尾に `+ Create feature/SPEC-{N}`）
  - テスト: ブランチ選択のナビゲーションと確定
  - テスト: 空候補リストでの安全な動作

## Phase 5: WizardState拡張 (US-1, US-5)

- [ ] T007: [TDD] open_for_spec() メソッドとステップスキップ
  - `crates/gwt-tui/src/screens/wizard.rs`
  - WizardState に `pub from_spec: bool`, `pub spec_id: Option<String>` 追加
  - `open_for_spec(spec_id, branch_name, history)` メソッド追加
    - from_spec = true, branch_name 設定、is_new_branch は候補に応じて設定
    - 履歴あり → QuickStart、なし → AgentSelect から開始
  - next_step(): from_spec 時のスキップ（BranchAction, BranchTypeSelect, IssueSelect, AIBranchSuggest, BranchNameInput を飛ばす）
  - prev_step(): from_spec 時のバック（AgentSelect → None で Wizard クローズ）
  - テスト: open_for_spec_with_history — QuickStart から開始
  - テスト: open_for_spec_without_history — AgentSelect から開始
  - テスト: next_step_from_spec_skips_branch_steps
  - テスト: prev_step_from_spec_agent_select_returns_none
  - テスト: build_launch_config_from_spec — 正しいブランチ名と設定

## Phase 6: app.rs統合 (US-1, US-2, US-3, US-4, US-5)

- [ ] T008: LaunchAgent インターセプト処理
  - `crates/gwt-tui/src/app.rs`
  - ManagementTab::Specs の handle_key で SpecsMessage::LaunchAgent をインターセプト:
    1. 選択中 SPEC の phase 確認 → draft/blocked なら specs_state.confirm_launch = true で中断
    2. ブランチ検索: metadata branches → git branch grep → 候補数で分岐
    3. 0件: branch_name = feature/SPEC-{N}、is_new_branch = true
    4. 1件: 自動選択
    5. 複数: specs_state.branch_select_mode = true で中断
    6. QuickStart 履歴読み込み（get_branch_tool_history）
    7. WizardState::open_for_spec() で Wizard 起動

- [ ] T009: ConfirmLaunch / SelectBranch インターセプト
  - `crates/gwt-tui/src/app.rs`
  - ConfirmLaunch → T008 のステップ2以降を実行
  - SelectBranch → 選択されたブランチで T008 のステップ6以降を実行

- [ ] T010: Agent起動後のmetadata書き戻し
  - `crates/gwt-tui/src/app.rs`
  - Wizard Complete（from_spec == true）時:
    1. spawn_agent_session() に auto_worktree = true 設定
    2. 起動成功後、save_spec_branch() で metadata.json 更新
    3. メインレイヤーに切替

## Phase 7: 統合検証

- [ ] T011: ビルド・テスト・lint検証
  - `cargo build -p gwt-tui`
  - `cargo test -p gwt-core -p gwt-tui`
  - `cargo clippy --all-targets --all-features -- -D warnings`

- [ ] T012: 手動E2E検証 (SC-001〜SC-007)
  - SPECs一覧 Shift+Enter → Wizard → Agent起動 → Agentペイン
  - SPECs詳細 Shift+Enter → 同上
  - draft phase → 確認ダイアログ → Y → 起動
  - 既存ブランチ自動検出 → Worktree再利用
  - 複数ブランチ候補 → 選択ダイアログ
  - metadata.json branches 配列書き込み確認
  - QuickStart履歴連携確認

## Traceability

| US | Tasks |
|----|-------|
| US-1 (一覧からAgent起動) | T003, T004, T007, T008 |
| US-2 (詳細からAgent起動) | T003, T004, T007, T008 |
| US-3 (Phase警告) | T005, T008, T009 |
| US-4 (ブランチ自動解決) | T001, T002, T006, T008, T009, T010 |
| US-5 (QuickStart連携) | T007, T008 |
