### EditMode

1. `StudioLayout` 初期値
   - `Width=16`, `Height=12`, `DoorPosition=(Width/2, 0)` を満たす
2. デスク追加・削除・検索
   - `AddDesk` は重複座標を拒否する
   - `FindDeskByBranch` / `FindDeskByAgent` が正しく検索できる
3. 動的拡張・縮小
   - 収容数超過で `ExpandIfNeeded()` が `true`
   - **デスク削除後 `ShrinkIfNeeded()` は全デスク空席の行のみ縮小する**
   - `ShrinkIfNeeded()` が `MinHeight` 未満にしない
4. 下方向拡張
   - `StudioLayout.Expansion == Down`
   - 拡張後も既存デスク座標と `DoorPosition.x` が壊れない

### PlayMode

1. プロジェクト初期表示
   - スタジオロード後に `Lead + main + develop` の基本構成が生成される
2. world 更新
   - worktree 追加イベントで新しいデスクが出現する
   - CI 状態変更で `AtmosphereState` に応じた見た目へ切り替わる
3. カメラ
   - パン操作でスタジオ外へ無制限に逸脱しない
4. **Issue マーカー 100+ 全件表示**
   - 100 件以上の Issue マーカーが全件表示される
   - 重なり回避アルゴリズムにより視認性が保たれる
