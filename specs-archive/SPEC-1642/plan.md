# Plan: SPEC-1642 — Docker 実行ターゲットとサービス検出

## Summary

Docker / DevContainer の launch target 検出とサービス選択導線だけを扱う plan に縮小する。runtime lifecycle や監視は `SPEC-1552` へ委譲する。

## Technical Context

- `crates/gwt-core/src/docker/*` の検出ロジックを再利用する。
- `crates/gwt-tui/src/screens/wizard.rs` と launch フローへサービス選択を接続する。
- Docker 利用不可時はホスト実行へ戻るエラー導線を定義する。

## Phased Implementation

### Phase 1: Scope Refresh

1. `SPEC-1552` との責務境界を spec.md と tasks.md に固定する。
2. launch target として必要な検出対象を compose / devcontainer に限定する。

### Phase 2: Launch Flow

1. 起動前にサービス選択が必要な条件と UI を定義する。
2. 選択結果を Agent launch config へ流す契約を定める。

### Phase 3: Verification

1. Docker 利用不可時の host fallback とログ出力を確認する。
