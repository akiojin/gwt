# 実装計画: タブ有効化時チラつきとタブ切替カクつきの改善

## 目的
- ターミナルタブ有効化時の描画チラつきを抑止する。
- タブ切替時の全体カクつきを低減する。

## 方針
1. `TerminalView` に ready 通知を追加し、有効化時は fit/resize 完了後に通知する。
2. `MainArea` でターミナル表示を二段階化し、ready 後に `.terminal-wrapper.active` を付与する。
3. 非ターミナルタブを Keep-Alive 化して再マウントを削減する。
4. `resize_terminal` の重複通知を抑止する。
5. E2E で Terminal↔Terminal の切替時間を測定し、退行を検知する。

## 変更対象
- `gwt-gui/src/lib/terminal/TerminalView.svelte`
- `gwt-gui/src/lib/components/MainArea.svelte`
- `gwt-gui/src/lib/terminal/TerminalView.test.ts`
- `gwt-gui/src/lib/components/MainArea.test.ts`
- `gwt-gui/e2e/tab-switch-performance.spec.ts`

## テスト戦略
1. ユニットテスト
- ready 通知タイミング（fit+resize 後）
- inactive 時の observer fit 抑制
- resize 重複通知の抑制
- MainArea の ready 待ち表示

1. E2E
- ターミナルを2枚生成し、連続切替の平均/p95/max を採取
- 予算超過時に失敗させる（緩めの退行検知）
