# Agent Management -- Tasks

## Phase 0: Agent Launch Environment and Permission Mode

- [x] T-A01 Add Claude Code telemetry disable env vars to AgentLaunchBuilder (DISABLE_TELEMETRY, DISABLE_ERROR_REPORTING, DISABLE_FEEDBACK_COMMAND, CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY, CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC)
- [x] T-A02 Add CLAUDE_CODE_NO_FLICKER=1 env var to AgentLaunchBuilder
- [x] T-A03 Add PermissionMode enum (Default/AcceptEdits/Plan/Auto/DontAsk/BypassPermissions) with --permission-mode CLI flag
- [x] T-A04 Write test: Claude Code build includes all telemetry disable env vars
- [x] T-A05 Write test: Claude Code build with PermissionMode::Auto emits --permission-mode auto
- [x] T-A06 Update SPEC-3 spec.md with complete env var table and CLI flags

## Phase 1: Version Cache -- Core

- [x] T001 [P] Write RED test: cache file round-trip (write versions, read back, verify content matches).
- [x] T002 [P] Write RED test: cache TTL expiry (fresh cache returns versions, expired cache triggers refresh).
- [x] T003 [P] Write RED test: corrupted cache file triggers graceful fallback (empty version list, no crash).
- [x] T004 Define cache schema struct: `AgentVersionCache { agents: HashMap<String, AgentVersionEntry> }` with `AgentVersionEntry { versions: Vec<String>, fetched_at: DateTime }`.
- [x] T005 Implement cache read: deserialize from `~/.gwt/cache/agent-versions.json`, return empty on error.
- [x] T006 Implement cache write: atomic write (temp file + rename) to prevent corruption.
- [x] T007 Implement TTL check: compare `fetched_at` with current time, return stale if beyond 24 hours.
- [x] T008 Verify cache core tests pass GREEN.

## Phase 2: Version Cache -- npm Registry Fetch

- [x] T009 [P] Write RED test: npm registry fetch returns parsed version list for a known package.
- [x] T010 [P] Write RED test: network failure during fetch returns error without panic.
- [x] T011 Implement npm registry HTTP client: GET `https://registry.npmjs.org/{package}` and parse `versions` field.
- [x] T012 Extract last 10 versions sorted by semver descending.
- [x] T013 Verify registry fetch tests pass GREEN.

## Phase 3: Version Cache -- Startup Integration

- [x] T014 Write RED test: startup spawns async cache refresh when cache is expired.
- [x] T015 Write RED test: startup does not block on cache refresh (UI is interactive immediately).
- [x] T016 Implement async startup task: check TTL, if expired spawn tokio task to fetch and update cache.
- [x] T017 Wire cached versions into wizard model selection step.
- [x] T018 Verify startup integration tests pass GREEN.

## Phase 4: Session Conversion UI

- [x] T019 [P] Write RED test: session conversion replaces PTY with new agent, preserves working directory.
- [x] T020 [P] Write RED test: session conversion failure restores original session.
- [x] T021 Implement session conversion action: terminate current PTY, launch new agent.
- [x] T022 Implement conversion error handling: restore original session on failure, display notification.
- [x] T023 Wire conversion into session context keybinding.
- [x] T024 Verify session conversion tests pass GREEN.

## Phase 5: Regression and Polish

- [x] T025 Run full existing test suite and verify no regressions.
- [x] T026 Run `cargo clippy` and `cargo fmt` on all changed files.
- [ ] T027 Update SPEC-3 progress artifacts with verification results.
