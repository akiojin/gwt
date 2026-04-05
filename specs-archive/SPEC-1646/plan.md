# Plan: SPEC-1646 — エージェント検出・起動・ライフサイクル

## Summary

ビルトイン Agent の catalog / detection / version / launch contract に責務を絞り、Assistant 制御と custom registration を切り離す。

## Technical Context

- `crates/gwt-core/src/agent/*` と `launch.rs` が正本。
- `SPEC-1636` は Assistant shell 内の送信制御、`SPEC-1779` は custom registration を扱う。
- 起動失敗は UI と structured logs へ出す必要がある。

## Phased Implementation

### Phase 1: Scope Refresh

1. ビルトイン Agent と custom agent の境界を固定する。
2. version/model/reasoning/permissions の launch contract を整理する。

### Phase 2: Runtime Contract

1. 検出・起動・失敗時のエラーパスを定義する。
2. UI 表示名と内部 ID の対応を明文化する。

### Phase 3: Verification

1. 各 Agent の launch option と失敗時表示を検証する。
