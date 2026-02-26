# タスクリスト: Windows版 Launch Agent で `npx.cmd` 起動失敗を防止する

## Phase 1: セットアップ

- [x] T001 [US1] 仕様書を作成し Issue #1265 の受け入れ条件を定義する `specs/SPEC-1265/spec.md`
- [x] T002 [US1] 実装計画とタスクを作成する `specs/SPEC-1265/plan.md`, `specs/SPEC-1265/tasks.md`

## Phase 2: ストーリー 1

- [x] T003 [US1] 外側クォート混入コマンドの再現テストを先に追加する `crates/gwt-core/src/terminal/pty.rs`
- [x] T004 [US1] Windows コマンド正規化を実装し batch 判定と式構築へ適用する `crates/gwt-core/src/terminal/pty.rs`

## Phase 3: 仕上げ・横断

- [x] T005 [共通] 関連ユニットテストを実行し回帰がないことを確認する `crates/gwt-core/src/terminal/pty.rs`
