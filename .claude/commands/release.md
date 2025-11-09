---
description: developからrelease/vX.Y.Zブランチを作成し、完全自動リリースフローを開始します。
tags: [project]
---

# リリースコマンド

developブランチから`release/vX.Y.Z`ブランチを自動作成し、unity-mcp-serverと共通のリリースパイプラインを起動します。

## 実行内容

1. 現在のブランチがdevelopであることを確認
2. developブランチを最新に更新（`git pull`）
3. `npx semantic-release --dry-run --branches develop` を実行してバージョン番号を判定
4. `release/vX.Y.Z` ブランチを develop から作成
5. リモートに push (`origin/release/vX.Y.Z`)
6. GitHub Actions が以下を自動実行:
   - **create-release.yml**: releaseブランチ作成フェーズのログを記録
   - **release.yml (release/**)**: semantic-releaseで CHANGELOG とタグを生成し、release → main を直接マージ
   - **publish.yml (main)**: npm publish（必要に応じて）、main → develop バックマージ

## 前提条件

- develop ブランチ上でクリーンな作業ツリーになっていること
- GitHub CLI (`gh`) が認証済み (`gh auth login`)
- 最新コミットが Conventional Commits 形式であること
- semantic-release がバージョンを決定できる差分が存在すること

## スクリプト実行

ローカルから同じ処理を実行するには `scripts/create-release-branch.sh` を使います。

```bash
scripts/create-release-branch.sh
```

スクリプトは GitHub Actions の `create-release.yml` を起動し、以下を自動で行います:

1. develop ブランチの semantic-release dry-run
2. 次のバージョン番号を算出
3. `release/vX.Y.Z` ブランチを作成して push
4. 直近のワークフロー実行 ID を表示し、`gh run watch` で追跡できるようにします

## トラブルシューティング

- release ブランチ push 後に release.yml が失敗した場合は、Actions から再実行し、semantic-release ログを確認してください。
- main で npm publish が有効な場合は `NPM_TOKEN` が正しく設定されていることを確認してください。
- release ブランチが既に存在する場合は、既存の release/vX.Y.Z を削除または完了させてから再実行してください。
