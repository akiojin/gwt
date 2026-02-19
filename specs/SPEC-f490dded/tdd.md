# TDD記録: タブ並び替えD&D安定化

**仕様ID**: `SPEC-f490dded`
**更新日**: 2026-02-14
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
