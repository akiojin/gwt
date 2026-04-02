# Plan: SPEC-1652 — ビルドと配布パイプライン

## Summary

現行の gwt-tui release pipeline を正本化し、Tauri build / auto-update 前提を除去する。

## Technical Context

- README と GitHub Release、Conventional Commits、git-cliff を前提とする。
- release PR は develop から main への導線で運用する。
- インストーラスクリプトと Release asset 名の整合が必要。

## Phased Implementation

### Phase 1: Current Pipeline Refresh

1. 現行 README / workflow / release flow を spec に反映する。

### Phase 2: Release Contract

1. version 判定、CHANGELOG 生成、asset 公開の契約を定義する。

### Phase 3: Verification

1. build / lint / test と release asset 導線の受け入れ条件を揃える。
