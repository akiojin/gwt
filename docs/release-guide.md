# Release Guide (Overview)

This page gives a maintainer-friendly snapshot of the release automation.

## Flow at a Glance

```text
feature/* → PR → develop (auto merge)
                           ↓
              /release (prepare-release.yml)
                           ↓
              Conventional Commits analysis → version bump
              git-cliff → CHANGELOG.md update
              Cargo.toml, package.json → version update
                           ↓
              release/YYYYMMDD-HHMMSS → main PR
                           ↓ (release.yml - on merge)
              Create tag & GitHub Release
              cross-compile binaries → GitHub Release upload
              npm publish (with provenance)
```

## Maintainer Checklist (TL;DR)

1. **Prepare** – ensure `develop` has all desired commits and `cargo test && cargo build --release` succeed.
2. **Trigger** – run `/release` in Claude Code or execute `gh workflow run prepare-release.yml --ref develop` locally (requires `gh auth login`).
3. **Monitor** – watch `prepare-release.yml` until the Release PR is created, then monitor `release.yml` from the Actions tab.
4. **Verify** – confirm the `vX.Y.Z` tag, GitHub Release binaries, and npm package version.

## Where to Read More

- User-facing summary (lightweight): README.md
- Release command: `.claude/commands/release.md`

## References

- [akiojin/create-release-pr](https://github.com/akiojin/create-release-pr) – Reusable action for creating release PRs
- `.github/workflows/prepare-release.yml` – Workflow to trigger release
- `.github/workflows/release.yml` – Workflow for publishing
- `cliff.toml` – git-cliff configuration for CHANGELOG generation
