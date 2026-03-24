---
name: release
description: "developブランチでバージョン更新を行い、mainへのRelease PRを作成します。Use when the user says '/release', 'リリース', 'release PR', or wants to create a new version release."
---

# Release

developブランチでバージョン更新・CHANGELOG更新を行い、main への Release PR を作成します。

## Instructions

このスキルは `.claude/commands/release.md` に定義されたリリースフローに従って実行してください。

詳細な手順は `.claude/commands/release.md` を参照してください。

## Quick Reference

### フロー概要

```
develop (バージョン更新・CHANGELOG更新) → main (PR)
                                            ↓
                                  GitHub Release & npm publish (自動)
```

### 前提条件

- `develop` ブランチにチェックアウトしていること
- `git-cliff` がインストールされていること
- `gh` CLI が認証済み
- 前回リリースタグ以降にコミットがあること

### 主な手順

1. ブランチ確認（developであること）
2. リモート同期（fetch & pull）
3. リリース対象コミット確認
4. バージョン判定（git-cliff --bumped-version）
5. ファイル更新（Cargo.toml, package.json, Cargo.lock, CHANGELOG.md）
6. リリースコミット作成
7. developをプッシュ
8. develop → main のPR作成
9. 完了メッセージ表示
