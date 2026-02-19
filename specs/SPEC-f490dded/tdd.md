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

### 実行コマンド（RED）

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

### 実行コマンド（GREEN）

```bash
pnpm --dir gwt-gui exec vitest run src/lib/components/MainArea.test.ts src/lib/appTabs.test.ts
```

### GREEN結果

- `MainArea.test.ts`: 7 tests passed
- `appTabs.test.ts`: 5 tests passed

## REFACTOR

- pointer D&D のイベントライフサイクルを `MainArea.svelte` 内で明示的に管理し、状態リセット時に必ず `window` リスナー解除を行う構成へ整理。

---

## TDD記録: エージェントタブの wheel フォールバック

**仕様ID**: `SPEC-f490dded`
**更新日**: 2026-02-19
**対象**: `gwt-gui/src/lib/terminal/TerminalView.svelte`

### 変更理由（エージェント）

- エージェントタブで出力行数が増えると、`deltaY` のみに依存した処理や trackpad 判定により wheel スクロールが再発し、再現時にはスクロールバーのドラッグは動作するがトラックパッド入力だけ失敗する事例が報告されているため。
- `deltaX` のみのイベント（水平成分優位）でも `wheel` を拾ってローカルスクロールできるよう、軸非依存で扱えるようにする必要がある。

### RED（エージェント）

### 追加テスト

- `gwt-gui/src/lib/terminal/TerminalView.test.ts`
- テスト名: `uses dominant axis for mixed wheel input`
- テスト名: `falls back when wheel has horizontal-only delta`
- テスト名: `keeps clear mouse-like repeated integer wheel events for terminal consumption`
- テスト名: `accumulates fractional wheel input to preserve sub-pixel scroll`

### 期待

- `deltaY` と `deltaX` の両方があるときに主要軸の delta を採用し、`xterm-viewport.scrollTop` が増減すること
- 連続した 120/240 系の固定ステップ入力で、時間差が十分にあれば明確なマウス入力として判定し、フォールバックを避けてターミナルの既存入力経路へ委譲されること
- 小数点デルタでも、整数化しやすい環境で移動量が蓄積されること

### 実行コマンド（AGENT RED）

```bash
pnpm --dir gwt-gui exec vitest run src/lib/terminal/TerminalView.test.ts -t "uses dominant axis for mixed wheel input" -t "falls back when wheel has horizontal-only delta" -t "keeps clear mouse-like repeated integer wheel events for terminal consumption" -t "accumulates fractional wheel input to preserve sub-pixel scroll"
```

### RED結果

- `deltaX` が主要軸となる入力（`deltaX` 優位）を `deltaY` に委ねていた実装では、想定するスクロール方向へ移動しないため再現条件を再現できなかった。
- 100/120/240 周辺の固定ステップをすべて trackpad 扱いし続けると、focused 時に `preventDefault` されやすく、純粋なマウス入力を奪う再現条件が残っていた。

### GREEN（エージェント）

### 実装

- `isTrackpadLikeWheel` を履歴付きロジック付きで再導入し、`mouseWheelStepHistory` で同一固定ステップ連打を明確なマウス入力として判定できる場合のみ除外する
- `pickWheelAxis` と `pickWheelDelta` を導入し、軸方向を明示しつつ `WheelScrollState` でサブピクセル残差を保持してスクロール距離へ反映する
- `scrollViewportByWheel` と `handleWheel` を状態を注入する形に更新し、判定・加算・反映の経路を一貫化

### 実行コマンド（AGENT GREEN）

```bash
pnpm --dir gwt-gui exec vitest run src/lib/terminal/TerminalView.test.ts
```

### GREEN結果

- `TerminalView.test.ts`: 25 tests passed

### REFACTOR（エージェント）

- 固定ステップマウス入力と小数点 trackpad 入力を別軸で扱えるよう、判定とスクロール反映の責務を分けて再発時の原因追跡性を改善。
