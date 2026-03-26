### Rust バックエンド変更

| ファイル | 変更 |
|---|---|
| `crates/gwt-core/src/terminal/pane.rs` | `PaneConfig` と `TerminalPane` に `project_root: PathBuf` 追加 |
| `crates/gwt-core/src/terminal/manager.rs` | `panes_for_project()` メソッド追加 |
| `crates/gwt-tauri/src/commands/terminal.rs` | `list_terminals` にプロジェクトフィルタ、`send_keys_to_pane`/`capture_scrollback_tail` にプロジェクト検証追加 |
| `crates/gwt-tauri/src/mcp_handlers.rs` | `handle_list_tabs`/`handle_send_message` にプロジェクトフィルタ追加 |

### スキル更新

| ファイル | 変更 |
|---|---|
| `plugins/gwt-integration/skills/gwt-project-search/SKILL.md` | `--db-path` を `$GWT_PROJECT_ROOT` 使用に変更 |
| `plugins/gwt-integration/skills/gwt-issue-spec-ops/SKILL.md` | `## Requirements` に CWD/リポジトリ前提を追記 |
| `plugins/gwt-integration/skills/gwt-spec-to-issue-migration/SKILL.md` | Preconditions に `$GWT_PROJECT_ROOT` 推奨追記 |
| `plugins/gwt/skills/gwt-agent-dispatch/SKILL.md` | `## Environment` セクション追加 |

### クリーンアップ

- `.codex/skills/gwt-spec-to-issue-migration/` 削除（plugins/ と完全重複）
