---
description: develop→main リリースPRを自動生成し、main push で semantic-release を発火させるカスタムスラッシュコマンドです。
---

## User Input

```text
$ARGUMENTS
```

入力を受け取った場合は内容を考慮してください（通常は空で実行）。

# Release Command

`/release` コマンドは develop ブランチから main ブランチへのリリースPRを生成し、必要な自動化をまとめて起動するためのエントリポイントです。PRが Required チェックを通過して main にマージされると、push をトリガーに semantic-release が自動実行されます。

## 実行手順

1. develop ブランチを最新化し、リリースしたいコミットが揃っていることを確認します。
2. Claude Code で `/release` コマンドを実行するか、`scripts/create-release-pr.sh` をローカルで実行、あるいは `gh workflow run release-trigger.yml --ref develop -f confirm=release` を使って GitHub Actions から起動します。
3. コマンド/ワークフローは `develop` → `main` の PR を作成または更新し、Auto Merge を有効化します（Required チェック: lint / test など）。
4. Required チェックがすべて成功すると PR が自動的に main にマージされ、`main` への push を起点に `.github/workflows/release.yml` が semantic-release を実行します。
5. release.yml が `package.json` / `CHANGELOG.md` の更新、タグ作成、GitHub Release、npm publish（設定時のみ）、develop へのバックマージを完了させます。

## 実行内容

release コマンド／`release-trigger.yml` ワークフロー／`scripts/create-release-pr.sh` は次の処理を行います:

1. develop ブランチをチェックアウトし、`git pull --ff-only origin develop` で最新状態に同期。
2. 既存の `develop` → `main` PR があればタイトル・本文を更新、なければ新規作成。
3. PR 本文には semantic-release が main で行う処理（バージョン決定、CHANGELOG更新、タグ・GitHub Release、npm publish（任意）、develop への自動バックマージ）を明記。
4. `gh pr merge --auto --merge` を呼び出し、Required チェックを条件に Auto Merge を設定。
5. Step Summary へ PR URL と head SHA を記録し、モニタリング手順を案内。

## 注意事項

- release.yml は main ブランチへの push だけをトリガーとします。release ブランチは存在しません。
- Required チェックはリポジトリの Branch Protection で定義されたジョブ（例: `lint`, `test`）。semantic-release 自体は main push の Release ワークフロー内で実行されます。
- npm publish を有効化する場合は `.releaserc.json` の `@semantic-release/npm` セクションで `npmPublish: true` に変更し、`NPM_TOKEN` シークレットを設定してください。
- release ワークフローは、develop との同期に失敗した場合 `sync/main-to-develop-<timestamp>` ブランチを作成し、`develop` 向けの同期PRを自動で開きます。コンフリクトを解消後にPRをマージしてください。
