# Research: SPEC-1786 — Codex hooks.json merge

## Codex CLI hooks.json 仕様

- Codex CLI は `~/.codex/hooks.json`（ユーザーレベル）と `.codex/hooks.json`（リポジトリレベル）の2階層
- 両方が存在する場合、全 hooks を並列実行（上書きではなく累積）
- `hooks.local.json` は非対応（Claude Code の `settings.local.json` に相当する仕組みがない）
- hooks.json 構造: `{ "hooks": { "EventName": [{ "matcher": "...", "hooks": [...] }] } }`

## hooks.json のマージ戦略

### 選択肢比較

| 戦略 | メリット | デメリット |
|------|---------|----------|
| A: 全上書き（現状） | シンプル | ユーザー hooks 消失 |
| B: event 単位マージ | ユーザー hooks 保持 | event 内の重複管理が必要 |
| C: matcher 単位マージ | 最も細かい粒度 | 複雑すぎる |

### 採用: B — event 単位マージ

各 event（PreToolUse, PostToolUse 等）の hooks 配列内で:
1. `gwt-` を含む command を持つエントリを除去
2. managed hooks を追加
3. ユーザー定義エントリはそのまま保持

Claude Code の `prune_managed_hook_entries()` と同じパターン。

## gwt managed hooks の識別方法

managed hooks の command フィールドは現行では以下の no-Node パターン:

```
'<absolute-path-to-gwt>' hook runtime-state <event>
'<absolute-path-to-gwt>' hook block-bash-policy
```

識別キー: `hook runtime-state` / `hook block-bash-policy` を含む gwt 管理 command 形状。旧 `gwt-*.mjs` 文字列は migration 用の legacy 判定としてのみ扱う。

## 確認ダイアログの UX

### 表示条件

- agent_id が "codex" の場合のみ
- `codex_hooks_needs_update()` が true の場合のみ
- 2回目以降は差分がないためダイアログ非表示

### 英語テキスト案

```
Update Codex Hooks

gwt needs to update .codex/hooks.json with managed hooks.
This is a tracked file — a git diff will appear.
You may commit this change, but it is not required.

[Embed]  [Skip]
```
