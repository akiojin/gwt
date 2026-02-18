# 要件品質チェックリスト: TUI→Tauri GUI完全移行 Phase 1

**目的**: SPEC-d6210238 の仕様品質と完全性を検証する
**作成日**: 2026-02-08
**機能**: [spec.md](../spec.md)

## gwt-core TUI依存除去

- [ ] CHK001 ratatui 依存が Cargo.toml から除去されている
- [ ] CHK002 vt100 依存が Cargo.toml から除去されている
- [ ] CHK003 crossterm 依存が gwt-core から参照されていない
- [ ] CHK004 terminal/emulator.rs が削除されている
- [ ] CHK005 terminal/renderer.rs が削除されている
- [ ] CHK006 terminal/mod.rs から emulator, renderer モジュール宣言が除去されている
- [ ] CHK007 BuiltinLaunchConfig.agent_color が独自の色型を使用している
- [ ] CHK008 PaneConfig.agent_color が独自の色型を使用している
- [ ] CHK009 pane.rs の render() メソッドが削除されている
- [ ] CHK010 pane.rs の screen() メソッドが削除されている
- [ ] CHK011 pane.rs から TerminalEmulator フィールドが除去されている
- [ ] CHK012 pane.rs の process_bytes() からエミュレータ処理が除去されている
- [ ] CHK013 pane.rs の mouse_protocol_enabled() が削除されている
- [ ] CHK014 `cargo tree -p gwt-core` に ratatui, vt100 が含まれない

## gwt-core テスト

- [ ] CHK015 pty.rs のテストが全通過する
- [ ] CHK016 scrollback.rs のテストが全通過する
- [ ] CHK017 pane.rs のテストが全通過する（emulator/renderer 依存テスト除去後）
- [ ] CHK018 manager.rs のテストが全通過する（Color型置換後）
- [ ] CHK019 ipc.rs のテストが全通過する
- [ ] CHK020 `cargo clippy -p gwt-core -- -D warnings` が警告なし

## Tauri バックエンド

- [ ] CHK021 crates/gwt-tauri/Cargo.toml が存在する
- [ ] CHK022 gwt-tauri が gwt-core を依存として宣言している
- [ ] CHK023 crates/gwt-tauri/src/main.rs が存在する
- [ ] CHK024 tauri.conf.json が適切に設定されている
- [ ] CHK025 `cargo build -p gwt-tauri` が成功する

## Svelte フロントエンド

- [ ] CHK026 gwt-gui/package.json が存在する
- [ ] CHK027 Svelte 5 + TypeScript + Vite が設定されている
- [ ] CHK028 gwt-gui/src/App.svelte が存在する
- [ ] CHK029 ダークテーマのスタイルが適用されている
- [ ] CHK030 サイドバー + メインエリアの2カラムレイアウトが実装されている

## ワークスペース構成

- [ ] CHK031 Cargo.toml の members に gwt-cli が含まれない
- [ ] CHK032 Cargo.toml の members に gwt-web が含まれない
- [ ] CHK033 Cargo.toml の members に gwt-frontend が含まれない
- [ ] CHK034 Cargo.toml の members に gwt-tauri が含まれる
- [ ] CHK035 `cargo build` がワークスペースルートで成功する
- [ ] CHK036 `cargo test` が全テストで通過する

## 注記

- 完了した項目にチェックマーク: `[x]`
- インラインでコメントや調査結果を追加
- 関連リソースまたはドキュメントへのリンク
- 項目は参照しやすいように連番で番号付け
