- 既存実装では `voiceInputController.ts` が設定値ベースで hotkey を都度解釈していた
- `TerminalView.svelte` は `voiceInputAvailable === false` で Voice ボタンを disabled にしており、runtime 未準備時の bootstrap 導線を自分で塞いでいた
- `crates/gwt-tauri/src/commands/settings.rs` と `crates/gwt-core/src/config/settings.rs` が hotkey フィールドを保持していたため、UI だけ隠す修正では不十分だった

---
