<!-- GWT_SPEC_ARTIFACT:doc:plan.md -->
doc:plan.md

# Plan: パフォーマンスプロファイリング基盤 (#1705)

## Summary

既存 profiling に「project open / cold-start restore から frontend hydrated までの startup total time」と「startup/hydration path の deep trace」を追加する。frontend は `performance.mark/measure` で phase を刻み、backend は `#[instrument]` と明示 span で blocking 境界を trace 化する。

## Technical Context

### 現状アーキテクチャ

- backend profiling は `logger.rs` の `tracing-chrome` と `commands/system.rs` の heartbeat / `report_frontend_metrics` が中心
- frontend profiling は `profiling.svelte.ts` の invoke RTT バッチ送信まで。startup/hydration token の概念は無い
- `App.svelte` では `openProjectAndApplyCurrentWindow()` と `restoreCurrentWindowSession()` が project 起動入口となり、その後 `handleOpenedProjectPath()` が `fetchCurrentBranch`、`refreshCanvasWorktrees`、`restoreProjectAgentTabs` を並列に走らせる
- startup の実ユーザー体感は `open_project` 単体ではなく、上記 hydration 3 本が落ち着くまでで決まる
- startup 直後に backend 側では `prefetch_version_history_inner`、`spawn_background_index`、issue cache bootstrap/full sync も走るが、これらは background であり stable 条件には含めない

### 影響ファイル / モジュール

- frontend: `gwt-gui/src/App.svelte`, `gwt-gui/src/lib/profiling.svelte.ts`, `gwt-gui/src/lib/tauriInvoke.ts`, `gwt-gui/src/lib/windowSessionRestore.ts`
- backend profiling: `crates/gwt-tauri/src/commands/system.rs`, `crates/gwt-tauri/src/app.rs`, `crates/gwt-tauri/src/commands/project.rs`
- startup/hydration hot paths: `crates/gwt-tauri/src/commands/branches.rs`, `cleanup.rs`, `terminal.rs`, `version_history.rs`, `project_index.rs`

### 設計方針

- `stable` は **Frontend Hydrated** に固定する
- startup metrics は token 単位で追跡し、古い token は捨てる
- invoke RTT は既存の仕組みを維持し、payload を拡張して startup measures を同居させる
- deep trace は repo 全 private helper ではなく、startup/hydration path とその blocking 境界に集中させる

## Constitution Check

- Spec Before Implementation: ✅ rev2 要件と tasks を先に更新する
- Test-First Delivery: ✅ startup token 管理と payload 拡張は先にテストで固定する
- No Workaround-First Changes: ✅ `open_project` 単体でなく hydration 完了まで計測して根本の体感遅延を捉える
- Minimal Complexity: ✅ 新しい profiling サブシステムは作らず、既存 `profiling.svelte.ts` と `report_frontend_metrics` を拡張する
- Verifiable Completion: ✅ startup total, token discard, deep trace spans を acceptance で確認可能

## Complexity Tracking

| 項目 | 複雑度 | 理由 |
|------|--------|------|
| startup token 管理 | Medium | frontend の並列 hydration を 1 つの total に束ねる必要がある |
| frontend metric payload 拡張 | Low | 既存コマンドを保ったまま schema を広げるだけ |
| deep trace の追加 | Medium | startup/hydration path の blocking 境界を絞って span を足す必要がある |
| slow-path warn threshold | Low | トラブルシュートしやすくするための補助ログ |

## Phased Implementation

### Phase 1: Spec rev2 + frontend metric contract
- `#1705` artifact を rev2 に更新
- frontend metric payload を startup/invoke 共通 schema に拡張
- startup token / stable 定義をコードと spec で一致させる

### Phase 2: Frontend startup instrumentation
- `openProjectAndApplyCurrentWindow()` と `restoreCurrentWindowSession()` の開始点で startup token を開始
- `fetchCurrentBranch`、`refreshCanvasWorktrees`、`restoreProjectAgentTabs` を token に紐づけて measure
- 3 本が settle したら `frontend_hydrated.total` を送信

### Phase 3: Backend deep trace
- `open_project` の subphase span を追加
- `list_worktree_branches`, `list_worktrees`, `list_terminals`, `prefetch_version_history_inner`, `spawn_background_index` などの blocking hot path を span 化
- 主要 slow path に warn threshold を追加

### Phase 4: Verification
- frontend token discard / restore path / total time のテスト
- backend payload / span path / slow threshold のテスト
- profiling を有効にして trace/log から startup blocker を確認する
