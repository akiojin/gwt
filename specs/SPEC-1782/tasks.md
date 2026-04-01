# Tasks: SPEC-1782 — Quick Start ワンクリックエージェント起動

## Phase 1: session_id 検出基盤 (US-4)

- [x] T001: [TDD] detect_session_id_for_tool() を gwt-core に移植
  - `crates/gwt-core/src/ai/session_detect.rs` (新規)
  - gwt-cli の detect_claude/codex/gemini/opencode_session_id_at() を移植
  - `encode_claude_project_path()` (`claude_paths.rs`) を再利用
  - `crates/gwt-core/src/ai/mod.rs` から pub export
  - テスト: 各エージェント形式の tempdir テスト + ファイルなし/不一致で None

## Phase 2: Quick Start Wizard 再設計 (US-1, US-2, US-3)

- [x] T002: [TDD] Quick Start ステップの表示条件と選択動作を変更
  - `crates/gwt-tui/src/screens/wizard.rs`
  - session_id がある最新 1 ツールのみ表示、なければスキップ (FR-001〜FR-003)
  - Resume / Start New でワンクリック WizardAction::Complete (FR-012)
  - Choose Different → BranchAction 遷移 (FR-040)
  - apply_quick_start_selection() で全設定復元 (FR-021)
  - QuickStartEntry に reasoning_level / fast_mode / collaboration_modes 追加

- [x] T003: [TDD] ExecutionMode ステップ削除
  - `crates/gwt-tui/src/screens/wizard.rs`
  - next_step() / prev_step() から ExecutionMode 分岐削除 (FR-060)
  - WizardExecutionMode::Continue 削除
  - フルウィザードは常に Normal モード

- [x] T004: Quick Start フラット 3 項目 UI レンダリング
  - `crates/gwt-tui/src/screens/wizard.rs`
  - タイトル: "Quick Start — {Agent名} ({Model})" (FR-011)
  - Resume session ({id 先頭 8 文字}...) / Start new session / Choose different settings (FR-010)

## Phase 3: 統合レイヤー (US-1, US-2, US-4)

- [x] T005: [TDD] Quick Start 履歴読み込みと SessionMode ブリッジ
  - `crates/gwt-tui/src/app.rs`
  - load_quick_start_history(): session_id がある最新 1 ツールをフィルタして返す
  - OpenWizard / BranchesMessage::Enter で呼び出し (FR-001, FR-002)
  - spawn_agent_session() で WizardExecutionMode → SessionMode 変換 (FR-020)
  - resume_session_id / fast_mode / reasoning_level を builder に渡す

- [x] T006: [TDD] 履歴保存改善と session_id 自動検出
  - `crates/gwt-tui/src/app.rs`
  - save_session_entry() で tool_label=display_name, session_id, mode, reasoning_level を保存 (FR-070, FR-071)
  - エージェント起動後にバックグラウンドで detect_session_id_for_tool() → save_session_entry() 更新 (FR-050, FR-051, NFR-002)

## Phase 4: 統合検証

- [x] T007: ビルド・テスト・lint 検証
  - `cargo build -p gwt-tui` / `cargo test -p gwt-core -p gwt-tui` / `cargo clippy --all-targets --all-features -- -D warnings`

- [ ] T008: 手動 E2E 検証 (SC-001〜SC-006 + Edge Cases)
  - Quick Start 表示 → Resume 即起動 → Start New 即起動 → Choose Different フルウィザード遷移
  - session_id 検出フロー → 次回 Quick Start 使用可能
  - session_id なしブランチ → Quick Start スキップ
  - 失効 session_id で Resume → エージェントが PTY にエラー表示（gwt 側クラッシュなし）

## Traceability

| US | Tasks |
|----|-------|
| US-1 (Resume) | T002, T004, T005, T008 |
| US-2 (Start New) | T002, T004, T005, T006, T008 |
| US-3 (Choose Different) | T002, T003, T004, T008 |
| US-4 (session_id 検出) | T001, T006, T008 |
