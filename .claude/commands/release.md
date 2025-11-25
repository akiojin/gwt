---
description: developからmainへのRelease PRを作成し、完全自動リリースフローを開始します。
tags: [project]
---

# リリースコマンド

develop から main への Release PR を作成し、マージ後に完全自動リリースフローを実行します。

## 実行内容

1. `gh workflow run create-release.yml --ref develop` を実行
2. ワークフローが Conventional Commits を解析し、次のバージョンを決定
3. develop → main の Release PR を作成し、自動マージを有効化
4. CI チェック通過後、Release PR が main に自動マージ
5. GitHub Actions が以下を自動実行:
   - **release.yml (main)**: release-please でタグ・GitHub Release を作成
   - **publish.yml (main)**: npm publish（必要に応じて）、main → develop バックマージ

## 前提条件

- develop ブランチにリリース対象コミットが揃っていること
- GitHub CLI (`gh`) が認証済み (`gh auth login`)
- 最新コミットが Conventional Commits 形式であること
- `feat:` または `fix:` コミットが存在すること

## コマンド実行

```bash
gh workflow run create-release.yml --ref develop
```

## Release PR の確認と操作

```bash
# Release PR を確認
gh pr list --base main --head develop

# Release PR を手動マージ（自動マージが有効でない場合）
gh pr merge <PR番号> --merge
```

## トラブルシューティング

- Release PR が作成されない場合は、`feat:` または `fix:` コミットが存在するか確認してください。
- Release PR 作成後に release.yml が失敗した場合は、Actions から再実行し、ログを確認してください。
- main で npm publish が有効な場合は `NPM_TOKEN` が正しく設定されていることを確認してください。
- Release PR が既に存在する場合は、既存の PR を確認して対応してください。
