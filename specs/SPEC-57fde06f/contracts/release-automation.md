# Release Automation Contract

## `/release` コマンド契約

| Aspect | Expectation |
| --- | --- |
| Input | Working tree: `develop` checked out、`git status` clean、`SEMANTIC_RELEASE_TOKEN` available |
| Steps | 1) `git fetch origin --prune` 2) `git push origin develop:release --force-with-lease=refs/heads/release` 3) `gh pr list --base main --head release` 4) `gh pr create` (if none) 5) `gh pr merge --auto --merge <PR#>` |
| Output | Release PR URL、Actions run URL、semantic-release logs (link) |
| Failure modes | push rejected (release diverged) → abort with diff; PR creation fails → show gh stderr; Auto Merge forbidden → instruct to update Branch Protection |

## GitHub Actions `release.yml` 契約

| Aspect | Expectation |
| --- | --- |
| Trigger | `push` on `release` branch, `workflow_dispatch` for retries |
| Jobs | `lint` (`bun run lint`), `test` (`bun run test`), `semantic-release` (`semantic-release --ci`) |
| Artifacts | npm publish, GitHub Release, CHANGELOG.md + package.json commit (release branch) |
| Required Checks | job names `lint`, `test`, `semantic-release` must succeed before Auto Merge |

## release→main PR 契約

| Aspect | Expectation |
| --- | --- |
| Title | `Release YYYY-MM-DD` or semantic version |
| Labels | `release`, `auto-merge` |
| Body | 最新の semantic-release ノート、成功した Actions へのリンク、再実行手順 |
| Auto Merge | Enabled via `gh pr merge --auto --merge`; only Required Checks enforced |
| Notifications | When Auto Merge completes, GitHub posts standard merge summary; failure should @ mention release maintainers |

## Branch Protection 契約

| Aspect | Expectation |
| --- | --- |
| main | Direct push禁止、Allow auto-merge=ON、Required checks=`lint`,`test`,`semantic-release`、Require PR approvals=0 (チェックのみ) |
| release | Push可能: `/release` コマンド実行者 + CI。Required checksなし。 |
| Enforcement | Settings documented in CLAUDE.md; admins verify monthly |
