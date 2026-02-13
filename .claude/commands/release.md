---
description: developブランチでバージョン更新を行い、mainへのRelease PRを作成します（LLMベース）。
tags: [project]
---

# リリースコマンド（LLMベース）

develop ブランチでバージョン更新・CHANGELOG更新を行い、main への Release PR を作成します。

## フロー概要

```
develop (バージョン更新・CHANGELOG更新) → main (PR)
                                            ↓
                                  GitHub Release & npm publish (自動)
```

## 前提条件

- `develop` ブランチにチェックアウトしていること
- `git-cliff` がインストールされていること（`cargo install git-cliff`）
- `gh` CLI が認証済み（`gh auth login`）
- 前回リリースタグ以降にコミットがあること

## 処理フロー

以下の手順を **順番に** 実行してください。エラーが発生した場合は即座に中断し、エラーメッセージを日本語で表示してください。

### 1. ブランチ確認

```bash
git rev-parse --abbrev-ref HEAD
```

**判定**: 結果が `develop` でなければ、以下のメッセージを表示して中断：
> 「エラー: developブランチでのみ実行可能です。現在のブランチ: {ブランチ名}」

### 2. リモート同期

```bash
git fetch origin main develop
git pull origin develop
```

### 3. リリース対象コミット確認

```bash
PREV_TAG=$(git describe --tags --abbrev=0 2>/dev/null || echo "")
```

上記で取得したタグから現在までのコミット数を確認:

```bash
# タグが存在する場合
git rev-list {PREV_TAG}..HEAD --count

# タグが存在しない場合（初回リリース）
git rev-list --count HEAD
```

**判定**:
- タグが存在しない場合: 初回リリースとして続行（全コミットがリリース対象）
- タグが存在し、コミット数が 0 の場合、以下のメッセージを表示して中断：
> 「エラー: リリース対象のコミットがありません。」

### 4. バージョン判定

```bash
git-cliff --bumped-version
```

**出力例**: `v6.5.2`

このバージョンを `NEW_VERSION` として記録（例: `6.5.2`、`v` は除去）。

### 5. ファイル更新

以下のファイルを更新してください：

#### 5.1 ルート Cargo.toml

`version = "X.Y.Z"` を `version = "{NEW_VERSION}"` に更新

#### 5.2 package.json

`"version": "X.Y.Z"` を `"version": "{NEW_VERSION}"` に更新

#### 5.3 Cargo.lock

```bash
cargo update -w
```

#### 5.4 CHANGELOG.md

前回リリースタグ以降の変更のみを追加してください。git-cliffが過去の変更を含める場合は、手動でv{PREV_TAG}以降の変更のみを追加してください。

```bash
git-cliff --unreleased --tag v{NEW_VERSION} --prepend CHANGELOG.md
```

**注意**: CHANGELOGに既に含まれている変更が重複しないよう確認してください。

### 6. リリースコミット作成

```bash
git add -A
git commit -m "chore(release): v{NEW_VERSION}"
```

### 7. developをプッシュ

```bash
git push origin develop
```

**失敗時**: 最大3回リトライ。それでも失敗した場合：
> 「エラー: pushに失敗しました。ネットワーク接続を確認してください。」

### 8. PR作成

まず既存PRを確認：

```bash
gh pr list --base main --head develop --state open --json number,title
```

#### 既存PRがある場合

> 「既存のRelease PR（#{PR番号}）を更新しました。」
> 「URL: {PR URL}」

#### 既存PRがない場合

PRを作成：

```bash
gh pr create \
  --base main \
  --head develop \
  --title "chore(release): v{NEW_VERSION}" \
  --label release \
  --body "{PR_BODY}"
```

**PR_BODY の内容**（LLMが生成）：

PR bodyには以下を含めてください：
- `## Summary` - このリリースの概要（変更内容を要約）
- `## Changes` - 主な変更点をリスト形式で
- `## Version` - バージョン番号

### 9. 完了メッセージ

> 「リリース準備が完了しました。」
> 「バージョン: v{NEW_VERSION}」
> 「PR URL: {PR URL}」
> 「PRがマージされると、GitHub ReleaseとnpmへのPublishが自動実行されます。」

## マージ後の自動処理

PRがmainにマージされると、`.github/workflows/release.yml` が以下を自動実行：

1. Git タグを作成 (`v{NEW_VERSION}`)
2. GitHub Release を作成
3. クロスコンパイル済みバイナリをアップロード
4. npm へ publish

## トラブルシューティング

### git-cliff がインストールされていない場合

```bash
cargo install git-cliff
```

### 認証エラーが発生した場合

```bash
gh auth login
```

### push が拒否された場合

ブランチ保護ルールを確認するか、管理者に連絡してください。
