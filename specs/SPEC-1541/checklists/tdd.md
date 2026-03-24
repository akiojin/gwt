### EditMode tests
- `TerminalRichTextBuilder`
  - plain text はタグなしで表示
  - ANSI color が `<color>` に変換される
  - bold / italic / underline を正しくタグ化する
  - 特殊文字 `< > &` を escape する
  - 同色連続セルを 1 span にまとめる
- `XtermSharpTerminalAdapter`
  - main thread `Feed` が即時 buffer に反映される
  - background thread `Feed` が queue 経由で `ProcessPendingData` 後に反映される
  - `Resize` が rows/cols を更新する
  - `BufferChanged` が発火する
  - pending input なしの `ProcessPendingData` は false
- `TerminalPaneManager`
  - add/remove で active pane と active index が正しく更新される
  - next/prev tab が循環する
  - `GetPaneByAgentSessionId` が動作する

### PlayMode / integration tests
- PTY 出力が terminal buffer まで到達する
- 複数 terminal pane が独立して描画される
- `TerminalInputField` の submit が `IPtyService.WriteAsync` に流れる
- overlay open 時に初回 shell が生成される
- active pane 切替で renderer/input binding が更新される

### Pending RED tests
- URL 検出とクリック遷移
- マウスドラッグ選択 + clipboard copy
- **仮想スクロールで可視行+上下50行マージンのみ描画（固定数TMPオブジェクトプール方式）**
- 代替画面バッファ
- 全ターミナルへのキーブロードキャスト
