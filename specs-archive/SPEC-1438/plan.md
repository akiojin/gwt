# SPEC-1438 Codex Hooks 対応 — 実装計画

## 概要

Codex CLI v0.116.0+ の Hooks フレームワークに対応し、gwt が Codex エージェント起動時に `.codex/hooks.json` と hook スクリプトを自動配置する。Claude Code と共通の hook ロジックを `skill_registration.rs` で一元管理する。

## アプローチ

Claude Code の hooks 実装パターン（`settings.local.json` + `.claude/hooks/scripts/*.mjs`）を Codex 向けに移植。Codex の hooks.json フォーマットは Claude と同じ `{ hooks: { Event: [{ matcher, hooks: [{ type, command }] }] } }` 構造。

### Claude Code との差分

| 項目 | Claude Code | Codex |
|------|------------|-------|
| 設定ファイル | `.claude/settings.local.json` | `.codex/hooks.json` |
| スクリプト配置先 | `.claude/hooks/scripts/` | `.codex/hooks/scripts/` |
| 対応イベント | PreToolUse, PostToolUse, UserPromptSubmit, Notification, Stop | SessionStart, PreToolUse, PostToolUse, UserPromptSubmit, Stop |
| 設定マージ | 既存 settings.local.json にマージ | hooks.json を丸ごと生成 |

## 変更対象ファイル

1. **`.codex/hooks/scripts/gwt-*.mjs`** — Claude 版と同一の hook スクリプト（`include_str!` ソース）
2. **`crates/gwt-core/src/config/skill_registration.rs`** — `CODEX_HOOK_ASSETS`、`managed_codex_hooks_definition()`、`merge_managed_codex_hooks()`、登録フローの Codex 分岐追加
3. **`crates/gwt-core/src/config/session.rs`** — `agent_has_hook_support()` に Codex 追加
4. **`crates/gwt-core/src/config/codex_hook_events.rs`** — Codex hook イベント処理（新規）
5. **`crates/gwt-core/src/config.rs`** — `codex_hook_events` モジュール登録
6. **`crates/gwt-core/build.rs`** — `.codex/hooks/scripts` の監視追加
7. **exclude パターン** — `/.codex/hooks.json` と `/.codex/hooks/scripts/gwt-*.mjs` 追加

## 設計判断

- hook スクリプトは Claude 版と同じ `.mjs` ファイルを `.codex/` 配下にコピー配置する（エージェント間のクロス依存を避ける）
- `hooks.json` は `settings.local.json` と違い既存設定とのマージ不要（gwt 専用ファイル）なので、丸ごと上書き生成する
- `SessionStart` イベントは gwt forward hook で Running ステータスに更新
- `Notification` イベントは Codex 未対応のためスキップ
