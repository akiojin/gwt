# 実装計画: Version History を最新タグ10件に固定表示する（Issue #1230）

**仕様ID**: `SPEC-91f01230` | **日付**: 2026-02-25 | **仕様書**: `specs/SPEC-91f01230/spec.md`

## 目的

- Version History の表示を「最新タグ10件」に固定し、Unreleased を一覧から除外する。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/src/commands/version_history.rs`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/src/lib/components/VersionHistoryPanel.svelte`）
- **テスト**:
  - Frontend: `gwt-gui/src/lib/components/VersionHistoryPanel.test.ts`
  - Backend: `crates/gwt-tauri/src/commands/version_history.rs` 既存 `list_project_versions_*` テスト

## 実装方針

### Phase 1: 一覧取得パラメータの固定

- `VersionHistoryPanel` の `list_project_versions` 呼び出しを `limit: 11` にする。
- 理由: backend は `Unreleased` を 1 件含むため、タグ10件表示には 11 件取得が必要。

### Phase 2: 一覧表示対象のフィルタ

- 取得結果から `id === "unreleased"` を除外する。
- 除外後の先頭 10 件を一覧表示対象にする。
- 履歴取得 (`get_project_version_history`) は表示対象タグのみに行う。

### Phase 3: 既存API挙動の維持

- `list_project_versions` backend の件数制御（`limit>0`、Unreleased を含む合計件数）は維持する。
- 直前に追加された `limit=0` 無制限取得拡張は要件外のため撤回する。

### Phase 4: テスト更新

- Frontend テストを `unreleased` 除外前提へ更新。
- 「最新タグ10件のみ履歴取得」「unreleased 非表示」を追加検証。

## テスト

### フロントエンド

- `list_project_versions` が `limit: 11` で呼ばれること。
- `unreleased` が表示されず、履歴取得対象にもならないこと。
- タグ12件入力時に履歴取得対象が10件に制限されること。

### バックエンド

- `list_project_versions_includes_unreleased_and_tags`
- `list_project_versions_handles_unborn_head`

（`limit=0` 無制限取得テストは削除）
