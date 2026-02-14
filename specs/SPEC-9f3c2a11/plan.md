# 実装計画: Voice Input Mode（GUI 全入力 + Qwen3-ASR）

**仕様ID**: `SPEC-9f3c2a11`
**日付**: 2026-02-14

## 実装方針

1. 設定スキーマを Qwen 前提へ更新する
- `Settings` / `SettingsData` の `voice_input.engine` を `qwen3-asr` デフォルトへ更新する。
- 互換性のため `engine=whisper` は保存時に `qwen3-asr` へ正規化する。
- `hotkey` と `ptt_hotkey` の重複禁止、`language`/`quality` の列挙値制限を継続する。

1. 音声認識バックエンドを Qwen3-ASR 実行へ切り替える
- 既存 `voice` コマンド名（`get_voice_capability` / `prepare_voice_model` / `transcribe_voice_audio`）は維持する。
- Rust から Python サブプロセスを起動し、`qwen_asr` で認識する。
- Python ヘルパースクリプトを同梱し、実行時にローカルへ展開して利用する。
- 依存導入は利用者手動前提にせず、`~/.gwt/runtime/voice-venv` を自動作成して `qwen-asr` を自動セットアップする。

1. GPU とランタイム要件で可用性を制御する
- Frontend 側の GPU 判定と、Backend 側の Python/qwen ランタイム検査を組み合わせる。
- いずれか不足時は capability を unavailable とし、録音開始・準備・転写を実行しない。
- runtime 不足が原因の場合、音声開始時に一度だけ `ensure_voice_runtime` を自動実行して再判定する。

1. 品質プリセットとモデル解決
- `fast -> Qwen/Qwen3-ASR-0.6B`
- `balanced -> Qwen/Qwen3-ASR-1.7B`
- `accurate -> Qwen/Qwen3-ASR-1.7B`
- `model` フィールドは品質選択から自動更新し、Backend でも同一マップを適用する。

1. GUI 録音パイプラインは既存を維持
- `voiceInputController` の録音・入力ターゲット挿入ロジックは維持する。
- Backend 呼び出し payload を Qwen 前提（language 正規化と quality）へ合わせる。

1. TDD 方針
- Rust: 設定正規化、品質マッピング、GPU/ランタイムゲート、言語マッピングをテストする。
- TS: 入力欄挿入、terminal 送信、PTT フロー、無効設定時不開始をテストする。

1. 既存評価ハーネスは維持
- `voice_eval` CLI（Whisperベース）は比較基盤として継続利用する。
- Qwen 追加評価は別途比較ドキュメントで管理し、今回の実装ではランタイム切替を優先する。

## 主要変更ファイル

- `crates/gwt-core/src/config/settings.rs`
- `crates/gwt-tauri/src/commands/settings.rs`
- `crates/gwt-tauri/src/commands/voice.rs`
- `crates/gwt-tauri/src/python/qwen3_asr_runner.py`（新規）
- `gwt-gui/src/lib/types.ts`
- `gwt-gui/src/App.svelte`
- `gwt-gui/src/lib/components/SettingsPanel.svelte`
- `gwt-gui/src/lib/components/StatusBar.svelte`
- `gwt-gui/src/lib/voice/voiceInputController.ts`
- `gwt-gui/src/lib/voice/voiceInputController.test.ts`
- `specs/SPEC-9f3c2a11/spec.md`
- `specs/SPEC-9f3c2a11/tasks.md`
- `specs/SPEC-9f3c2a11/tdd.md`

## リスク

- Python ランタイム自体が存在しない環境では自動セットアップに失敗する。
- モデル初回ダウンロードで待ち時間とディスク使用量が大きい。
- フロント側 GPU 判定は環境依存の誤検知リスクがある。

## 緩和策

- capability API で利用不可理由を明示し、Settings/StatusBar に表示する。
- runtime 自動セットアップを導入し、`qwen_asr` 手動導入を不要化する。
- 初回準備時は `preparing` 状態を維持し、失敗時はエラー文言を返す。
- バックエンドでも GPU フラグを必須条件として二重ガードする。
