### Runtime Models
- `StudioLevel`
  - `Level: int`
  - `Experience: int`
  - `ExperienceToNextLevel: int`
- `Badge`
  - `Id: string`
  - `Name: string`
  - `Description: string`
  - `Unlocked: bool`
- `GamificationSnapshot`
  - `Level: StudioLevel`
  - `Badges: List`
  - `LastUpdatedAt: string`
- `AgentLimitTable`
  - `Lv1: 3`
  - `Lv5: 5`
  - `Lv10: 10`
  - `Lv20: unlimited (-1)`

### Badge Definitions
- `first_experience`
  - 初回 XP 取得
- `level_2`
  - スタジオレベル 2 到達
- `level_5`
  - スタジオレベル 5 到達
- `ci_streak_5`
  - 連続 CI green 5 回

### Level Curve
- Lv2 = 10コミット相当の経験値
- 上位レベルは指数的に増加
- 具体的な経験値テーブルはテストプレイで調整

### Service Contract
- `IGamificationService`
  - `CurrentLevel` は現在のスタジオ成長状態
  - `GetBadges()` は表示用スナップショットを返す
  - `AddExperience(amount)` は experience 加算と level/badge 更新を行う
  - `CheckBadge(id)` は badge の unlock 状態を返す
  - `GetMaxAgents()` は現在レベルに基づくAgent同時起動上限を返す

### Persistence
- ゲーミフィケーション状態は `~/.gwt/` 配下の JSON に保存する。
- 保存対象は `StudioLevel`, `Badge[]`, `LastUpdatedAt`。
- `#1542` の persistence レイヤーに依存し、 service 自体は DTO を組み立てる責務に留める。
