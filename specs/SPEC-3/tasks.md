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
- [x] T017 Wire cached versions into the wizard flow.
- [x] T018 Verify startup integration tests pass GREEN.

## Phase 4: Session Conversion UI

- [x] T019 [P] Write RED test: session conversion updates the active session agent identity and preserves repository context.
- [x] T020 [P] Write RED test: session conversion failure restores original session.
- [x] T021 Implement session conversion action: update active session metadata to the selected agent.
- [x] T022 Implement conversion error handling: restore original session on failure, display notification.
- [x] T023 Wire conversion into session context keybinding.
- [x] T024 Verify session conversion tests pass GREEN.

## Phase 5: Regression and Polish

- [x] T025 Run full existing test suite and verify no regressions.
- [x] T026 Run `cargo clippy` and `cargo fmt` on all changed files.
- [x] T027 Update SPEC-3 progress artifacts with verification results.

## Phase 6: Version Selection and Launch Materialization

- [x] T028 Write RED test: VersionSelect options include installed, `latest`,
  and cached semver entries without duplicating the installed version.
- [x] T029 Write RED test: wizard keeps model selection separate from version
  selection and shows the chosen version in the confirm summary.
- [x] T030 Implement dedicated VersionSelect option refresh when the selected
  agent changes or the wizard is prefilled from Quick Start.
- [x] T031 Write RED test: launch config omits default model labels, preserves
  the selected version, and resolves pending launch state into a persisted
  agent session.
- [x] T032 Implement pending launch materialization and update SPEC-3
  artifacts with focused verification evidence.

## Phase 7: Wizard UX Restoration

- [x] T033 Write RED test: branch launch create-new path enters BranchTypeSelect instead of jumping directly to agent selection.
- [x] T034 Write RED test: spec-prefilled wizard startup begins at BranchTypeSelect and preserves the SPEC branch seed.
- [x] T035 Implement branch-first wizard ordering (Branch Type -> Issue -> AI naming -> Branch Name -> Agent) while keeping current Confirm handoff.
- [x] T036 Restore old branch type and execution mode labels in the current wizard UI and update focused wizard tests.

## Phase 8: Old-TUI Wizard Step Machine Restoration

- [x] T037 [P] Write RED test: existing-branch launches use `BranchAction` as the first actionable step and reach completion without a separate `Confirm` step.
- [x] T038 [P] Write RED test: new-branch and spec-prefilled launches traverse `BranchType -> Issue -> AI Suggest -> BranchName -> Agent`.
- [x] T039 [P] Write RED test: `Convert` execution mode routes through `ConvertAgentSelect` and `ConvertSessionSelect`.
- [x] T040 Rewrite `WizardStep`, `next_step()`, and `prev_step()` to the old-TUI-aligned step machine while preserving version cache, AI suggestion, and session conversion state.
- [x] T041 Verify focused wizard tests, workspace checks, and refresh SPEC-3 artifacts.

## Phase 9: Old-TUI Wizard Option Formatting

- [x] T042 [P] Write RED test: `ModelSelect`, `ReasoningLevel`, `ExecutionMode`, and `SkipPermissions` render old-TUI-style label + description rows.
- [x] T043 [P] Write RED test: `VersionSelect` renders old-TUI-style `label - description` rows plus overflow indicators.
- [x] T044 Implement specialized wizard row rendering for the affected steps without changing launch semantics.
- [x] T045 Verify focused wizard render tests, workspace checks, and refresh SPEC-3 artifacts.

## Phase 10: Quick Start History Restoration

- [x] T046 [P] Write RED test: existing-branch wizard startup loads newest per-agent Quick Start history from persisted sessions for the current repository and branch.
- [x] T047 [P] Write RED test: `QuickStart` renders old-TUI grouped rows (`Branch: ...`, colored agent headers, `Resume`, `Start new`, `Choose different settings...`) and uses `entries * 2 + 1` selectable options.
- [x] T048 Implement persisted-session-backed Quick Start loading, old-TUI grouped rendering, and `Resume`/`Start new` selection semantics including resume-ID fallback to `Continue`.
- [x] T049 Verify focused Quick Start tests, workspace checks, and refresh SPEC-3 artifacts.

## Phase 11: Old-TUI AgentSelect and Popup Chrome

- [x] T050 [P] Write RED test: popup chrome shows the current step title in the border plus a right-aligned `[ESC]` hint.
- [x] T051 [P] Write RED test: existing-branch `AgentSelect` renders `Branch: ...` above name-only agent rows.
- [x] T052 Implement old-TUI popup chrome and specialized `AgentSelect` rendering without changing step transitions or launch semantics.
- [x] T053 Verify focused wizard render tests, workspace checks, and refresh SPEC-3 artifacts.

## Phase 12: Old-TUI Inline Input Prompts

- [x] T054 [P] Write RED test: `BranchNameInput` renders an inline `Branch Name:` prompt instead of a nested titled block.
- [x] T055 [P] Write RED test: `IssueSelect` renders an inline `Issue ID (optional):` prompt instead of a nested titled block.
- [x] T056 Update `wizard.rs` input-step rendering so branch and issue input reuse the popup chrome and show inline prompt/value rows.
- [x] T057 Verify focused wizard input tests and the wizard suite pass GREEN.
- [x] T058 Refresh SPEC-3 artifacts and verification evidence for the inline-prompt parity slice.

## Phase 13: Old-TUI Single-Surface Popup Content

- [x] T059 [P] Write RED test: generic option-list steps reuse the popup chrome without rendering a nested inner box.
- [x] T060 [P] Write RED test: `ModelSelect`, `VersionSelect`, and AI suggestion loading reuse the popup chrome without nested inner boxes.
- [x] T061 Remove nested content borders from wizard option-list renderers while keeping old-TUI row formatting and version overflow indicators.
- [x] T062 Keep AI suggestion loading/error copy visible after removing duplicate inner titles and borders.
- [x] T063 Verify focused popup-content tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 14: AI Suggestion Context Consistency

- [x] T064 [P] Write RED test: AI suggestion candidate rendering keeps `Context: ...` visible while still using the popup chrome as the only box.
- [x] T065 Update `render_ai_suggest()` so the context line stays visible in the suggestion-list state alongside candidates and `Manual input`.
- [x] T066 Verify focused AI suggestion render tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 15: AI Suggestion State Layout Alignment

- [x] T067 [P] Write RED test: AI suggestion loading state renders `Context: ...` as a standalone line above the loading copy.
- [x] T068 [P] Write RED test: AI suggestion error state renders `Context: ...` as a standalone line above the error copy.
- [x] T069 Update `render_ai_suggest()` so loading and error states use the same context-first layout as the suggestion-list state.
- [x] T070 Verify focused AI suggestion layout tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 16: AI Suggestion Body Copy Compaction

- [x] T071 [P] Write RED test: AI suggestion loading body copy omits duplicate manual-input guidance.
- [x] T072 [P] Write RED test: AI suggestion error body copy omits duplicate manual-input guidance.
- [x] T073 Compact AI suggestion loading/error body copy to a single status line while keeping footer hints as the sole manual-guidance surface.
- [x] T074 Verify focused AI suggestion compaction tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 17: Wizard Selection Highlight Consistency

- [x] T075 [P] Write RED test: generic wizard option lists use the old-TUI cyan selected-row highlight.
- [x] T076 [P] Write RED test: `ModelSelect` uses the same cyan selected-row highlight.
- [x] T077 Add a wizard-local selected-row style helper and apply it across list-based wizard steps.
- [x] T078 Verify focused highlight tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 18: Wizard Input Two-Row Layout

- [x] T079 [P] Write RED test: `BranchNameInput` renders prompt and value on separate rows.
- [x] T080 [P] Write RED test: `IssueSelect` renders prompt and value on separate rows.
- [x] T081 Update wizard input rendering to use a compact two-row layout while keeping popup chrome as the only box.
- [x] T082 Verify focused input-layout tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 19: QuickStart Density Restoration

- [x] T083 [P] Write RED test: `QuickStart` places the first grouped history entry directly below the branch context line.
- [x] T084 Tighten `QuickStart` vertical spacing so the grouped history starts immediately under `Branch: ...` while preserving grouped rows and separators.
- [x] T085 Verify focused QuickStart density tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 20: QuickStart Group Density Restoration

- [x] T086 [P] Write RED test: `QuickStart` places the next agent group directly below the previous group's action rows.
- [x] T087 Remove blank spacer rows between Quick Start agent groups while preserving group headers and the final separator.
- [x] T088 Verify focused group-density tests, broad workspace checks, and refresh SPEC-3 artifacts.
