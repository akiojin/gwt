# タスク: AIツール終了時の未コミット警告と未プッシュ確認

- [x] T001 テスト追加: `confirmYesNo` のTTY/非TTY/デフォルト挙動を検証（`src/utils/__tests__/prompt.test.ts`）
- [x] T002 実装: `confirmYesNo` を追加（`src/utils/prompt.ts`）
- [x] T003 テスト追加: 終了時の未コミット警告/未プッシュ確認/Push分岐を検証（`tests/unit/index.post-session-checks.test.ts`）
- [x] T004 実装: `handleAIToolWorkflow` に終了時チェックとpush処理を追加し、3秒待機順序を調整（`src/index.ts`）
- [x] T005 調整: 既存の `handleAIToolWorkflow` テストで新プロンプトをモック（`tests/unit/index.*.test.ts`）
- [x] T006 検証: `bun run test` を実行（必要に応じて対象テストに絞る）
- [x] T007 ドキュメント: SPEC/plan/tasks の整合性を確認
