# タスクリスト: macOS 断続入力不能の抑止

## Phase 1: MainArea

- [x] T001 `MainArea.svelte` に ready 済み terminal tab 記録を追加する
- [x] T002 ready 済み tab への再切替時は待機なしで即可視化する
- [x] T003 fallback 可視化 tab の ready 昇格と掃除ロジックを追加する

## Phase 2: TerminalView

- [x] T010 `TerminalView.svelte` に pointerdown 起点のフォーカス復帰を追加する
- [x] T011 `TerminalView.svelte` に window focus / visibilitychange 起点のフォーカス復帰を追加する
- [x] T012 有効化直後のフォーカス再試行タイマーを補強する

## Phase 3: テスト

- [x] T020 `MainArea.test.ts` に再切替即時表示の回帰テストを追加する
- [x] T021 `TerminalView.test.ts` に pointerdown / window focus フォーカス復帰テストを追加する
- [x] T022 関連テストを実行し、回帰がないことを確認する
