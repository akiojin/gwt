> **ℹ️ TUI MIGRATION NOTE**: This SPEC was completed during the gwt-tauri era. The gwt-tauri frontend has been replaced by gwt-tui (SPEC-1776). GUI-specific references are historical.

### 背景

gwt は複数プロジェクトを同時に開ける設計だが、以下の3箇所でプロジェクト間の分離が不完全:

**PTY 分離の欠如（最重要）:**

- `PaneManager` は `panes: Vec<TerminalPane>` のフラットなリストで、プロジェクトの概念がない
- `TerminalPane` に `project_root` フィールドがない
- `list_terminals`、`send_keys_to_pane`、`capture_scrollback_tail` 等の Tauri コマンドは全ペーンに無制限アクセス
- MCP ハンドラの `handle_list_tabs`、`handle_send_message` も同様
- 結果: Project A のエージェントが Project B のペーンを一覧・操作・読み取り可能

**ChromaDB 分離の欠如:**

- `gwt-project-search` スキルが `--db-path .gwt/index`（相対パス）を使用
- スキル側の記述のみの問題

**GitHub Issue 分離の欠如:**

- `gwt-issue-spec-ops` スキルの `gh` コマンドに `--repo` 指定がない

### ユーザーシナリオ

- US1 (P0): 2つのプロジェクト A, B を同時に開き、Project A のエージェントが `list_terminals` を実行 → Project A のペーンのみ返される
- US2 (P0): Project A のエージェントが `send_keys_to_pane` で Project B のペーン ID を指定 → エラーで拒否される
- US3 (P0): Project A のエージェントが ChromaDB 検索 → Project A の index のみ使用される
- US4 (P1): Project A のエージェントが `gh issue create` → Project A のリポジトリにのみ Issue が作成される

### 機能要件

- FR-001: `TerminalPane` に `project_root: PathBuf` フィールドを追加し、ペーン作成時に設定する
- FR-002: `PaneManager` にプロジェクトフィルタメソッド `panes_for_project(project_root)` を追加
- FR-003: `list_terminals` Tauri コマンドにプロジェクトフィルタを適用
- FR-004: `send_keys_to_pane`、`capture_scrollback_tail` でペーンのプロジェクト所属を検証
- FR-005: MCP ハンドラでも同様のプロジェクトフィルタを適用
- FR-006: `gwt-project-search` スキルで `$GWT_PROJECT_ROOT` を使った絶対パスを使用
- FR-007: `gwt-issue-spec-ops` スキルで CWD 前提を明文化
- FR-008: 各スキルに `## Environment` セクションを追加

### 成功基準

- SC-001: 複数プロジェクト同時利用時に PTY 通信がプロジェクト境界を超えない
- SC-002: ChromaDB 検索が正しいプロジェクトの index を使用する
- SC-003: `gh` コマンドが正しいリポジトリに対して実行される
- SC-004: `cargo test` パス、`cargo clippy` 警告なし
- SC-005: markdownlint エラーなし