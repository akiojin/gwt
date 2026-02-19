# クイックスタート: Windows シェル選択（Launch Agent / New Terminal）

**仕様ID**: `SPEC-1350124a` | **日付**: 2026-02-19

## 概要

Windows 環境で Launch Agent / New Terminal のシェルを PowerShell / Command Prompt / WSL から選択可能にする。全 OS で WorktreeSummaryPanel に New Terminal ボタンを追加する。

## ファイルマップ

### バックエンド（Rust）

| ファイル | 変更内容 |
|---|---|
| `crates/gwt-core/src/terminal/shell.rs` | **新規** WindowsShell enum, windows_to_wsl_path() |
| `crates/gwt-core/src/terminal/mod.rs` | `pub mod shell;` 追加 |
| `crates/gwt-core/src/config/settings.rs` | TerminalSettings 追加, Settings に terminal フィールド追加 |
| `crates/gwt-tauri/src/commands/terminal.rs` | get_available_shells コマンド, spawn_shell/launch_agent 拡張 |
| `crates/gwt-tauri/src/commands/mod.rs` | generate_handler! に get_available_shells 登録 |
| `crates/gwt-tauri/src/commands/settings.rs` | SettingsData に default_shell 追加 |

### フロントエンド（Svelte/TypeScript）

| ファイル | 変更内容 |
|---|---|
| `gwt-gui/src/lib/types.ts` | ShellInfo, SettingsData.default_shell, LaunchAgentRequest.terminal_shell 追加 |
| `gwt-gui/src/lib/agentLaunchDefaults.ts` | LaunchDefaults に selectedShell 追加 |
| `gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte` | onNewTerminal props, >_ ボタン追加 |
| `gwt-gui/src/lib/components/Sidebar.svelte` | onNewTerminal props 伝播 |
| `gwt-gui/src/App.svelte` | handleNewTerminal ハンドラ追加 |
| `gwt-gui/src/lib/components/AgentLaunchForm.svelte` | シェル選択ドロップダウン追加 |
| `gwt-gui/src/lib/components/SettingsPanel.svelte` | Terminal タブ追加 |

### テスト

| ファイル | 内容 |
|---|---|
| `crates/gwt-core/src/terminal/shell.rs` | WindowsShell ユニットテスト, windows_to_wsl_path テスト |
| `crates/gwt-core/src/config/settings.rs` | TerminalSettings config.toml テスト |
| `gwt-gui/src/lib/components/WorktreeSummaryPanel.test.ts` | New Terminal ボタンテスト |
| `gwt-gui/src/lib/components/AgentLaunchForm.test.ts` | シェル選択テスト |
| `gwt-gui/src/lib/components/SettingsPanel.test.ts` | Terminal タブテスト |

## 実装順序

```text
Phase 1: データモデル（shell.rs, settings.rs, types.ts）
    ↓
Phase 2: バックエンド統合（terminal.rs コマンド拡張）
    ↓
Phase 3: New Terminal ボタン（WorktreeSummaryPanel → Sidebar → App）
Phase 4: AgentLaunchForm シェル選択（並行可）
Phase 5: Settings Terminal タブ（並行可）
```

## 検証コマンド

```bash
# バックエンドテスト
cargo test

# バックエンド lint
cargo clippy --all-targets --all-features -- -D warnings

# フロントエンド型チェック
cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json

# フロントエンドテスト
cd gwt-gui && pnpm test
```
