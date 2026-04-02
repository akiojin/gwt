> **Historical Status**: この closed SPEC の未完了 task は旧 backlog の保存であり、現行の完了条件ではない。

# SPEC-1438 Codex Hooks 対応 — タスク一覧

## Phase 1: アセット準備

- [ ] `.codex/hooks/scripts/gwt-forward-hook.mjs` 作成（Claude版コピー、コメント調整）
- [ ] `.codex/hooks/scripts/gwt-block-git-branch-ops.mjs` 作成
- [ ] `.codex/hooks/scripts/gwt-block-cd-command.mjs` 作成
- [ ] `.codex/hooks/scripts/gwt-block-file-ops.mjs` 作成
- [ ] `.codex/hooks/scripts/gwt-block-git-dir-override.mjs` 作成
- [ ] `crates/gwt-core/build.rs` に `.codex/hooks/scripts` 監視追加

## Phase 2: skill_registration.rs 拡張

- [ ] `CODEX_HOOK_ASSETS` 定数追加（5 スクリプト、`include_str!` で埋め込み）
- [ ] `codex_hook_script_command()` 関数追加（`.codex/hooks/scripts/` 向け）
- [ ] `managed_codex_hooks_definition()` 関数追加（5 イベント: SessionStart, PreToolUse, PostToolUse, UserPromptSubmit, Stop）
- [ ] `merge_managed_codex_hooks()` 関数追加（`.codex/hooks.json` へ書き出し）
- [ ] `register_agent_skills_with_settings_at_project_root()` の Codex 分岐を `register_codex_assets_at()` に変更
- [ ] `PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_LINES` に `/.codex/hooks.json` と `/.codex/hooks/scripts/gwt-*.mjs` 追加

## Phase 3: Hook イベント処理

- [ ] `codex_hook_events.rs` 新規作成（SessionStart→Running 追加、Notification 除外）
- [ ] `config.rs` にモジュール登録 + `pub use`

## Phase 4: セッション管理

- [ ] `session.rs` の `agent_has_hook_support()` に codex 追加

## Phase 5: 検証

- [ ] `cargo test -p gwt-core -p gwt-tui`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo fmt`
