# 実装計画: Voice Input Mode（GUI）

**仕様ID**: `SPEC-9f3c2a11`  
**日付**: 2026-02-13

## 実装方針

1. 設定拡張
- `Settings` / `SettingsData` に `voice_input` を追加（enabled/hotkey/language/model）。
- 既存設定ファイルの後方互換を維持（デフォルト値で補完）。

1. 入力ターゲット抽象化
- フォーカス中DOM入力要素と terminal 入力先を判定する共通層を追加。
- Terminal は `paneId` と root element を登録し、フォーカス中 pane を解決する。

1. 音声コントローラ
- グローバルホットキーで start/stop。
- 認識結果を現在の入力ターゲットへ挿入。
- 自動送信はしない。

1. UI統合
- `App.svelte` でコントローラを初期化し、Settings更新イベントで再設定。
- `StatusBar` に Voice 状態表示を追加。
- `SettingsPanel` に Voice Input セクション追加。

1. テスト
- 設定の round-trip テスト（Rust）。
- 音声コントローラのホットキー/ターゲット解決テスト（TS）。
- 既存 terminal/agent mode テスト回帰確認。

## 主要変更ファイル

- `crates/gwt-core/src/config/settings.rs`
- `crates/gwt-tauri/src/commands/settings.rs`
- `gwt-gui/src/lib/types.ts`
- `gwt-gui/src/App.svelte`
- `gwt-gui/src/lib/components/SettingsPanel.svelte`
- `gwt-gui/src/lib/components/StatusBar.svelte`
- `gwt-gui/src/lib/terminal/TerminalView.svelte`
- `gwt-gui/src/lib/voice/*` (新規)

## リスク

- ブラウザ/OSごとの Speech API 差異。
- グローバルホットキーが既存ショートカットと衝突する可能性。
- terminal フォーカス判定の誤差。

## 緩和策

- Speech API 非対応時はエラーをStatusBarに表示し、既存入力は維持。
- ホットキーを設定可能にし、デフォルトは競合の少ない組み合わせを採用。
- terminal の root 登録 + active tab fallback の二段階判定を実施。
