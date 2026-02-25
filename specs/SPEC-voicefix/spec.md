# バグ修正仕様: ボイス入力設定フィールドが操作不能

**仕様ID**: `SPEC-voicefix`
**作成日**: 2026-02-25
**ステータス**: 承認済み
**カテゴリ**: GUI (bugfix)

## 背景

- Settings > Voice Input タブで、全フィールドに `disabled={!voiceAvailable || voiceCapabilityLoading}` が付与されている
- GPU やランタイムが未インストールの環境では `voiceAvailable=false` となり、設定変更が一切できない
- ユーザーはランタイムインストール前に hotkey / language / quality を事前設定できるべき

## ユーザーシナリオとテスト

### ユーザーストーリー 1 - ランタイム未準備でも設定可能 (優先度: P0)

ユーザーとして、GPU やランタイムが未インストールでも Voice Input の設定を変更・保存したい。

**受け入れシナリオ**:

1. **前提条件** `get_voice_capability` が `{ available: false, reason: "..." }` を返す、**操作** Settings > Voice Input タブを開く、**期待結果** Enable / Hotkey / PTT Hotkey / Language / Quality フィールドが全て操作可能
2. **前提条件** 上記状態、**操作** 各フィールドの値を変更し Save を押す、**期待結果** `save_settings` が更新値で呼ばれる
3. **前提条件** 上記状態、**操作** Voice Input タブを表示、**期待結果** ランタイム非利用可能の理由が情報バナーとして表示され、設定は引き続き可能である旨が表示される

### ユーザーストーリー 2 - ローディング中は操作不可 (優先度: P0)

**受け入れシナリオ**:

1. **前提条件** `voiceCapabilityLoading=true`、**操作** Voice Input タブを表示、**期待結果** 全フィールドが disabled

## エッジケース

- Model フィールドは quality から自動決定のため常に readonly/disabled（変更なし）
- `voiceCapabilityLoading` 中は従来通り disabled を維持

## 要件

### 機能要件

- **FR-001**: Voice Input 設定フィールド（#voice-input-enabled, #voice-hotkey, #voice-ptt-hotkey, #voice-language, #voice-quality）の `disabled` 条件から `!voiceAvailable` を削除する
- **FR-002**: ランタイム未準備時の情報バナーに「設定は引き続き可能」の旨を追記する
- **FR-003**: Model フィールドは変更しない（readonly/disabled を維持）

## 成功基準

- **SC-001**: `get_voice_capability` が `available: false` を返しても、5つの設定フィールドが操作可能
- **SC-002**: ユニットテスト・E2E テストが GREEN
- **SC-003**: `svelte-check` がエラーなしで通過
