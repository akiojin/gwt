# Release Guide (Overview)

This page gives a maintainer-friendly snapshot of the release automation. Full design artifacts live under `specs/SPEC-57fde06f/`.

## Flow at a Glance

```
feature/* → PR → develop (auto merge)
                           ↓
              /release (create-release.yml)
                           ↓
              Release PR created → auto-merge to main
                           ↓ (release.yml)
               tag & GitHub Release created
                           ↓ (publish.yml)
          npm publish (optional) → main → develop back-merge
```

## Maintainer Checklist (TL;DR)

1. **Prepare** – ensure `develop` has all desired commits and `bun run lint && bun run test && bun run build` succeed.
2. **Trigger** – run `/release` in Claude Code or execute `gh workflow run create-release.yml --ref develop` locally (requires `gh auth login`).
3. **Monitor** – watch `create-release.yml` until the Release PR is created, then monitor `release.yml` and `publish.yml` from the Actions tab.
4. **Verify** – confirm the `chore(release):` commit on `develop`, the `vX.Y.Z` tag, and (if enabled) the npm package version.
5. **Recover** – if any workflow fails, fix the cause, rerun the workflow, or close and recreate the Release PR as described in the spec.

## Where to Read More

- Detailed requirements, edge cases, and recovery procedures: `specs/SPEC-57fde06f/spec.md`
- Quickstart / contracts / data model: see the other files inside `specs/SPEC-57fde06f/`
- User-facing summary (lightweight): README.md

## References

- `.claude/commands/release.md`
- GitHub Actions workflows: `create-release.yml`, `release.yml`, `publish.yml`
