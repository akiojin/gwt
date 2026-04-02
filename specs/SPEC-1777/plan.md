# Plan: SPEC-1777 — SPECs タブ — 一覧・詳細・検索

## Summary

現在の `screens/specs.rs` 実装を正本に plan/tasks を backfill し、残る仕様差分だけを明示する。

## Technical Context

- `crates/gwt-tui/src/screens/specs.rs`
- `crates/gwt-tui/src/app.rs`
- `SPEC-1784` が semantic search、`SPEC-1785` が SPEC 起点 launch を担当。

## Phased Implementation

### Phase 1: Inventory

1. 現在の一覧・詳細・検索・Markdown 表示の実装状態を固定する。

### Phase 2: Gap Handling

1. ソートや watcher のような未実装・不要要件を整理する。

### Phase 3: Closure

1. 残差分がなければ close 候補へ移す。
