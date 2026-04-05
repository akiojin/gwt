# Progress: SPEC-1782

## 2026-04-01: T001-T007 完了

### Progress

- T001: detect_session_id_for_tool() を gwt-core に移植（12 テスト通過）
  - `crates/gwt-core/src/ai/session_detect.rs` 新規作成
  - Claude/Codex/Gemini/OpenCode の session_id 検出を実装
- T002: Quick Start ステップの表示条件と選択動作を変更
  - session_id がある最新 1 ツールのみ表示
  - Resume/Start New でワンクリック WizardAction::Complete
  - apply_quick_start_selection() で reasoning_level/fast_mode/collaboration_modes を含む全設定復元
- T003: ExecutionMode ステップ削除
  - next_step()/prev_step() から ExecutionMode 分岐削除
  - WizardExecutionMode::Continue 削除
  - VersionSelect → SkipPermissions 直接遷移
- T004: Quick Start フラット 3 項目 UI（既存 render_quick_start を活用）
- T005: Quick Start 履歴読み込みと SessionMode ブリッジ
  - load_quick_start_history(): session_id がある最新 1 ツールをフィルタ
  - spawn_agent_session() で WizardExecutionMode → SessionMode 変換
  - fast_mode/reasoning_level を AgentLaunchBuilder に渡す
- T006: 履歴保存改善と session_id 自動検出
  - save_session_entry() で tool_label=display_name, session_id, mode, reasoning_level を保存
  - バックグラウンドスレッドで detect_session_id_for_tool() → save_session_entry() 更新
- T007: ビルド・テスト・lint 検証通過
  - cargo build: OK
  - cargo test: 1604 (gwt-core) + 315 (gwt-tui) = 1919 テスト通過
  - cargo clippy: クリーン

### Done

T001-T007 全完了。T008（手動 E2E 検証）が残り。

### Next

- T008: 手動 E2E 検証（Quick Start 表示、Resume/Start New/Choose Different、session_id 検出）
