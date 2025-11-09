# Quickstart: releaseブランチ経由の自動リリース

> 詳細な設計とテストケースは `spec.md` / `data-model.md` / `contracts/` を参照。ここでは実務手順のみをまとめる。

## 1. 事前チェック

1. `develop` にリリース対象コミットが揃っていることを確認。
2. 作業ツリーがクリーンであることを確認: `git status`。
3. 依存を最新化: `bun install`。
4. GitHub CLI 認証: `gh auth status`（未設定なら `gh auth login`）。
5. main Branch Protection: `lint`, `test`, `semantic-release`（release.yml の job 名）を Required Checks に登録し、`Restrict who can push` を有効化。

## 2. リリース開始

ローカルまたは Claude Code から `/release` を実行する。ローカル CLI 例:

```bash
scripts/create-release-branch.sh
```

このスクリプトは以下を行う:
- `gh workflow run create-release.yml --ref develop` を呼び出し、semantic-release dry-run で次バージョンを決定。
- `release/vX.Y.Z` ブランチを自動作成して `origin` へ push。
- 最新の workflow run ID を表示（`gh run watch` で監視可能）。

## 3. ワークフロー監視

1. `create-release.yml` が完了するまで `gh run watch` で待機。
2. `release.yml` を監視し、`lint` → `test` → `semantic-release` すべて success になることを確認。
3. `semantic-release` ステップ成功後、ログに `Published release <version>` が出力され、`release/vX.Y.Z` が main にマージされブランチ削除される。
4. main への push で `publish.yml` が起動し、`npm publish`（有効時）と `main` → `develop` のバックマージを実行。

## 4. 検証

- Git タグ `vX.Y.Z` と GitHub Release が作成されているか。
- `main` に `chore(release): X.Y.Z` コミットが存在するか。
- `develop` が最新 `main` と一致しているか（`git fetch origin develop && git log --oneline origin/main..origin/develop` で差分ゼロ）。
- npm publish を有効化している場合は npm registry 上のバージョンを確認。

## 5. トラブルシューティング

| 症状 | 原因 | 対処 |
| --- | --- | --- |
| `create-release.yml` がバージョンを決定できない | Conventional Commits 形式でないコミットが含まれる | コミットを修正し再実行 |
| `release.yml` が main への push で失敗 | `PERSONAL_ACCESS_TOKEN` 権限不足 or merge コンフリクト | PAT を更新、または release/vX.Y.Z を残したまま問題を解消し再実行 |
| `publish.yml` の back-merge が失敗 | develop に手動変更があり fast-forward できない | 手動で `git checkout develop && git merge main` を行い、workflow を再実行 |
| npm publish がスキップされる | `NPM_TOKEN` 未設定 or release commit でない | トークン設定、`semantic-release` 成功を確認 |

## 6. Hotfix 手順

1. 緊急修正が必要な場合は `hotfix/<issue>` を **main** から作成し、PR でレビュー後 main にマージ。
2. 直後に `publish.yml`（back-merge 部分）の `workflow_dispatch` を手動実行するか、手動で `develop` に `main` を反映。
3. 必要に応じて `/release` を再度実行して release/vX.Y.Z を更新。

## 7. Agent Context の更新

Spec コンテンツを変更したら以下を実行し、CLAUDE.md のアクティブ技術情報を同期する:

```bash
SPECIFY_FEATURE=SPEC-57fde06f .specify/scripts/bash/update-agent-context.sh claude
```
