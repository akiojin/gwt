# Plan: SPEC-1786 — gwt-spec の hooks.json マージ

## Summary

`write_managed_codex_hooks()` を全上書きからマージ方式に変更し、ユーザー定義 hooks を保持する。
Codex agent 起動時に hooks.json への変更がある場合のみ、英語の確認ダイアログ（Embed/Skip）を表示する。

## Technical Context

### 影響ファイル

| ファイル | 変更内容 |
|---------|---------|
| `crates/gwt-core/src/config/skill_registration.rs` | `write_managed_codex_hooks()` をマージ方式に変更、差分検出 API 追加 |
| `crates/gwt-tui/src/app.rs` | `spawn_agent_session()` に Codex 用確認ダイアログフローを追加 |
| `crates/gwt-tui/src/screens/confirm.rs` | `ConfirmAction` に `EmbedCodexHooks` バリアントを追加 |

### 既存パターンの再利用

- **マージロジック**: Claude Code の `merge_managed_claude_hooks_into_settings()` + `prune_managed_hook_entries()` パターンを Codex 向けに適用
- **確認ダイアログ**: 既存の `ConfirmState`（`confirm.rs`）を再利用。`is_dangerous: false` で表示
- **managed hook 識別**: `is_managed_hook_command()` を Codex hooks にも適用（`gwt-` プレフィックス）

### 外部制約

- Codex CLI は `hooks.local.json` 非対応 → `hooks.json` 1ファイルで共存必須
- Codex hooks の event は配列で、同一 event に複数の hook グループ（matcher 別）が共存可能
- `.codex/hooks.json` は git tracked → 差分がユーザーの worktree に出る

## Constitution Check

| Rule | Status | Note |
|------|--------|------|
| §1 Spec Before Implementation | PASS | SPEC-1786 spec.md 策定済み |
| §2 Test-First Delivery | PASS | TDD で RED → GREEN |
| §3 No Workaround-First Changes | PASS | Claude の merge パターンを正式に Codex に適用 |
| §4 Minimal Complexity | PASS | 既存の merge ロジック + ConfirmState を再利用 |
| §5 Verifiable Completion | PASS | ユニットテスト + 手動 E2E |
| §6 SPEC vs Issue Separation | PASS | カテゴリ: CONFIG |

## Project Structure

```
crates/gwt-core/src/config/skill_registration.rs
  ├── write_managed_codex_hooks()      ← マージ方式に変更
  ├── merge_managed_codex_hooks()      ← 新規: マージロジック
  ├── codex_hooks_needs_update()       ← 新規: 差分検出
  └── managed_codex_hooks_definition() ← 既存（変更なし）

crates/gwt-tui/src/app.rs
  └── spawn_agent_session()            ← Codex 確認ダイアログ追加

crates/gwt-tui/src/screens/confirm.rs
  └── ConfirmAction::EmbedCodexHooks   ← 新規バリアント
```

## Complexity Tracking

| 項目 | 理由 |
|------|------|
| Codex hooks マージロジック追加 | Claude と同等の複雑さだが、Codex の hooks.json 構造が異なるため専用ロジックが必要。Claude の `settings.local.json` はフラットな hooks 配列だが、Codex は `{ hooks: { EventName: [...] } }` のネスト構造 |
| 確認ダイアログ追加 | 既存の ConfirmState を再利用するため追加複雑さは最小 |

## Phased Implementation

### Phase 1: gwt-core マージロジック（skill_registration.rs）

1. `merge_managed_codex_hooks(existing: &Value, managed: &Value) -> Value` 新規関数
   - 既存 JSON から gwt managed hooks を除去（`gwt-` プレフィックスで識別）
   - managed hooks を追加
   - ユーザー定義 hooks を保持
2. `codex_hooks_needs_update(codex_root: &Path) -> bool` 新規関数
   - 現在の hooks.json を読み込み、マージ結果と byte-for-byte 比較
   - 差分がある場合 true を返す
3. `write_managed_codex_hooks()` を変更
   - マージ結果が既存と同一ならスキップ（FR-030）
   - 差分がある場合のみ書き込み

### Phase 2: TUI 確認ダイアログ（app.rs + confirm.rs）

1. `ConfirmAction::EmbedCodexHooks` バリアントを追加
2. `spawn_agent_session()` を変更:
   - Codex agent の場合、skill registration 前に `codex_hooks_needs_update()` を呼ぶ
   - 差分がある場合 → 確認ダイアログを表示し、一旦 return
   - Embed 選択 → skill registration 実行 → agent 起動
   - Skip 選択 → skill registration スキップ → agent 起動
   - Codex 以外の agent → 既存フロー（確認なし）

### Phase 3: テスト・検証

1. ユニットテスト:
   - `merge_managed_codex_hooks()` — ユーザー hooks 保持、冪等性、バージョン更新
   - `codex_hooks_needs_update()` — 変更なし / 変更あり
2. 手動 E2E:
   - Codex agent 起動 → 確認ダイアログ表示 → Embed → hooks 更新確認
   - 2回目起動 → 確認ダイアログ非表示
   - Skip → hooks 未変更確認
   - Claude Code agent → 確認ダイアログ非表示
