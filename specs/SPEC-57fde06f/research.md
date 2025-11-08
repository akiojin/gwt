# Research: releaseブランチ経由の自動リリース＆Auto Mergeフロー

## Decision 1: develop→release は fast-forward + force-with-lease ではなく `git push --force-with-lease` を避ける
- **Rationale**: release ブランチは develop の公開済み履歴のみを含むべきなので、`git merge` や `--force` を行うと release 駆動のタグ履歴が乱れる。`git fetch origin release` → `git checkout release` → `git reset --hard origin/develop` では書き換えリスクがあるため、`git push origin develop:release` で fast-forward を強制するのが最も安全。`/release` コマンドからは `git update-ref refs/heads/release $(git rev-parse develop)` と同等の操作を gh CLI (`gh api repos/{owner}/{repo}/git/refs/heads/release -X PATCH`) で行い、失敗時は差分を出して中断する。
- **Alternatives considered**:
  - GitHub Actions 内で develop を release にマージ → CLI 実行から結果が見えにくくなるため却下。
  - release ブランチを一度削除して再作成 → 権限が必要で履歴閲覧性が失われる。

## Decision 2: semantic-release は release ブランチ push で実行し main は PR マージのみ
- **Rationale**: release ブランチで version 決定とタグ付けを行えば、main は単に release の内容を受け取るだけでよくなる。`release.yml` の `on` に `push: branches: [release]` を追加し main を除外。タグは release ブランチ上に作成されるが、release→main PR マージ時に main も同じタグを含むため整合が取れる。
- **Alternatives considered**:
  - semantic-release を main で継続し release は中継のみ → main への直接 push が必要になるため本仕様と矛盾。
  - release 専用ワークフローを新設 → 既存 `release.yml` を拡張したほうがメンテが容易。

## Decision 3: Auto Merge は `gh pr merge --auto --merge` を CLI から設定
- **Rationale**: GitHub UI での手動操作はヒューマンエラーを生む。`/release` コマンド完了時に `gh pr create --fill --base main --head release --label release --title "Release ${tag}" --body <notes>` を実行し、既存 PR があれば `gh pr edit` で更新。Auto Merge は `gh pr merge --auto --merge <PR#>` を即時有効化でき、Required チェック成功後に自動マージされる。
- **Alternatives considered**:
  - `github-script` step で GraphQL Mutation を呼ぶ → CLI のほうがローカルからも同じ操作ができ、スクリプト流用が容易。
  - Merge Queue を使う → 追加設定が増え、小規模プロジェクトではオーバーキル。

## Decision 4: Required チェックは semantic-release job + `bun run lint` + `bun run test`
- **Rationale**: release PR が main へ入る前に最小限の品質ゲートを残したい。既存 CI では lint/test が release workflow に含まれているため、同じジョブ名を `required_status_checks` に登録すれば Auto Merge 条件を満たせる。semantic-release job の完了は release.yml 内の `semantic-release` ステップ成功で代替可能。
- **Alternatives considered**:
  - `release-trigger.yml` 内で個別ジョブを走らせる → リリース処理とテストが同一 workflow に閉じないためロギングが散逸。
  - Required チェックを 1 つの aggregate job にまとめる → 失敗原因特定が難しくなる。

## Decision 5: Branch Protection は手動設定 + ドキュメント化
- **Rationale**: API で Branch Protection を変更するには管理者トークンが必要で自動化のコストが高い。今回は main への直接 push 禁止と Auto Merge 許可を管理者が一度設定し、その手順を CLAUDE.md / quickstart に明示。release ブランチには Required チェックは課さず、main のみ保護対象とする。
- **Alternatives considered**:
  - `gh api` を使って自動化 → PAT の権限管理が複雑化し、本機能のスコープを超える。
  - Branch Protection なし（レビューのみ） → Auto Merge が無秩序になり品質保証ができない。
