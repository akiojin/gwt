# 実装計画: macOS 断続入力不能の抑止

**仕様ID**: `SPEC-748cead5` | **日付**: 2026-02-22 | **仕様書**: `specs/SPEC-748cead5/spec.md`

## 目的

- terminal 出力は続くが入力不能になる状態を防ぐ。
- tab 再切替時の表示/入力の安定性を高める。

## 実装方針

### Phase 1: MainArea の可視化状態を安定化

- `terminalReadyTabIds` を導入し、ready 済み tab を記録する。
- active tab 変更時、ready 済み tab は待機を挟まず即可視化する。
- fallback で可視化した tab も ready 済みに昇格する。
- tab 削除時に ready 記録を掃除する。

### Phase 2: TerminalView のフォーカス復帰経路を増強

- `pointerdown` でフォーカス復帰を試行する。
- `window focus` / `visibilitychange(visible)` でフォーカス復帰を試行する。
- 既存の有効化直後リトライを 1 段追加して遅延競合を吸収する。

### Phase 3: TDD / 回帰確認

- `MainArea.test.ts` に「ready 済み tab へ戻ると即表示」テストを追加。
- `TerminalView.test.ts` に pointerdown / window focus のフォーカス復帰テストを追加。
- 既存関連テストをまとめて実行し、回帰がないことを確認する。

## テスト

- `cd gwt-gui && pnpm test src/lib/components/MainArea.test.ts src/lib/terminal/TerminalView.test.ts src/lib/systemMonitor.svelte.test.ts`
