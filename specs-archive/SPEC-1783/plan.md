# Plan: SPEC-1783 — ヘルプオーバーレイ

## Summary

現在未接続の `ShowHelp` アクションを overlay 実装へ結び、key binding 参照画面を完成させる。

## Technical Context

- `crates/gwt-tui/src/input/keybind.rs`
- `crates/gwt-tui/src/app.rs`
- `crates/gwt-tui/src/model.rs` overlay state

## Phased Implementation

### Phase 1: Wiring

1. ShowHelp アクションと overlay mode を接続する。

### Phase 2: Rendering

1. キーバインド一覧をカテゴリ別に描画する。

### Phase 3: Verification

1. 開閉・スクロール・context highlight を acceptance に落とす。
