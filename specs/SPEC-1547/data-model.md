### Entity Types

| Name | Kind | Fields | Notes |
|---|---|---|---|
| `DeskSlot` | class | `GridPosition`, `AssignedBranch`, `AssignedAgentId`, `AssignedAgentIds`, `IsRemote` | デスクの論理モデル |
| `DeskState` | enum | `Staffed`, `Empty`, `Remote` | デスクの表示・操作条件 |
| `ContextMenuItemType` | enum | `Terminal`, `Summary`, `Git`, `PR`, `FireAgent`, `HireAgent`, `DeleteWorktree` | デスクメニュー種別 |
| `ContextMenuItem` | class | `Type`, `IsEnabled` | 表示メニュー 1 項目 |
| `LeadCandidate` | class | `Id`, `DisplayName`, `Personality`, `Description`, `SpriteKey`, `VoiceKey` | Lead キャラクター候補 |
| `DetectedAgentType` | enum | `Claude`, `Codex`, `Gemini`, `OpenCode`, `GithubCopilot`, `Custom` | Developer 外見ラベルの元データ |
| `FurnitureType` | enum | `CoffeeMachine`, `Bookshelf`, `Whiteboard` | stopped 状態の行き先 |
| `CharacterState` | enum | `Idle`, `Walking`, `Working`, `WaitingInput`, `Stopped`, `Entering`, `Leaving` | キャラアニメーション状態 |

### Interaction Rules

- `Developer` クリックでライブターミナルを開く
- `DeskSlot.GetState()` により
  - `Staffed` → staffed desk context menu
  - `Empty` → empty desk context menu
  - `Remote` → worktree 作成確認フロー
- `ContextMenuBuilder`
  - staffed desk: `Terminal / Summary / Git / PR / FireAgent`
  - empty desk: `HireAgent / Terminal / Git / DeleteWorktree`

### Naming / Labels

- `RandomNameGenerator.Generate()` は Developer 表示名を生成
- `RandomNameGenerator.GetAgentTypeLabel()` は `DetectedAgentType` を UI ラベルへ変換する
