# 実装計画: SPEC-d7f2a1b3

## 概要

Cleanup 機能の serde シリアライズ不整合（camelCase vs snake_case）を修正する。

## 変更ファイル

| ファイル | 変更内容 |
|---------|---------|
| `crates/gwt-tauri/src/commands/cleanup.rs` | `WorktreeInfo`, `WorktreesChangedPayload` から `#[serde(rename_all = "camelCase")]` を削除 |

## 変更しないもの

| 対象 | 理由 |
|------|------|
| `SafetyLevel` の `#[serde(rename_all = "lowercase")]` | enum 値を lowercase にシリアライズしており正常 |
| `CleanupResult`, `CleanupProgressPayload`, `CleanupCompletedPayload` | 単語1つのフィールドのみで camelCase 属性なし |
| TypeScript 型定義・全フロントエンドコード | 既に snake_case で正しい |

## テスト計画

### Rust ユニットテスト（TDD 追加分）

1. `worktree_info_serializes_with_snake_case_keys` — `WorktreeInfo` をシリアライズし、JSON キーが snake_case であることを検証
2. `worktrees_changed_payload_serializes_with_snake_case_keys` — `WorktreesChangedPayload` をシリアライズし、JSON キーが snake_case であることを検証

### 既存テスト（影響なし）

- Rust: `compute_safety_level` 系テスト、`cleanup_single_branch` 系テスト（シリアライズ非依存）
- Frontend: `CleanupModal.test.ts`（フィクスチャが snake_case で手書き）

## 検証手順

1. `cargo test` — 全テストパス
2. `cargo clippy --all-targets --all-features -- -D warnings` — 警告なし
3. `cd gwt-gui && npx vitest run` — 全テストパス
