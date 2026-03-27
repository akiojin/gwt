### EditMode

1. `DeskSlot.GetState()`
   - `AssignedAgentId` あり → `Staffed`
   - `AssignedAgentIds` 非空 → `Staffed`
   - `IsRemote=true` → `Remote`
   - それ以外 → `Empty`
2. `StudioLayout`
   - `MoveDesk` は空き座標にだけ成功する
   - `GetEmptyDesks` / `GetStaffedDesks` が正しい一覧を返す
3. `RandomNameGenerator`
   - 空文字を返さない
   - 複数回呼び出しで複数候補が出る
   - Agent type label が `Claude Code / Codex / Gemini / OpenCode / Copilot / Custom`
4. `ContextMenuBuilder`
   - staffed desk は 5 項目
   - empty desk は 4 項目
   - `Summary` / `PR` の enable 条件が反映される

### PlayMode

1. キャラクター状態遷移
   - `Entering -> Idle -> Working/WaitingInput/Stopped -> Leaving`
2. 空席デスクと着席デスク
   - Fire 後に空席デスクへ遷移する
   - Remote desk クリックで worktree 作成フローへ入る
3. 家具インタラクション
   - stopped 状態で家具に遷移し、該当アニメーションを再生する
