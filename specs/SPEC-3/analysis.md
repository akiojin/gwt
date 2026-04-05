# Analysis: SPEC-3 - Agent management — detection, launch wizard, custom agents, version cache

## Analysis Report: SPEC-3

Status: CLEAR

## Blocking Items
- None.

## Checks
- Clarification completeness: no `[NEEDS CLARIFICATION]` markers remain in `spec.md`.
- Artifact completeness: `spec.md`, `plan.md`, `tasks.md`, supporting docs, `checklists/*`, `progress.md`, and `analysis.md` are present.
- Task traceability snapshot: `tasks.md` now records `166/166` completed items.
- Notes: Session-conversion wording now matches the implemented
  metadata-driven agent switch and its focused tests.
- Notes: Version selection and launch materialization semantics are now
  aligned across `spec.md`, `plan.md`, `tasks.md`, and focused tests.
- Notes: The wizard now restores the branch-first launch flow, old branch
  type / execution mode labels, and the old-TUI-aligned step machine.
- Notes: The wizard now restores old-TUI option-list formatting for
  `ModelSelect`, `ReasoningLevel`, `ExecutionMode`, `SkipPermissions`, and
  `VersionSelect`.
- Notes: Existing-branch launches now restore Quick Start history from
  persisted sessions with the old-TUI grouped layout and `Resume` / `Start
  new` semantics.
- Notes: Popup chrome now renders the current step title in the border with a
  right-aligned `[ESC]` hint, and `AgentSelect` restores old-TUI
  existing-branch context plus name-only rows.
- Notes: `BranchNameInput` and `IssueSelect` now render as inline old-TUI
  prompts so the popup chrome remains the only boxed title surface.
- Notes: Generic option lists, `VersionSelect`, and AI suggestion
  loading/error now reuse the popup chrome as the only boxed surface, which
  removes the remaining double-border wizard content.
- Notes: The AI suggestion candidate list now keeps `Context: ...` visible,
  so loading, error, and suggestion-list states share the same popup-content
  contract.
- Notes: AI suggestion loading and error states now render `Context: ...` as
  a standalone line above the body copy, matching the suggestion-list layout.
- Notes: AI suggestion loading and error body copy is now compact and leaves
  manual-input guidance to the footer hint row instead of duplicating it in
  popup content.
- Notes: Wizard list-based steps now share a wizard-local cyan selected-row
  highlight, which removes the remaining style mismatch between generic
  option lists and specialized steps like `QuickStart` / `AgentSelect`.
- Notes: `BranchNameInput` and `IssueSelect` now render as compact two-row
  input steps, so prompt and value each occupy their own row while the popup
  chrome remains the wizard's only boxed surface.
- Notes: `QuickStart` now starts its grouped history immediately below the
  compact branch-name context line, restoring the denser old-TUI popup
  layout without changing grouped actions.
- Notes: `QuickStart` agent groups no longer insert blank spacer rows between
  groups, so grouped headers render back-to-back while the final action stays
  in the same denser rhythm.
- Notes: The final `Choose different` action now follows the last
  grouped `Start new` row directly without an extra separator line.
- Notes: `QuickStart` action rows now use the shorter old-TUI labels
  `Resume` and `Start new` while preserving resume-session ID snippets.
- Notes: The final Quick Start action label now matches the old-TUI copy
  `Choose different` without an ellipsis.
- Notes: Single-entry Quick Start now promotes its agent/model summary into
  the popup title and omits the duplicated grouped header row from the body,
  while multi-entry grouped history keeps the generic `Quick Start` title.
- Notes: Multi-entry Quick Start now inlines the agent label into each action
  row, which keeps the grouped list denser without affecting the single-entry
  title summary contract.
- Notes: Multi-entry Quick Start now reserves the short resume-session ID
  hint for the selected `Resume` row, leaving unselected rows on the plain
  label to reduce visual noise.
- Notes: Multi-entry Quick Start grouped action rows now use the denser
  old-TUI copy `Resume` / `Start new`.
- Notes: The final Quick Start action now stays label-only on both wide and
  narrow popups, which removes the rebuilt inline description text from the
  footer row.
- Notes: Quick Start state-derived option labels now mirror the rendered
  grouped rows, eliminating the previous mismatch where single-entry or
  multi-entry `current_options()` drifted from the rendered compact copy.
- Notes: The wizard popup now uses the border title as its only step-context
  chrome, removing the separate `Step N/M` row and reclaiming that line for
  content.
- Notes: `QuickStart` now renders the branch name as a compact context line
  without the rebuilt `Branch: ...` prefix, which restores the denser old-TUI
  copy while keeping grouped ordering unchanged.
- Notes: Single-entry `QuickStart` title promotion now keeps only `Agent
  (Model)` and no longer repeats the rebuilt reasoning copy, which tightens
  the popup chrome without changing body behavior.
- Notes: Multi-entry `QuickStart` now inlines the agent label into each
  action row, which removes the final standalone grouped-header chrome while
  preserving compact `Resume` / `Start new` copy and the selected-row
  resume-session hint.
- Notes: Single-entry `QuickStart` title promotion now falls back to the bare
  agent label when no model was persisted, so the popup no longer invents a
  `default` model placeholder.
- Notes: The final Quick Start footer label is now compacted from `Choose
  different settings` to `Choose different`, while keeping render and
  `current_options()` aligned.
- Notes: Multi-entry Quick Start now keeps the agent label only on the
  `Resume` row, while the paired `Start new` row falls back to the compact
  plain label in both render and `current_options()`.
- Notes: Multi-entry Quick Start now renders the plain `Start new` rows with
  neutral styling, leaving the inline-labeled `Resume` rows as the only
  agent-colored identity rows in each entry block.
- Notes: Multi-entry Quick Start now indents the plain `Start new` rows
  beneath the paired `Resume` row, restoring the old-TUI primary/secondary
  action hierarchy without bringing back standalone headers.
- Notes: Existing-branch `AgentSelect` now uses the same compact branch-name
  context line as `QuickStart`, which removes the rebuilt `Branch: ...`
  prefix and the extra spacer row before the agent list.
- Notes: Valid `[tools.customCodingAgents.*]` entries in
  `~/.gwt/config.toml` now load into the wizard and launch path, so config-
  backed custom agents can be selected and started with their configured
  command/path/bunx runner plus mode args and env vars.
- Notes: US-4 remains `PARTIALLY IMPLEMENTED` because the Settings-side CRUD
  UI still needs to move beyond the current placeholder category fields.

## Next
- `gwt-spec-implement`
- This report is a readiness gate, not a completion certificate.
