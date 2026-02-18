---
description: "SPEC-488af8e2 実装タスク"
---

# タスク: gwt GUI Docker Compose 統合（起動ウィザード + Quick Start）

**仕様**: `specs/SPEC-488af8e2/spec.md`  
**計画**: `specs/SPEC-488af8e2/plan.md`

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能
- **[ストーリー]**: US1..US3

## フェーズ0: ドキュメント/下準備

- [ ] **T001** [P] [共通] `specs/SPEC-488af8e2/spec.md` を最終確認し、`specs/specs.md` を再生成する（`.specify/scripts/bash/update-specs-index.sh`）
- [ ] **T002** [共通] `PLANS.md` を本仕様に合わせて更新する

## US1（P0）: Compose 検出と起動ウィザード

- [ ] **T101** [US1] `crates/gwt-core/src/docker/` に compose services 抽出ヘルパーを追加し、ユニットテストを追加する
- [ ] **T102** [US1] `crates/gwt-tauri/src/commands/` に `detect_docker_context` コマンドを追加する（Settings の docker.force_host / docker 可用性 / compose services）
- [ ] **T103** [US1] `gwt-gui/src/lib/types.ts` に `DockerContext` と `LaunchAgentRequest` の docker fields を追加する
- [ ] **T104** [US1] `gwt-gui/src/App.svelte` から `AgentLaunchForm` に `projectPath` を渡し、`AgentLaunchForm` で `detect_docker_context` を呼んで Docker セクションを表示する

## US2（P0）: Docker 起動 + コンテナ内 exec

- [ ] **T201** [US2] `crates/gwt-tauri/src/commands/terminal.rs` の `LaunchAgentRequest` に docker fields を追加する
- [ ] **T202** [US2] `crates/gwt-tauri/src/commands/terminal.rs` に docker compose up/down + exec 起動（PTY）の実装を追加し、ユニットテストを追加する（引数生成）
- [ ] **T203** [US2] `crates/gwt-tauri/src/state.rs` の `PaneLaunchMeta` に docker fields を追加し、終了時の履歴追記と down 実行に利用する
- [ ] **T204** [US2] `crates/gwt-tauri/src/commands/terminal.rs` の `ToolSessionEntry` 保存に docker fields を反映する（Launch/Exit）

## US3（P1）: Quick Start 復元

- [ ] **T301** [US3] `gwt-gui/src/lib/types.ts` の `ToolSessionEntry` に docker fields を追加する
- [ ] **T302** [US3] `gwt-gui/src/lib/components/MainArea.svelte` の Quick Start 起動リクエストに docker fields を含めて復元する
- [ ] **T303** [US3] `gwt-gui/src/lib/components/Sidebar.svelte` の `normalizeBranchName` 等の処理に影響が無いことを確認する

## 検証/デリバリー

- [ ] **T901** [検証] `cargo test`
- [ ] **T902** [検証] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] **T903** [検証] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json`
- [ ] **T904** [デリバリー] Conventional Commits で分割コミットし、各コミットで `bunx commitlint --from HEAD~1 --to HEAD` を通して push する
