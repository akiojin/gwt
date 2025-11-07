# Quickstart: releaseブランチ経由の自動リリース＆Auto Mergeフロー

## 1. 事前準備

```bash
# Worktree ルートで SPECIFY_FEATURE を設定（Plan/Tasks 用）
export SPECIFY_FEATURE=SPEC-57fde06f

# 依存を最新化
bun install
```

- develop が最新 remote を追従していることを確認: `git checkout develop && git pull --ff-only`
- release ブランチが存在しない場合は管理者が一度 `git push origin develop:release` で作成。
- GitHub Branch Protection: main に対し "Require status checks" に `lint`, `test`, `semantic-release` の 3 ジョブを登録し、"Allow auto-merge" をオン、"Restrict who can push" で管理者のみ許可。

## 2. `/release` コマンド実行

```bash
# develop ブランチ上で実行
bun run cli /release
# もしくは Claude Code から /release を呼び出す
```

- スクリプトは `git push origin develop:release --force-with-lease=refs/heads/release` を実行し、既存 release→main PR を再利用。
- PR が存在しない場合は `gh pr create --base main --head release --label release --fill --title "Release $(date +%Y-%m-%d)"` を呼び出す。
- 直後に `gh pr merge --auto --merge <PR_NUMBER>` で Auto Merge をセット。

## 3. CI / Required チェックの監視

```bash
# release ブランチの workflow を確認
open "https://github.com/<org>/<repo>/actions/workflows/release.yml"
```

- `lint` → `test` → `semantic-release` の順に成功することを確認。
- semantic-release が完了すると GitHub Release + npm publish が行われ、PR 本文にリリースノートリンクが追記される。
- 全チェック success 後は Auto Merge により main に取り込まれる。マージ完了までは 10 分以内が目安。

## 4. トラブルシューティング

| 症状 | 原因 | 対処 |
| --- | --- | --- |
| Auto Merge が pending のまま | Required チェックが success になっていない | Actions タブで失敗ジョブを再実行。`gh pr checks <PR#>` で状況確認。
| PR が複数存在 | `/release` 実行前に手動 PR が作成された | `/release` に古い PR を close させるか `gh pr close <id>` を手動で実施し再実行。
| semantic-release が release ブランチで失敗 | BREAKING CHANGE などの解析結果 | コミットログを修正し rebase → `/release` を再実行。タグが不完全な場合は `semantic-release --no-ci` で確認。
| main へ直接 push してしまった | Branch Protection を無効化している | 直ちに revert し、ドキュメント通り release フローを使用。CLAUDE.md のフロー節を参照。

## 5. Hotfix プロセス

1. main で致命的不具合が発覚した場合は `hotfix/<issue>` ブランチを main から起こす。
2. 修正後に main へ PR を作成しレビュー / Required チェックを通過させて手動マージ。
3. `git checkout develop && git merge main` を行い release ブランチへも反映させた上で `/release` を通常実行。

## 6. Agent Context 同期

Plan/Quickstart/Contracts を更新したら以下を実行し CLAUDE.md の技術情報を最新化する:

```bash
SPECIFY_FEATURE=$SPECIFY_FEATURE ./.specify/scripts/bash/update-agent-context.sh claude
```
