# Tasks: divergence検知後のEnter待ち修正

**仕様ID**: `SPEC-40c7b4f1`

## Phase 1: TDD - Test First

- [x] T001 テスト追加: TTY/非TTY の waitForEnter 振る舞いを検証するユニットテストを作成

## Phase 2: Implementation

- [x] T002 実装: waitForEnter を共通モジュール化し、getTerminalStreams を使用するようリファクタ
- [x] T003 実装: EOF/SIGINT 時のクリーンアップと raw モード解除を追加

## Phase 3: Verification

- [x] T004 検証: bun run test を実行し回帰を確認

## Phase 4: Documentation

- [x] T005 ドキュメント: SPEC/plan/tasks への反映確認
