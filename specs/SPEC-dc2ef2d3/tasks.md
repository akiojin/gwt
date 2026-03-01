# タスクリスト: Worktree詳細ビューでCLAUDE.md/AGENTS.md/GEMINI.mdを確認・修正し編集起動

## Phase 1: 仕様

- [x] T001 [P] [共通] `spec.md` を作成して受け入れ条件とFRを定義する `specs/SPEC-dc2ef2d3/spec.md`
- [x] T002 [P] [共通] `plan.md` を作成して実装方針を確定する `specs/SPEC-dc2ef2d3/plan.md`

## Phase 2: Backend

- [x] T010 [US1] 新規 command `check_and_fix_agent_instruction_docs` を実装する `crates/gwt-tauri/src/commands/clause_docs.rs`
- [x] T011 [US1] `CLAUDE.md` 生成テンプレート（Qiita構成反映）を定義する `crates/gwt-tauri/src/commands/clause_docs.rs`
- [x] T012 [US3] worktree 未解決時にエラー中断する実装を追加する `crates/gwt-tauri/src/commands/clause_docs.rs`
- [x] T013 [US1] command module と invoke handler を登録する `crates/gwt-tauri/src/commands/mod.rs` `crates/gwt-tauri/src/app.rs`

## Phase 3: Frontend

- [x] T020 [US1] WorktreeSummary ヘッダーに「Check/Fix Docs + Edit」ボタンを追加する `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`
- [x] T021 [US1] command 呼び出し・実行中ガード・エラー表示を追加する `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`
- [x] T022 [US1] `onOpenDocsEditor` コールバックを Sidebar 経由で配線する `gwt-gui/src/lib/components/Sidebar.svelte` `gwt-gui/src/App.svelte`
- [x] T023 [US2] App 側で shell別編集起動コマンド（WSL/PowerShell/cmd）を実装する `gwt-gui/src/App.svelte`

## Phase 4: テスト

- [x] T030 [US1] backend unit test（作成/補完/未存在branchエラー）を追加する `crates/gwt-tauri/src/commands/clause_docs.rs`
- [x] T031 [US1] WorktreeSummaryPanel test（成功/失敗時UI）を追加する `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts`
- [x] T032 [US4] [RED] docs editor コマンド/終了判定の単体テストを追加し、未実装状態で失敗を確認する `gwt-gui/src/lib/docsEditor.test.ts`
- [x] T033 [US4] [GREEN] docs editor ロジックを `docsEditor.ts` へ切り出して App に適用する `gwt-gui/src/lib/docsEditor.ts` `gwt-gui/src/App.svelte`

## Phase 5: 検証

- [x] T040 [P] [共通] `cargo test -p gwt-tauri clause_docs` を実行して成功を確認する
- [x] T041 [P] [共通] `pnpm --dir gwt-gui test -- WorktreeSummaryPanel.test.ts` を実行して成功を確認する
- [x] T042 [P] [共通] `pnpm --dir gwt-gui exec vitest run src/lib/docsEditor.test.ts` を実行して成功を確認する
- [x] T043 [P] [共通] `pnpm --dir gwt-gui check` を実行して型/構文エラーがないことを確認する
