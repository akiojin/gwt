# タスクリスト: Migration backup copy の Windows 互換修正

## Phase 1: 仕様・テスト

- [x] T1: `spec.md` / `plan.md` / `tasks.md` を作成
- [x] T2: `cp` 非存在環境で `create_backup()` が成功するテストを追加

## Phase 2: 実装

- [x] T3: `crates/gwt-core/src/migration/backup.rs` の `copy_dir_recursive()` をWindows互換に修正
- [x] T4: 非Windowsで `cp` 失敗時のフォールバックを実装

## Phase 3: 検証

- [x] T5: `cargo test -p gwt-core migration::backup` を実行して成功を確認
