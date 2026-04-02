# Plan: SPEC-1775 — gwt-pr-check — 統合ステータスレポート

## Summary

既存の `check_pr_status.py` と `gwt-pr-fix` 補助スクリプトを正本に、PR 状態サマリの仕様と実装差分を埋める。

## Technical Context

- `.claude/skills/gwt-pr-check/scripts/check_pr_status.py`
- `.claude/skills/gwt-pr-fix/scripts/inspect_pr_checks.py`
- `SPEC-1643` が GitHub discovery/search の正本。

## Phased Implementation

### Phase 1: Inventory

1. 現行スクリプトが既に返している状態と spec 差分を棚卸しする。

### Phase 2: Output Contract

1. CI / merge / review / recommendation の出力形式を固定する。

### Phase 3: Verification

1. REST fallback と no-PR ケースを acceptance と検証手順へ落とす。
