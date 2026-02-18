# Architecture

gwt は Git worktree とコーディングエージェント起動を扱うデスクトップ GUI アプリです。

## Components

- `gwt-gui/`
  - Svelte 5 + TypeScript + Vite
  - xterm.js による内蔵ターミナル UI
- `crates/gwt-tauri/`
  - Tauri v2 バックエンド（Rust）
  - Tauri Commands とアプリ状態管理
- `crates/gwt-core/`
  - コアロジック（Git/Worktree/設定/ログ/Docker/PTY）

## Data Flow

UI は `@tauri-apps/api` の `invoke()` でバックエンドのコマンドを呼び出し、結果を表示します。

- Project:
  - open/create
  - repo type check
- Branch/Worktree:
  - list branches/worktrees
  - create worktree (必要に応じて新規ブランチ作成)
- Terminal/Agent:
  - PTY 起動
  - エージェント起動（Claude Code/Codex/Gemini/OpenCode）
- Settings/Profiles:
  - グローバル/ローカル設定の読み書き
  - プロファイル（環境変数、AI 設定）の読み書き

## Persistence

設定と履歴はローカルファイルとして保存します（DB なし）。

- 設定（優先順）:
  - `<repo>/.gwt.toml`
  - `<repo>/.gwt/config.toml`
  - `~/.gwt/config.toml`
  - `~/.config/gwt/config.toml` (legacy)
- プロファイル:
  - `~/.gwt/profiles.yaml`
- ログ:
  - `~/.gwt/logs/`
