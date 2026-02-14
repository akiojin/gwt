# 実装タスク: Voice Input Mode（GUI 全入力 + Qwen3-ASR）

## フェーズ1: 仕様・設定更新

- [x] T001 `specs/SPEC-9f3c2a11/spec.md` を Qwen3-ASR 方針へ更新
- [x] T002 `specs/SPEC-9f3c2a11/plan.md` と `specs/SPEC-9f3c2a11/tdd.md` を更新
- [x] T003 `crates/gwt-core/src/config/settings.rs` の voice default を `qwen3-asr` へ更新
- [x] T004 `crates/gwt-tauri/src/commands/settings.rs` で `engine=whisper` を `qwen3-asr` へ正規化

## フェーズ2: 音声バックエンド（Qwen3-ASR）

- [x] T101 `crates/gwt-tauri/src/python/qwen3_asr_runner.py` を追加（probe/prepare/transcribe）
- [x] T102 `crates/gwt-tauri/src/commands/voice.rs` を Qwen 実行方式へ更新
- [x] T103 `get_voice_capability` で GPU + Python/qwen ランタイムの可用性判定を実装
- [x] T104 `prepare_voice_model` で品質別モデル（0.6B/1.7B）の事前準備を実装
- [x] T105 `transcribe_voice_audio` で録音PCMを一時WAV化して Qwen 推論へ接続
- [x] T106 `ensure_voice_runtime` を追加し、`~/.gwt/runtime/voice-venv` へ依存自動セットアップを実装
- [x] T107 runtime probe キャッシュを再試行可能な方式へ修正

## フェーズ3: GUI 設定・表示の同期

- [x] T201 `gwt-gui/src/lib/types.ts` の voice engine 型を Qwen 前提へ更新
- [x] T202 `gwt-gui/src/App.svelte` の voice default/normalize を Qwen 前提へ更新
- [x] T203 `gwt-gui/src/lib/components/SettingsPanel.svelte` の quality/model 同期を Qwen マップへ更新
- [x] T204 `gwt-gui/src/lib/components/StatusBar.svelte` の unavailable 表示を理由ベースへ改善

## フェーズ4: コントローラとテスト

- [x] T301 `gwt-gui/src/lib/voice/voiceInputController.ts` の言語正規化とエラーメッセージを Qwen 方針へ更新
- [x] T302 `crates/gwt-tauri/src/commands/settings.rs` テストを Qwen 正規化仕様へ更新
- [x] T303 `crates/gwt-core/src/config/settings.rs` テスト期待値を Qwen デフォルトへ更新
- [x] T304 `crates/gwt-tauri/src/commands/voice.rs` のユニットテストを Qwen マッピング/ゲートへ更新
- [x] T305 `gwt-gui/src/lib/voice/voiceInputController.test.ts` を Qwen 設定値で更新
- [x] T306 ランタイム未準備時に `ensure_voice_runtime` を自動試行するフローを実装・テスト追加

## フェーズ5: ドキュメント

- [x] T401 README（日/英）へ「Pythonのみ必須・`qwen_asr` 自動導入」を追記

## フェーズ6: 検証

- [x] T501 `cargo test -p gwt-tauri voice -- --nocapture`
- [x] T502 `cargo test -p gwt-tauri settings::tests:: -- --nocapture`
- [x] T503 `cargo test -p gwt-core settings::tests::test_default_settings -- --nocapture`
- [x] T504 `cargo test -p gwt-core settings::tests::test_load_global -- --nocapture`
- [x] T505 `cd gwt-gui && pnpm exec vitest run src/lib/voice/voiceInputController.test.ts`
