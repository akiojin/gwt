# Plan: SPEC-1651 — 通知とエラーバス

## Summary

Tauri 前提の toast / OS notification を捨て、status bar・modal・error queue・logs の TUI 通知経路に再定義する。

## Technical Context

- `crates/gwt-tui/src/widgets/status_bar.rs` と `screens/error.rs` が UI 面の中心。
- severity は `gwt-core` と `gwt-tui` で揃える必要がある。
- 失敗は Logs タブでも追跡できる必要がある。

## Phased Implementation

### Phase 1: Notification Surfaces

1. 非モーダル通知と重大エラーの出し分けを定義する。

### Phase 2: Error Bus

1. severity と log 連携の責務を揃える。

### Phase 3: Verification

1. UI と Logs の両方で失敗追跡できることを確認する。
