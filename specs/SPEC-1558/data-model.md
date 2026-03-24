### Current Models
- `OpenProjects: List<ProjectInfo>`
- `ActiveProjectIndex: int` (`-1` = none)

### Planned Switch Context
- `ProjectSwitchRequest`
  - `TargetIndex`, `FromIndex`, `TransitionType`
- `ProjectSwitchSnapshot`
  - `ProjectPath`, `DeskStateKey`, `IssueMarkerStateKey`, `AgentStateKey`

### Runtime Invariants
- `OpenProjects[ActiveProjectIndex]` should align with `IProjectLifecycleService.CurrentProject`
- removing the last project sets `ActiveProjectIndex = -1`
- non-active project PTY/session state is preserved outside scene switch logic
- **全PTYプロセスは一時停止なく継続動作する（制限なし）**
