# Agent Management -- Tasks

## Phase 0: Agent Launch Environment and Permission Mode

- [x] T-A01 Add Claude Code telemetry disable env vars to AgentLaunchBuilder (DISABLE_TELEMETRY, DISABLE_ERROR_REPORTING, DISABLE_FEEDBACK_COMMAND, CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY, CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC)
- [x] T-A02 Add CLAUDE_CODE_NO_FLICKER=1 env var to AgentLaunchBuilder
- [x] T-A03 Add PermissionMode enum (Default/AcceptEdits/Plan/Auto/DontAsk/BypassPermissions) with --permission-mode CLI flag
- [x] T-A04 Write test: Claude Code build includes all telemetry disable env vars
- [x] T-A05 Write test: Claude Code build with PermissionMode::Auto emits --permission-mode auto
- [x] T-A06 Update SPEC-3 spec.md with complete env var table and CLI flags

## Phase 0b: SkipPermissions Legacy Flags

- [x] T-A07 Write RED tests: SkipPermissions adds legacy flags per agent (Claude: --dangerously-skip-permissions, Codex/Gemini/Copilot: --yolo).
- [x] T-A08 Write RED tests: Custom Agent schema accepts skipPermissionsArgs and applies them on launch.
- [x] T-A09 Update launch builder + wizard to use legacy SkipPermissions flags and custom args.
- [x] T-A10 Update SPEC-3 spec.md with legacy flags and custom agent schema field.

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
- [x] T035 Implement branch-first wizard ordering (Branch Type -> Issue -> Branch Name -> Agent) while keeping current Confirm handoff and leaving AI naming dormant in the standard flow.
- [x] T036 Restore old branch type and execution mode labels in the current wizard UI and update focused wizard tests.

## Phase 8: Old-TUI Wizard Step Machine Restoration

- [x] T037 [P] Write RED test: existing-branch launches use `BranchAction` as the first actionable step and reach completion without a separate `Confirm` step.
- [x] T038 [P] Write RED test: new-branch and spec-prefilled launches traverse `BranchType -> Issue -> BranchName -> Agent` in the standard flow.
- [x] T039 [P] Write RED test: `Convert` execution mode routes through `ConvertAgentSelect` and `ConvertSessionSelect`.
- [x] T040 Rewrite `WizardStep`, `next_step()`, and `prev_step()` to the old-TUI-aligned step machine while preserving version cache, dormant AI suggestion support, and session conversion state.
- [x] T041 Verify focused wizard tests, workspace checks, and refresh SPEC-3 artifacts.

## Phase 9: Old-TUI Wizard Option Formatting

- [x] T042 [P] Write RED test: `ModelSelect`, `ReasoningLevel`, `ExecutionMode`, and `SkipPermissions` render old-TUI-style label + description rows.
- [x] T043 [P] Write RED test: `VersionSelect` renders old-TUI-style `label - description` rows plus overflow indicators.
- [x] T044 Implement specialized wizard row rendering for the affected steps without changing launch semantics.
- [x] T045 Verify focused wizard render tests, workspace checks, and refresh SPEC-3 artifacts.

## Phase 10: Quick Start History Restoration

- [x] T046 [P] Write RED test: existing-branch wizard startup loads newest per-agent Quick Start history from persisted sessions for the current repository and branch.
- [x] T047 [P] Write RED test: `QuickStart` renders old-TUI grouped rows (`Branch: ...`, colored agent headers, `Resume`, `Start new`, `Choose different`) and uses `entries * 2 + 1` selectable options.
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

## Phase 21: QuickStart Footer Separator Compaction

- [x] T089 [P] Write RED test: `QuickStart` uses a compact separator before `Choose different` instead of a full-width rule.
- [x] T090 Replace the full-width footer separator with a compact rule while preserving the final action row.
- [x] T091 Verify focused footer-separator tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 22: QuickStart Footer Action Description

- [x] T092 [P] Write RED test: `Choose different` shows a description on wide popups.
- [x] T093 [P] Write RED test: `Choose different` falls back to the label-only row on narrow popups.
- [x] T094 Render the final QuickStart action as a `label - description` row on wide widths while preserving narrow-width fallback.
- [x] T095 Verify focused footer-action tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 23: QuickStart Footer Separator Removal

- [x] T096 [P] Write RED test: `Choose different` follows the last grouped `Start new` row without an extra separator line.
- [x] T097 Remove the remaining footer separator so the final action follows the last grouped row directly while preserving selection semantics.
- [x] T098 Verify focused footer-density tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 24: QuickStart Action Label Restoration

- [x] T099 [P] Write RED test: `QuickStart` uses `Resume session` / `Start new session` labels in the grouped history render and option list.
- [x] T100 Restore the old-TUI action labels while preserving resume-session ID snippets and existing selection semantics.
- [x] T101 Verify focused label-restoration tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 25: QuickStart Final Label Copy

- [x] T102 [P] Write RED test: the final Quick Start action uses `Choose different` without an ellipsis in wide and narrow render paths.
- [x] T103 Remove the rebuilt ellipsis from the final Quick Start action label while preserving the wide description row and narrow fallback.
- [x] T104 Verify focused final-label tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 26: QuickStart Single-Entry Title Promotion

- [x] T105 [P] Write RED test: a single-entry Quick Start promotes the agent/model summary into the popup title and removes the duplicated body header.
- [x] T106 Update `wizard.rs` so single-entry Quick Start titles render `Quick Start — ...` while multi-entry grouped headers remain unchanged.
- [x] T107 Verify focused single-entry Quick Start tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 27: QuickStart Multi-Entry Header Simplification

- [x] T108 [P] Write RED test: multi-entry Quick Start grouped headers render the agent label only instead of model/reasoning detail.
- [x] T109 Update `wizard.rs` so multi-entry grouped headers use `tool_label` only while single-entry title promotion remains intact.
- [x] T110 Verify focused multi-entry Quick Start tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 28: QuickStart Selected Resume Hint

- [x] T111 [P] Write RED test: multi-entry Quick Start shows the resume-session ID snippet only on the selected `Resume session` row.
- [x] T112 Update `wizard.rs` so unselected multi-entry resume rows fall back to `Resume session` while the selected row keeps the short session ID hint.
- [x] T113 Verify focused selected-resume-hint tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 29: QuickStart Multi-Entry Action Copy Compaction

- [x] T114 [P] Write RED test: multi-entry Quick Start uses the compact old-TUI action labels `Resume` / `Start new` in the grouped body render.
- [x] T115 Update `wizard.rs` so multi-entry grouped history uses `Resume` / `Start new` while single-entry Quick Start keeps `Resume session` / `Start new session`.
- [x] T116 Verify focused compact-action-label tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 30: QuickStart Footer Label-Only Copy

- [x] T117 [P] Write RED test: the final Quick Start action uses label-only `Choose different` even on wide popups.
- [x] T118 Update `wizard.rs` so `Choose different` stays label-only across wide and narrow render paths.
- [x] T119 Verify focused footer-copy tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 31: QuickStart Option Copy Alignment

- [x] T120 [P] Write RED test: multi-entry `QuickStart` `current_options()` uses compact `Resume` / `Start new` labels and selected-row resume hints.
- [x] T121 Update `wizard.rs` so multi-entry `current_options()` mirrors the rendered compact grouped-row copy while single-entry Quick Start keeps the longer labels.
- [x] T122 Verify focused option-copy-alignment tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 32: Wizard Progress Chrome Removal

- [x] T123 [P] Write RED test: the wizard popup omits the separate `Step N/M` row while keeping popup chrome and branch-context content visible.
- [x] T124 Remove the redundant progress row from `wizard.rs` so the popup title remains the only step-context chrome and content regains the reclaimed line.
- [x] T125 Verify focused wizard-layout tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 33: Single-Entry QuickStart Action Copy Compaction

- [x] T126 [P] Write RED test: single-entry `QuickStart` render and `current_options()` use compact `Resume` / `Start new` labels while keeping the resume-session hint.
- [x] T127 Update `wizard.rs` so single-entry `QuickStart` matches the compact body-action copy already used by multi-entry history.
- [x] T128 Verify focused single-entry copy tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 34: QuickStart Branch Context Compaction

- [x] T129 [P] Write RED test: `QuickStart` uses a compact branch-name context line without the rebuilt `Branch: ...` prefix.
- [x] T130 Update `wizard.rs` so `QuickStart` renders the branch name as the compact context line while preserving grouped ordering and title-promotion behavior.
- [x] T131 Verify focused branch-context tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 35: Single-Entry QuickStart Title Compaction

- [x] T132 [P] Write RED test: single-entry `QuickStart` title uses `Agent (Model)` without the rebuilt reasoning copy.
- [x] T133 Update `wizard.rs` so single-entry title promotion keeps only the model-level summary while body actions stay compact.
- [x] T134 Verify focused title-compaction tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 36: Multi-Entry QuickStart Inline Agent Rows

- [x] T135 [P] Write RED test: multi-entry `QuickStart` renders agent-labeled action rows without standalone header rows.
- [x] T136 Update `wizard.rs` so multi-entry render and `current_options()` inline the agent label into each action row while preserving selection semantics.
- [x] T137 Verify focused inline-agent-row tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 37: Single-Entry QuickStart Title Placeholder Removal

- [x] T138 [P] Write RED test: single-entry `QuickStart` title omits the synthesized `default` model placeholder when no model was persisted.
- [x] T139 Update `wizard.rs` so single-entry title promotion falls back to the agent label alone when `model` is absent.
- [x] T140 Verify focused title-summary tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 38: QuickStart Footer Label Compaction

- [x] T141 [P] Write RED test: render and `current_options()` both use `Choose different` for the final Quick Start action.
- [x] T142 Update `wizard.rs` so the final Quick Start action label is compacted to `Choose different` without changing selection semantics.
- [x] T143 Verify focused footer-label tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 39: AgentSelect Branch Context Compaction

- [x] T144 [P] Write RED test: existing-branch `AgentSelect` uses the compact branch-name line and places the first agent row directly below it.
- [x] T145 Update `wizard.rs` so existing-branch `AgentSelect` removes the `Branch: ...` prefix and closes the extra spacer row.
- [x] T146 Verify focused agent-select tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 40: QuickStart Start-New Copy Compaction

- [x] T147 [P] Write RED test: multi-entry `QuickStart` render removes the agent prefix from the paired `Start new` row while keeping the inline-labeled `Resume` row.
- [x] T148 [P] Write RED test: multi-entry `QuickStart` `current_options()` mirrors the same split between agent-labeled `Resume` and plain `Start new`.
- [x] T149 Update `wizard.rs` so multi-entry `QuickStart` keeps the agent label only on `Resume` rows while `Start new` stays compact and plain.

## Phase 41: QuickStart Start-New Neutral Styling

- [x] T150 [P] Write RED test: non-selected multi-entry `QuickStart` `Start new` rows render with neutral styling instead of agent-colored text.
- [x] T151 Update `wizard.rs` so multi-entry `Start new` rows use neutral styling while `Resume` rows keep inline agent identity.
- [x] T152 Verify focused quick-start styling tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 42: QuickStart Start-New Hierarchy Indent

- [x] T153 [P] Write RED test: multi-entry `QuickStart` renders the plain `Start new` row two columns deeper than the paired `Resume` row.
- [x] T154 Update `wizard.rs` so multi-entry `Start new` rows render with a child-action indent while single-entry rendering stays unchanged.
- [x] T155 Verify focused quick-start indent tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 43: Config-Backed Custom Agent Runtime

- [x] T156 [P] Write RED test: custom agent loader parses `[tools.customCodingAgents]` entries from config TOML.
- [x] T157 [P] Write RED test: wizard AgentSelect appends config-backed custom agents after the built-in entries.
- [x] T158 [P] Write RED test: custom-agent launch config uses the configured command, display name, mode args, and env vars.
- [x] T159 Implement config-backed custom agent load/list/launch integration in `app.rs` without reopening the broader settings model.
- [x] T160 Verify focused custom-agent tests and refresh SPEC-3 artifacts.

## Phase 44: Settings-Backed Custom Agent CRUD

- [x] T161 [P] Write RED test: Settings > Custom Agents loads persisted custom-agent fields from `~/.gwt/config.toml`.
- [x] T162 [P] Write RED test: add/edit/delete interactions in Settings persist immediately without an explicit save step.
- [x] T163 Extract shared custom-agent config load/save helpers into `crates/gwt-tui/src/custom_agents.rs`, preserving unrelated settings and unknown nested custom-agent tables.
- [x] T164 Implement Settings > Custom Agents selector/edit/action rows in `crates/gwt-tui/src/screens/settings.rs` and wire them to the shared helper.
- [x] T165 Verify focused `custom_agents` and `screens::settings` tests, then refresh SPEC-3 artifacts.

## Phase 45: Issue Detail Launch Agent Restoration

- [x] T166 Write RED test: `Shift+Enter` on Issue detail opens the wizard with prefilled issue context and the standard new-branch flow.
- [x] T167 Restore the Issues detail route in `app.rs` so `Shift+Enter` opens the wizard, seeds issue-derived branch context, and prefills `issue_id`.
- [x] T168 Verify focused issue-launch tests, broad workspace checks, and refresh SPEC-3 artifacts.

## Phase 46: Codex Model Snapshot Sync

- [x] T169 Write RED test: Codex model options and old-TUI model-step rendering match the current Codex CLI snapshot.
- [x] T170 Update `wizard.rs` so the Launch Agent Codex model list and descriptions match the current CLI snapshot while keeping `Default (Auto)` as the non-explicit override.
- [x] T171 Verify focused wizard tests, broad workspace checks, and refresh SPEC-3 artifacts.
