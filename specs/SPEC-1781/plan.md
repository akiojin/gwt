# Plan: SPEC-1781 — AI ブランチ命名

## Summary

既存の `AIBranchSuggest` ステップを正本にし、実際の suggestion backend と手動 fallback の契約を固める。

## Technical Context

- `crates/gwt-tui/src/screens/wizard.rs`
- `gwt-core` の branch suggest API
- `SPEC-1644` が branch validation を担当。

## Phased Implementation

### Phase 1: Inventory

1. 現行 wizard step と未接続の backend を棚卸しする。

### Phase 2: Suggestion Contract

1. AI 候補生成、手動入力、validation の契約を定義する。

### Phase 3: Verification

1. 成功・timeout・manual fallback の acceptance を揃える。
