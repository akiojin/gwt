# 実装計画: ウィンドウ・タブ切り替えショートカット

**仕様ID**: `SPEC-e7b3a1d2` | **日付**: 2026-02-15 | **仕様書**: `specs/SPEC-e7b3a1d2/spec.md`

## 目的

- タブ切り替え（Cmd+Shift+\[/\]）とウィンドウ切り替え（Cmd+\`）のキーボードショートカットを追加
- Window メニューに macOS 標準項目（Minimize/Zoom/Bring All to Front）を追加
- README に包括的キーボードショートカット一覧を記載

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **テスト**: cargo test（Rust）/ vitest（フロントエンド）
- **前提**: Tauri v2 の accelerator API を使用。未サポートキーの場合は別キーに変更

## 実装方針

### Phase 1: Accelerator 検証 + タブ切り替え基盤

1. **Tauri accelerator 検証**: `CmdOrCtrl+Shift+[` / `CmdOrCtrl+Shift+]` / `CmdOrCtrl+Backquote` が Tauri v2 で動作するか検証する。`crates/gwt-tauri/src/menu.rs` に仮のメニュー項目を追加して `cargo tauri dev` で確認
2. **Window メニューにタブナビゲーション項目追加**: `menu.rs` の `build_window_submenu()` に "Previous Tab" / "Next Tab" の `MenuItem::with_id()` を追加し、対応する accelerator を割り当てる
3. **menu-action イベントのハンドリング**: `app.rs` の `on_menu_event` で新しいメニュー ID（`window-previous-tab` / `window-next-tab`）をフォーカス中ウィンドウに emit
4. **フロントエンドでのタブ切り替えロジック**: `gwt-gui/src/lib/tabNavigation.ts` に純粋なタブ切り替え関数を分離（テスト容易性のため）。`App.svelte` の `handleMenuAction()` から呼び出す

### Phase 2: ウィンドウ切り替え + MRU 管理

1. **AppState に MRU リスト追加**: `state.rs` に `window_focus_history: Mutex<Vec<String>>` を追加。ウィンドウフォーカス変更時に先頭に push し、ウィンドウ破棄時に除去
2. **フォーカス変更イベントの検出**: `app.rs` の `on_window_event` で `WindowEvent::Focused(true)` を検出し、MRU リストを更新
3. **Window メニューにウィンドウナビゲーション項目追加**: `menu.rs` に "Next Window" / "Previous Window" を追加
4. **ウィンドウ切り替えロジック**: `app.rs` の `on_menu_event` で MRU リストから次/前のウィンドウを取得し、`window.show()` + `window.set_focus()` を実行。非表示ウィンドウも対象に含める

### Phase 3: macOS 標準 Window メニュー項目

1. **Minimize / Zoom / Bring All to Front 追加**: `menu.rs` の `build_window_submenu()` に `#[cfg(target_os = "macos")]` で Minimize (Cmd+M), Zoom, Bring All to Front を追加
2. **アクションハンドリング**: Minimize は `window.minimize()`、Zoom は `window.maximize()` のトグル、Bring All to Front は全ウィンドウの `window.show()` + `window.set_focus()`

### Phase 4: README 更新 + 仕上げ

1. **README.md / README.ja.md 更新**: キーボードショートカット一覧セクションを追加。既存ショートカット（Cmd+N, Cmd+O, Cmd+C/V, Cmd+Shift+K, Cmd+,）と今回追加分を網羅
2. **テスト**: 各フェーズのユニットテストを作成

## テスト

### バックエンド

- MRU リストの追加・除去・順序が正しいこと（`state.rs` のユニットテスト）
- メニュー ID パーサーのテスト追加（既存テストの拡張）

### フロントエンド

- タブ切り替えロジックのユニットテスト（`gwt-gui/src/lib/` にテスト追加）
  - 次のタブへの切り替え
  - 前のタブへの切り替え
  - 端での停止（ラップしない）
  - タブ1つの場合は何もしない
  - Summary パネルの包含
  - D&D 並べ替え後の順序追従
