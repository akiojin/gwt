# Release Guide (Overview)

This page gives a maintainer-friendly snapshot of the release automation. Full design artifacts live under `specs/SPEC-57fde06f/`.

## Flow at a Glance

```
feature/* → PR → develop (auto merge)
                           ↓
             /release or scripts/create-release-branch.sh
                           ↓ (create-release.yml)
                 release/vX.Y.Z push to origin
                           ↓ (release.yml)
     semantic-release → merge release/vX.Y.Z → main → delete branch
                           ↓ (publish.yml)
          npm publish (optional) → main → develop back-merge
```

## Maintainer Checklist (TL;DR)

1. **Prepare** – ensure `develop` has all desired commits and `bun run lint && bun run test && bun run build` succeed.
2. **Trigger** – run `/release` in Claude Code or execute `scripts/create-release-branch.sh` locally (requires `gh auth login`).
3. **Monitor** – watch `create-release.yml` until the branch push completes, then monitor `release.yml` and `publish.yml` from the Actions tab.
4. **Verify** – confirm the `chore(release):` commit on `main`, the `vX.Y.Z` tag, and (if enabled) the npm package version.
5. **Recover** – if any workflow fails, fix the cause, rerun the workflow, or recreate the release branch as described in the spec.

## Where to Read More

- Detailed requirements, edge cases, and recovery procedures: `specs/SPEC-57fde06f/spec.md`
- Quickstart / contracts / data model: see the other files inside `specs/SPEC-57fde06f/`
- User-facing summary (lightweight): README.md

## References

- `.claude/commands/release.md`
- `scripts/create-release-branch.sh`
- GitHub Actions workflows: `create-release.yml`, `release.yml`, `publish.yml`
