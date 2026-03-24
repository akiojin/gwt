### Background

AssistantPanel の textarea で日本語 IME 入力時、変換確定の Enter キーでメッセージが送信されてしまう。
現在の実装（`AssistantPanel.svelte` L76-81, L167-168）では `isComposing` フラグと `compositionstart`/`compositionend` イベントで IME 状態を管理しているが、OS の WebView エンジンごとにイベント発火順序が異なるため、特定プラットフォームでガードが効かない。

### Root Cause

Tauri v2 は OS ごとに異なる WebView エンジンを使用する:

| OS | WebView エンジン | IME イベント順序 | 問題 |
|---|---|---|---|
| macOS | WKWebView (WebKit) | `keydown` → `compositionend` の場合あり | `keydown` 時点で `isComposing=true` のためガード成功するケースが多いが、タイミング依存 |
| Windows | WebView2 (Chromium/Edge) | `compositionend` → `keydown` | `compositionend` で `isComposing=false` になった後に `keydown` が発火し、ガードが効かない |
| Linux | WebKitGTK | WebKit ベースだがエンジン差あり | macOS と同様の問題が発生する可能性 |

### Current Implementation

```
gwt-gui/src/lib/components/AssistantPanel.svelte
```

- L10-11: `isComposing` state 変数
- L57: `sendMessage()` 内の `if (isComposing) return` ガード
- L76-81: `handleKeydown` で `!isComposing` チェック
- L167-168: `compositionstart`/`compositionend` イベントハンドラ

### User Scenarios

**シナリオ 1: 日本語 IME で文章入力**
1. AssistantPanel の textarea にフォーカス
2. 日本語 IME で「こんにちは」と入力
3. 変換候補から確定して Enter を押す
4. **期待**: テキストが確定されるだけでメッセージは送信されない
5. **現状 (Windows)**: 変換確定と同時にメッセージが送信される

**シナリオ 2: IME 確定後に通常 Enter で送信**
1. IME で文章を確定した後
2. 追加テキストを入力するか、そのまま Enter を押す
3. **期待**: Enter でメッセージが送信される

**シナリオ 3: Shift+Enter で改行**
1. IME が非アクティブ状態で Shift+Enter を押す
2. **期待**: 改行が挿入される（送信されない）

### Requirements

- **FR-1**: 全プラットフォーム（macOS / Windows / Linux）で、IME 変換確定の Enter がメッセージ送信をトリガーしないこと
- **FR-2**: IME 非使用時の Enter キーによる送信動作は現状維持
- **FR-3**: Shift+Enter による改行動作は現状維持
- **FR-4**: `KeyboardEvent.isComposing` プロパティを `compositionstart`/`compositionend` の手動フラグより優先して使用すること（ブラウザ標準 API でイベント順序に依存しない）
- **FR-5**: `keyCode === 229`（IME 処理中の keydown）のフォールバックチェックを含めること

### Proposed Fix Direction

1. `handleKeydown` で `e.isComposing` (KeyboardEvent 標準プロパティ) を優先チェック
2. フォールバックとして `e.keyCode === 229` をチェック（IME 処理中は keyCode が 229 になる）
3. 既存の `isComposing` state 変数はバックアップガードとして残す

### Success Criteria

- macOS (WKWebView): IME 確定 Enter で送信されない
- Windows (WebView2): IME 確定 Enter で送信されない
- Linux (WebKitGTK): IME 確定 Enter で送信されない
- IME 非使用時: Enter で正常に送信される
- Shift+Enter: 改行が挿入される

### Related

- #1636 (Assistant Mode SPEC) — AssistantPanel を含む上位仕様
