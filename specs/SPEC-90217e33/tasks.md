---
description: "SPEC-90217e33 実装タスク"
---

# タスク: gwt GUI コーディングエージェント機能のTUI完全移行

**仕様**: `specs/SPEC-90217e33/spec.md`  
**計画**: `specs/SPEC-90217e33/plan.md`

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能
- **[ストーリー]**: US1..US4

## フェーズ0: ドキュメント/下準備

- [ ] **T001** [P] [共通] `specs/SPEC-90217e33/spec.md` の記述を最終確認し、`specs/specs.md` を再生成する（`.specify/scripts/bash/update-specs-index.sh`）
- [ ] **T002** [共通] `PLANS.md` を本仕様に合わせて更新する

## US1（P0）: Mode/Skip/Reasoning/Version を指定して起動

- [ ] **T101** [US1] `crates/gwt-tauri/src/commands/terminal.rs` の `LaunchAgentRequest` を拡張（mode/skip/reasoning/collaboration/toolVersion/extraArgs/envOverrides/resumeSessionId）
- [ ] **T102** [US1] `crates/gwt-tauri/src/commands/terminal.rs` にエージェント別の引数組み立てヘルパーを追加し、ユニットテストを追加する
- [ ] **T103** [US1] `gwt-gui/src/lib/components/AgentLaunchForm.svelte` に Session Mode/Skip/Reasoning/Runner/Extra Args/Env overrides UI を追加し、`launch_agent` へ渡す

## US2（P0）: Quick Start + 履歴（起動時追記/終了後sessionId検出）

- [ ] **T201** [US2] `crates/gwt-tauri/src/commands/session.rs`（新規 or 既存適所）に `get_branch_quick_start` コマンドを追加し、`gwt_core::config::get_branch_tool_history` を返す
- [ ] **T202** [US2] `gwt-gui/src/lib/components/MainArea.svelte`（Summary）に Quick Start 表示を追加し、Continue/New をウィザード無しで実行できるようにする
- [ ] **T203** [US2] `crates/gwt-tauri/src/commands/terminal.rs` で `save_session_entry` による履歴追記（起動時）を追加する（失敗しても起動を継続）
- [ ] **T204** [US2] `crates/gwt-core/src/ai/session_parser/claude.rs` の worktree フィルタを修正し、worktree → project dir 解決を安定化する（ユニットテスト追加）
- [ ] **T205** [US2] `crates/gwt-tauri/src/commands/terminal.rs` でエージェント終了後に sessionId 検出を試み、成功時のみ `save_session_entry` で追記する（ベストエフォート）

## US3（P1）: ブランチ一覧に直近ツール表示 + 色分け

- [ ] **T301** [US3] `crates/gwt-tauri/src/commands/branches.rs` の `BranchInfo` に `last_tool_usage` を追加し、`ts_session::get_last_tool_usage_map` を使って付与する
- [ ] **T302** [US3] `gwt-gui/src/lib/types.ts` と `gwt-gui/src/lib/components/Sidebar.svelte` を更新し、`last_tool_usage` を表示する（ツール種別で色分け）

## US4（P1）: OpenCode 対応

- [ ] **T401** [US4] `crates/gwt-tauri/src/commands/terminal.rs` / `crates/gwt-tauri/src/commands/agents.rs` に OpenCode の定義を追加する（detect + launch）
- [ ] **T402** [US4] `gwt-gui/src/lib/components/AgentLaunchForm.svelte` の Agent カードに OpenCode を表示し、モデル未指定でも起動が止まらないことを確認する

## 検証/デリバリー

- [ ] **T901** [検証] `cargo test`
- [ ] **T902** [検証] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] **T903** [検証] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json`
- [ ] **T904** [デリバリー] Conventional Commits で分割コミットし、各コミットで `bunx commitlint --from HEAD~1 --to HEAD` を通して push する

