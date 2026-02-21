# タスクリスト: Windows 移行プロジェクトの Docker 起動でポート競合を回避する

## Phase 1: セットアップ

- [x] T001 [US1] Issue #1161 の受け入れ条件を仕様化する `specs/SPEC-1161b0a1/spec.md`
- [x] T002 [US1] 実装計画を作成する `specs/SPEC-1161b0a1/plan.md`

## Phase 2: RED

- [x] T003 [US1] `merge_compose_env_for_docker` で使用中ポートへ巻き戻る再現テストを追加する（RED） `crates/gwt-tauri/src/commands/terminal.rs`

## Phase 3: 実装

- [x] T004 [US1] compose env マージに使用中ポート保護ロジックを追加する `crates/gwt-tauri/src/commands/terminal.rs`

## Phase 4: 検証

- [x] T005 [US1] `cargo test -p gwt-tauri merge_compose_env_for_docker_keeps_existing_allocated_port_when_incoming_is_occupied` を実行する
- [x] T006 [US2] `cargo test -p gwt-tauri merge_compose_env_for_docker_includes_non_allowlisted_compose_keys` を実行する
