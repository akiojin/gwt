# タスクリスト: Windows での外部プロセス実行時コンソール点滅抑止

## Phase 1: セットアップ

- [x] T001 [P] [US1] 共通プロセスヘルパー `crates/gwt-core/src/process.rs` を追加
- [x] T002 [P] [US1] `crates/gwt-core/src/lib.rs` で `process` モジュールを公開

## Phase 2: 基盤

- [x] T003 [US1] `crates/gwt-core/src` の外部コマンド実行を `process` ヘルパー経由へ統一
- [x] T004 [US1] `crates/gwt-tauri/src` の外部コマンド実行を `gwt_core::process` ヘルパー経由へ統一
- [x] T005 [US1] 置換に伴う未使用 import と軽微な整形差分を解消

## Phase 3: ストーリー 1

- [x] T006 [US1] Windows で `CREATE_NO_WINDOW` を適用する実装を追加 (`crates/gwt-core/src/process.rs`)
- [x] T007 [US1] `process.rs` のユニットテスト追加（std/tokio 両方）

## Phase 4: ストーリー 2

- [x] T008 [US2] 直接 `Command::new(...)` / `tokio::process::Command::new(...)` 混入を検出するガードテスト追加 (`crates/gwt-core/tests/no_direct_git_command.rs`)
- [x] T014 [US2] `crates/gwt-core/tests` / `crates/gwt-core/benches` の `Command::new("git")` を `gwt_core::process::git_command()` に統一
- [x] T009 [US2] `cargo check -q` 実行
- [x] T010 [US2] `cargo test -q` 実行
- [x] T011 [US2] `cargo test -q -p gwt-core` 実行

## Phase 5: 仕上げ・横断

- [x] T012 [P] [共通] `spec.md` / `plan.md` / `tasks.md` / `tdd.md` を作成
- [x] T013 [P] [共通] `specs/specs.md` に仕様一覧を追加
