# タスクリスト: マイグレーション時の `Directory not empty` を非致命化

## Phase 1: 仕様・テスト

- [x] T1: `spec.md` / `plan.md` / `tasks.md` を作成
- [x] T2: `Directory not empty` 判定ロジックのユニットテストを追加

## Phase 2: 実装

- [x] T3: `cleanup_root_files()` に `Directory not empty` を許容する削除ヘルパーを導入
- [x] T4: 非対象エラーを従来どおり `MigrationError::IoError` として返す経路を維持

## Phase 3: 検証

- [x] T5: `cargo test -p gwt-core migration::executor` を実行して成功を確認
