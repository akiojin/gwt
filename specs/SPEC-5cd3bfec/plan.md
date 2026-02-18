# 実装計画: 設定画面のスクロールをタブ切り替えに変更

**仕様ID**: `SPEC-5cd3bfec` | **日付**: 2026-02-18 | **仕様書**: `specs/SPEC-5cd3bfec/spec.md`

## 目的

- 設定画面（SettingsPanel.svelte）の `<details>` アコーディオン4セクションをタブ切り替えに変更し、1画面に1セクションのみ表示してスクロール量を削減する

## 技術コンテキスト

- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/src/lib/components/SettingsPanel.svelte`）
- **テスト**: vitest + @testing-library/svelte（`gwt-gui/src/lib/components/SettingsPanel.test.ts`）
- **前提**: バックエンド変更なし。GitSection.svelte の `.git-tabs` / `.git-tab-btn` タブパターンを踏襲

## 実装方針

### Phase 1: タブUI基盤

- `SettingsPanel.svelte` に `SettingsTabId` 型と `activeSettingsTab` 状態を追加
- タブは `"appearance" | "voiceInput" | "mcpBridge" | "profiles"` の4つ
- 初期値は `"appearance"`

### Phase 2: テンプレート変更

- settings-body 内の `<details>` × 4 + divider × 3 を、タブバー + `{#if}` による条件表示に置き換える
- タブバーは `.settings-tabs` クラスで GitSection の `.git-tabs` と同じ flex レイアウト
- 各タブボタンは `.settings-tab-btn` クラスで GitSection の `.git-tab-btn` と同じスタイル
- タブコンテンツ領域は `.settings-tab-content` で `overflow-y: auto; flex: 1` を設定し、個別セクションのスクロールを実現

### Phase 3: CSS 整理

- 不要になった `.settings-section`, `summary.section-title` 関連のスタイルを削除
- `.settings-tabs`, `.settings-tab-btn`, `.settings-tab-content` のスタイルを追加
- settings-body から `overflow-y: auto` を `.settings-tab-content` へ移動

## テスト

### フロントエンド

- 既存テスト（SettingsPanel.test.ts）のセレクタを `<details>` から タブ操作に更新
- タブクリックで対応セクションが表示されることを確認するテストを追加
- 初期表示で Appearance タブが選択されていることを確認するテストを追加
