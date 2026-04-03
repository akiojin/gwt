# Data Model: SPEC-4 - GitHub Integration

## Primary Objects
- **Issues screen** - `IssuesState` and issue list items hold local selection, detail, and search state.
- **Git View** - `GitViewState` holds file entries, commit entries, and expanded diff rows.
- **PR Dashboard** - `PrDashboardState` tracks PR list selection plus detail view state.
- **GitHub status** - `PrStatus` and `gh`-derived payloads form the integration boundary for CI and review data.
