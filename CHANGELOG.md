# Changelog

All notable changes to this project will be documented in this file.

## [6.0.5] - 2026-01-15

### Bug Fixes

- Use cross for Linux ARM64 musl build (pinned to v0.2.5 for reproducible builds)

## [6.0.3] - 2026-01-15

### Bug Fixes

- Bump version to 6.0.3 (crates.io already has 6.0.2)
- Bump package.json version to 6.0.3
- Add version to gwt-core dependency for crates.io publishing

## [6.0.1] - 2026-01-15

### Bug Fixes

- Support merge commit in release workflow trigger
- Add version to gwt-core dependency for crates.io publishing
- Use cross for Linux ARM64 musl build
- Support merge commit in release workflow trigger
- Add workflow_dispatch support to release workflow trigger

### Documentation

- パッケージ公開状況をCLAUDE.mdに追記

### Miscellaneous Tasks

- Sync main to develop after v6.0.0 release
- バージョンを 6.0.3 に統一

### Refactor

- Use workspace dependencies for internal crates

### Ci

- Use cargo-workspaces for crates.io publishing
- Remove crates.io publishing, distribute via GitHub Release and npm only

## [6.0.0] - 2026-01-15

### Bug Fixes

- Update migration status in README

### Miscellaneous Tasks

- Sync main to develop after v5.5.0 release

## [5.5.0] - 2026-01-15

### Bug Fixes

- Use PR-based sync for main to develop after release
- Bump version to 6.0.3 for crates.io compatibility

### Miscellaneous Tasks

- Sync main to develop after v5.4.0 release

## [5.4.0] - 2026-01-15

### Bug Fixes

- Use workspace version inheritance for subcrates
- Windows NTSTATUSコードを人間可読形式で表示 (#609)
- Use musl static linking for Linux binaries to resolve GLIBC dependency (#610)

### Features

- ログビューア機能の実装と構造化ログの強化 (#606)

### Miscellaneous Tasks

- Sync main to develop after v5.3.0 release

## [5.1.0] - 2026-01-14

### Bug Fixes

- Remove publish-crates dependency from upload-release job
- Add sync-develop job to sync main back to develop after release
- Use -X theirs option in sync-develop to resolve conflicts automatically
- Add on-demand binary download for bunx compatibility (#600)
- Filter key events by KeyEventKind::Press to prevent double input on Windows (#601)

### Features

- Bun-to-rust移行と周辺改善 (#602)
- Add structured debug logging for worktree change detection (#603)
- Migrate from release-please to custom release action (#587)

### Miscellaneous Tasks

- Remove release-please manifest (migrating to custom action)
- Remove release-please config (migrating to custom action)
- Sync version with main (5.1.0)
- Sync version with main (5.1.0)
- Sync CHANGELOG.md from main after v5.1.0 release
- Sync main to develop after v5.1.0 release (#594)
