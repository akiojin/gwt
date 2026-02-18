# タスクリスト: 全画面テキストコピー (Cmd+Shift+C)

**仕様ID**: `SPEC-0e6abbb7`

## タスク

### T-001: テスト作成 (screenCapture)

- **種別**: テスト
- **ファイル**: `gwt-gui/src/lib/screenCapture.test.ts`
- **内容**: `collectScreenText()` のユニットテストを作成
  - 構造化テキスト形式の検証（ヘッダー、セクション区切り）
  - メタデータ（ブランチ名、タブ名、ウィンドウサイズ）の挿入
  - モーダル表示時のセクション追加
  - 空ターミナル時の "(empty)" 表示
- **依存**: なし
- **状態**: [ ] 未着手

### T-002: screenCapture.ts 実装

- **種別**: 実装
- **ファイル**: `gwt-gui/src/lib/screenCapture.ts`
- **内容**: 画面テキスト収集・構造化ロジック
  - `collectScreenText()` 関数: 各セクションのテキストを収集し構造化テキストを返す
  - サイドバー・メインエリア・ステータスバーの DOM テキスト取得
  - xterm.js buffer API による可視行取得
  - モーダル検出とテキスト取得
- **依存**: T-001 (テストが RED であること)
- **状態**: [ ] 未着手

### T-003: Rust バックエンド - メニュー追加

- **種別**: 実装
- **ファイル**: `crates/gwt-tauri/src/menu.rs`, `crates/gwt-tauri/src/app.rs`
- **内容**:
  - Edit メニューに "Copy Screen Text" (CmdOrCtrl+Shift+C) を追加
  - `menu_action_from_id()` に `"screen-copy"` マッピングを追加
- **依存**: なし
- **状態**: [ ] 未着手

### T-004: App.svelte - ハンドラ統合

- **種別**: 実装
- **ファイル**: `gwt-gui/src/App.svelte`
- **内容**:
  - メニューアクション `"screen-copy"` のハンドラを追加
  - `collectScreenText()` を呼び出し → clipboard 書き込み
  - コピーフラッシュ + トースト表示
- **依存**: T-002, T-003
- **状態**: [ ] 未着手

### T-005: コピーフラッシュ視覚フィードバック

- **種別**: 実装
- **ファイル**: `gwt-gui/src/App.svelte`
- **内容**:
  - アクセントカラー半透明オーバーレイ (200ms フェードイン・アウト)
  - CSS animation で実装
  - トースト "Copied to clipboard" 表示
- **依存**: T-004
- **状態**: [ ] 未着手

### T-006: 結合テスト・手動検証

- **種別**: テスト
- **内容**:
  - `pnpm test` でユニットテストが全て GREEN
  - `svelte-check` で型エラーなし
  - `cargo clippy` で警告なし
  - 手動検証: ターミナルタブ、非ターミナルタブ、モーダル表示時のコピー
- **依存**: T-004, T-005
- **状態**: [ ] 未着手

## 依存関係

```text
T-001 ──→ T-002 ──→ T-004 ──→ T-005 ──→ T-006
                      ↑
T-003 ───────────────┘
```
