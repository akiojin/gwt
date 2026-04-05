# Data Model: SPEC-1786 — Codex hooks.json merge

## hooks.json 構造

```json
{
  "hooks": {
    "<EventName>": [
      {
        "matcher": "<pattern>",
        "hooks": [
          {
            "type": "command",
            "command": "<shell command>"
          }
        ]
      }
    ]
  }
}
```

### Event 一覧

| Event | gwt managed | ユーザー定義可 |
|-------|-------------|--------------|
| SessionStart | Yes (gwt-forward-hook) | Yes |
| UserPromptSubmit | Yes (gwt-forward-hook) | Yes |
| PreToolUse | Yes (forward + block scripts) | Yes |
| PostToolUse | Yes (gwt-forward-hook) | Yes |
| Stop | Yes (gwt-forward-hook) | Yes |

### matcher パターン

- `"*"` — 全ツールに適用（gwt-forward-hook で使用）
- `"Bash"` — Bash ツールのみ（block scripts で使用）
- ユーザー定義の任意の matcher も可能

## マージの単位

マージは **event × matcher** 単位ではなく、**event 配列内の個別エントリ** 単位で行う:

1. event 配列（例: `hooks.PreToolUse`）を走査
2. 各エントリの `hooks` 配列内の `command` を検査
3. `gwt-` を含む command を持つエントリを「managed」と判定
4. managed エントリを除去し、新しい managed 定義に置換
5. non-managed エントリはそのまま保持

## 差分検出

```rust
fn codex_hooks_needs_update(codex_root: &Path) -> bool
```

1. `codex_root/hooks.json` を読み込み
2. `managed_codex_hooks_definition()` で managed hooks を取得
3. マージ結果を `serde_json::to_string_pretty()` で文字列化
4. 既存ファイルの内容と byte-for-byte 比較
5. 差分があれば `true`

## ConfirmAction 拡張

```rust
pub enum ConfirmAction {
    // 既存
    Delete(String),
    ExitWithRunningAgents,
    TerminateAgent { branch: String, agent_name: String },
    Custom(String),
    // 新規
    EmbedCodexHooks,
}
```
