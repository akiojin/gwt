# Research: SPEC-1654

## Decision Log

### D1: Primary shell entry

**Decision**: `Branches` is the primary entry.

**Why**: This restores the old TUI's orientation and keeps branch state understandable before the user enters a session surface.

### D2: Session model

**Decision**: One permanent multi-session workspace, not tmux and not split hidden-pane modes.

**Why**: `gwt-core` already has a native PTY runtime. The rebuilt shell should reuse it instead of reviving tmux semantics.

### D3: Session layout

**Decision**: `equal grid` by default, `maximize + tab switch` when focusing.

**Why**: Grid supports concurrent monitoring; maximize supports concentrated work without abandoning the multi-session model.

### D4: Management workspace

**Decision**: Keep tabbed management, but limit the initial tabs to `Branches / SPECs / Issues / Profiles`.

**Why**: This keeps information architecture explicit while avoiding the initial sprawl of `Settings / Logs / Versions`.
