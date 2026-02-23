# 実装計画: マイグレーション時の `Directory not empty` を非致命化

**仕様ID**: `SPEC-c2d9e741` | **日付**: 2026-02-23 | **仕様書**: `specs/SPEC-c2d9e741/spec.md`

## 目的

- ルートクリーンアップで発生する `Directory not empty` によるマイグレーション失敗を防ぐ
- 非対象のIOエラーは従来どおり失敗させ、挙動を維持する

## 技術コンテキスト

- バックエンド: Rust (`crates/gwt-core/src/migration/executor.rs`)
- 問題箇所: `cleanup_root_files()` 内の `std::fs::remove_dir_all`
- 期待動作: `Directory not empty` は警告として扱い処理継続
- テスト: `cargo test -p gwt-core migration::executor`

## 実装方針

### Phase 1: テスト追加（RED）

- `Directory not empty` 判定ロジック向けユニットテストを追加
- 非対象エラー（`PermissionDenied`）のユニットテストも追加

### Phase 2: クリーンアップ耐障害性の実装（GREEN）

- `cleanup_root_files()` にディレクトリ削除ヘルパーを導入
- `Directory not empty` は警告ログを出して継続
- それ以外のエラーは `MigrationError::IoError` へ変換して返却

### Phase 3: 検証

- 対象ユニットテストを実行して成功を確認
