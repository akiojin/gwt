# fix: Cleanup — 保護ブランチの事前表示とエラーハンドリング (#1404)

## 背景

Cleanup処理でリポジトリルールにより保護されたブランチの削除が HTTP 422 でハードエラーになる問題の修正。
事前表示 + エラーハンドリングの2段階で対応。

## 実装ステップ

- [x] T001 gwt-spec Issue 作成 (#1404)
- [x] T002 Rust テスト追加 — classify_delete_branch_error / get_branch_deletion_rules
- [x] T003 `classify_delete_branch_error` に protected ケース追加
- [x] T004 `get_branch_deletion_rules()` 追加
- [x] T005 `get_cleanup_branch_protection` Tauri コマンド追加 + 登録
- [x] T006 `cleanup_worktrees` の "Protected:" ハンドリング
- [x] T007 Frontend テスト追加 — branchProtection / badge (4テスト)
- [x] T008 `CleanupModal.svelte` に保護状態の取得・表示
- [x] T009 全テスト通過確認
- [x] T010 clippy 検証

## 検証結果

- [x] `cargo test -p gwt-core` — 全テスト通過
- [x] `cargo test -p gwt-tauri` — 540テスト通過
- [x] `cargo clippy --all-targets --all-features -- -D warnings` — 警告なし
- [x] `cd gwt-gui && pnpm test src/lib/components/CleanupModal.test.ts` — 39テスト通過
- [x] `npx markdownlint-cli` — 4つの SKILL.md エラーなし

---

## TODO: Issue #1265 再発（v8.1.1）対策の正規化境界強化

## 背景（v8.1.1 再発）

Issue #1265 にて `v8.1.1` でも Windows Launch Agent の `npx.cmd` 起動失敗が報告されたため、コマンド正規化を launch/probe/path-resolve 境界で再統一し、再発検知テストを拡張する。

## 実装ステップ（v8.1.1 再発）

- [x] T001 `runner.rs` で command path resolve 前にトークン正規化を適用
- [x] T002 `build_fallback_launch` の resolved command を共通正規化
- [x] T003 `terminal.rs` の launch command 正規化を OS 条件分岐から共通化
- [x] T004 `terminal.rs` の command probe (`--version`, `features list`) で同一正規化を適用
- [x] T005 回帰テスト追加（wrapped resolved path / wrapped lookup token / probe normalization）
- [x] T006 対象テストと `cargo fmt --check` で検証

## 検証結果（v8.1.1 再発）

- [x] `cargo test -p gwt-core terminal::runner -- --test-threads=1`
- [x] `cargo test -p gwt-core terminal::pty -- --test-threads=1`
- [x] `cargo test -p gwt-tauri normalize_launch_command_for_platform -- --test-threads=1`
- [x] `cargo test -p gwt-tauri normalized_process_command -- --test-threads=1`
- [x] `cargo fmt --all -- --check`
