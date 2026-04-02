# Quick Start: SPEC-1786 — Codex hooks.json merge

## 最小検証フロー

### 1. マージロジック検証

```bash
cargo test -p gwt-core -- codex_hooks_merge
```

期待: ユーザー定義 hooks が保持され、managed hooks が追加/更新される

### 2. 差分検出検証

```bash
cargo test -p gwt-core -- codex_hooks_needs_update
```

期待: 変更なし → false、変更あり → true

### 3. TUI 確認ダイアログ検証（手動 E2E）

```bash
cargo run -p gwt-tui
```

1. Branches → ブランチ選択 → Wizard → Agent: Codex → Launch
2. 確認ダイアログが英語で表示される（初回のみ）
3. Embed 選択 → `.codex/hooks.json` が更新される
4. 再度 Codex agent 起動 → 確認ダイアログが表示されない（差分なし）
5. Claude Code agent 起動 → 確認ダイアログが表示されない

### 4. ユーザー定義 hooks 保持検証（手動）

1. `.codex/hooks.json` にカスタム PreToolUse hook を手動追加
2. Codex agent 起動 → Embed 選択
3. `.codex/hooks.json` にカスタム hook が残っていることを確認

### 5. 回帰テスト

```bash
cargo test -p gwt-core -p gwt-tui
cargo clippy --all-targets --all-features -- -D warnings
```
