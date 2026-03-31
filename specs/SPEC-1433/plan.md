1. `SettingsPanel.test.ts` に回帰テスト追加（SVGアイコン存在、underscore値保持）
2. 追加テストが RED であることを確認
3. `SettingsPanel.svelte` の API Key ボタンを疑似要素依存から SVG 実装へ変更
4. API Key 入力欄の文字描画（line-height）を調整し `_` の視認性を改善
5. テストを GREEN 化し、`svelte-check` を実行
