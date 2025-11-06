# Release Command

developブランチの変更をmainブランチにマージし、semantic-releaseによる自動リリースを実行します。

## 実行手順

1. GitHub Actions workflow_dispatchでrelease-triggerワークフローを起動します
2. gh CLIコマンドを使用: `gh workflow run release-trigger.yml --ref develop -f confirm=release`
3. ワークフロー実行状態を監視し、ユーザーに結果を報告します

## 実行内容

release-triggerワークフローは以下を自動実行します:

1. developブランチをチェックアウト
2. developをmainにマージ（可能ならfast-forward、不可能ならmerge commit作成）
3. mainへpush
4. mainへのpushトリガーでReleaseワークフローが自動起動
5. Releaseワークフローがsemantic-releaseを実行:
   - バージョン判定
   - CHANGELOG.md生成
   - npm公開
   - GitHubリリース作成
   - package.jsonとCHANGELOG.mdをコミット

## 注意事項

- このコマンドはdevelopブランチで実行する必要があります
- developとmainに競合がある場合、マージコミットが作成されます
- リリース可能なコミットがない場合、semantic-releaseは何もしません
- リリースには`SEMANTIC_RELEASE_TOKEN`と`NPM_TOKEN`のシークレットが必要です
