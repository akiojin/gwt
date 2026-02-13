# タスクリスト: Windows 移行プロジェクトで Docker mount エラーを回避する

## Phase 1: セットアップ

- [x] T001 [US1] 仕様書を追加し、Issue 1028 の再現条件と受け入れ条件を明文化する `specs/SPEC-4e2f1028/spec.md`

## Phase 2: 基盤

- [x] T002 [US1] Git bind mount 計画ヘルパー（path 正規化・重複判定）を追加する `crates/gwt-tauri/src/commands/terminal.rs`

## Phase 3: ストーリー 1

- [x] T003 [US1] compose override 生成を long syntax bind mount に変更する `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T004 [US1] docker run の `-v` 生成に共通 mount 計画を適用する `crates/gwt-tauri/src/commands/terminal.rs`
- [x] T005 [US1] mixed path 再現テストを追加する `crates/gwt-tauri/src/commands/terminal.rs`

## Phase 4: ストーリー 2

- [x] T006 [US2] POSIX パス互換性テストを追加する `crates/gwt-tauri/src/commands/terminal.rs`

## Phase 5: 仕上げ・横断

- [x] T007 [共通] `cargo test` で追加テストを実行し結果を確認する
