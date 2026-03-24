- Voice Input の設定 shape を frontend / tauri / core で縮退し、hotkey 系フィールドを削除する
- `VoiceInputController` を固定 PTT 専用に整理し、キーボードとオーバーレイボタンの押下元を共通化する
- `TerminalView` の Voice ボタンを press-and-hold 動作へ変更し、初回 bootstrap 導線を塞いでいた disabled 条件を修正する
- Settings / README / E2E を新しい固定 PTT 仕様へ更新する

---
