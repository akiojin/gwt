---
description: developブランチでバージョン更新を行い、mainへのRelease PRを作成します（LLMベース）。
tags: [project]
---

# リリースコマンド（LLMベース）

develop ブランチでバージョン更新・CHANGELOG更新を行い、main への Release PR を作成します。

## 推奨: Prepare Release ワークフロー（どのブランチからでも）

**develop に移動できない／したくない場合（work worktree 等）は、ローカル手順ではなく
GitHub Actions の `Prepare Release` ワークフローを使う。** GitHub の
Actions → `Prepare Release` → `Run workflow` を押すだけで、CI が develop 上で
バージョン更新（`scripts/compute_release_version.py` の最新タグ相対計算、`cargo set-version`、
`cargo update -w`、git-cliff）・`chore(release): vX.Y.Z` コミット・develop→main の
Release PR 作成までを実行する。`bump` 入力は `auto`（既定。breaking 検出時は失敗するので
major は明示）/ `patch` / `minor` / `major`。

承認は **生成された Release PR をレビューして merge** で行う（実 diff・CHANGELOG を確認）。
merge 後は `release.yml` がタグ・GitHub Release・5プラットフォームビルドを自動実行する。

バージョン算出ロジックの正本は `scripts/compute_release_version.py`（ユニットテスト
`scripts/test_compute_release_version.py`）。`git-cliff --bumped-version` は使わない
（全履歴再計算でバージョン後退するため）。

ワークフローは push と PR 作成が別ステップのため、push 成功後に PR 作成が失敗すると
develop に bump コミットだけが残ることがある（GitHub Actions が失敗を可視化する）。その場合は
**同じワークフローを再実行**すればよい（`git pull --rebase` は no-op、既存 PR は更新される）。

以下の手動手順は、develop 上で対話的に実行したい場合の **fallback** として残す。

## フロー概要

```
[完了ゴール arm] → develop (バージョン更新・CHANGELOG更新) → main (Release PR)
                                                              ↓ merge
                                              release.yml (タグ・Release・5プラットフォームビルド)
                                                              ↓ 監視・エラー検知・transient 再実行
                                              GitHub Release 公開 (draft=false / assets 付き) = 完了
```

リリースは **PR 作成では完了しない**。`release.yml` が完走し GitHub Release が assets 付きで
公開されて初めて完了する。エージェントは完了ゴール（ステップ 5.4）を arm し、マージ後の
`release.yml` を監視（ステップ 13）してエラーを検知・復旧してから完了報告する。

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

### 1.5 リリース bypass の arm（必須・Issue #3267）

リリースは owner Issue を持たない chore 作業だが、workflow-policy の owner guard は
owner 未リンク session の mutating コマンド（`git fetch` / `git commit` /
`cargo update` / `Cargo.toml` 編集 / `gh run rerun` など）をブロックする。
以降のステップを通すため、**literal な `gwtd` コマンド名の単一 heredoc** で
session に Release bypass を arm する（変数経由の呼び出しは arm 前の hook に
envelope として認識されない場合がある）:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"workflow.bypass","params":{"mode":"release"}}
JSON
```

- bypass は **6 時間で自動失効**する（disarm 忘れの恒久化防止）。長時間の
  transient 復旧で失効した場合は同じコマンドで再 arm する。
- `unknown operation` エラーになる場合は gwtd が古い。`cargo build -p gwt --bin gwtd`
  で dev バイナリをビルドし `./target/debug/gwtd` で実行するか、Issue #3267 を参照。
- **リリース完了時（ステップ 13.5）と全ての中断パスで必ず disarm する**:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"workflow.bypass","params":{"mode":"off"}}
JSON
```

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

#### 5.4 リリース完了ゴールの設定（承認後・必須）

> 🚨 **リリースは「PR 作成」では完了しない。`release.yml` が完走し GitHub Release が
> assets 付きで公開（`draft=false`）されて初めて完了である。** PR 作成後にエージェントが
> ターンを終えると、マージ後の `release.yml` 失敗（例: crates.io download の transient
> 失敗）を誰も検知できない。これを防ぐため、ユーザー承認の直後・ファイル更新の前に、
> **Codex / Claude Code いずれの runtime でも必ず「リリース完了ゴール」を arm する**。

ゴール条件（`{NEW_VERSION}` を確定値に置換して使う）:

```text
v{NEW_VERSION} のリリースを完了する: Release PR が main に merge され、release.yml の
全ジョブが success になり、GitHub Release v{NEW_VERSION} が draft=false かつ全プラット
フォーム asset 添付で公開されるまで。release.yml の transient な失敗（crates.io download /
curl / HTTP2 framing / registry update / runner provisioning など）は失敗ジョブを再実行
して復旧する。非 transient な失敗（compile error / test 失敗 / clippy / signing など）は
ユーザーに報告して停止する。最大 60 分または 30 ターンで打ち切る。
```

runtime 別の arm 方法（`gwt-discussion` SKILL の goal-start 契約と同じ）:

- **Codex**（goals 有効。gwt は `--enable goals` で Codex を起動）: `create_goal` を呼び、
  上記条件を objective として渡す。goals tool 契約によりモデル自身が Goal を開始できる。
- **Claude Code**（v2.1.139 以降）: 組込 `/goal` はエージェントが自己 invoke できない。
  代わりに自分の pane へ queue する。`GWT_BIN` をステップ 10 の `resolve_gwt_bin` で解決し、
  JSON operation `pane.send` で `/goal <条件>` を注入する（現ターン終了時に自動送信。
  `pane.send` は self-only で `GWT_SESSION_ID` の pane のみを対象にする）:

  ```bash
  GWT_BIN="$(resolve_gwt_bin)" || exit $?
  CONDITION="v{NEW_VERSION} のリリースを完了する: release.yml 全ジョブ success かつ GitHub Release v{NEW_VERSION} が draft=false で assets 付き公開まで。transient build 失敗は再実行で復旧、非 transient 失敗はユーザー報告で停止。最大 60 分 / 30 ターンで打ち切り。"
  python3 - "$CONDITION" <<'PY' | "$GWT_BIN"
  import json, sys
  print(json.dumps({"schema_version":1,"operation":"pane.send","params":{"text":"/goal "+sys.argv[1]}}))
  PY
  ```

- **ゴールを arm できない場合**（古い Claude Code、trust dialog 未承認、goals 無効、
  `pane.send` 失敗）は、失敗理由を明示し、上記 `/goal <条件>` 行をそのまま出力して
  ユーザーが手動実行できるようにする。その上でフローは継続する（ゴール開始失敗は
  リリース手順を止めない）。ゴールを arm できなかった場合でも、**ステップ 13 の
  マージ後監視は省略せず必ず実行する**（ゴールは「停止しない」担保であって、監視自体の
  代替ではない）。

### 6. ファイル更新

以下のファイルを更新してください：

#### 6.1 ルート Cargo.toml

`version = "X.Y.Z"` を `version = "{NEW_VERSION}"` に更新

#### 6.2 Cargo.lock

```bash
cargo update -w
```

#### 6.3 CHANGELOG.md

前回リリースタグ以降の変更のみを追加してください。git-cliffが過去の変更を含める場合は、手動でv{PREV_TAG}以降の変更のみを追加してください。

```bash
GITHUB_TOKEN=$(gh auth token) git-cliff --unreleased --tag v{NEW_VERSION} --prepend CHANGELOG.md
```

**注意**: CHANGELOGに既に含まれている変更が重複しないよう確認してください。

### 7. リリースコミット作成

```bash
git add Cargo.toml Cargo.lock CHANGELOG.md
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

#### 9.1 リリース範囲内の参照番号を収集

コミットメッセージ末尾の `(#N)` から参照番号を抽出する。

**注意**: `(#N)` は PR 番号の場合も Issue 番号の場合もある（GitHub のスカッシュマージは PR 番号を付与するが、手動で Issue 番号を付けるケースもある）。そのため、PR / Issue の両方として処理する。

```bash
# squash マージ: コミットメッセージ末尾の (#N) から番号を抽出
SQUASH_REFS=$(git log --pretty=%s "$RANGE" --no-merges \
  | grep -Eo '\(#[0-9]+\)$' \
  | grep -Eo '[0-9]+' \
  | sort -u)

# マージコミット: "Merge pull request #N" から PR 番号を抽出
MERGE_PRS=$(git log --merges --pretty=%s "$RANGE" \
  | sed -n 's/^Merge pull request #\([0-9]\+\).*$/\1/p' \
  | sort -u)

# 結合・重複排除
ALL_REFS=$(printf '%s\n%s\n' "$SQUASH_REFS" "$MERGE_PRS" | awk 'NF' | sort -nu)
```

#### 9.2 各番号を分類し、Closing Issue を収集

各番号について GitHub API で PR か Issue かを判定し、それぞれ適切に処理する。  
**必ず** 以下のヘルパースクリプトを使って収集結果を JSON で取得すること：

```bash
ISSUE_REF_JSON=$(python3 scripts/release_issue_refs.py --range "$RANGE" --format json)

ISSUE_NUMBERS=$(printf '%s\n' "$ISSUE_REF_JSON" | jq -r '.auto_close_issues[]?')
REFERENCE_ONLY_ISSUES=$(printf '%s\n' "$ISSUE_REF_JSON" | jq -r '.reference_only_issues[]?')
ISSUE_WARNINGS=$(printf '%s\n' "$ISSUE_REF_JSON" | jq -r '.warnings[]?')
```

`ISSUE_NUMBERS` が空でなければ、PR 本文の `## Closing Issues` セクションに **1行ずつ** 以下を追加：

```text
Closes #123
Closes #456
```

#### 9.3 warning の扱い

`REFERENCE_ONLY_ISSUES` または `ISSUE_WARNINGS` がある場合、それらは **自動クローズ対象ではない**。  
この場合、ステップ5の承認時とステップ10の PR 本文生成時に必ず可視化すること。

- `REFERENCE_ONLY_ISSUES`: `## Related Issues / Links` に残す
- `ISSUE_WARNINGS`: ユーザー承認時にそのまま表示する
- `ISSUE_NUMBERS` が空でも `REFERENCE_ONLY_ISSUES` がある場合:
  - 「関連Issueは見つかったが、自動クローズ対象は 0 件」と明示する
  - 対象 Issue を本当に閉じたいなら `Closing Issues` 側へ移す必要があると説明する

### 10. PR作成/更新

まず現在の develop 向け PR を確認：

```bash
resolve_gwt_bin() {
  if [ -n "${GWT_BIN_PATH:-}" ] && [ -x "$GWT_BIN_PATH" ]; then
    printf '%s\n' "$GWT_BIN_PATH"
    return 0
  fi
  if command -v gwtd >/dev/null 2>&1; then
    command -v gwtd
    return 0
  fi
  repo_root="${GWT_PROJECT_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
  if [ -x "$repo_root/target/debug/gwtd" ]; then
    printf '%s\n' "$repo_root/target/debug/gwtd"
    return 0
  fi
  printf '%s\n' "gwtd not found; set GWT_BIN_PATH, install gwtd into PATH, or run cargo build -p gwt --bin gwtd." >&2
  return 127
}

GWT_BIN="$(resolve_gwt_bin)" || exit $?
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"pr.current","params":{}}
JSON
```

#### 既存PRがある場合

`pr.current` の JSON envelope 出力に PR 番号が含まれている場合、以下を実行してタイトル・ラベル・本文を更新（`## Closing Issues` を反映）：
本文は `params.body` に入れること。

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"pr.edit","params":{"number":123,"title":"chore(release): v{NEW_VERSION}","body":"<PR body>","add_labels":["release"]}}
JSON
```

> 「既存のRelease PR（#{PR番号}）を更新しました。」
> 「URL: {PR URL}」

#### 既存PRがない場合

PRを作成：

```bash
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"pr.create","params":{"base":"main","head":"develop","title":"chore(release): v{NEW_VERSION}","body":"<PR body>","labels":["release"],"draft":false}}
JSON
```

**PR_BODY の内容**（LLMが生成）：

PR bodyには以下を含めてください：
- `## Summary` - このリリースの概要（変更内容を要約）
- `## Changes` - 主な変更点をリスト形式で
- `## Version` - バージョン番号
- `## Closing Issues` - main マージ時にクローズしたい Issue を `Closes #<番号>` の生テキストで列挙（`ISSUE_NUMBERS` が空の場合は `None` と記載）
- `## Related Issues / Links` - `REFERENCE_ONLY_ISSUES` を `#<番号>` で列挙（空の場合は `None`）

**重要**: `Closes #<番号>` はコードブロックに入れず、通常の本文として記載すること。
**重要**: `#<番号>` を `## Related Issues / Links` にだけ書いても auto-close されない。

### 11. Closing Issue へのコメント追記

`ISSUE_NUMBERS` が空でない場合、各 Issue に対してリリースに含まれる旨のコメントを追加する。

まず、ステップ10の直後に JSON operation `pr.current` を再実行し、出力から PR 番号を取得する：

```bash
GWT_BIN="$(resolve_gwt_bin)" || exit $?
PR_CURRENT=$("$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"pr.current","params":{}}
JSON
)
PR_NUMBER=$(printf '%s\n' "$PR_CURRENT" | sed -n 's/^#\([0-9]\+\).*/\1/p' | head -1)
```

各 Issue にコメントを追記：

```bash
GWT_BIN="$(resolve_gwt_bin)" || exit $?

for NUM in $ISSUE_NUMBERS; do
  python3 - "$NUM" "{NEW_VERSION}" "$PR_NUMBER" <<'PY' | "$GWT_BIN" || true
import json
import sys

number = int(sys.argv[1])
version = sys.argv[2]
pr_number = sys.argv[3]
print(json.dumps({
    "schema_version": 1,
    "operation": "issue.comment",
    "params": {
        "number": number,
        "body": f"Included in release v{version} (#{pr_number})",
    },
}))
PY
done
```

- `ISSUE_NUMBERS` が空の場合はこのステップ全体をスキップ
- コメント本文にはバージョン番号（`v{NEW_VERSION}`）と Release PR 番号を含める
- `|| true` により、個別の Issue へのコメント失敗（既にクローズ済み等）でもリリースフロー全体を中断しない

### 12. PR 作成完了メッセージ（まだ完了ではない）

> 「Release PR を作成しました。」
> 「バージョン: v{NEW_VERSION}」
> 「PR URL: {PR URL}」
> 「これから PR の merge と release.yml の完走を監視し、GitHub Release が assets 付きで
> 公開されるまで見届けます。」
>
> 🚨 **ここで「リリース完了」と報告して終了してはならない。** ステップ 5.4 で arm した
> ゴールに従い、ステップ 13 のマージ後監視を必ず実行する。release.yml が完走し GitHub
> Release が公開されるまでリリースは完了していない。

### 13. マージ後リリースの監視・エラー検知・完了確認（必須）

> 🚨 **このステップを省略しない。** 過去にこのステップが無かったため、PR 作成後に
> エージェントがターンを終え、マージ後の `release.yml` ビルド失敗（crates.io download の
> transient エラー）が長時間検知されなかった事例がある。リリースは `release.yml` が完走し
> GitHub Release が公開されて初めて完了する。

#### 利用できるコマンド面（gwt の surface 制約）

- `gh release view`（`gh release`）は **ブロック対象外**。リリース公開の確定シグナルに使う。
- `gh run list` / `gh run rerun <run-id> --failed` は利用可能（status 取得・再実行）。
- `gh run view` は managed hook によりブロックされることがある。**依存しない**。
- ログ精査は gwt 推奨の JSON operation `actions.logs`（`params.run_id`）/
  `actions.job_logs`（`params.job_id`）を使う。**`actions.logs` は run 完了後のみ取得可能**
  （in-progress では "still in progress" を返す）。
- PR の merge 状態は JSON operation `pr.view` / `pr.checks`。

#### 13.1 PR が main に merge されるまで監視

`pr.view` をポーリングし、Release PR が `[MERGED]` になるまで待つ（auto-merge 有効なら
必須 CI 通過後に自動マージされる。CodeRabbit など非必須 check は pending でもブロックしない）。

- PR 側 CI が **非 transient** で fail し merge できない場合は、原因を特定して報告し停止する。
- BEHIND（base が先行）でも auto-merge は通常マージするため、それ自体は失敗ではない。

#### 13.2 release.yml run の特定と完走待ち

merge 後、main の `release.yml` run を特定する:

```bash
gh run list --workflow release.yml --branch main --limit 5 --repo akiojin/gwt
```

最新の該当 run の `run_id` と status を取得し、`completed` になるまでポーリングする
（クロスコンパイルは 5 プラットフォームで 10〜20 分かかる）。

#### 13.3 失敗時のエラー分類（transient vs 非 transient）

run が `completed` かつ `failure` の場合、`actions.logs`（run 完了後に取得可）で
失敗ジョブのログを取得し、原因を分類する:

- **transient / インフラ起因 → 自動で再実行**: 以下のシグナルは crates.io やランナー側の
  一過性障害であり、コードの問題ではない。失敗ジョブを再実行する。
  - `unable to update registry`, `download of .* failed`, `curl failed`,
    `Error in the HTTP2 framing layer`, `Connection reset`, `timed out`,
    `TLS connect error`, `429` / `rate limit`, `503`, runner provisioning 失敗 など

  ```bash
  gh run rerun <run-id> --failed --repo akiojin/gwt
  ```

  再実行後は 13.2 に戻って完走を待つ。再実行は **最大 3 回** まで。3 回連続で同じ
  transient 失敗なら、状況をユーザーに報告して判断を仰ぐ。

- **非 transient → 停止して報告**: 以下はコード／設定の問題であり、再実行では直らない。
  失敗ジョブ・該当ログ抜粋・推定原因をユーザーに報告して停止する。**盲目的に再実行しない**。
  - `error[E####]` などの Rust compile error、test 失敗（`FAILED` / `panicked`）、
    `clippy` 警告、署名／keychain エラー、必須 secret 欠落、lint エラー など

#### 13.4 リリース公開の確定確認

`release.yml` の全ジョブが success になったら、GitHub Release が実際に公開されたことを確認する:

```bash
gh release view v{NEW_VERSION} --repo akiojin/gwt --json isDraft,assets,publishedAt
```

- `isDraft=false` かつ `publishedAt` が設定済み、かつ `assets` に全プラットフォームの
  成果物（各 OS のバイナリ／インストーラ）が揃っていることを確認する。
- まだ `isDraft=true` / assets 不足なら、`release.yml` のアップロードジョブ完了を待って再確認する。

#### 13.5 完了報告

公開を確認できたら、初めて「リリース完了」を報告する:

> 「リリースが完了しました。」
> 「バージョン: v{NEW_VERSION}」
> 「Release URL: `https://github.com/akiojin/gwt/releases/tag/v{NEW_VERSION}`」
> 「公開済み assets: {asset 一覧}」
> （transient 失敗を再実行した場合はその回数も報告する）

完了報告の直後に、ステップ 1.5 で arm した Release bypass を必ず disarm する
（中断でリリースを終える場合も同様）:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"workflow.bypass","params":{"mode":"off"}}
JSON
```

ステップ 5.4 で arm したゴールは、この確定確認をもって満たされ自動的に解除される。

## マージ後の自動処理

PRがmainにマージされると、`.github/workflows/release.yml` が以下を自動実行：

1. Git タグを作成 (`v{NEW_VERSION}`)
2. GitHub Release を作成（最初は draft）
3. クロスコンパイル済みバイナリをアップロードし、Release を公開（draft 解除）

> この自動処理は **失敗しうる**（特にビルド時の crates.io download の transient 失敗）。
> エージェントはステップ 13 でこの run を必ず監視し、transient 失敗は再実行で復旧、
> 非 transient 失敗はユーザーに報告すること。「自動だから完了」とみなして終了しない。

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

### release.yml のビルドジョブが transient エラーで失敗した場合

マージ後の `release.yml` ビルドで、crates.io からの依存ダウンロード失敗
（`download of <crate> failed` / `curl failed` / `Error in the HTTP2 framing layer` /
`unable to update registry`）が出ることがある。これは GitHub Actions ランナー↔crates.io
間の一過性ネットワーク障害でコードの問題ではない。失敗ジョブを再実行すれば復旧する:

```bash
gh run rerun <run-id> --failed --repo akiojin/gwt
```

ステップ 13 はこの分類と再実行を自動で行う。3 回再実行しても同じ transient 失敗が続く
場合のみユーザーに報告する。
