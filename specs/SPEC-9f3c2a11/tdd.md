# TDD: Voice Input Mode（GUI 全入力 + Qwen3-ASR）

**仕様ID**: `SPEC-9f3c2a11`
**作成日**: 2026-02-13
**更新日**: 2026-02-14

## テスト方針

- 受け入れシナリオを Rust（設定/バックエンド）と TypeScript（GUI 制御）へ分割して担保する。
- 破壊的回帰が起きやすい「入力先分岐」「GPU/ランタイムゲート」「ホットキー競合」を優先的に自動化する。
- 単体テストは副作用を最小化するため、I/O と runtime API はモック可能な境界で切る。

## RED -> GREEN 対応ケース

1. 設定のエンジン正規化とホットキー検証
- RED: `voice_input.engine=whisper` が保存で失敗する、または `hotkey` と `ptt_hotkey` の重複が通る。
- GREEN: `whisper -> qwen3-asr` へ正規化され、重複時は `must differ` エラーになる。
- 対象: `crates/gwt-tauri/src/commands/settings.rs`

1. GPU 非可用時の利用拒否
- RED: GPU 無効でも音声認識コマンドが実行される。
- GREEN: capability が unavailable を返し、転写開始が拒否される。
- 対象: `crates/gwt-tauri/src/commands/voice.rs`

1. Qwen ランタイム未準備時の自動セットアップ
- RED: Python はあるが `qwen_asr` 未導入時、開始前に即失敗する。
- GREEN: start 時に `ensure_voice_runtime` が一度実行され、成功時はそのまま録音開始できる。
- 対象: `crates/gwt-tauri/src/commands/voice.rs`, `gwt-gui/src/lib/voice/voiceInputController.ts`

1. 品質プリセットとモデル解決
- RED: `quality` の不正値が通る、または `fast/balanced/accurate` が意図しないモデルへ解決される。
- GREEN: `0.6B/1.7B` のマッピングが固定される。
- 対象: `crates/gwt-tauri/src/commands/voice.rs`

1. 入力先への転写挿入
- RED: 音声結果がフォーカス中 input/textarea に入らない。
- GREEN: 音声停止後にテキストが挿入される。
- 対象: `gwt-gui/src/lib/voice/voiceInputController.test.ts`

1. terminal フォールバック送信
- RED: terminal フォーカス時に転写が送信されない。
- GREEN: `write_terminal` が期待 payload で呼ばれる。
- 対象: `gwt-gui/src/lib/voice/voiceInputController.test.ts`

1. Push-to-talk と無効設定
- RED: PTT が通常トグルと同動作になる、または `enabled=false` でも録音開始する。
- GREEN: PTT フローが成立し、無効設定時は録音開始しない。
- 対象: `gwt-gui/src/lib/voice/voiceInputController.test.ts`

## 実行コマンド

- `cargo test -p gwt-tauri voice -- --nocapture`
- `cargo test -p gwt-tauri settings::tests:: -- --nocapture`
- `cargo test -p gwt-core settings::tests::test_default_settings -- --nocapture`
- `cargo test -p gwt-core settings::tests::test_load_global -- --nocapture`
- `cd gwt-gui && pnpm exec vitest run src/lib/voice/voiceInputController.test.ts`
