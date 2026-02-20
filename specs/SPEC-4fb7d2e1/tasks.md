# タスクリスト: タブ有効化時チラつきとタブ切替カクつきの改善

## Phase 1: TerminalView
- [x] T001 `onReady` コールバックを追加し、active 遷移時に fit+resize 完了後通知する
- [x] T002 inactive 時の ResizeObserver fit を抑制する
- [x] T003 `resize_terminal` の同一サイズ重複通知を抑制する

## Phase 2: MainArea
- [x] T004 ターミナル表示を ready 待ちの二段階化に変更する
- [x] T005 ready 未達時のフォールバック表示（短タイムアウト）を追加する
- [x] T006 非ターミナルタブを Keep-Alive 化する

## Phase 3: テスト
- [x] T007 `TerminalView.test.ts` に ready/resize最適化テストを追加する
- [x] T008 `MainArea.test.ts` に ready 待ち表示テストを追加する
- [x] T009 Playwright E2E にタブ切替性能測定テストを追加する
