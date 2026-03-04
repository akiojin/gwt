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
git fetch origin main develop --tags
git pull origin develop
```

### 3. リリース対象コミット確認

```bash
PREV_TAG=$(git tag --list 'v[0-9]*' --sort=-version:refname | head -1)
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

**注意**: `git-cliff --bumped-version` は全履歴からバージョンを再計算するため、過去に non-conventional なコミットが多いリポジトリではバージョン後退が起きる。代わりに **最新タグからの相対バージョン判定** を行う。

#### 4.1 最新タグのパース

`PREV_TAG`（ステップ3で取得済み）からメジャー・マイナー・パッチを分解：

```bash
# v8.3.0 → MAJOR=8, MINOR=3, PATCH=0
MAJOR=$(echo "$PREV_TAG" | sed 's/^v//' | cut -d. -f1)
MINOR=$(echo "$PREV_TAG" | sed 's/^v//' | cut -d. -f2)
PATCH=$(echo "$PREV_TAG" | sed 's/^v//' | cut -d. -f3)
```

タグが存在しない場合（初回リリース）は `MAJOR=0, MINOR=0, PATCH=0` とする。

#### 4.2 unreleased コミットの種別判定

前回タグから HEAD までのコミットメッセージを分析し、バージョン種別を決定：

```bash
# BREAKING CHANGE の検出（コミットメッセージに ! または本文に BREAKING CHANGE）
HAS_BREAKING=$(git log ${PREV_TAG}..HEAD --pretty=format:"%s%n%b" | grep -cE '(^[a-z]+(\(.+\))?!:|BREAKING CHANGE)' || true)

# feat の検出
HAS_FEAT=$(git log ${PREV_TAG}..HEAD --pretty=format:"%s" --no-merges | grep -cE '^feat(\(.+\))?[!:]' || true)

# fix の検出
HAS_FIX=$(git log ${PREV_TAG}..HEAD --pretty=format:"%s" --no-merges | grep -cE '^fix(\(.+\))?[!:]' || true)
```

#### 4.3 バージョン算出

```text
- HAS_BREAKING > 0 → MAJOR + 1, MINOR = 0, PATCH = 0（※ 自動適用しない。ステップ5で必ずユーザー承認）
- HAS_FEAT > 0     → MINOR + 1, PATCH = 0
- HAS_FIX > 0      → PATCH + 1
- いずれもない場合  → PATCH + 1（docs/chore のみでも patch bump）
```

算出結果を `NEW_VERSION`（`v` なし、例: `8.4.0`）として記録。

**メジャーバージョン更新の場合**: 自動でメジャーバージョンを確定しない。ステップ5でユーザーが明示的に承認するまで仮バージョンとして扱う。

#### 4.4 重複チェック

```bash
git tag --list "v{NEW_VERSION}"
```

**判定**: タグが既に存在する場合、以下のメッセージを表示して中断：
> 「エラー: タグ v{NEW_VERSION} は既に存在します。コミット履歴を確認してください。」

### 5. リリース内容確認（ユーザー承認）

**このステップでは必ずユーザーの承認を得てから次に進むこと。**

#### 5.1 変更内容のプレビュー生成

前回タグからの変更ログを生成：

```bash
GITHUB_TOKEN=$(gh auth token) git-cliff --unreleased --tag v{NEW_VERSION}
```

#### 5.2 コミット一覧を表示

```bash
# タグが存在する場合
git log {PREV_TAG}..HEAD --oneline --no-merges

# タグが存在しない場合（初回リリース）
git log --oneline --no-merges
```

#### 5.3 ユーザーに確認

以下の情報をまとめてユーザーに提示し、AskUserQuestion で承認を求める：

- **現在のバージョン**: `{PREV_TAG}` （タグがない場合は「初回リリース」）
- **次のバージョン**: `v{NEW_VERSION}`
- **バージョン種別**: major / minor / patch のいずれか（Conventional Commits から判定した理由も簡潔に）
- **変更内容**: git-cliff が生成した変更ログ（Features, Bug Fixes 等のカテゴリ別）
- **コミット一覧**: 上記で取得したコミットログ

**メジャーバージョン更新の場合（MAJOR bump）**:

メジャーバージョン更新は破壊的変更を伴うため、通常より慎重な確認が必要。以下を追加で提示する：

- 破壊的変更の該当コミット一覧（`!` 付きまたは `BREAKING CHANGE` を含むコミット）
- 「このリリースはメジャーバージョン更新（破壊的変更）です。本当にメジャーバージョンを上げますか？」と明示的に警告

AskUserQuestion のオプション（メジャーバージョン時）:
- 「メジャーバージョンでリリースする」: メジャーバージョンとして承認
- 「マイナーバージョンに変更してリリースする」: MINOR + 1 に変更して続行
- 「中断する」: リリースを中止する

AskUserQuestion のオプション（minor / patch の場合）:
- 「リリースを実行する」: 承認。次のステップに進む
- 「中断する」: リリースを中止する

**判定**: ユーザーが「中断する」を選択した場合、以下のメッセージを表示して中断：
> 「リリースを中断しました。」

**判定**: ユーザーが「マイナーバージョンに変更してリリースする」を選択した場合、`NEW_VERSION` を MINOR + 1 に再計算して続行。

### 6. ファイル更新

以下のファイルを更新してください：

#### 6.1 ルート Cargo.toml

`version = "X.Y.Z"` を `version = "{NEW_VERSION}"` に更新

#### 6.2 package.json

`"version": "X.Y.Z"` を `"version": "{NEW_VERSION}"` に更新

#### 6.3 crates/gwt-tauri/tauri.conf.json

`"version": "X.Y.Z"` を `"version": "{NEW_VERSION}"` に更新

#### 6.4 Cargo.lock

```bash
cargo update -w
```

#### 6.5 CHANGELOG.md

前回リリースタグ以降の変更のみを追加してください。git-cliffが過去の変更を含める場合は、手動でv{PREV_TAG}以降の変更のみを追加してください。

```bash
GITHUB_TOKEN=$(gh auth token) git-cliff --unreleased --tag v{NEW_VERSION} --prepend CHANGELOG.md
```

**注意**: CHANGELOGに既に含まれている変更が重複しないよう確認してください。

### 7. リリースコミット作成

```bash
git add Cargo.toml Cargo.lock package.json crates/gwt-tauri/tauri.conf.json CHANGELOG.md
git commit -m "chore(release): v{NEW_VERSION}"
```

### 8. developをプッシュ

```bash
git push origin develop
```

**失敗時**: 最大3回リトライ。それでも失敗した場合：
> 「エラー: pushに失敗しました。ネットワーク接続を確認してください。」

### 9. Closing Issue の収集

`develop` 向けPRに書かれた `Closes #...` は自動クローズされないため、release PR（`develop -> main`）本文に再掲します。

まず、今回のリリース範囲を決定：

```bash
if [ -n "$PREV_TAG" ]; then
  RANGE="${PREV_TAG}..HEAD"
else
  RANGE="HEAD"
fi
```

#### 9.1 リリース範囲内の PR 番号を収集

squash マージとマージコミットの両方から PR 番号を抽出：

```bash
# squash マージ: コミットメッセージ末尾の (#N) から PR 番号を抽出
SQUASH_PRS=$(git log --pretty=%s "$RANGE" --no-merges \
  | grep -Eo '\(#[0-9]+\)$' \
  | grep -Eo '[0-9]+' \
  | sort -u)

# マージコミット: "Merge pull request #N" から PR 番号を抽出
MERGE_PRS=$(git log --merges --pretty=%s "$RANGE" \
  | sed -n 's/^Merge pull request #\([0-9]\+\).*$/\1/p' \
  | sort -u)

# 結合・重複排除
ALL_PRS=$(printf '%s\n%s\n' "$SQUASH_PRS" "$MERGE_PRS" | awk 'NF' | sort -nu)
```

#### 9.2 各 PR の本文から Closing Issue 番号を抽出

**重要**: コミットメッセージからは抽出しないこと。コミットタイトルの `(#N)` は PR 番号であり Issue 番号ではない。

```bash
CANDIDATE_ISSUES=""
for PR_NUMBER in $ALL_PRS; do
  PR_BODY=$(gh pr view "$PR_NUMBER" --json body --jq '.body' 2>/dev/null || true)
  if [ -n "$PR_BODY" ]; then
    FOUND=$(printf '%s\n' "$PR_BODY" \
      | grep -Eio '(close[sd]?|fix(e[sd])?|resolve[sd]?)\s+#[0-9]+' \
      | grep -Eo '#[0-9]+' \
      | tr -d '#' || true)
    if [ -n "$FOUND" ]; then
      CANDIDATE_ISSUES="${CANDIDATE_ISSUES}\n${FOUND}"
    fi
  fi
done

CANDIDATE_ISSUES=$(printf '%b\n' "$CANDIDATE_ISSUES" | awk 'NF' | sort -nu)
```

#### 9.3 Issue であることを検証（PR 番号を除外）

収集した番号には PR 番号が含まれる可能性があるため、GitHub API で Issue であることを確認する。`pull_request` フィールドが存在すれば PR なので除外：

```bash
REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner)
ISSUE_NUMBERS=""
for NUM in $CANDIDATE_ISSUES; do
  IS_PR=$(gh api "repos/$REPO/issues/$NUM" --jq '.pull_request // empty' 2>/dev/null || true)
  if [ -z "$IS_PR" ]; then
    ISSUE_NUMBERS="${ISSUE_NUMBERS} ${NUM}"
  fi
done
ISSUE_NUMBERS=$(echo "$ISSUE_NUMBERS" | tr ' ' '\n' | awk 'NF' | sort -nu)
```

`ISSUE_NUMBERS` が空でなければ、PR 本文の `## Closing Issues` セクションに **1行ずつ** 以下を追加：

```text
Closes #123
Closes #456
```

### 10. PR作成/更新

まず既存PRを確認：

```bash
gh pr list --base main --head develop --state open --json number,title
```

#### 既存PRがある場合

以下を実行して、タイトル・ラベル・本文を更新（`## Closing Issues` を反映）：

```bash
gh pr edit {PR番号} \
  --title "chore(release): v{NEW_VERSION}" \
  --add-label release \
  --body "{PR_BODY}"
```

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
- `## Closing Issues` - main マージ時にクローズしたい Issue を `Closes #<番号>` の生テキストで列挙（`ISSUE_NUMBERS` が空の場合は `None` と記載）

**重要**: `Closes #<番号>` はコードブロックに入れず、通常の本文として記載すること。

### 11. 完了メッセージ

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
