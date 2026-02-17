# タスク: SPEC-bare-wt01

## T1: テスト作成（TDD）

- [ ] ベアリポジトリで未フェッチリモートブランチからワークツリー作成テスト
- [ ] ベアリポジトリで未フェッチリモートブランチをベースに新規ブランチ作成テスト

## T2: Change A — `remote_exists` の `ls-remote` 改善

- [ ] `run_git_with_timeout` に置き換え
- [ ] タイムアウト/エラー時は `Ok(false)` を返す

## T3: Change B + C — `create_for_branch` のブランチ解決ロジック修正

- [ ] リモート解決前にローカルブランチを先行チェック (Change C)
- [ ] `fetch_all` 後にローカルブランチをフォールバック確認 (Change B)

## T4: Change D — `create_new_branch` のフォールバック追加

- [ ] `remote_exists` 失敗時に `fetch_all` + ローカル存在チェック

## T5: 検証

- [ ] `cargo test` 通過
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` 通過
