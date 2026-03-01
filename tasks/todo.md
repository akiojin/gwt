# TODO: プロジェクト単位の完全分離（PTY・ChromaDB・GitHub Issue）

## 背景

gwt で複数プロジェクト同時利用時に PTY 通信・ChromaDB 検索・GitHub Issue がプロジェクト境界を越える問題を修正する。

## 実装ステップ

- [x] T001 gwt-spec Issue 作成 (#1395)
- [x] T002 `TerminalPane`/`PaneConfig` に `project_root` フィールド追加
- [x] T003 `PaneManager` に `panes_for_project()` メソッド追加
- [x] T004 `list_terminals` にプロジェクトフィルタ適用
- [x] T005 `send_keys_to_pane`/`capture_scrollback_tail` にプロジェクト検証追加
- [x] T006 MCP ハンドラにプロジェクトフィルタ追加
- [x] T007 `gwt-project-index` SKILL.md 更新
- [x] T008 `gwt-issue-spec-ops` SKILL.md 更新
- [x] T009 `gwt-spec-to-issue-migration` SKILL.md 更新
- [x] T010 `gwt-pty-communication` SKILL.md 更新
- [x] T011 `.codex/skills/gwt-spec-to-issue-migration/` 削除
- [x] T012 `cargo test` + `cargo clippy` + markdownlint 検証

## 検証結果

- [x] `cargo test` — 534 tests passed (gwt-tauri) + 4 tests passed (voice_eval)
- [x] `cargo clippy --all-targets --all-features -- -D warnings` — 警告なし
- [x] `npx markdownlint-cli` — 4つの SKILL.md エラーなし
