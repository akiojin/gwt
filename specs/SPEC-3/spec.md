# Agent Management -- Detection, Launch Wizard, Custom Agents, Version Cache

## Background

gwt detects and launches coding agents (Claude Code, Codex, Gemini,
OpenCode, Copilot) with a dynamic wizard. Agents are detected via PATH
lookup combined with `--version` invocation. Custom agents are configurable
via the Settings management tab. The current ratatui wizard now restores the
branch-first launch flow, old-TUI-aligned step machine and popup chrome,
version cache, session conversion, and Settings-backed custom-agent CRUD.
This SPEC covers the complete agent management domain including detection,
wizard flow, custom agents, version cache, launch resolution, and the
reviewer-facing acceptance contract for that restored UX.

## User Stories

### US-1: Launch Agent via Wizard (P0) -- IMPLEMENTED

As a developer, I want to launch a coding agent through a guided wizard so that I can configure the session correctly before starting.

**Acceptance Scenarios**

1. Given I initiate agent launch from an existing branch, when the wizard starts, then I first see the branch action step before agent selection.
2. Given I initiate agent launch from SPEC context, when the wizard starts, then I begin at branch type selection with the SPEC branch seed prefilled.
3. Given I proceed through the wizard, when I reach the final selection
   step, then the launch completes directly without a trailing confirm
   screen.
4. Given I confirm the wizard, when the agent launches, then a new persisted
   session is created with the configured parameters and the actual launched
   worktree path.
5. Given I create a new branch from the Branches flow, when launch
   materialization runs, then gwt creates a sibling worktree whose path
   mirrors that branch hierarchy (for example `feature/aaa` ->
   `../feature/aaa`) before spawning the agent PTY.
6. Given the new-branch flow starts from a selected branch, when launch
   materialization runs, then gwt resolves `origin/<selected-branch>` as the
   base branch, creates the remote target branch first, and only then
   materializes the local worktree.
7. Given the new-branch flow starts from SPEC or Issue context without a
   selected base branch, when launch materialization runs, then
   `origin/develop` is used as the default base branch and launch fails if it
   does not exist.
8. Given I cancel at any wizard step, when I press Escape, then no session is created and I return to the previous view.

### US-7: Restore Old-TUI Wizard Step Machine (P0) -- IMPLEMENTED

As a developer, I want the new ratatui wizard to follow the old-TUI launch
flow so that the daily launch UX matches existing muscle memory while
preserving new capabilities such as version cache, AI branch suggestion, and
session conversion.

**Acceptance Scenarios**

1. Given I launch from an existing branch, when the wizard opens, then I see
   `BranchAction` before any agent configuration step.
2. Given I choose to create a new branch, when I continue, then the wizard
   runs `BranchType -> Issue -> AI Suggest -> Branch Name -> Agent`.
3. Given I select Codex, when I continue through the wizard, then the flow
   includes `Model -> Reasoning -> Version -> Execution Mode -> Skip Permissions -> Codex Fast Mode`
   without requiring a trailing confirm screen. When I select Claude Code
   with an effort-capable model (Opus 4.6 / Sonnet 4.6), the same
   `Model -> Reasoning -> Version -> Execution Mode -> Skip Permissions`
   sequence runs (with `Reasoning` mapped to `CLAUDE_CODE_EFFORT_LEVEL` as
   described in US-10); Claude Code has no Fast Mode step.
4. Given I choose session conversion, when I pick `Convert` from execution
   mode, then the wizard routes through `ConvertAgentSelect` and
   `ConvertSessionSelect` before `SkipPermissions`.
5. Given I reach the last step, when I confirm the selection there, then the
   launch completes directly from the final step instead of a separate
   `Confirm` screen.

### US-8: Restore Old-TUI Wizard Option Formatting (P1) -- IMPLEMENTED

As a developer, I want the wizard option lists to match the old-TUI visual
format so that model, reasoning, version, execution mode, and skip
permissions plus Codex fast-mode settings are easier to scan at a glance.

**Acceptance Scenarios**

1. Given I am on `ModelSelect`, when the list renders, then each row shows a
   concise label plus description in the old-TUI `label - description`
   format.
2. Given I am on `ReasoningLevel`, `ExecutionMode`, `SkipPermissions`, or
   `CodexFastMode`,
   when the list renders, then each row uses the old-TUI fixed-width label
   plus description layout.
3. Given `VersionSelect` has more options than fit in the popup, when I
   scroll, then the popup shows `^ more above ^` / `v more below v`
   indicators.

### US-9: Restore Old-TUI AgentSelect and Popup Chrome (P1) -- IMPLEMENTED

As a developer, I want the AgentSelect step and popup chrome to match the
old-TUI layout so that the wizard feels visually consistent with the branch-
first flow that was already restored.

**Acceptance Scenarios**

1. Given I enter `AgentSelect` from an existing branch, when the popup
   renders, then the content shows the compact branch-name context line above
   the agent list.
2. Given `AgentSelect` renders, when the list is shown, then rows use agent
   names only instead of status-heavy labels, with the selected row using the
   old-TUI cyan highlight.
3. Given the popup renders any wizard step, when the chrome is drawn, then
   the border title uses the current step name and shows a right-aligned
   `[ESC]` close hint like the old TUI.

### US-2: Detect Installed Agents (P0) -- IMPLEMENTED

As a developer, I want gwt to automatically detect which coding agents are installed so that I only see available options.

**Acceptance Scenarios**

1. Given Claude Code is installed and in PATH, when gwt detects agents, then Claude Code appears in the agent selection list.
2. Given an agent is not installed, when gwt detects agents, then that agent does not appear in the selection list.
3. Given an agent binary exists but `--version` fails, when gwt detects agents, then that agent is marked as unavailable with a warning.

### US-3: Quick Start from Branch History (P0) -- IMPLEMENTED

As a developer, I want to quickly re-launch a previous agent session configuration so that I can resume common workflows without re-configuring.

**Acceptance Scenarios**

1. Given I have previously launched an agent on branch `feature/foo`, when I open Quick Start, then the previous configuration for that branch is listed.
2. Given Quick Start history has multiple entries, when I select one, then the wizard pre-fills all fields from the selected history entry.
3. Given Quick Start history is empty, when I launch from an existing branch, then the wizard skips Quick Start and starts at `BranchAction`.
4. Given Quick Start history exists for multiple agents, when the list renders, then each agent shows its own compact action rows, `Resume`, `Start new`, and a final `Choose different` row in the old-TUI layout.
5. Given the selected history entry has a persisted resume session ID, when I choose `Resume`, then launch configuration restores `Resume` mode with that session ID. When no resume session ID exists, the wizard falls back to `Continue`.
6. Given the selected Quick Start entry is Claude and persisted `skip_permissions=true`, when I select that entry, then the wizard resets skip permissions to `No` by default and launch args do not auto-append Claude bypass flags unless re-selected explicitly.

### US-4: Manage Custom Agents (P1) -- IMPLEMENTED

As a developer, I want to add, edit, and remove custom agents via Settings so that I can use agents not built into gwt.

**Acceptance Scenarios**

1. Given I am in Settings > Custom Agents, when I add a new agent with display name, type (Command/Path/Bunx), and command, then it appears in the agent selection list.
2. Given a custom agent exists, when I edit its configuration, then the changes are saved and reflected on next use.
3. Given a custom agent exists, when I delete it, then it is removed from the agent selection list.
4. Given a custom agent's command is invalid, when I try to launch it, then an error message is displayed with the failing command.

### US-5: Cache Agent Version List at Startup (P1) -- IMPLEMENTED

As a developer, I want gwt to cache available agent versions at startup so that version selection in the wizard is fast and does not block the UI.

**Acceptance Scenarios**

1. Given gwt starts, when the version cache is empty or expired (TTL 24 hours), then gwt fetches the last 10 versions per agent from the npm registry asynchronously.
2. Given the version cache is fresh, when I open the agent wizard, then
   version options load instantly from cache in a dedicated VersionSelect
   step.
3. Given the network is unavailable during cache refresh, when I open the wizard, then stale cached versions are shown with a "cache outdated" indicator.
4. Given a new agent version is published, when the cache refreshes after TTL
   expiry, then the new version appears in the list alongside the installed
   and `latest` options.

### US-11: Codex Reasoning Level UI Labeling (P1) -- NOT IMPLEMENTED

As a developer launching Codex, I want the `ReasoningLevel` wizard step to
mirror the Codex CLI's own `/model` reasoning selection so that the level
names, ordering, default marker, and current-selection marker match what I
already see in the Codex CLI.

The wizard `ReasoningLevel` step for Codex must render exactly four rows in
the order below. The header line above the list must read
`Select Reasoning Level for <model>` where `<model>` is the model id chosen
on `ModelSelect` (for example `gpt-5.4`). The actual `-c
model_reasoning_effort=<value>` value sent to Codex is NOT identical to the
displayed label and is mapped per row by the launch builder; the wizard is
responsible only for displaying these four rows and recording the user's
selection.

```
Select Reasoning Level for <model>

  1. Low                       Fast responses with lighter reasoning
  2. Medium (default)          Balances speed and reasoning depth for everyday tasks
  3. High                      Greater reasoning depth for complex problems
> 4. Extra high (current)      Extra high reasoning depth for complex problems
```

**Acceptance Scenarios**

1. Given I reach the Codex `ReasoningLevel` step, when the popup renders,
   then the header line reads `Select Reasoning Level for <model>` using
   the model id selected on `ModelSelect`.
2. Given the popup renders, when I scan the list, then I see exactly the
   four labels `Low`, `Medium`, `High`, `Extra high` in this fixed order,
   each followed by the description copy shown above.
3. Given the popup renders, when I look at the labels, then `Medium`
   carries a `(default)` annotation regardless of which row is selected,
   marking the Codex-side default reasoning level.
4. Given a row is currently selected, when the popup renders, then that
   row's label carries a `(current)` annotation in addition to any
   `(default)` annotation. Only the selected row receives `(current)`.
5. Given the user has not picked a level yet on this launch, when the
   popup first renders, then the `Medium (default)` row is the initial
   selection and therefore renders as `Medium (default) (current)` — the
   two annotations stack on the default-and-currently-selected row.
6. Given the user picks any row, when launch materializes, then the
   launch builder maps the displayed label to the upstream
   `-c model_reasoning_effort=<value>` token expected by the installed
   Codex CLI version. The label-to-value mapping is owned by the launch
   builder and tracked by a Codex CLI snapshot date in this SPEC; the
   mapping is not part of the wizard's UI contract.

### US-10: Launch Claude Code with Effort Level (P1) -- NOT IMPLEMENTED

As a developer launching Claude Code, I want to pick an adaptive reasoning
effort level in the same wizard step that Codex already uses, so that I can
persist `max` effort across sessions through `CLAUDE_CODE_EFFORT_LEVEL` the
same way Codex uses `model_reasoning_effort`.

The wizard `ReasoningLevel` step for Claude Code renders the five rows
below when the selected model is effort-capable (Opus 4.6 or Sonnet 4.6).
The header line above the list reads
`Select Effort Level for <model>` where `<model>` is the model id chosen
on `ModelSelect` (for example `opus-4-6`). Unlike Codex, the Claude Code
label set maps 1:1 (case-folded) to the `CLAUDE_CODE_EFFORT_LEVEL`
environment variable value, and the wizard does not accept any label that
is not in this list.

```
Select Effort Level for <model>

  1. Auto                            Let the model decide how deeply to think (no env var exported)
> 2. Low                             Fast, cheap responses for simple renames, greps, and quick questions
  3. Medium (default)                Balanced reasoning for everyday agentic coding and tool-heavy work
  4. High                            Deeper reasoning for complex problems (API/Team/Enterprise default)
  5. Max (Opus 4.6 only) (current)   Deepest reasoning with no token-spending constraint; env-var-only persistence
```

**Label → env-var value mapping** (owned by `AgentLaunchBuilder`):

| UI row | Description anchor | `CLAUDE_CODE_EFFORT_LEVEL` value | Notes |
|---|---|---|---|
| `Auto` | model default | _(not set)_ | Neither the env var nor `--effort` is passed. Any inherited profile value falls through unchanged. |
| `Low` | fast / cheap | `low` | Persists across sessions. |
| `Medium` | balanced | `medium` | Anthropic-side default for Pro / Max subscribers. |
| `High` | deep reasoning | `high` | Anthropic-side default for API key / Team / Enterprise / Bedrock / Vertex / Foundry. |
| `Max` | deepest reasoning | `max` | **Opus 4.6 only.** Persists across sessions only when set via env var (by design of upstream Claude Code). |

**Acceptance Scenarios**

1. Given I select Claude Code in `AgentSelect` with a supported model
   (Opus 4.6 or Sonnet 4.6), when I continue through the wizard, then the
   flow includes the `ReasoningLevel` step before `VersionSelect`, using the
   same old-TUI row formatting Codex already uses, and the header line
   reads `Select Effort Level for <model>` using the model id selected on
   `ModelSelect`.
2. Given I am on `ReasoningLevel` for Claude Code with Opus 4.6, when the
   list renders, then I see exactly the five rows `Auto`, `Low`,
   `Medium`, `High`, `Max` in this fixed order with the description copy
   shown in the ASCII mock above.
3. Given I am on `ReasoningLevel` for Claude Code with Sonnet 4.6, when
   the list renders, then the `Max` row is omitted (because `max` is
   Opus 4.6 only) and the other four rows render in order
   `Auto`, `Low`, `Medium`, `High`.
4. Given the popup renders, when I look at the labels, then `Medium`
   always carries a `(default)` annotation regardless of which row is
   selected, and only the currently selected row carries a `(current)`
   annotation. On first render with no prior selection, `Low` is the
   initial selection (matching the ASCII mock's selection marker) and
   there is no stacking with `(default)`.
5. Given I pick a non-`Auto` row, when launch materializes the PTY, then
   the spawned Claude Code process receives `CLAUDE_CODE_EFFORT_LEVEL`
   set to the lower-case of the label (`low` / `medium` / `high` /
   `max`) and no `--effort` CLI flag is appended.
6. Given I keep `Auto` selected, when launch materializes the PTY, then
   `CLAUDE_CODE_EFFORT_LEVEL` is not exported and no `--effort` flag is
   appended, letting any inherited profile value or model default apply.
7. Given I select Claude Code with a model that is not effort-capable,
   when the wizard runs, then the `ReasoningLevel` step is skipped
   exactly as it is today for agents without reasoning support.
8. Given Quick Start has a persisted Claude Code entry with a non-`Auto`
   row, when I choose `Resume` or `Start new` from that entry, then the
   wizard pre-fills the effort row and a subsequent launch exports the
   same `CLAUDE_CODE_EFFORT_LEVEL` value. Persisted rows survive upstream
   label renames because the Quick Start entry stores the row identifier,
   not the env-var token.
9. Given the user already has `CLAUDE_CODE_EFFORT_LEVEL` set in the
   profile environment, when the wizard selects a non-`Auto` row, then
   the launch-injected value overrides the inherited profile value for
   the spawned PTY only (the parent process environment is untouched);
   when the wizard selects `Auto`, the inherited profile value is left
   untouched and reaches the child process unchanged.

### US-6: Convert Sessions Between Agent Types (P2) -- IMPLEMENTED

As a developer, I want to convert an existing session to a different agent type so that I can switch tools mid-workflow.

**Acceptance Scenarios**

1. Given an active agent session, when I initiate conversion, then I can select a target agent type from available agents.
2. Given a session conversion is confirmed, when the conversion completes, then the active session is re-labeled and reconfigured for the new agent while preserving repository context.
3. Given conversion fails (target agent not available), when the error occurs, then the original session remains intact with an error notification.

## Edge Cases

- Agent binary exists in PATH but is a broken symlink.
- Multiple versions of the same agent installed (e.g., via nvm, different PATH entries).
- Custom agent command contains spaces or special characters in the path.
- npm registry returns unexpected JSON format during version cache fetch.
- Version cache file is corrupted on disk.
- Network timeout during version fetch (should not block startup).
- Quick Start history file grows very large (hundreds of entries).
- Agent detection runs concurrently with user opening the wizard.
- Session conversion attempted while the session has active PTY I/O.
- Session conversion updates agent identity without reopening the current transcript buffer.
- Installed version also appears in the cached version list and must not be
  duplicated in the UI.
- User keeps the default model label selected; launch should omit an explicit
  `--model` override instead of passing the human-readable placeholder text.
- User selects Claude Code with an effort-capable model and then switches
  back to a non-effort-capable model before leaving `ModelSelect`; the
  wizard must drop any staged `ReasoningLevel` choice so launch does not
  export a stale `CLAUDE_CODE_EFFORT_LEVEL`.
- User picks Claude effort `max` on a non-Opus model; the wizard must
  either hide `max` or refuse the selection so launch never exports an
  unsupported combination.
- Upstream Codex CLI renames or removes one of the four reasoning levels;
  the launch builder mapping must be updated and the wizard contract
  reviewed before the next release. The wizard UI must never silently
  fall through to a label that no longer corresponds to a valid Codex
  value.
- `CLAUDE_CODE_EFFORT_LEVEL` already exists in the inherited profile
  environment; selecting `auto` in the wizard must not unset the inherited
  value via `env -u` tricks — gwt simply does not inject the var, and the
  child process inherits whatever the profile provided.

## Functional Requirements

- **FR-001**: `AgentTrait::detect()` checks PATH for agent binary and invokes `--version` to confirm availability.
- **FR-002**: `AgentLaunchBuilder` constructs launch configuration including model, `skip_permissions`, `codex_fast_mode`, reasoning_level, and environment variables.
- **FR-003**: Wizard flow proceeds through dynamic steps chosen by branch
  context and agent capabilities: existing-branch launches start at
  `BranchAction`, new-branch launches run `Branch Type -> Issue -> AI Branch
  Suggestion -> Branch Name` before agent configuration, and the final step
  completes directly without a trailing `Confirm` screen.
- **FR-004**: Custom agent CRUD operations available in Settings > Custom Agents tab.
- **FR-005**: `CustomCodingAgent` structure: id, display_name, agent_type (Command/Path/Bunx), command string.
- **FR-006**: Version list cache fetches last 10 versions per agent from npm registry on startup.
- **FR-007**: Cache stored in `~/.gwt/cache/agent-versions.json` with 24-hour TTL.
- **FR-008**: Quick Start stores per-branch launch history in persistent storage.
- **FR-008a**: Existing-branch wizard startup loads Quick Start history from persisted agent session metadata under `~/.gwt/sessions/`, filtered by repository path and branch name.
- **FR-009**: Session resume via `agent_session_id` for agents that support session continuity.
- **FR-010**: Codex hooks confirmation flow integrated into the wizard when Codex agent is selected.
- **FR-011**: Agent detection timeout: individual agent detection must complete within 5 seconds.
- **FR-012**: Version cache fetch is async and non-blocking; does not delay startup or wizard display.
- **FR-013**: VersionSelect options include an installed runner when detected,
  a `latest` npm runner when supported, and cached semver versions without
  duplicating the installed version.
- **FR-014**: Wizard confirmation materializes a persisted agent session in
  `~/.gwt/sessions/` before activating the new tab.
- **FR-015**: Launch resolution uses the direct installed binary for
  `installed` or empty version selection and `bunx`/`npx` package runners for
  `latest` or a specific cached semver version when the agent supports npm
  distribution.
- **FR-016**: The ratatui wizard restores the old-TUI step machine with the
  step set `QuickStart`, `BranchAction`, `AgentSelect`, `ModelSelect`,
  `ReasoningLevel`, `VersionSelect`, `ExecutionMode`,
  `ConvertAgentSelect`, `ConvertSessionSelect`, `SkipPermissions`,
  `CodexFastMode`,
  `BranchTypeSelect`, `IssueSelect`, `AIBranchSuggest`, and
  `BranchNameInput`.
- **FR-017**: `ModelSelect`, `ReasoningLevel`, `ExecutionMode`,
  `SkipPermissions`, `CodexFastMode`, and `VersionSelect` use old-TUI-style
  row formatting with descriptive text and version-list scroll indicators.
- **FR-018**: `QuickStart` renders old-TUI-style history rows with
  a compact branch-name context line, colored per-agent action rows, two
  selectable rows per entry (`Resume` / `Start new`), and a trailing
  `Choose different` action.
- **FR-019**: `AgentSelect` renders old-TUI-style existing-branch context as
  the compact branch-name line plus name-only agent rows, while the popup
  chrome shows the current step title in the border and a right-aligned
  `[ESC]` hint.
- **FR-020**: `BranchNameInput` and `IssueSelect` render as old-TUI inline
  prompts inside the popup body, reusing the popup chrome instead of adding
  nested titled input boxes.
- **FR-021**: Generic option-list steps, `VersionSelect`, and AI suggestion
  loading/error states reuse the popup chrome as the only boxed surface,
  keeping old-TUI rows and scroll indicators without nested inner borders or
  duplicate titles.
- **FR-022**: The AI branch suggestion step shows `Context: ...` consistently
  in loading, error, and suggestion-list states while still reusing the popup
  chrome as the only boxed surface.
- **FR-023**: AI suggestion loading and error states render `Context: ...` as
  the same standalone cyan line used by the suggestion-list state rather than
  embedding the context string inside the body copy.
- **FR-024**: AI suggestion loading and error states keep their body copy
  compact and do not duplicate the manual-input guidance that is already
  present in the footer hint row.
- **FR-025**: Wizard list-based steps share the same old-TUI cyan selected-row
  highlight across generic option lists, `ModelSelect`, `QuickStart`, and
  `AgentSelect`.
- **FR-026**: `BranchNameInput` and `IssueSelect` render as compact two-row
  input steps with a cyan prompt line above a yellow value line, while still
  reusing the popup chrome as the only boxed surface.
- **FR-027**: `QuickStart` starts its grouped history immediately below the
  compact branch-name context line instead of reserving an extra spacer row,
  so the popup matches the old-TUI information density.
- **FR-028**: `QuickStart` does not insert blank spacer rows between agent
  sections; the next agent-labeled action row follows directly after the
  previous `Start new` action while preserving the final action.
- **FR-029**: The final `Choose different` action follows the last
  grouped `Start new` row directly without an extra separator line, so the
  `QuickStart` popup keeps the denser old-TUI rhythm.
- **FR-030**: The wizard popup does not render a separate `Step N/M`
  progress row above the chrome; the popup border title is the only
  step-context chrome so the content area keeps the reclaimed row.
- **FR-031**: `QuickStart` action rows use the shorter old-TUI labels
  `Resume` and `Start new`, while still showing the resume session ID
  snippet when one exists.
- **FR-032**: The final Quick Start action label matches the old-TUI copy
  `Choose different` without an ellipsis.
- **FR-033**: When `QuickStart` has exactly one persisted entry, the popup
  title promotes that entry's compact agent/model summary (`Quick Start —
  Agent (Model)`, or just `Agent` when no model was persisted) and the body
  omits the duplicated grouped header so the first action row starts directly
  below the compact branch-name context line.
- **FR-034**: When `QuickStart` has multiple persisted entries, each
  `Resume` row inlines the agent label while the paired `Start new` row
  stays plain and compact; the model-only summary remains reserved for the
  single-entry title variant.
- **FR-035**: In multi-entry `QuickStart`, resume session ID snippets are
  shown only on the currently selected `Resume` row so unselected rows stay
  visually compact.
- **FR-036**: In multi-entry `QuickStart`, grouped action rows use the more
  compact old-TUI copy `Resume` / `Start new`, and single-entry Quick
  Start now matches that compact copy because the popup title already carries
  the agent/model context.
- **FR-037**: The final `QuickStart` action uses the label-only copy
  `Choose different` on both wide and narrow popups, without an
  inline description row.
- **FR-038**: `QuickStart` state-derived option labels stay aligned with the
  rendered grouped rows, so multi-entry history uses compact `Resume` /
  `Start new` labels in both the visual render and `current_options()`, and
  single-entry history now uses the same compact action wording.
- **FR-039**: In multi-entry `QuickStart`, the plain `Start new` rows render
  one level deeper than the paired agent-labeled `Resume` rows so the
  old-TUI primary/secondary action hierarchy stays visible without adding
  back standalone headers or extra copy.
- **FR-039**: In multi-entry `QuickStart`, the plain `Start new` rows use a
  neutral text color so agent identity stays anchored to the inline-labeled
  `Resume` rows only.
- **FR-040**: `SkipPermissions` and `CodexFastMode` are independent controls:
  `SkipPermissions` only controls approval behavior, while
  `CodexFastMode` only controls Codex service-tier behavior.
- **FR-041**: When `CodexFastMode=On`, Codex launch args include
  `-c service_tier=fast`; when Off, that key is omitted.
- **FR-042**: New-branch launches materialize the requested branch into a
  sibling git worktree before PTY spawn, rather than running the agent from
  the repository root.
- **FR-043**: When the new-branch flow starts from Branches,
  `BranchAction -> Create new from selected` resolves the selected branch to
  `origin/<selected>` as the remote base branch, creates `origin/<new-branch>`
  first, and then materializes the local worktree from that remote branch.
  When no selected base branch exists, Launch Agent uses `origin/develop`; if
  the required remote base branch does not exist, launch must fail before PTY
  spawn.
- **FR-044**: After launch materialization, `GWT_PROJECT_ROOT` and persisted
  session metadata use the actual launched worktree path, and any
  materialization error aborts launch before PTY spawn.
- **FR-045**: Quick Start restores persisted `skip_permissions` for all built-in agents, including Claude.
- **FR-046**: Claude launch sets `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` so Agent Teams capability is always available without an explicit pre-launch toggle.
- **FR-047**: Every agent launch appends a structured audit line to `~/.gwt/logs/agent-launch.jsonl` including command/args/cwd/env for the spawned PTY config, with sensitive env values redacted.
- **FR-048**: The `ReasoningLevel` wizard step applies to Claude Code when
  the selected model is an effort-capable Claude model (Opus 4.6 or
  Sonnet 4.6), using the same old-TUI row formatting already used for
  Codex. When the selected Claude model does not support effort, the step
  is skipped. The header line above the list reads
  `Select Effort Level for <model>` using the model id selected on
  `ModelSelect`.
- **FR-049**: Claude Code `ReasoningLevel` rows are, in fixed order:
  `Auto`, `Low`, `Medium`, `High`, `Max`. The `Max` row is shown only
  when the selected model is Opus 4.6; Sonnet 4.6 renders the first four
  rows only. Each row uses the description copy specified in US-10.
  `Medium` always carries a `(default)` annotation, and the currently
  selected row additionally carries a `(current)` annotation; first
  render defaults to `Low` as the initial selection.
- **FR-050**: `AgentLaunchBuilder` owns the label-to-env-var mapping for
  Claude Code:
  `Auto` → unset (neither env var nor CLI flag),
  `Low` → `CLAUDE_CODE_EFFORT_LEVEL=low`,
  `Medium` → `CLAUDE_CODE_EFFORT_LEVEL=medium`,
  `High` → `CLAUDE_CODE_EFFORT_LEVEL=high`,
  `Max` → `CLAUDE_CODE_EFFORT_LEVEL=max`.
  The `--effort` CLI flag is never appended. The wizard never passes the
  displayed label verbatim; it records only the row identifier and the
  launch builder performs the mapping. Values outside
  `{low, medium, high, max}` must never reach the spawned process.
- **FR-051**: Claude Code Quick Start entries persist the effective
  reasoning row identifier (including `Auto`) alongside model and
  skip_permissions, so relaunch through `Resume` / `Start new` restores
  the same `CLAUDE_CODE_EFFORT_LEVEL` behavior as the original launch
  even if the displayed label copy is later renamed.
- **FR-052**: The launch-injected `CLAUDE_CODE_EFFORT_LEVEL` value takes
  precedence over any inherited profile environment value for the spawned
  PTY. When the wizard row is `Auto`, gwt does not unset inherited
  profile values; the child process inherits whatever the profile
  provided. The parent gwt process environment is never mutated.
- **FR-053**: The Codex `ReasoningLevel` wizard step renders exactly four
  rows in fixed order: `Low`, `Medium`, `High`, `Extra high`, each with
  the description copy specified in US-11. The header line above the list
  reads `Select Reasoning Level for <model>` using the model id selected
  on `ModelSelect`.
- **FR-054**: `Medium` always carries a `(default)` annotation regardless
  of selection state, marking the Codex-side default. The currently
  selected row additionally carries a `(current)` annotation; on first
  render with no prior selection, `Medium` is the initial selection and
  therefore renders as `Medium (default) (current)`.
- **FR-055**: The Codex `ReasoningLevel` wizard step is responsible only
  for displaying labels and recording the user's row selection. The
  launch builder owns mapping the chosen label to the upstream
  `-c model_reasoning_effort=<value>` token, and the mapping is tracked
  by a Codex CLI snapshot date in this SPEC. Adding or renaming Codex
  reasoning levels in upstream requires updating the mapping but does
  not change the wizard's UI contract unless the row count or order
  changes.

## Non-Functional Requirements

- **NFR-001**: Total agent detection for all known agents completes under 2 seconds.
- **NFR-002**: Version cache fetch runs asynchronously and does not block UI rendering.
- **NFR-003**: Cache file I/O uses atomic write (write to temp, rename) to prevent corruption.
- **NFR-004**: Quick Start history is bounded to 100 entries per branch to limit file size.
- **NFR-005**: Custom agent configuration changes are persisted immediately (no explicit save step).

## Implementation Details

### Agent-Specific Environment Variables

#### Claude Code

| Variable | Value | Purpose |
|----------|-------|---------|
| `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS` | `1` | Enable agent teams |
| `CLAUDE_CODE_EFFORT_LEVEL` | `low` \| `medium` \| `high` \| `max` | Adaptive reasoning effort level for Opus 4.6 / Sonnet 4.6. Exported only when the wizard selects a non-`auto` level. `max` is Opus 4.6 only and persists across sessions only through this env var. Takes precedence over `/effort`, `--effort`, `settings.json`, and skill/subagent frontmatter. |
| `CLAUDE_CODE_NO_FLICKER` | `1` | Disable TUI flicker |
| `DISABLE_TELEMETRY` | `1` | Disable Statsig metrics |
| `DISABLE_ERROR_REPORTING` | `1` | Disable Sentry error reporting |
| `DISABLE_FEEDBACK_COMMAND` | `1` | Disable feedback command |
| `CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY` | `1` | Disable session surveys |
| `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` | `1` | Disable all non-essential traffic |
| `IS_SANDBOX` | `1` | Sandbox mode (Unix/macOS only) |

#### Codex

| Variable | Value | Purpose |
|----------|-------|---------|
| `OPENAI_API_KEY` | (from config) | Authentication |

#### Gemini

| Variable | Value | Purpose |
|----------|-------|---------|
| `GOOGLE_API_KEY` or `GEMINI_API_KEY` | (from config) | Authentication |

#### Common (All Agents)

| Variable | Value | Purpose |
|----------|-------|---------|
| `GWT_PROJECT_ROOT` | active worktree or repo root path | Launch target for agent context |
| `TERM` | `xterm-256color` | Terminal type |
| `COLORTERM` | `truecolor` | Color support |
| Profile env vars | (from profile) | User-defined environment overrides |

### Agent CLI Flags

#### Claude Code

| Flag | Description |
|------|-------------|
| `--print` | Non-interactive mode (SDK mode) |
| `--dangerously-skip-permissions` | Skip permission prompts (legacy behavior) |
| `--permission-mode bypassPermissions` | Bypass all permission checks (alternate permission-mode form) |
| `--model <model>` | Model selection (alias: `sonnet`, `opus`, or full name) |
| `--allowedTools <tools>` | Tools that execute without prompting (pattern matching supported) |
| `--disallowedTools <tools>` | Tools removed from context entirely |
| `--effort <level>` | Effort level: low/medium/high/max (Opus 4.6 only). gwt does not use this flag; it exports `CLAUDE_CODE_EFFORT_LEVEL` instead so `max` persists across sessions (see env var table). |
| `--continue`, `-c` | Continue most recent conversation |
| `--resume <id>`, `-r` | Resume specific session by ID or name |
| `--name <name>`, `-n` | Set session display name |
| `--append-system-prompt <text>` | Append to system prompt |
| `--max-turns <n>` | Limit agentic turns (print mode) |
| `--max-budget-usd <amount>` | Limit API spend (print mode) |
| `--worktree <name>`, `-w` | Run in isolated git worktree |
| `--bare` | Minimal mode (skip auto-discovery) |
| `--verbose` | Verbose logging |

#### Codex

Model list snapshot: **2026-04-06**.

| Flag | Description |
|------|-------------|
| `resume [SESSION_ID]` | Resume a specific Codex interactive session by session/thread ID |
| `resume --last` | Continue the most recent Codex interactive session |
| `--model <model>` | Default: `gpt-5.4`; available: `gpt-5.4`, `gpt-5.4-mini`, `gpt-5.3-codex`, `gpt-5.3-codex-spark`, `gpt-5.2-codex`, `gpt-5.2`, `gpt-5.1-codex-max`, `gpt-5.1-codex-mini` |
| `-c model_reasoning_effort=<level>` | Reasoning level token sent to Codex. Mapped from the wizard's four-row UI (`Low`, `Medium`, `High`, `Extra high`) by the launch builder; mapping owned by SPEC-3 and tracked by the Codex model list snapshot date above. Wizard never sends the displayed label verbatim. |
| `-c service_tier=fast` | Fast mode (Codex-only speed tier). Independent from skip-permission settings |
| `--full-auto` | Approval/sandbox automation convenience alias (not a Fast mode toggle) |
| `--yolo` | Skip permissions (Codex legacy flag) |
| `--enable web_search` | Enable web search (v0.90.0+) |
| `--enable collaboration_modes` | Enable collaboration (v0.91.0+) |
| `-c shell_environment_policy=inherit` | Shell policy |

#### Gemini

| Flag | Description |
|------|-------------|
| `--non-interactive` | Non-interactive mode |
| `--yolo` | Skip permissions (Gemini legacy flag) |

#### GitHub Copilot

| Flag | Description |
|------|-------------|
| `--yolo` | Skip permissions (Copilot legacy flag) |

### Session File Schema (`~/.gwt/sessions/{base64_path}.toml`)

```toml
[session]
id = "uuid-v4"
worktree_path = "/absolute/path"
branch = "feature/foo"
agent = "claude"  # agent identifier
agent_label = "Claude Code"
agent_session_id = "session-xxx"  # for resume
tool_version = "1.0.0"
model = "claude-sonnet-4-5"
skip_permissions = false
codex_fast_mode = false
created_at = "2026-01-01T00:00:00Z"
updated_at = "2026-01-01T00:00:00Z"
last_activity_at = "2026-01-01T00:00:00Z"
status = "running"  # unknown | running | waiting_input | stopped
display_name = "My Session"
```

- File path: Base64 URL-safe no-pad encoding of worktree path
- Idle timeout: 60 seconds → status changes to `stopped`

### Custom Agent Schema (`~/.gwt/config.toml`)

```toml
[tools.customCodingAgents.my-agent]
id = "my-agent"
displayName = "My Agent"
agentType = "command"  # command | path | bunx
command = "my-agent-cli"
defaultArgs = ["--flag"]
skipPermissionsArgs = ["--yolo"]

[tools.customCodingAgents.my-agent.modeArgs]
normal = []
continue = ["--continue"]
resume = ["--resume"]

[tools.customCodingAgents.my-agent.models]
default = { id = "default", label = "Default", arg = "" }
```

### Version Cache Schema (`~/.gwt/cache/agent-versions.json`)

```json
{
  "claude": {
    "versions": ["1.0.54", "1.0.53", ...],
    "fetched_at": "2026-01-01T00:00:00Z"
  },
  "codex": { ... }
}
```

- TTL: 24 hours from `fetched_at`
- Max 10 versions per agent

## Success Criteria

- **SC-001**: All known agents (Claude Code, Codex, Gemini, OpenCode, Copilot) are correctly detected when installed.
- **SC-002**: Launch wizard completes without errors for all agent types.
- **SC-003**: Custom agent CRUD works end-to-end via Settings UI.
- **SC-004**: Version cache fetches, stores, and serves cached versions correctly.
- **SC-005**: Version cache gracefully degrades when network is unavailable.
- **SC-006**: Quick Start history correctly records and retrieves per-branch configurations.
- **SC-007**: Session conversion preserves repository context, updates agent identity safely, and handles errors gracefully.
- **SC-008**: Version selection remains separated from model selection and the
  launch summary shows the effective version that will be used.
- **SC-009**: New-branch launches from Branches, SPEC detail, and Issue
  detail start inside a materialized sibling worktree whose path mirrors the
  branch hierarchy instead of falling back to the repository root checkout.
- **SC-010**: Claude Code launches with an effort-capable model route
  through `ReasoningLevel` with the five-row `Auto / Low / Medium / High
  / Max` contract (four rows on Sonnet 4.6), export the chosen
  `CLAUDE_CODE_EFFORT_LEVEL` (or omit it for `Auto`) using the mapping
  in FR-050, and Quick Start relaunch reproduces the same row selection
  and env-var behavior.
- **SC-011**: Codex `ReasoningLevel` renders the four UI rows specified
  in US-11 with stable `(default)` and `(current)` annotations, and the
  launch builder maps each row to the upstream
  `-c model_reasoning_effort=<value>` token without leaking the
  human-readable label to Codex.
