# 実装計画: Version History で古いタグが表示されない（Issue #1230）

**仕様ID**: `SPEC-91f01230` | **日付**: 2026-02-25 | **仕様書**: `specs/SPEC-91f01230/spec.md`

## 目的

- Version History 一覧の固定件数制限を撤廃し、古いタグまで表示できるようにする。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/src/commands/version_history.rs`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/src/lib/components/VersionHistoryPanel.svelte`）
- **テスト**:
  - Rust: `crates/gwt-tauri/src/commands/version_history.rs` 内ユニットテスト
  - Frontend: `gwt-gui/src/lib/components/VersionHistoryPanel.test.ts`
- **前提**: `list_project_versions(limit)` の API 互換性（`limit>0`）は維持する

## 実装方針

### Phase 1: 仕様どおりの取得件数ルールを定義

- `list_project_versions` の `limit=0` を無制限取得として扱う仕様を Rust 側に実装する。
- `limit>0` の既存挙動（`Unreleased` を含む件数上限）は維持する。

### Phase 2: Version History タブの呼び出し修正

- `VersionHistoryPanel` の `list_project_versions` 呼び出しを `limit: 10` から `limit: 0` に変更する。

### Phase 3: TDD と回帰確認

- 先にテストを更新し、現状 RED を確認する。
- 実装修正後に対象テストを GREEN 化する。

## テスト

### バックエンド

- `list_project_versions(limit=0)` で全タグ取得になることを確認するテストを追加する。

### フロントエンド

- `VersionHistoryPanel` が `list_project_versions` を `limit: 0` で呼び出すことを確認する。
