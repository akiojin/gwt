# 実装計画: From Issue の Branch Exists 誤判定（stale remote-tracking ref）

**仕様ID**: `SPEC-c2df2e7a` | **日付**: 2026-02-25 | **仕様書**: `specs/SPEC-c2df2e7a/spec.md`

## 目的

- stale remote-tracking ref による `Branch Exists` 誤判定を除去し、From Issue の選択不可誤表示を防止する。

## 技術コンテキスト

- **バックエンド**: Rust 2021（`crates/gwt-core/src/git/issue.rs`）
- **フロントエンド**: 変更不要（既存API結果を利用）
- **Git連携**: `git branch --list` / `git branch -r --list` / `git ls-remote --heads`
- **テスト**: `cargo test -p gwt-core git::issue::tests::`, `cargo clippy -p gwt-core --all-targets -- -D warnings`
- **前提**: `find_existing_issue_branch` のI/Fは維持し、判定精度のみ改善する

## 実装方針

### Phase 1: TDD（RED）

- `find_branch_for_issue()` の再現テストを追加する。
1. ローカル branch 優先で検出される
2. 実リモート branch がある場合に検出される
3. stale remote-tracking のみの場合は検出されない
- remote-tracking 解析用ユーティリティのユニットテストを追加する。

### Phase 2: 判定ロジック改修

- `find_branch_for_issue()` の remote 分岐で、候補ごとに `ls-remote` 実在確認を挟む。
- symbolic ref（`origin/HEAD -> ...`）を候補から除外する。
- 候補重複時の `ls-remote` 再実行を避ける。
- `ls-remote` 失敗時はエラー返却し、曖昧な成功を返さない。

### Phase 3: GREEN 検証

- `cargo test -p gwt-core git::issue::tests::` を実行し、追加ケースを含めて全件成功を確認する。
- `cargo clippy -p gwt-core --all-targets -- -D warnings` を実行し、警告ゼロを確認する。

## テスト

### バックエンド

- `test_find_branch_for_issue_finds_local_branch`
- `test_find_branch_for_issue_confirms_remote_branch_with_ls_remote`
- `test_find_branch_for_issue_ignores_stale_remote_tracking_ref`
- `test_split_remote_tracking_branch_parses_valid_ref`
- `test_split_remote_tracking_branch_rejects_symbolic_ref`

### フロントエンド

- 変更なし（判定APIの戻り値改善のみ）。既存UIテスト対象外。
