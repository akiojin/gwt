# Release Automation Contract

## `/release` コマンド / `scripts/create-release-branch.sh`

| Aspect | Expectation |
| --- | --- |
| Input | `develop` checkout / clean tree, `gh auth login` 済み、必要な PAT 環境変数を設定 (`PERSONAL_ACCESS_TOKEN`, `NPM_TOKEN` optional) |
| Invocation | `/release` (Claude) or `scripts/create-release-branch.sh` (local) must run `gh workflow run create-release.yml --ref develop` |
| Output | 成功時に最新 `create-release.yml` run ID / URL を表示し、release branch 名 (`release/vX.Y.Z`) をログに出す |
| Failure modes | semantic-release dry-run がバージョンを決定できない、release branch 既存 → スクリプトは明示的に失敗しログを表示 |

## `create-release.yml`

| Aspect | Expectation |
| --- | --- |
| Trigger | `workflow_dispatch` のみ（/release or helper から呼び出し） |
| Steps | checkout develop → install deps → `npx semantic-release --dry-run --branches develop` → バージョン抽出 → `git checkout -b release/vX.Y.Z` → push |
| Output | GitHub Step Summary に version / branch / 次のワークフロー (`release.yml`, `publish.yml`) を掲載 |
| Failure modes | バージョン未決定・非 Conventions commit → workflow fails; release branch 既存 → fail with message |

## `release.yml`

| Aspect | Expectation |
| --- | --- |
| Trigger | `push` to `release/**` + manual `workflow_dispatch` |
| Jobs | 1) `lint` (`bun run lint`), 2) `test` (`bun run test`), 3) `semantic-release` (updates CHANGELOG/tag) |
| Merge | semantic-release success時のみ release/vX.Y.Z を main に `--no-ff` でマージし、ブランチを削除する |
| Summary | version / tag / release notes URL / run URLs を Step Summary に記載 |
| Failure handling | ジョブ失敗時は main を変更せず release/vX.Y.Z を残す。再実行手順を Summary で案内 |

## `publish.yml`

| Aspect | Expectation |
| --- | --- |
| Trigger | `push` to `main` |
| Jobs | `npm-publish`（`chore(release):` コミットのみ）と `backmerge-to-develop` |
| Output | npm publish (任意) の結果と develop バックマージのログを残し、失敗時は `needs` チェックを fail させる |
| Failure handling | back-merge 失敗時は workflow を fail とし、手動で conflict 解消後に `workflow_dispatch` で再実行 |

## Branch Protection

| Branch | Requirement |
| --- | --- |
| `main` | Direct push禁止、Allow auto-merge=OFF（CI のみ push）、Required checks=`lint`,`test`,`semantic-release`（release.yml の job 名） |
| `release/*` | 制限なし（CI/maintainer push 可）。Required checks なし。 |
| Enforcement | 設定手順を CLAUDE.md / specs quickstart に記載し、月次で確認する |
