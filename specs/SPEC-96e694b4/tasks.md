# タスク: Codex CLI gpt-5.2-codex 対応

**仕様ID**: `SPEC-96e694b4`
**ポリシー**: CLAUDE.md の TDD ルールに基づき、必ず RED→GREEN→リグレッションチェックの順に進める。

## フェーズ1: RED

- [x] **T3201** [US1] `crates/gwt-cli/src/tui/screens/wizard.rs` に Codex モデル一覧のテストを追加し、非defaultのモデルIDが `gpt-5.2-codex`, `gpt-5.1-codex-max`, `gpt-5.1-codex-mini`, `gpt-5.2` の順で4件であることと、`gpt-5.2-codex` が Extra high を含むことを検証する。
- [x] **T3202** [US2] `crates/gwt-core/src/agent/codex.rs` のデフォルトモデルテストを `gpt-5.2-codex` 期待に更新する。
- [x] **T3203** [US2] `crates/gwt-cli/src/main.rs` の起動ログテストを `gpt-5.2-codex` 期待に更新する。

## フェーズ2: GREEN

- [x] **T3204** [US1] `crates/gwt-cli/src/tui/screens/wizard.rs` の Codex モデル一覧に `gpt-5.2-codex` を追加し、並び順を仕様に合わせて調整する。
- [x] **T3205** [US2] `crates/gwt-core/src/agent/codex.rs` のデフォルトモデルを `gpt-5.2-codex` に更新する。

## フェーズ3: リグレッションチェック

- [x] **T3206** `cargo test -p gwt-cli -p gwt-core` を実行し、モデル更新に起因する失敗がないことを確認する。
- [x] **T3207** `cargo build --release` を実行し、ビルドが成功することを確認する。
