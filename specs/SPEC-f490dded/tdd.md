# TDD記録: タブ並び替えD&D安定化

**仕様ID**: `SPEC-f490dded`
**更新日**: 2026-02-19
**対象**: `gwt-gui/src/lib/components/MainArea.svelte`

## 変更理由

- Tauri WebView 環境でタブD&Dが no-op になり、順番変更できない事象を修正するため。
- タブバー外を経由した pointer 移動でも並び替え追跡を継続する必要があるため。

## RED

### 追加テスト

- `gwt-gui/src/lib/components/MainArea.test.ts`
- テスト名: `emits onTabReorder when pointermove is dispatched on window`

### 期待

- `pointerdown` 後、`window` に `pointermove` を dispatch したとき `onTabReorder` が1回呼ばれること。

### 実行コマンド

```bash
pnpm --dir gwt-gui exec vitest run src/lib/components/MainArea.test.ts -t "pointermove is dispatched on window"
```

### RED結果

- 失敗: `expected "vi.fn()" to be called 1 times, but got 0 times`
- 原因: ドラッグ追跡が `.tab-bar` 要素の pointer イベント受信に依存していた。

## GREEN

### 実装

- ドラッグ開始時に `window` の `pointermove / pointerup / pointercancel` を登録。
- ドラッグ終了時にリスナーを確実に解除。
- close ボタン押下時のドラッグ開始を抑止。
- タブ要素の `draggable` を `false` に変更し、pointer ベース経路を主動線に統一。

### 実行コマンド

```bash
pnpm --dir gwt-gui exec vitest run src/lib/components/MainArea.test.ts src/lib/appTabs.test.ts
```

### GREEN結果

- `MainArea.test.ts`: 7 tests passed
- `appTabs.test.ts`: 5 tests passed

## REFACTOR

- pointer D&D のイベントライフサイクルを `MainArea.svelte` 内で明示的に管理し、状態リセット時に必ず `window` リスナー解除を行う構成へ整理。

---

# TDD記録: エージェントタブの wheel フォールバック

**仕様ID**: `SPEC-f490dded`
**更新日**: 2026-02-19
**対象**: `gwt-gui/src/lib/terminal/TerminalView.svelte`

## 変更理由

- エージェントタブで出力行数が増えると、`deltaY` のみに依存した処理や trackpad 判定により wheel スクロールが再発し、再現時にはスクロールバーのドラッグは動作するがトラックパッド入力だけ失敗する事例が報告されているため。
- `deltaX` のみのイベント（水平成分優位）でも `wheel` を拾ってローカルスクロールできるよう、軸非依存で扱えるようにする必要がある。

## RED

### 追加テスト

- `gwt-gui/src/lib/terminal/TerminalView.test.ts`
- テスト名: `uses dominant axis for mixed wheel input`
- テスト名: `falls back when wheel has horizontal-only delta`

### 期待

- `deltaY` と `deltaX` の両方があるときに主要軸の delta を採用し、`xterm-viewport.scrollTop` が増減すること

### 実行コマンド

```bash
pnpm --dir gwt-gui exec vitest run src/lib/terminal/TerminalView.test.ts -t "uses dominant axis for mixed wheel input" -t "falls back when wheel has horizontal-only delta"
```

### RED結果

- `deltaX` が主要軸となる入力（`deltaX` 優位）を `deltaY` に委ねていた実装では、想定するスクロール方向へ移動しないため再現条件を再現できなかった。

## GREEN

### 実装

- `isTrackpadLikeWheel` を `isScrollableWheelInput` に置き換え、`deltaX`/`deltaY` のいずれかが非零ならフォールバック対象とする
- `pickWheelDelta` を追加し、`deltaY` と `deltaX` のどちらが絶対値で大きいかを採用して `scrollViewportByWheel` に適用する
- 既存実装の mode 変換（`line`/`page`）ロジックを維持し、フォールバック不能時のみイベント伝播を許可
- 早期 return を `event.deltaY===0` から `isScrollableWheelInput` 判定へ置換し、`deltaY=0` でも `deltaX` 主導入力を通す

### 実行コマンド

```bash
pnpm --dir gwt-gui exec vitest run src/lib/terminal/TerminalView.test.ts
```

### GREEN結果

- `TerminalView.test.ts`: 23 tests passed

## REFACTOR

- 既存の trackpad 判定ヒューリスティクスを `deltaX`/`deltaY` 判定へ簡素化し、入力条件が増えて再発条件を吸収しやすい構造へ整理。
