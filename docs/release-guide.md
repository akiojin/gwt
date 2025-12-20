# Release Guide (Overview)

This page gives a maintainer-friendly snapshot of the release automation.

## Flow at a Glance

```text
feature/* → PR → develop (auto merge)
                           ↓
              /release (prepare-release.yml)
                           ↓
              develop → main PR 作成 → auto-merge
                           ↓ (release.yml)
               release-please がタグ・GitHub Release・Release PR 作成
                           ↓ (Release PR auto-merge)
               main にリリースコミット反映
                           ↓ (publish.yml - v* tag trigger)
                        npm publish
```

## Maintainer Checklist (TL;DR)

1. **Prepare** – ensure `develop` has all desired commits and `bun run lint && bun run test && bun run build` succeed.
2. **Trigger** – run `/release` in Claude Code or execute `gh workflow run prepare-release.yml --ref develop` locally (requires `gh auth login`).
3. **Monitor** – watch `prepare-release.yml` until the Release PR is created, then monitor `release.yml` and `publish.yml` from the Actions tab.
4. **Verify** – confirm the `vX.Y.Z` tag and (if enabled) the npm package version.

## Where to Read More

- User-facing summary (lightweight): README.md
- Release command: `.claude/commands/release.md`

## References

- `.claude/commands/release.md`
- GitHub Actions workflows: `prepare-release.yml`, `release.yml`, `publish.yml`, `auto-merge.yml`
