# タスク: AIツール起動機能（Codex gpt-5.2対応）

- [x] **T3001** 仕様書にgpt-5.2とExtra high推論レベルを追加する（受け入れシナリオ、FR追記）
- [x] **T3002** Codexモデル選択肢にgpt-5.2を追加し、推論レベルxhighを選択可能にする実装をTDDで更新する（モデル一覧・推論レベルUI・Quick Start表示のテスト含む）
- [x] **T3003** Codex起動オプションが`--model=gpt-5.2`を渡せること、xhigh指定が`model_reasoning_effort`に反映されることをユニットテストで検証する
- [x] **T3004** Worktree再利用時のブランチ整合性検証とモデル名正規化の要件を仕様に追記する
- [x] **T3005** Worktree整合性チェックとモデル名正規化のテストを追加する（worktreeExists/モデル選択/Quick Start）
- [x] **T3006** Worktree整合性チェック・警告表示とモデル名正規化を実装する
