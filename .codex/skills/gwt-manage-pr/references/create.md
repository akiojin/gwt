## Mode: Create

Detailed logic in [references/create-flow.md](create-flow.md).

### Decision Rules

Create mode is entered from the Preflight 2×2 matrix when `N > 0`.

1. **Do not create or switch branches.** Always use the current branch as head.
2. **If open PR exists and is `CONFLICTING` / `DIRTY` / `BEHIND`** → enter Fix mode before any push-only path.
3. **If the work fails the Ready PR Gate** → repair the failure before creating a PR. Draft PRs are prohibited.
4. **If open PR exists and merge state is clean** → push only, return existing PR URL, enter Fix mode. A Ready/non-draft PR still requires the Ready PR Gate.
5. **If no open PR** → create a non-Draft PR only after the Ready PR Gate is satisfied.
6. **Branch sync:** Being behind `origin/$base` alone never requires a merge before PR creation. Probe with `git merge-tree --write-tree HEAD "origin/$base"`; merge `origin/$base` (never rebase) and re-run `verify.plan` / `verify.run` only when it reports a conflict. Otherwise create the PR at the verified HEAD and sync afterwards with `pr.update_branch` when the base is strict (see [references/create-flow.md](create-flow.md) Step 4).

### PR Title Rules

- Format: `<type>(<scope>): <subject>` (Conventional Commits)
- type: `feat`/`fix`/`docs`/`chore`/`refactor`/`test`/`ci`/`perf`
- subject: imperative mood, under 70 chars, no capital, no period
- Branch prefix `feat/`/`fix/` must match title type

### PR Body Rules

- Template: the current runtime's `gwt-manage-pr/references/pr-body-template.md`
- Required sections: Summary, Changes, Testing, PR Readiness, Closing Issues, Related Issues, Checklist
- Conditional sections (remove if N/A): Context, Risk/Impact, Screenshots, Deployment
- Remove all `<!-- GUIDE: ... -->` comments from final output
- No `TODO` in required sections; derive content from diff/Issues/SPECs before asking user
- `Closing Issues`: `Closes #N` or `None` only. Bare `#N` without keyword is forbidden.
- Add reason to every unchecked checklist item

### Create/Update Commands

- Create: JSON operation `pr.create` with `params.base`, optional
  `params.head`, `params.title`, `params.body`, and optional `params.labels`.
- Update: JSON operation `pr.edit` with `params.number`, optional
  `params.title`, optional `params.body`, and optional `params.add_labels`
  (at least one of them is required).
- Recover an existing Draft: JSON operation `pr.ready` with `params.number`
  marks it Ready for review after the automated gates pass. Draft creation
  and demotion are unavailable.
- Transport note: the current implementation may still call GitHub REST / `gh`
  internally, but agent-facing workflow should stay on gwtd JSON operations.

### Post-Create

After PR creation or push, automatically enter **fix** mode and use JSON
operations `pr.checks`, `pr.reviews`, `pr.review_threads`, `actions.logs`, and
`actions.job_logs` as the normal inspection path.
