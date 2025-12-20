# タスク: divergence検知後のEnter待ち修正

- [x] T001 テスト追加: TTY/非TTY の waitForEnter 振る舞いを検証するユニットテストを作成
- [x] T002 実装: waitForEnter を共通モジュール化し、getTerminalStreams を使用するようリファクタ
- [x] T003 実装: EOF/SIGINT 時のクリーンアップと raw モード解除を追加
- [x] T004 検証: bun run test を実行し回帰を確認
- [x] T005 ドキュメント: SPEC/plan/tasks への反映確認
