# TDD計画と結果: SPEC-8ad13230

**仕様ID**: `SPEC-8ad13230` | **日付**: 2026-02-17

## テスト戦略

- Unit test を中心に parser/merge/enum parsing を先に固定し、実装を追随させる。
- 外部 API（GitHub）依存部分は副作用を持たない helper 単位で検証する。

## RED -> GREEN の対象

1. `issue_spec` の section parser が `TDD` と legacy `Checklist` を扱えること。
2. artifact comment parser が marker 形式と legacy 形式を扱えること。
3. `agent_master` の spec マージが `_TODO_` 置換と追記を行うこと。
4. `commands::issue_spec` の artifact kind parse が `contract/checklist` を受理すること。

## 実行ログ

- [x] `cargo test -p gwt-core issue_spec -- --nocapture`
- [x] `cargo test -p gwt-tauri agent_master::tests -- --nocapture`
- [x] `cargo test -p gwt-tauri commands::issue_spec::tests -- --nocapture`
- [x] `cargo test -p gwt-tauri agent_tools::tests -- --nocapture`
- [x] `python3 -m py_compile scripts/gwt_issue_spec_mcp.py`
- [x] `pnpm vitest run src/lib/components/AgentModePanel.test.ts src/lib/components/MainArea.test.ts`

## 既知の非対象失敗

- `pnpm test` 全体では `WorktreeSummaryPanel.test.ts` が既存理由で失敗する（本仕様の変更範囲外）。
