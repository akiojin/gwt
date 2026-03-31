1. `git fetch origin`
2. `fetch_branch_pr_preflight(projectPath, headBranch, baseBranch)` で状態確認
3. `behind/diverged` なら update を案内し、PR 作成を止める
4. `up_to_date/ahead` なら既存の PR workflow へ進む
