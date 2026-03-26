<!-- GWT_SPEC_ARTIFACT:doc:tasks.md -->
doc:tasks.md

## Phase 0: Setup

- [ ] TASK-0-1: coverage matrix の評価対象領域を `crates/gwt-core/src/logging/logger.rs`, `crates/gwt-tauri/src/main.rs`, `crates/gwt-tauri/src/commands/report.rs`, `gwt-gui/src/lib/diagnostics.ts`, `gwt-gui/src/lib/components/ReportDialog.svelte` を起点に確定する
- [ ] TASK-0-2: `tracing` 呼び出しが高密度な優先 subsystem を `crates/gwt-tauri/src/commands/project.rs`, `terminal.rs`, `assistant.rs`, `settings.rs`, `issue.rs`, `system.rs`, `crates/gwt-core/src/worktree/manager.rs`, `git/repository.rs`, `git/branch.rs`, `docker/manager.rs`, `migration/executor.rs`, `config/settings.rs` に固定する

## Phase 1: Foundational

### Shared Logging Contract

- [ ] TASK-1-1: [P] テスト: `crates/gwt-core/src/logging/logger.rs` の JSON Lines 出力に必須フィールド契約を検証する unit test を追加する
- [ ] TASK-1-2: [P] テスト: `crates/gwt-tauri/src/commands/report.rs` の `read_recent_logs` が `profile.json` を対象にしないことを固定する unit test を追加する
- [ ] TASK-1-3: `crates/gwt-core/src/logging/logger.rs` に通常ログ用 field contract / helper を追加する
- [ ] TASK-1-4: `crates/gwt-core/src/logging/mod.rs` と `reader.rs` に contract 連携を反映する
- [ ] TASK-1-5: coverage matrix artifact または同等の判定表を `doc:*` artifact に追加し、feature / incident coverage の source of truth を固定する

## Phase 2: US-1 Feature Logging Coverage

### App / Project / Settings

- [ ] TASK-2-1: [P] テスト: `crates/gwt-tauri/src/main.rs`, `crates/gwt-tauri/src/app.rs`, `crates/gwt-tauri/src/commands/project.rs`, `crates/gwt-tauri/src/commands/settings.rs` で start/success/failure の不足箇所を検出する
- [ ] TASK-2-2: `crates/gwt-tauri/src/main.rs`, `crates/gwt-tauri/src/app.rs`, `crates/gwt-tauri/src/commands/project.rs`, `crates/gwt-tauri/src/commands/settings.rs` に feature-flow ログを補完する

### Git / Worktree / PR

- [ ] TASK-2-3: [P] テスト: `crates/gwt-core/src/worktree/manager.rs`, `crates/gwt-core/src/git/repository.rs`, `crates/gwt-core/src/git/branch.rs`, `crates/gwt-tauri/src/commands/issue.rs` の feature-flow coverage を検証する
- [ ] TASK-2-4: `crates/gwt-core/src/worktree/manager.rs`, `crates/gwt-core/src/git/repository.rs`, `crates/gwt-core/src/git/branch.rs`, `crates/gwt-tauri/src/commands/issue.rs` に start/success/failure ログを補完する

### Agent / Terminal / Docker / Migration

- [ ] TASK-2-5: [P] テスト: `crates/gwt-tauri/src/commands/assistant.rs`, `crates/gwt-tauri/src/commands/terminal.rs`, `crates/gwt-core/src/docker/manager.rs`, `crates/gwt-core/src/migration/executor.rs` の feature-flow coverage を検証する
- [ ] TASK-2-6: `crates/gwt-tauri/src/commands/assistant.rs`, `crates/gwt-tauri/src/commands/terminal.rs`, `crates/gwt-core/src/docker/manager.rs`, `crates/gwt-core/src/migration/executor.rs` に feature-flow ログを補完する

## Phase 3: US-2 Incident Response Coverage

- [ ] TASK-3-1: [P] テスト: git/github/docker/network/process/fs 失敗時に `error_code` / `error_detail` / context が残ることを `crates/gwt-tauri/src/commands/project.rs`, `terminal.rs`, `assistant.rs`, `system.rs`, `crates/gwt-core/src/worktree/manager.rs`, `git/repository.rs`, `docker/manager.rs`, `migration/executor.rs` で検証する
- [ ] TASK-3-2: 上記モジュールに障害対応ログの context fields を追加する
- [ ] TASK-3-3: `crates/gwt-core/src/config/settings.rs` と `crates/gwt-tauri/src/commands/settings.rs` で configuration failure の障害対応ログを補完する

## Phase 4: US-3 Report and Diagnostics

- [ ] TASK-4-1: [P] テスト: `gwt-gui/src/lib/diagnostics.ts`, `gwt-gui/src/lib/components/ReportDialog.svelte`, `gwt-gui/src/lib/issueTemplate.ts`, `crates/gwt-tauri/src/commands/report.rs` で Application Logs 導線が通常ログだけを収集することを固定する
- [ ] TASK-4-2: `crates/gwt-tauri/src/commands/report.rs` の candidate selection と `gwt-gui/src/lib/diagnostics.ts` / `ReportDialog.svelte` の説明文を、通常ログと profiling の責務分離に合わせて調整する
- [ ] TASK-4-3: privacy / masking の観点で `gwt-gui/src/lib/privacyMask.ts`, `gwt-gui/src/lib/diagnostics.ts`, `crates/gwt-tauri/src/commands/report.rs` を見直し、障害対応ログ強化で機密情報が漏れないことを検証する

## Phase 5: Polish / Cross-Cutting

- [ ] TASK-5-1: feature coverage matrix と incident coverage matrix の 90% 判定を更新し、ギャップ一覧を artifact に反映する
- [ ] TASK-5-2: [P] `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --check`, `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` を実行し、ログ仕様変更に伴う検証を完了する
- [ ] TASK-5-3: README か reviewer 向けドキュメントで profiling と通常ログの違いを説明し、Issue report の Application Logs が通常ログ専用であることを明記する

## Traceability Matrix

| User Story | Tasks |
|-----------|-------|
| US-1 | TASK-1-1, TASK-1-3, TASK-2-1, TASK-2-2, TASK-2-3, TASK-2-4, TASK-2-5, TASK-2-6 |
| US-2 | TASK-1-1, TASK-3-1, TASK-3-2, TASK-3-3, TASK-5-1 |
| US-3 | TASK-1-2, TASK-4-1, TASK-4-2, TASK-4-3 |
| US-4 | TASK-1-3, TASK-1-4, TASK-5-1, TASK-5-3 |

| Acceptance Scenario | Verification Task |
|--------------------|------------------|
| AS-1 | TASK-2-1, TASK-2-2 |
| AS-2 | TASK-2-1, TASK-2-2 |
| AS-3 | TASK-3-1, TASK-3-2 |
| AS-4 | TASK-3-1, TASK-3-2 |
| AS-5 | TASK-1-2, TASK-4-1, TASK-4-2 |
| AS-6 | TASK-1-2, TASK-4-1 |
| AS-7 | TASK-5-1 |
| AS-8 | TASK-5-1 |
