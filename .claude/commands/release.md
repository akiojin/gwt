---
description: developからmainへのRelease PRを作成し、完全自動リリースフローを開始します。
tags: [project]
---

# リリースコマンド

develop から main への Release PR を作成し、マージ後に完全自動リリースフローを実行します。

## 実行内容

1. `gh workflow run prepare-release.yml --ref develop` を実行
2. develop → main の Release PR を作成
3. CI チェック通過後、Release PR が main に自動マージ
4. GitHub Actions が以下を自動実行:
   - **release.yml (main)**: release-please でタグ・GitHub Release・Release PR を作成
   - **publish.yml (v* tag)**: npm publish

## 前提条件

- develop ブランチにリリース対象コミットが揃っていること
- GitHub CLI (`gh`) が認証済み (`gh auth login`)
- 最新コミットが Conventional Commits 形式であること
- `feat:` または `fix:` コミットが存在すること

## コマンド実行

```bash
gh workflow run prepare-release.yml --ref develop
```

## Release PR の確認と操作

```bash
# Release PR を確認
gh pr list --base main --head develop

# Release PR を手動マージ（自動マージが有効でない場合）
gh pr merge <PR番号> --merge
```

## トラブルシューティング

- Release PR が作成されない場合は、develop ブランチから main へのコミットがあるか確認してください。
- release.yml が失敗した場合は、Actions から再実行し、ログを確認してください。
- npm publish が有効な場合は `NPM_TOKEN` が正しく設定されていることを確認してください。
- Release PR が既に存在する場合は、既存の PR を確認して対応してください。
