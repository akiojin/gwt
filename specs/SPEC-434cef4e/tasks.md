# タスクリスト: v7.11.0 起動不能（Issue #1219）

## Phase 1: 原因固定化

- [x] T001 [US1] v7.11.0 の配布成果物を検証し、`gwt.app/Contents/MacOS` に `voice_eval` のみが入る再現を確認する

## Phase 2: バイナリ選択修正

- [x] T002 [US1] `crates/gwt-tauri/Cargo.toml` に `default-run` と `[[bin]]` を追加してメイン実行バイナリを固定する
- [x] T003 [US1] `.github/workflows/release.yml` を更新し、`cargo tauri build -- --bin gwt-tauri` を適用する

## Phase 3: 互換性維持

- [x] T004 [US2] `voice_eval` を補助バイナリとして維持し、GUI バンドル対象と分離されていることを確認する

## Phase 4: 検証

- [x] T005 [US1] `cargo metadata` で `default_run=gwt-tauri` とバイナリ定義を確認する
- [x] T006 [US1,US2] `cargo check -p gwt-tauri --bin gwt-tauri --bin voice_eval` を実行してビルド成立を確認する
