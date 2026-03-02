# fix: Cleanup — 保護ブランチの事前表示とエラーハンドリング (#1404)

## 背景（Cleanup保護ブランチ）

Cleanup処理でリポジトリルールにより保護されたブランチの削除が HTTP 422 でハードエラーになる問題の修正。
事前表示 + エラーハンドリングの2段階で対応。

## 実装ステップ（Cleanup保護ブランチ）

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

## 検証結果（Cleanup保護ブランチ）

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

---

## TODO: Worktree詳細ビューPRタブが常に"No PR"になるバグ修正

## 背景（Worktree PR表示バグ）

コミット `18ee87c4` で `resolvedPrNumber` に `state !== "OPEN"` チェック追加。
副作用として MERGED/CLOSED PR しか持たないブランチで常に "No PR" となるリグレッション発生。
修正方針: PR state ではなくブランチ名で staleness を判定する。

## 実装ステップ（Worktree PR表示バグ）

- [x] T001 テスト修正（TDD RED）: MERGED PR表示のアサーション変更
- [x] T002 RED確認: テスト実行し失敗を確認
- [x] T003 実装: `latestBranchPrBranch` 変数追加・resolvedPrNumber ロジック変更
- [x] T004 GREEN確認: テスト全87件 pass
- [x] T005 Lint/型チェック検証: svelte-check 0 errors

---

## TODO: Windows タブ切り替え時のフリッカー修正

## 背景（Windows タブフリッカー）

Windows 環境でタブ切り替え時に `$derived`（同期）と `$effect`（非同期）のタイミング差で 1 フレーム全ターミナル非表示のギャップが生じ、背景フラッシュが発生する。`isTerminalTabVisible()` が `visibleTerminalTabId`（`$effect` で非同期更新）に依存していることが根本原因。

## 実装ステップ（Windows タブフリッカー）

- [x] T001 gwt-spec Issue 作成 (#1410)
- [x] T002 TDD テスト追加（jsdom では $effect フラッシュ済みのため GREEN だが仕様テストとして有効）
- [x] T003 `isTerminalTabVisible()` 修正（GREEN 化）
- [x] T004 テスト GREEN 確認（33/33 pass）
- [x] T005 型チェック・lint 確認（svelte-check 0 errors）
- [x] T006 コミット＆プッシュ（e637213f）

## 検証結果（Windows タブフリッカー）

- [x] `pnpm test src/lib/components/MainArea.test.ts` — 33 tests passed
- [x] `npx svelte-check --tsconfig ./tsconfig.json` — 0 errors, 1 warning（既存・変更対象外）
- [x] `bunx commitlint --from HEAD~1 --to HEAD` — 通過
