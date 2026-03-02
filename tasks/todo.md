# TODO: GitHub Copilot CLI 対応

## 背景

gwt に 5 番目の AI コーディングエージェントとして GitHub Copilot CLI（`copilot` コマンド、npm: `@github/copilot`）を追加する。既存の 4 エージェント（Claude Code / Codex / Gemini / OpenCode）と同じパターンに従い、最小限の変更で統合する。

仕様 Issue: #1411

## 実装ステップ

- [x] T000 gwt-spec Issue 作成 (#1411)
- [x] T001 Rust テスト追加（terminal.rs — TDD）
- [x] T002 フロントエンドテスト追加（TDD）
- [x] T003 terminal.rs — 5 つの match 関数に copilot アーム追加
- [x] T004 agents.rs — detect_copilot() 追加 + detect_agents 登録
- [x] T005 agentUtils.ts — AgentId 型 + inferAgentId に copilot 追加
- [x] T006 agentLaunchFormHelpers.ts — supportsModelFor() に copilot 追加
- [x] T007 AgentLaunchForm.svelte — modelOptions に copilot 用モデル一覧追加
- [x] T008 agentLaunchFormHelpers.test.ts — copilot テストアサーション追加
- [x] T009 agentUtils.test.ts — copilot テストアサーション追加
- [x] T010 cargo test 検証
- [x] T011 フロントエンドテスト検証（pnpm test）

## 検証結果

- [x] `cargo test -p gwt-tauri -- copilot` — 6 テスト全パス（548 テスト中）
- [x] `cd gwt-gui && pnpm test` — 65 ファイル / 1394 テスト全パス
- [x] `npx svelte-check` — エラー 0 件
