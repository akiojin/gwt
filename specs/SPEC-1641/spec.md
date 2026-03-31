> **📜 HISTORICAL (SPEC-1776)**: This SPEC was written for the previous GUI stack (Tauri/Svelte/C#). It is retained as a historical reference. The gwt-tui migration (SPEC-1776) supersedes GUI-specific design decisions described here.

### 背景
ターミナルへの音声入力機能を提供する。ローカル ASR（Qwen3-ASR）を使用し、GPU 搭載環境でのみ利用可能。既存実装では設定可能ホットキーとオーバーレイ Voice ボタンが不安定で、起動導線として機能していなかったため、操作モデルを固定 PTT に単純化して安定化する。

### ユーザーシナリオとテスト

**S1: Terminal オーバーレイの Voice ボタンで PTT 録音する**
- Given: Voice Input が有効
- When: Terminal オーバーレイの Voice ボタンを押している間だけ話す
- Then: 認識結果が terminal またはフォーカス中の入力欄へ入力される

**S2: 固定 PTT キーで録音する**
- Given: Voice Input が有効で、アプリ内の terminal / textarea / input / contenteditable にフォーカスがある
- When: macOS では `Cmd+Shift+Space`、Windows/Linux では `Ctrl+Shift+Space` を押している間だけ話す
- Then: キーを押している間だけ録音し、離した時点で確定する

**S3: GPU 非対応または runtime 未準備でも設定画面は操作できる**
- Given: GPU 非対応環境または Voice runtime 未準備
- When: Settings の Voice Input タブを開く
- Then: unavailable reason を表示しつつ、enabled / language / quality / model の設定は操作できる

**S4: 旧ホットキー設定が残る既存 config をロードできる**
- Given: 旧 config に `voice_input.hotkey` / `voice_input.ptt_hotkey` が残っている
- When: アプリが設定をロードする
- Then: ロードは成功し、新規保存時にはこれらの項目を書き戻さない

### 機能要件

**FR-01: ASR エンジン**
- ローカル ASR: Qwen3-ASR
- GPU 必須（未対応環境では機能無効）
- フォールバックなし

**FR-02: 入力先**
- フォーカス中の terminal を優先する
- アクティブな textarea / input / contenteditable がある場合はそこへ挿入する

**FR-03: トリガーモデル**
- 設定可能ホットキーは廃止する
- キーボード操作は固定 PTT 1 本のみとする
- 固定 PTT は `macOS: Cmd+Shift+Space`、`Windows/Linux: Ctrl+Shift+Space`
- Terminal オーバーレイの Voice ボタンは押下中のみ録音する PTT ボタンにする
- runtime / model / microphone 初期化中にキーまたはボタンが離された場合、録音を開始しない

**FR-04: Settings 画面**
- `voice_input` 設定は `enabled` / `engine` / `language` / `quality` / `model` のみを編集対象とする
- ホットキー入力欄は表示しない
- 固定 PTT キーの説明を表示する

**FR-05: 設定永続化と互換性**
- frontend / tauri / core の `voice_input` から hotkey 系フィールドを削除する
- 旧 config の hotkey 系フィールドは読み込み時に無視する
- 新規保存時に hotkey 系フィールドは出力しない

**FR-06: Capability / bootstrap**
- Voice ボタンは `available=false` だけを理由に無効化しない
- 初回利用時の runtime / model 準備経路へボタンから到達できるようにする

### 成功基準

1. Settings からホットキー設定 UI が消えている
2. 固定 PTT キーで押下中のみ録音できる
3. オーバーレイ Voice ボタンが押下中のみ録音できる
4. 旧 hotkey 設定を含む config をロードしても壊れず、再保存で hotkey 系が消える
5. GUI 単体テスト、設定関連 Rust テスト、設定系 Playwright E2E が通る

---
