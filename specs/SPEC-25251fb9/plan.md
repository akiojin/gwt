# 実装計画: SPEC-25251fb9

## 変更対象ファイル

1. `gwt-gui/src/lib/terminal/TerminalView.svelte` - ホイールスクロールロジックの改修
2. `gwt-gui/src/lib/terminal/TerminalView.test.ts` - テストの更新・追加

## 実装ステップ

### Step 1: テスト更新（TDD - RED）

Terminal モックに `scrollLines` メソッドと `buffer.active` プロパティを追加し、新しい挙動に合わせたテストを作成する。

### Step 2: TerminalView.svelte 改修

1. `isTrackpadLikeWheel` 関数と関連定数・型を削除
2. `pickWheelDelta` を `pickWheelLines` に置換（ピクセル→ライン変換＋リメインダー蓄積）
3. `scrollViewportByWheel` を `terminal.scrollLines()` API ベースに変更
4. `handleWheel` を簡素化:
   - alternate buffer → xterm.js に委譲（早期リターン）
   - 通常バッファ → 常にカスタムハンドリング
5. `WheelScrollState` の `remainder` をライン単位の蓄積に変更

### Step 3: テスト実行（GREEN）

全テストが通過することを確認する。
