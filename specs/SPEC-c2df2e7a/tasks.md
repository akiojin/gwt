# タスクリスト: From Issue の Branch Exists 誤判定（stale remote-tracking ref）

## Phase 1: セットアップ

- [x] T001 [P] [US1] 再現条件（stale remote-tracking による誤判定）を整理し、対象関数を `find_branch_for_issue` に確定する `crates/gwt-core/src/git/issue.rs`

## Phase 2: 基盤

- [x] T002 [US1] remote-tracking 候補の分解ヘルパー（symbolic ref 除外）を追加する `crates/gwt-core/src/git/issue.rs`
- [x] T003 [US1] remote 実在確認ヘルパー（`git ls-remote --heads`）を追加する `crates/gwt-core/src/git/issue.rs`

## Phase 3: ストーリー 1（stale ref を除外）

- [x] T004 [US1] (RED) stale remote-tracking のみで `None` を期待するテストを追加する `crates/gwt-core/src/git/issue.rs`
- [x] T005 [US1] (GREEN) `find_branch_for_issue` の remote 判定に `ls-remote` 実在確認を組み込み、stale 候補を無視する `crates/gwt-core/src/git/issue.rs`

## Phase 4: ストーリー 2（実remote検出維持）

- [x] T006 [US2] (RED) 実remote branch で `Some(...)` を期待するテストを追加する `crates/gwt-core/src/git/issue.rs`
- [x] T007 [US2] (GREEN) 実remote branch を返し、既存 I/F を維持することを確認する `crates/gwt-core/src/git/issue.rs`

## Phase 5: ストーリー 3（ローカル優先維持）

- [x] T008 [US3] (RED) ローカル branch 優先のテストを追加する `crates/gwt-core/src/git/issue.rs`
- [x] T009 [US3] (GREEN) ローカルヒット時に追加 remote 照会を行わない既存動作を維持する `crates/gwt-core/src/git/issue.rs`

## Phase 6: 仕上げ・横断

- [x] T010 [P] [共通] `spec.md` / `plan.md` / `tasks.md` を作成・更新する `specs/SPEC-c2df2e7a/spec.md` `specs/SPEC-c2df2e7a/plan.md` `specs/SPEC-c2df2e7a/tasks.md`
- [x] T011 [P] [共通] 仕様索引を更新する `specs/specs.md`
- [x] T012 [共通] `cargo test -p gwt-core git::issue::tests::` と `cargo clippy -p gwt-core --all-targets -- -D warnings` を実行して成功を確認する `crates/gwt-core/src/git/issue.rs`
