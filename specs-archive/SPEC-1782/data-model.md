# Data Model: SPEC-1782

## QuickStartEntry

Quick Start に表示される 1 ツールのエントリ。`get_branch_tool_history()` の結果からフィルタリングして生成。

```
QuickStartEntry
  tool_id: String          — エージェント ID ("claude", "codex", etc.)
  tool_label: String       — 表示名 ("Claude Code", "Codex CLI", etc.)
  model: Option<String>    — 使用モデル ("opus", "gpt-5.4", etc.)
  version: Option<String>  — ツールバージョン
  session_id: Option<String> — セッション ID（Resume に必要）
  skip_permissions: Option<bool>
  reasoning_level: Option<String> — Codex のみ
  fast_mode: Option<bool>  — Codex のみ
  collaboration_modes: Option<bool> — Codex のみ
  branch: String           — ブランチ名
```

## ToolSessionEntry（既存 gwt-core）

`~/.gwt/sessions/` の TOML に保存されるエントリ。Quick Start のデータソース。

```
ToolSessionEntry
  branch: String
  worktree_path: Option<String>
  tool_id: String
  tool_label: String
  session_id: Option<String>  ← detect_session_id_for_tool() で後から更新
  mode: Option<String>
  model: Option<String>
  reasoning_level: Option<String>
  skip_permissions: Option<bool>
  tool_version: Option<String>
  collaboration_modes: Option<bool>
  timestamp: i64
```

## データフロー

```
エージェント起動
  → save_session_entry(session_id=None)
  → PTY 起動
  → (バックグラウンド) detect_session_id_for_tool()
  → save_session_entry(session_id=detected)

次回 Quick Start
  → get_branch_tool_history(branch)
  → session_id がある最新ツールをフィルタ
  → QuickStartEntry に変換
  → open_for_branch(branch, [entry])
```
