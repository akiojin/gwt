---
description: develop から release ブランチを作成し、完全自動リリースフローを開始するカスタムスラッシュコマンドです。
---

## User Input

```text
$ARGUMENTS
```

入力を受け取った場合は内容を考慮してください(通常は空で実行)。

# Release Command

`/release` コマンドは develop ブランチから release ブランチを作成し、完全自動リリースフローを開始します。

## リリースフロー全体像

```
develop
  ↓ /release コマンド実行
release/vX.Y.Z (作成)
  ↓ release.yml 自動実行
  - semantic-release 実行
  - CHANGELOG.md, package.json 更新
  - Git タグ作成
  - GitHub Release 作成
  - release/vX.Y.Z → main へ直接マージ
  - main → develop へバックマージ
  - release/vX.Y.Z ブランチ削除
main
  ↓ publish.yml 自動実行
  - npm publish (設定時)
develop (最新状態に戻る)
```

## 実行手順

1. develop ブランチを最新化し、リリースしたいコミットが揃っていることを確認します。
2. Claude Code で `/release` コマンドを実行するか、`gh workflow run create-release.yml --ref develop` を使って GitHub Actions から起動します。
3. `create-release.yml` が semantic-release をドライラン実行し、次のバージョンを決定して `release/vX.Y.Z` ブランチを作成します。
4. `release.yml` が自動的にトリガーされ、以下を順次実行します：
   - semantic-release による CHANGELOG/タグ/GitHub Release 作成
   - release/vX.Y.Z → main への直接マージ
   - main → develop へのバックマージ
   - release/vX.Y.Z ブランチの削除
5. `publish.yml` が main への push をトリガーに実行され、npm publish（設定時）を行います。

## 実行内容

`/release` コマンドは `create-release.yml` ワークフローを起動し、次の処理を行います:

1. develop ブランチをチェックアウトし、最新状態を取得。
2. `npx semantic-release --dry-run --branches develop` を実行し、次のバージョン番号を決定。
3. Conventional Commits に準拠していないコミットがあればエラーで停止。
4. `release/vX.Y.Z` ブランチを作成してプッシュ。
5. `release.yml` が自動的にトリガーされます。

## CLI コマンド実装

`/release` コマンドは以下を実行します:

```bash
gh workflow run create-release.yml --ref develop
```

## 注意事項

- semantic-release は `release/*` ブランチで実行されます (`.releaserc.json` で設定)。
- main ブランチは常にクリーンな状態を保ち、リリース済みのコードのみが含まれます。
- **PR を経由せず直接マージ**されるため、高速にリリースが完了します。
- npm publish を有効化する場合は `.releaserc.json` の `@semantic-release/npm` セクションで `npmPublish: true` に変更し、`NPM_TOKEN` シークレットを設定してください。
- バックマージは release.yml 内で自動実行されます。コンフリクトが発生した場合はワークフローが失敗します。
- `PERSONAL_ACCESS_TOKEN` シークレットが設定されていることを確認してください (マージとバックマージに必要)。

実行してください。
