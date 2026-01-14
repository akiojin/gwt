# Release Guide (Overview)

This page gives a maintainer-friendly snapshot of the release automation.

## Flow at a Glance

```text
feature/* → PR → develop (auto merge)
                           ↓
              /release (prepare-release.yml)
                           ↓
              develop → main PR → auto-merge
                           ↓ (release.yml)
               release-please: tag, GitHub Release, Release PR
                           ↓ (release.yml - on release created)
               crates.io publish (Trusted Publishing)
               cross-compile binaries → GitHub Release upload
               npm publish (with provenance)
```

## Maintainer Checklist (TL;DR)

1. **Prepare** – ensure `develop` has all desired commits and `cargo test && cargo build --release` succeed.
2. **Trigger** – run `/release` in Claude Code or execute `gh workflow run prepare-release.yml --ref develop` locally (requires `gh auth login`).
3. **Monitor** – watch `prepare-release.yml` until the Release PR is created, then monitor `release.yml` from the Actions tab.
4. **Verify** – confirm the `vX.Y.Z` tag, crates.io publication, GitHub Release binaries, and npm package version.

## Where to Read More

- User-facing summary (lightweight): README.md
- Release command: `.claude/commands/release.md`

## References

- `.claude/commands/release.md`
- GitHub Actions workflows: `prepare-release.yml`, `release.yml`, `auto-merge.yml`
