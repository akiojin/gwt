# 実装計画: ペーストが常にエージェントタブに送られるバグの修正

## 概要

ペースト操作が常にエージェントタブに送られる問題を修正する。

## 根本原因

1. DOMフォーカスの残留: 非アクティブタブの xterm hidden textarea がフォーカスを保持
2. active ガードの欠如: handlePaste と isPasteShortcut が active をチェックしない
3. 編集可能要素の優先度不足: emitTerminalEditAction がサイドバー入力を考慮しない

## 修正方針

### Fix 1: 非アクティブターミナルの blur

TerminalView.svelte に `$effect` を追加し、`active` が `false` になった時に `terminal.blur()` を呼ぶ。

### Fix 2: handlePaste に active ガード

ClipboardEvent の paste ハンドラに `if (!active) return;` を追加。

### Fix 3: xterm custom key handler に active ガード

isPasteShortcut 判定に `if (!active) return true;` を追加。

### Fix 4: emitTerminalEditAction で編集可能要素を優先

編集可能要素が `[data-pane-id]` コンテナ外にある場合、fallbackMenuEditAction にルーティング。

## 修正対象ファイル

| ファイル | 変更 |
|---------|------|
| `gwt-gui/src/lib/terminal/TerminalView.svelte` | Fix 1, 2, 3 |
| `gwt-gui/src/App.svelte` | Fix 4 |
