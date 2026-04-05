# Plan: SPEC-1645 — 設定画面と設定カテゴリ構成

## Summary

現行 Settings タブのカテゴリ構成を正本化し、個別設定仕様との境界を切る。

## Technical Context

- `crates/gwt-tui/src/screens/settings.rs` の現カテゴリを正本にする。
- Voice / Custom Agent / Docker など詳細仕様は子 SPEC へ委譲する。
- 永続化は `gwt-core` の設定ストレージに委譲する。

## Phased Implementation

### Phase 1: Category Model

1. General / Worktree / Agent / Custom / Env / AI のカテゴリ責務を整理する。

### Phase 2: Editing Flow

1. 一覧編集とフォーム編集の切替を定義する。
2. 保存失敗時の validation / recovery を定義する。

### Phase 3: Verification

1. Settings 画面のカテゴリ移動、編集、保存の受け入れ条件を揃える。
