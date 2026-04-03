# Data Model: SPEC-2 - Workspace Shell

## Primary Objects
- **Root model** - `Model` tracks active layer, sessions, management tab, overlays, notifications, and repo path.
- **Management tabs** - `ManagementTab::ALL` currently enumerates 8 tabs, including `PrDashboard` and `Logs`.
- **Branch detail state** - `BranchesState` carries detail sections plus pending action flags such as launch-agent and open-shell.
- **Session persistence** - `SessionState` persists display mode, management visibility, active tab, and session count.
