# Plan: SPEC-1779 — カスタムエージェント登録

## Summary

既存の Settings CustomAgents UI と `tools.rs` を正本に、custom agent 仕様を plan/tasks と同期させる。

## Technical Context

- `crates/gwt-tui/src/screens/settings.rs`
- `crates/gwt-core/src/config/tools.rs`
- `SPEC-1646` が built-in agent runtime を担当。

## Phased Implementation

### Phase 1: Inventory

1. 既存 CRUD / validation / persistence の実装範囲を固定する。

### Phase 2: Integration

1. Wizard / launch builder との境界を明確にする。

### Phase 3: Closure

1. 残差分がなければ close 候補へ移す。
