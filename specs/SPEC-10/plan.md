# Project Workspace — Implementation Plan

## Summary

Add workspace initialization flow to gwt-tui: repo detection on startup, clone wizard for non-repo directories, bare repo migration guidance, and develop branch commit protection.

## Technical Context

- **Model**: `crates/gwt-tui/src/model.rs` — add `ActiveLayer::Initialization`
- **App**: `crates/gwt-tui/src/app.rs` — add repo detection in startup, Initialization layer rendering
- **Main**: `crates/gwt-tui/src/main.rs` — detect repo before creating Model
- **New screen**: `crates/gwt-tui/src/screens/initialization.rs` — clone wizard UI
- **gwt-git**: `crates/gwt-git/src/repository.rs` — add repo detection helpers

## Phased Implementation

### Phase 1: Repo Detection + ActiveLayer::Initialization

1. Add `ActiveLayer::Initialization` to model.rs
2. Add `detect_repo_type(path) -> RepoType` to gwt-git (Normal/Bare/NonRepo)
3. In main.rs: detect repo type before Model::new(), set initial layer accordingly
4. Create `screens/initialization.rs` with URL input + clone progress

### Phase 2: Clone Wizard

1. Implement `clone_repo(url, target_dir)` in gwt-git using `git clone --depth=1`
2. Wire clone into initialization screen: Enter → start clone → progress → success/error
3. On success: Model::reset(new_repo_root) → transition to Management layer

### Phase 3: Bare Repo Migration + Pre-commit Hook

1. Bare repo detection → show migration error screen
2. Pre-commit hook installation after clone
3. Hook preserves existing hooks

## Dependencies

- gwt-git: Repository::detect_type()
- gwt-core: process::run_command() for git clone

## Verification

1. `cargo test -p gwt-tui` — all pass
2. `cargo test -p gwt-git` — all pass
3. E2E: initialization screen snapshot, clone flow, bare migration screen
