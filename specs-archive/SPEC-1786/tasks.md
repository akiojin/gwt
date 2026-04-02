# Tasks: SPEC-1786 — gwt-spec の hooks.json マージ

## Phase 1: Foundational — gwt-core マージロジック

### US1/US2: マージロジックと差分検出

- [x] T001: [TDD] `merge_managed_codex_hooks()` のテスト — ユーザー hooks 保持、gwt hooks 追加/置換、冪等性
  - `crates/gwt-core/src/config/skill_registration.rs` `#[cfg(test)] mod tests`
  - テストケース: (1) ユーザー定義 PreToolUse + managed → 両方保持、(2) managed のみ → managed で初期化、(3) 2回実行 → 同一結果（冪等性）、(4) managed バージョン更新 → 古い managed 除去+新規追加
- [x] T002: `merge_managed_codex_hooks(existing: &Value, managed: &Value) -> Value` 実装
  - `crates/gwt-core/src/config/skill_registration.rs`
  - event 配列内で `gwt-` を含む command を持つエントリを除去し、managed エントリを追加
  - `is_managed_hook_command()` を再利用

- [x] T003: [TDD] `codex_hooks_needs_update()` のテスト — 変更なし→false、変更あり→true、ファイル未存在→true
  - `crates/gwt-core/src/config/skill_registration.rs` `#[cfg(test)] mod tests`
- [x] T004: `codex_hooks_needs_update(codex_root: &Path) -> bool` 実装
  - `crates/gwt-core/src/config/skill_registration.rs`
  - 既存 hooks.json を読み込み → マージ結果と byte-for-byte 比較 → 差分があれば true
  - pub 公開（gwt-tui から呼ぶため）

- [x] T005: `write_managed_codex_hooks()` をマージ方式に変更 [P]
  - `crates/gwt-core/src/config/skill_registration.rs`
  - 既存ファイルを読み込み → `merge_managed_codex_hooks()` でマージ → 既存と同一ならスキップ（FR-030）→ 差分がある場合のみ書き込み

### エラーハンドリング

- [x] T006: [TDD] 不正 JSON のバックアップ + 新規作成テスト
  - `crates/gwt-core/src/config/skill_registration.rs` `#[cfg(test)] mod tests`
  - 既存ファイルが不正 JSON → `.codex/hooks.json.bak` にバックアップ → managed hooks で新規作成（FR-010）
- [x] T007: 不正 JSON ハンドリング実装 [P]
  - `crates/gwt-core/src/config/skill_registration.rs` `write_managed_codex_hooks()` 内

## Phase 2: TUI 確認ダイアログ

### US1/US2: 事前確認フロー

- [x] T010: `ConfirmAction::EmbedCodexHooks` バリアント追加
  - `crates/gwt-tui/src/screens/confirm.rs`
  - 英語ダイアログテキスト: title="Update Codex Hooks", message=tracked file 説明, confirm="Embed", cancel="Skip"

- [x] T011: `spawn_agent_session()` に Codex 確認ダイアログフローを追加
  - `crates/gwt-tui/src/app.rs`
  - Codex agent の場合: `codex_hooks_needs_update()` 呼び出し → true なら `ConfirmState` を model に設定して return
  - Codex 以外: 既存フロー（確認なし）（FR-034）

- [x] T012: 確認ダイアログの応答ハンドリング
  - `crates/gwt-tui/src/app.rs` update 関数内の confirm ハンドラ
  - Embed → skill registration 実行 → `spawn_agent_session()` を再呼び出し（skip_hooks_confirm フラグで再確認を防止）
  - Skip → skill registration スキップ → agent 起動のみ

## Phase 3: US3 — managed hooks の識別

- [x] T020: [TDD] managed hook 識別テスト — `gwt-` プレフィックスで正しく判別されること [P]
  - `crates/gwt-core/src/config/skill_registration.rs` `#[cfg(test)] mod tests`
  - 既存の `is_managed_hook_command()` が Codex の command 形式にも対応していることを確認

## Phase 4: Polish / Cross-Cutting

- [x] T030: 既存テスト回帰確認
  - `cargo test -p gwt-core -p gwt-tui`
  - `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] T031: 手動 E2E テスト
  - (1) Codex agent 初回起動 → 確認ダイアログ表示 → Embed → hooks 更新
  - (2) 2回目起動 → ダイアログ非表示
  - (3) ユーザー hooks 追加後に起動 → ダイアログ表示 → Embed → ユーザー hooks 保持
  - (4) Skip → hooks 未変更
  - (5) Claude Code agent → ダイアログ非表示
