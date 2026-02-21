# タスクリスト: Windows Docker Launch で `service "dev" is not running` を防止する

## Phase 1: セットアップ

- [x] T001 [US1] 仕様書を作成し Issue #1162 の受け入れ条件を定義する `specs/SPEC-17e47ece/spec.md`
- [x] T002 [US1] 実装計画とタスクを作成する `specs/SPEC-17e47ece/plan.md`, `specs/SPEC-17e47ece/tasks.md`

## Phase 2: ストーリー 1

- [x] T003 [US1] `build_docker_compose_up_args` のサービス指定ケースを先にテスト追加する `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T004 [US1] Compose/DevContainer の `docker_compose_up` 呼び出しにサービス名を渡すよう実装する `crates/gwt-tauri/src/commands/terminal.rs`

## Phase 3: 仕上げ・横断

- [x] T005 [共通] 追加した関連テストを実行して結果を確認する `crates/gwt-tauri/src/commands/terminal.rs`
