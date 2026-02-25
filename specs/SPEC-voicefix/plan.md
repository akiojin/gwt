# 実装計画: SPEC-voicefix

## 概要

SettingsPanel.svelte の Voice Input フィールドの `disabled` 条件から `!voiceAvailable` を除去し、情報バナーの文言を改善する。テストカバレッジを追加する。

## 修正対象

| ファイル | 変更内容 |
|---------|---------|
| `gwt-gui/src/lib/components/SettingsPanel.svelte` | disabled 条件の修正 (5箇所) + 情報バナー文言改善 |
| `gwt-gui/src/lib/components/SettingsPanel.test.ts` | ユニットテスト追加 |
| `gwt-gui/e2e/support/tauri-mock.ts` | `get_voice_capability` モック追加 |
| `gwt-gui/e2e/voice-input-settings.spec.ts` | E2E テスト新規作成 |

## バックエンド変更

なし。`get_voice_capability` は正しく動作している。

## リスク

- 低リスク: フロントエンドのみの修正、disabled 条件の緩和のみ
