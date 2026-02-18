# TDD: gwt GUI マルチウィンドウ + Native Windowメニュー

**仕様ID**: `SPEC-4470704f`
**作成日**: 2026-02-10
**対象**: `crates/gwt-tauri/src/menu.rs`

## テスト方針

- ユニットテストで Window メニュー用の対象ウィンドウ抽出が正しく行われることを担保する
- 非表示ウィンドウ（close により hide された状態）は一覧に含めない

## テストケース

1. `collect_window_entries` は非表示ウィンドウを除外する
2. `collect_window_entries` はプロジェクト未選択ウィンドウを除外する
3. `disambiguate_project_displays` は同名ディレクトリのときフルパスを付与する

## 受け入れ確認

- 2つ以上のウィンドウを開き、片方を閉じる（hide）
- `Window` メニューに閉じたウィンドウが残らない
