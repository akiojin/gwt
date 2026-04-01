# Feature Specification: Codex hooks.json merge with user-defined hooks

## Background

gwt のスキル登録機能（`register_codex_assets_at`）は、agent起動時に `.codex/hooks.json` へ gwt managed hooks（PreToolUse, PostToolUse, SessionStart, Stop, UserPromptSubmit）を書き込む。

現在の `write_managed_codex_hooks()` は `.codex/hooks.json` を**全上書き**しており、ユーザーが自分で定義したカスタム hooks が消失する。

Claude Code では `settings.json`（tracked / ユーザー定義）と `settings.local.json`（gitignored / gwt managed）で分離できるが、Codex CLI は `hooks.local.json` をサポートしていないため、1つの `hooks.json` に gwt managed hooks とユーザー定義 hooks を共存させる必要がある。

### 現在の実装

- `crates/gwt-core/src/config/skill_registration.rs` の `write_managed_codex_hooks()` が `.codex/hooks.json` を全上書き
- `.codex/hooks.json` は git tracked（v8.17.2 以降）
- Codex CLI は hooks.json 内の全 hook を並列実行する（上書きではなく累積）

### 関連SPEC

- SPEC-1438: スキル埋め込み — launch target worktree への project-scoped registration（closed）

## User Stories

### US1 - gwt managed hooks とユーザー定義 hooks の共存 (P1)

As a developer, I want gwt's skill registration to preserve my custom Codex hooks when it updates `.codex/hooks.json`, so that my project-specific hooks are not overwritten every time an agent launches.

### US2 - gwt managed hooks の安全な更新 (P1)

As a developer, I want gwt to update only its own managed hooks in `.codex/hooks.json` without touching my custom entries, so that I can safely commit the file and share hooks with my team.

### US3 - gwt managed hooks の識別と除去 (P2)

As a developer, I want to be able to identify which hooks in `.codex/hooks.json` are managed by gwt, so that I can manually review or remove them if needed.

## Acceptance Scenarios

1. `.codex/hooks.json` にユーザー定義の PreToolUse hook が存在する状態で agent を起動 → ユーザー定義 hook が保持され、gwt managed hooks が追加/更新される
2. `.codex/hooks.json` が存在しない状態で agent を起動 → gwt managed hooks のみで新規作成される
3. 既に gwt managed hooks が書き込まれた `.codex/hooks.json` で再度 agent を起動 → gwt managed hooks が重複しない（冪等性）
4. ユーザーが gwt managed hook と同じ event（例: PreToolUse）にカスタム hook を追加 → 両方が保持される
5. gwt managed hooks のバージョンが更新された場合 → 古い gwt managed hooks が新しいものに置き換わる

## Edge Cases

- `.codex/hooks.json` が不正な JSON の場合: エラーログを出力し、バックアップを作成してから新規作成
- `.codex/hooks.json` のパーミッションエラー: エラーログを出力し、agent起動は続行
- Codex hooks の event 名が将来追加された場合: 既存のマージロジックがそのまま動作する設計
- 同一 event に gwt managed hook が複数ある場合（将来拡張）: matcher ベースで個別に管理

## Functional Requirements

### マージロジック

- FR-001: `write_managed_codex_hooks()` は既存の `.codex/hooks.json` を読み込み、gwt managed hooks のみを追加/更新し、ユーザー定義 hooks を保持する
- FR-002: gwt managed hooks は識別子（例: `command` パスに `gwt-` プレフィックスを含む）で判別する
- FR-003: マージ処理は冪等であること（同じ状態で複数回実行しても結果が変わらない）
- FR-004: マージ後の JSON は `serde_json::to_string_pretty` でフォーマットする

### エラーハンドリング

- FR-010: 既存ファイルが不正な JSON の場合、警告ログを出力し `.codex/hooks.json.bak` にバックアップしてから新規作成する
- FR-011: ファイル書き込みに失敗した場合、`GwtError::ConfigWriteError` を返すが agent 起動は続行する

### 識別

- FR-020: gwt managed hooks は `command` フィールドに `gwt-` を含むスクリプトパスで識別する（例: `gwt-forward-hook.mjs`, `gwt-block-git-branch-ops.mjs`）
- FR-021: 将来的にマーカーコメントや metadata フィールドによる識別への移行を妨げない設計とする

### dirty worktree 防止

- FR-030: マージ結果が既存ファイルの内容と byte-for-byte 同一の場合、ファイルを書き換えない（不要な git 差分を防止）
- FR-031: マージ結果が既存ファイルと異なり実際にファイルを更新した場合、TUI 上でユーザーに通知する。通知内容は「`.codex/hooks.json` が更新されました。コミットの必要はありませんが、コミットしても構いません」に相当するメッセージ
- FR-032: 通知はステータスバーまたは軽量モーダルで表示し、agent 起動フローをブロックしない

## Non-Functional Requirements

- NFR-001: マージ処理の実行時間は 10ms 以内（ファイル I/O 除く）
- NFR-002: 既存のテストが破壊されないこと

## Success Criteria

- SC-001: ユーザー定義 hooks が存在する `.codex/hooks.json` で agent 起動後、ユーザー定義 hooks が保持されていること
- SC-002: gwt managed hooks が正しく追加/更新されていること
- SC-003: 同じ操作を複数回実行しても hooks が重複しないこと（冪等性）
- SC-004: 既存の `cargo test -p gwt-core` が全通過すること
- SC-005: managed hooks に変更がない場合、`.codex/hooks.json` に git 差分が出ないこと
- SC-006: managed hooks が実際に更新された場合、TUI 上に通知が表示されること
