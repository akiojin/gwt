> **⚠️ DEPRECATED (SPEC-1776)**: This SPEC describes GUI-only functionality (Tauri/Svelte/xterm.js) that has been superseded by the gwt-tui migration. The gwt-tui equivalent is defined in SPEC-1776.

<!-- GWT_SPEC_ARTIFACT:doc:spec.md -->
doc:spec.md

# Feature Specification: パフォーマンスプロファイリング基盤

## Background

gwt には profiling 基盤が導入され、Chrome Trace (`profile.json`) と heartbeat/watchdog、frontend invoke RTT が取得できるようになったが、実際に知りたい「project を開いてから UI が安定するまでの所要時間」と「どの関数/blocking 境界が詰まっているか」はまだ直接は読めない。

現在の startup/hydration は `open_project` または cold-start restore の後に、frontend 側で `fetchCurrentBranch`、`refreshCanvasWorktrees`、`restoreProjectAgentTabs` が非同期に走る。ユーザー体感の遅さやブロッキングを見つけるには、この hydration 完了点を明示的に計測し、さらに backend 側の blocking path に深い span を追加する必要がある。

本 rev2 では、既存 profiling を拡張し、project 起動から frontend hydrated までの total time と、起動/hydration 経路上の deep trace を取得可能にする。

## User Stories

### User Story 1 - tracing 有効化 (Priority: P0)
As a developer, I want tracing output to actually work so that I can see existing debug/info/warn/error logs when investigating issues.

### User Story 2 - Settings からの Chrome Trace プロファイリング (Priority: P1)
As a developer, I want to enable profiling from the Settings > Developer tab so that I can generate Chrome Trace Event Format output (profile.json) without editing config files manually.

### User Story 3 - Tauri コマンド計測 (Priority: P1)
As a developer, I want Tauri commands and startup-related hot paths instrumented so that I can see execution times and identify bottlenecks.

### User Story 4 - フリーズ検知 (Priority: P1)
As a developer, I want a heartbeat/watchdog mechanism that detects and logs freezes so that I can identify when and where the application hangs.

### User Story 5 - フロントエンド計測 (Priority: P2)
As a developer, I want invoke round-trip times measured and reported so that I can identify slow frontend-backend communication.

### User Story 6 - project 起動 total time 計測 (Priority: P0)
As a developer, I want the total time from project open / session restore start to frontend hydrated so that I can measure the startup path users actually feel.

### User Story 7 - ブロッキング関数の洗い出し (Priority: P0)
As a developer, I want deep trace spans around startup and hydration hot paths so that I can identify which function or blocking boundary is stalling startup.

### User Story 8 - UI インジケータ (Priority: P2)
As a user, I want to see a PROFILING badge when profiling is active so that I do not accidentally leave it on.

## Acceptance Scenarios

1. Given `profiling=false` (default), when the app starts, then no `profile.json` is created and there is zero measurable overhead
2. Given profiling is enabled from Settings > Developer, when the app starts or settings are saved and the user performs operations, then `profile.json` is generated with Chrome Trace Event Format spans
3. Given profiling is enabled, when `profile.json` is opened in `chrome://tracing` or Perfetto, then Tauri command spans and startup-related backend spans are visible
4. Given the app freezes for >3 seconds, when profiling is enabled, then a WARN log with freeze duration is emitted
5. Given `profiling=true`, when frontend invokes Tauri commands, then round-trip times are measured and reported to backend logs with command name and duration
6. Given the user opens a project, when `open_project` starts and the frontend completes the hydration set for the same token, then `project_start.open_project.total` and subphase durations are recorded
7. Given the app restores a window session on cold start, when restore begins and the frontend completes the hydration set for the same token, then `project_start.restore_session.total` and subphase durations are recorded
8. Given project hydration is still running, when the user switches project or a new token starts, then stale startup metrics for the previous token are discarded
9. Given profiling is enabled, when startup is slow, then `open_project`, `list_worktree_branches`, `list_worktrees`, `list_terminals`, `prefetch_version_history_inner`, or background index bootstrap spans show the blocking section in trace/logs
10. Given the `PROFILING` badge is shown in the status bar, when the user sees it, then they know profiling is active

## Edge Cases

- Project open と cold-start restore は別 metric 名で記録し、混在させない
- `stable` は background prefetch 完了ではなく frontend hydration 完了を意味する
- `fetchCurrentBranch` / `refreshCanvasWorktrees` / `restoreProjectAgentTabs` のうち一部が失敗しても、token 単位で completion を判定し、失敗フラグを metrics に残す
- 別 project への切り替えや token mismatch 時は古い測定結果を送信しない
- `profile.json` は長時間実行で肥大化しうるため、deep trace は startup/hydration path に集中させる
- profiling 有効時でも通常ログ・Issue report 導線は壊してはならない

## Functional Requirements

- FR-001: Settings に `profiling: bool` フィールドを保持する（default: false, `serde(default)`）
- FR-001a: Settings 画面に `Developer` タブを追加し、profiling の ON/OFF を切り替えるトグルを表示する
- FR-002: `init_logger()` を `main.rs` から呼び出し、既存の tracing 出力を有効化する
- FR-003: `profiling=true` 時に `tracing-chrome` レイヤーで Chrome Trace Event Format を出力する
- FR-004: Tauri commands と startup-related hot paths を `#[instrument]` または明示 span で計装する
- FR-005: heartbeat Tauri コマンドと watchdog background task でフリーズを検知する
- FR-006: フロントエンド invoke のラウンドトリップ時間を計測する
- FR-007: profiling 有効時にステータスバーへ `PROFILING` バッジを表示する
- FR-008: `report_frontend_metrics` でフロントエンドメトリクスをバックエンドログへ記録する
- FR-009: project startup metrics は `open_project` 経路と cold-start restore 経路を別名で計測しなければならない
- FR-010: startup total time の完了点は **Frontend Hydrated** とし、同一 token の `fetchCurrentBranch`、`refreshCanvasWorktrees`、`restoreProjectAgentTabs` が全て settle した時点とする
- FR-011: frontend metrics payload は invoke RTT と startup/hydration measure の両方を表現できなければならない
- FR-012: `list_worktree_branches`、`list_worktrees`、`list_terminals`、`prefetch_version_history_inner`、project index bootstrap、issue cache bootstrap に deep trace を追加しなければならない
- FR-013: startup/hydration の主要 slow path には warn threshold を持たせ、遅延時に構造化 WARN を出力しなければならない
- FR-014: stale token の startup metrics は破棄し、別 project/path の hydration と混線させてはならない

## Non-Functional Requirements

- NFR-001: `profiling=false` 時のオーバーヘッドはゼロに近いこと
- NFR-002: `profiling=true` 時の追加オーバーヘッドは startup/hydration path の原因分析に必要な範囲に留めること
- NFR-003: `profile.json` は既存出力先に維持されること
- NFR-004: frontend metrics は起動 token と phase 名で相関できること
- NFR-005: profiling 拡張後も既存テストが通ること

## Success Criteria

- SC-001: `profiling=true` で `open_project` から frontend hydrated までの total time を取得できる
- SC-002: `profiling=true` で cold-start restore から frontend hydrated までの total time を取得できる
- SC-003: `profile.json` または profiling logs から startup/hydration の blocking 関数を追跡できる
- SC-004: stale token / project switch 時に誤った startup total が記録されない
- SC-005: invoke RTT と startup metrics が共存し、既存 profiling UX を壊さない
