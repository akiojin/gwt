# TDD計画と結果: SPEC-e7b3a1d2

**仕様ID**: `SPEC-e7b3a1d2`  
**実施日**: 2026-02-22  
**対象**: Cmd+Backquote / Cmd+Shift+Backquote の巡回対象から `New Window`（project 未選択）を除外する

## 1. RED（失敗テストを先に追加）

### 追加したテスト

1. `state::tests::clear_project_for_window_removes_window_from_mru_history`
2. `state::tests::window_rotation_skips_window_after_project_close`

### RED確認コマンドと結果

1. `cargo test -p gwt-tauri clear_project_for_window_removes_window_from_mru_history`  
   - 結果: `FAILED`
   - 失敗要約: `clear_project_for_window("B")` 後も履歴が `["A","B","C"]` のまま
2. `cargo test -p gwt-tauri window_rotation_skips_window_after_project_close`  
   - 結果: `FAILED`
   - 失敗要約: `next_window()` が `Some("B")` を返し、close 済みウィンドウをスキップできていない

## 2. GREEN（最小実装で通す）

### 実装内容

1. `crates/gwt-tauri/src/state.rs`
   - `clear_project_for_window()` で `remove_window_from_history(window_label)` を実行
2. `crates/gwt-tauri/src/commands/project.rs`
   - `open_project()` 成功時（`Opened` / `FocusedExisting`）に `state.push_window_focus(...)` を実行
3. `crates/gwt-tauri/src/app.rs`
   - `WindowEvent::Focused(true)` 時、`project_for_window(label).is_some()` の場合のみ MRU 更新

### GREEN確認コマンドと結果

1. `cargo test -p gwt-tauri clear_project_for_window_removes_window_from_mru_history` → `ok`
2. `cargo test -p gwt-tauri window_rotation_skips_window_after_project_close` → `ok`

## 3. 回帰確認

1. `cargo test -p gwt-tauri state::tests::` → `23 passed`
2. `cargo test -p gwt-tauri commands::project::tests::` → `10 passed`
3. `cargo fmt --all --check` → `ok`（rustfmt nightly option warningのみ）

## 4. 判定

- RED→GREEN の流れで不具合再現と修正完了を確認
- Cmd+Backquote 巡回対象は `project` を開いている既存ウィンドウに限定される前提がテストで固定された
