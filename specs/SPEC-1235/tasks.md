# タスクリスト: SVN 混在リポジトリで Migration が失敗する問題を解消する

## Phase 1: セットアップ

- [x] T001 [US1] 仕様書を作成し Issue #1235 の受け入れ条件を定義する `specs/SPEC-1235/spec.md`
- [x] T002 [US1] 実装計画とタスクを作成する `specs/SPEC-1235/plan.md`, `specs/SPEC-1235/tasks.md`

## Phase 2: ストーリー 1

- [x] T003 [US1] `MigrationConfig` に退避テンポラリパスヘルパーを追加する `crates/gwt-core/src/migration/config.rs`
- [x] T004 [US1] dirty main repository 退避を move 化し manifest を追加する `crates/gwt-core/src/migration/executor.rs`
- [x] T005 [US1] 退避復元処理を move 化する `crates/gwt-core/src/migration/executor.rs`

## Phase 3: ストーリー 2

- [x] T006 [US2] rollback に退避復旧処理を追加する `crates/gwt-core/src/migration/rollback.rs`
- [x] T007 [US2] manifest 読み込み失敗時のフォールバックを実装する `crates/gwt-core/src/migration/rollback.rs`

## Phase 4: 仕上げ・横断

- [x] T008 [共通] executor の退避/復元テストを追加する `crates/gwt-core/src/migration/executor.rs`
- [x] T009 [共通] rollback の退避復旧テストを追加する `crates/gwt-core/src/migration/rollback.rs`
- [x] T010 [共通] migration 関連テストを実行し結果を記録する
