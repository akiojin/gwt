### User Scenario

- ユーザーが Launch Agent ダイアログを開き、エージェントを切り替えても "Not authenticated" 警告が表示されない
- エージェントの `available` / `unavailable` 状態表示は引き続き表示される

### Functional Requirements

- FR-001: エージェント選択ドロップダウンから "Not authenticated" 警告を削除する
- FR-002: `Unavailable` 警告は維持する（エージェント未インストール時）
- FR-003: GitHub CLI の認証状態表示（ブランチ/Issue 用）は維持する

### Success Criteria

- Launch Agent ダイアログでエージェント選択時に "Not authenticated" が表示されない
- `Unavailable` 表示は変わらず動作する
- 既存テストが更新され全パスする
