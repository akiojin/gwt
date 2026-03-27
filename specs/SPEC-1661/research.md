### KeyboardEvent.isComposing

- W3C UIEvents 仕様で標準化されたプロパティ
- `compositionstart` と `compositionend` のイベント順序に依存せず、keydown イベント発火時点で IME 変換中かどうかを判定できる
- WebKit / Chromium / Gecko すべてでサポート済み

### keyCode === 229

- IME 処理中の keydown では `keyCode` が `229` になる（Process key）
- レガシー API だが、IME 判定のフォールバックとして広く使われている
- `isComposing` が未サポートの古いブラウザ向けセーフティネット
