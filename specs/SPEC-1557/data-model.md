### Current Models
- `ProjectInfo`
  - `Name`, `Path`, `LastOpenedAt`, `DefaultBranch`, `HasGwt`
- `RecentProjectsWrapper`
  - `Projects: List<ProjectInfo>`

### Planned Extension Models
- `ProjectOpenResult`
  - `ProjectInfo`, `WorktreesCount`, `BranchesCount`, `IssuesCount`
- `MigrationJob`
  - `Id`, `Status`, `Progress`, `SourcePath`, `TargetPath`, `Error`
- `QuitState`
  - `PendingSessions`, `UnsavedChanges`, `CanQuit`

### Persistence
- `~/.gwt/recent-projects.json`
  - max 20 entries
  - latest-opened order
  - dedupe by path
