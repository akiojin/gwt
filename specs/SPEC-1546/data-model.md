### Runtime State

| Name | Kind | Fields | Notes |
|---|---|---|---|
| `StudioLayout` | class | `Width`, `Height`, `Desks`, `DoorPosition`, `Expansion` | スタジオ全体の寸法とデスク配置。下方向拡張を前提にする |
| `DeskSlot` | class | `GridPosition`, `AssignedBranch`, `AssignedAgentId`, `AssignedAgentIds`, `IsRemote` | デスク 1 台分。ローカル/空席/半透明デスクの基礎単位 |
| `DeskState` | enum | `Staffed`, `Empty`, `Remote` | デスクの見た目・操作を決定する状態 |
| `AtmosphereState` | enum | `Normal`, `CISuccess`, `CIFail` | スタジオ全体の雰囲気ライティング制御 |
| `ProjectInfo` | class | `Name`, `Path`, `UnityVersion`, `MigrationState` | 現在表示中スタジオのプロジェクト情報 |
| `Worktree` | class | `Path`, `Branch`, `Status`, `IsMain`, `HasChanges`, `HasUnpushed` | デスク生成元の Git 状態 |

### World Mapping

- `main` / `develop` は通常 `DeskSlot` として配置する
- `Issue` に紐づく worktree は、Issue マーカー位置を優先して `DeskSlot.GridPosition` を決定する
- リモートブランチのみの対象は `DeskSlot.IsRemote = true`
- `StudioLayout.DoorPosition` はキャラクター入退場とカメラ初期注視位置の基準に使う

### Service Boundary

- `IWorldService`: `ProjectInfo` / `Worktree` / `StudioLayout` を入力としてシーン上オブジェクトを構築・更新する
- `IProjectLifecycleService`: スタジオのロード対象となるプロジェクト切替を管理する
- `IGitService` / `IGitHubService`: デスク・Issue マーカー配置のデータソース
