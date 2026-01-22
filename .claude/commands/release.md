---
description: developブランチからmainへのRelease PRを作成します（LLMベース）。
tags: [project]
---

# リリースコマンド（LLMベース）

develop ブランチ上でリリース準備を行い、main への Release PR を作成します。

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

### 2. main同期チェック

```bash
git fetch origin main develop
git merge-base --is-ancestor origin/main origin/develop
```

**判定**: コマンドが失敗（exit code != 0）した場合、以下のメッセージを表示して中断：
> 「エラー: mainとdevelopに差異があります。先にmainをdevelopにマージしてください。」
> 「実行: `git merge origin/main`」

### 3. 既存リリースコミット確認

```bash
git log -1 --pretty=%s
```

**判定**: 結果が `chore(release):` で始まる場合、以下のメッセージを表示して中断：
> 「エラー: 既にリリースコミットが存在します。追加の変更をコミットしてから再実行してください。」

### 4. リリース対象コミット確認

```bash
git describe --tags --abbrev=0 2>/dev/null || echo "v0.0.0"
```

上記で取得したタグから現在までのコミット数を確認:

```bash
git rev-list {前回タグ}..HEAD --count
```

**判定**: コミット数が 0 の場合、以下のメッセージを表示して中断：
> 「エラー: リリース対象のコミットがありません。」

### 5. バージョン判定

```bash
git-cliff --bumped-version
```

**出力例**: `v6.5.2`

このバージョンを `NEW_VERSION` として記録（例: `6.5.2`、`v` は除去）。

### 6. ファイル更新

以下のファイルを更新してください：

#### 6.1 ルート Cargo.toml

`version = "X.Y.Z"` を `version = "{NEW_VERSION}"` に更新

#### 6.2 crates/配下の全 Cargo.toml

以下のファイルの `version = "X.Y.Z"` を更新：
- `crates/gwt-cli/Cargo.toml`
- `crates/gwt-core/Cargo.toml`
- その他 `crates/*/Cargo.toml` が存在する場合

#### 6.3 package.json

`"version": "X.Y.Z"` を `"version": "{NEW_VERSION}"` に更新

#### 6.4 Cargo.lock

```bash
cargo update -w
```

#### 6.5 CHANGELOG.md

```bash
git-cliff --unreleased --tag v{NEW_VERSION} --prepend CHANGELOG.md
```

### 7. リリースコミット作成

```bash
git add -A
git commit -m "chore(release): v{NEW_VERSION}"
```

### 8. developへプッシュ

```bash
git push origin develop
```

**失敗時**: 最大3回リトライ。それでも失敗した場合：
> 「エラー: pushに失敗しました。ネットワーク接続を確認し、手動で `git push origin develop` を実行してください。」

### 9. PR作成または確認

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

### 10. 完了メッセージ

> 「リリース準備が完了しました。」
> 「バージョン: v{NEW_VERSION}」
> 「PR URL: {PR URL}」
> 「PRをレビューし、問題なければmainにマージしてください。」

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
