# 実装計画: SVN 混在リポジトリで Migration が失敗する問題を解消する

**仕様ID**: `SPEC-1235` | **日付**: 2026-02-27 | **仕様書**: `specs/SPEC-1235/spec.md`

## 目的

- dirty main repository の退避処理を copy から move に変更し、`.svn/pristine` を含むケースでも Migration を継続可能にする。
- Migration 失敗時に `.gwt-migration-temp` から source root へ退避データを復旧できるようにする。

## 技術コンテキスト

- **バックエンド**: Rust 2021（`crates/gwt-core/src/migration/*`）
- **フロントエンド**: 変更なし
- **公開 API**: 変更なし
- **テスト**: `cargo test -p gwt-core migration::executor::tests migration::rollback::tests`

## 実装方針

### Phase 1: 退避・復元ロジックの move 化

- `MigrationConfig` に `.gwt-migration-temp` 取得ヘルパーを追加する。
- `executor.rs` の `evacuate_main_repo_files` を move 化する。
- 退避エントリ一覧を `evacuation-manifest.json` に保存する。
- `restore_evacuated_files` を move 化し、manifest を優先利用する。

### Phase 2: rollback 復旧ロジック追加

- `rollback.rs` に `.gwt-migration-temp` から source root へ move-back する処理を追加する。
- manifest 読み込み失敗時は temp ディレクトリ走査へフォールバックする。
- rollback フロー内で退避復旧処理を実行する。

### Phase 3: テストと回帰確認

- executor の退避・復元 move 挙動をテスト追加で固定化する。
- rollback の退避復旧挙動をテスト追加で固定化する。
- migration 関連テストを実行して回帰を確認する。

## リスク

- **中**: rollback が Phase 8 以降の失敗ケースで全ファイルを完全復旧できない可能性
- **低**: manifest が破損した場合でも directory scan フォールバックで継続可能
