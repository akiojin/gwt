# 通知とエラーバス

> **Canonical Boundary**: 本 SPEC は gwt-tui の status bar / modal / log 連携による通知経路を扱う。Tauri OS 通知前提は捨て、TUI 内の可観測性を正本とする。

## Background

- gwt-tui では status bar、error queue、confirm/modal、structured logs が通知経路として存在する。
- 既存の SPEC-1651 は Tauri 時代の toast / OS 通知を前提にしており、現行 TUI 実装と一致していない。
- 本 SPEC は「どのイベントをどの UI レイヤへ出すか」を定義し、通知とエラーの責務を統一する。

## User Stories

### US-1: 軽微な通知を status bar で確認する

開発者として、保存成功や軽微な警告を作業を中断せず確認したい。

### US-2: 重大な失敗を modal または error queue で確認する

開発者として、Agent 起動失敗や destructive action の失敗を見逃したくない。

### US-3: 失敗を Logs タブで追跡する

開発者として、UI 通知だけでなく structured log から根本原因を調査したい。

## Acceptance Scenarios

1. 軽微な成功/警告は status bar や一時的メッセージで表示される。
2. 重大な失敗は modal または error queue に残り、即座に消えない。
3. 同じ失敗は structured logs にも記録され、Logs タブから追跡できる。
4. 通知表示中でも PTY 出力や管理画面操作が壊れない。
5. OS 通知がなくても gwt 内部だけで失敗調査が完結する。

## Edge Cases

- 短時間に大量の warning/error が発生する。
- 同一失敗が UI と log のどちらか片方にしか出ない。
- 終了済みセッションの最後のエラーを読む前に UI から消える。

## Functional Requirements

- FR-001: 軽微な通知は status bar など非モーダル経路へ出す。
- FR-002: 重大な失敗は modal または error queue に残す。
- FR-003: UI へ出した失敗は Logs タブでも追跡可能にする。
- FR-004: 通知 severity の基準を gwt-core / gwt-tui 間で一致させる。
- FR-005: Tauri 依存の OS 通知は本 SPEC の正本要件に含めない。

## Success Criteria

- 通知経路が TUI 実装と一致する。
- 重大エラーを見失わない。
- UI 通知と Logs の役割分担が明確になる。
