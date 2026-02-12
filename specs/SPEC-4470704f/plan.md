# 実装計画: gwt GUI マルチウィンドウ + Native Windowメニュー

**仕様ID**: `SPEC-4470704f` | **日付**: 2026-02-09 | **仕様書**: `specs/SPEC-4470704f/spec.md`
**入力**: `specs/SPEC-4470704f/spec.md` からの機能仕様

## 概要

本機能は以下を実現する：

- ネイティブメニューバーへ操作導線を統合し、in-app menubar（WebView上の疑似メニューUI）を廃止する
- `File > New Window` で新規ウィンドウを開ける
- `Window` メニューで、プロジェクトを開いているウィンドウへ切り替え（再表示 + フォーカス）できる
- プロジェクト状態はウィンドウ単位で保持し、別ウィンドウの操作に干渉しない
- 非表示（閉じて hide）になったウィンドウは `Window` メニューに表示しない

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript + Vite（`gwt-gui/`）
- **権限**: Tauri v2 capabilities（`crates/gwt-tauri/capabilities/`）
- **テスト**: `cargo test -p gwt-tauri`（ユニットテスト）、`cd gwt-gui && npm run check`（型/静的チェック）
- **制約**: CloseRequested は hide（トレイ Quit 以外で終了しない）を維持する

## 変更点（ソース）

### バックエンド（Tauri）

- `crates/gwt-tauri/src/state.rs`
  - グローバル単一の `project_path` を廃止し、ウィンドウラベルごとの projectPath 管理へ移行する
- `crates/gwt-tauri/src/menu.rs`（新規）
  - ネイティブメニューを構築し、`Window` メニューはウィンドウ一覧を動的生成する
- `crates/gwt-tauri/src/app.rs`
  - アプリ起動時にネイティブメニューをセットする
  - メニューイベントを処理し、必要に応じてフロントへ `menu-action` を送る
  - ウィンドウ作成/フォーカス変化/破棄でメニューを再構築する
- `crates/gwt-tauri/src/commands/project.rs`
  - `open_project` は呼び出し元ウィンドウに対して project をセットする
  - `close_project` を追加し、呼び出し元ウィンドウの project をクリアする
- `crates/gwt-tauri/src/commands/terminal.rs`
  - `launch_terminal` / `launch_agent` の project 解決を「呼び出し元ウィンドウ」へ変更する
- `crates/gwt-tauri/src/commands/settings.rs`
  - 設定読み込み/保存の project 解決を「呼び出し元ウィンドウ」へ変更する
- `crates/gwt-tauri/capabilities/default.json`
  - 追加ウィンドウ（`project-*`）が dialog/store 等を使用できるよう、windows スコープを拡張する

### フロントエンド（Svelte）

- `gwt-gui/src/App.svelte`
  - `MenuBar.svelte` を廃止し、レイアウトから除去する
  - Tauri event `menu-action` を listen し、既存の処理（Open Project 等）を呼び出す
- `gwt-gui/src/lib/components/MenuBar.svelte`
  - 未使用化するため削除（または参照を完全に外す）

## メニュー仕様（ネイティブ）

- トップレベル: gwt / File / Edit / Git / Tools / Window
- `File`
  - New Window
  - Open Project...
  - Close Project
- `Edit`
  - Undo / Redo
  - Cut / Copy / Paste / Select All
- `Git`
  - Cleanup Worktrees...
- `Tools`
  - Launch Agent...
  - List Terminals
  - Terminal Diagnostics
- `Window`
  - プロジェクトが開かれているウィンドウ一覧（同名の場合はパスを付加して区別）
  - 選択時: 対象ウィンドウを show + focus
- `gwt`
  - About gwt
  - Preferences...

## テスト戦略

### ユニットテスト（Rust）

- ウィンドウラベルごとの projectPath 参照/更新/クリアが正しく動くこと
- Window menu の項目ラベル生成（同名ディレクトリの区別）が安定していること
- Window menu 用の menu-id のパースが正しく動くこと

### 手動受け入れ（GUI）

1. `File > New Window` でウィンドウが増える
2. 2ウィンドウで別プロジェクトを開く
3. `Window` メニューで切替できる（非表示になっている場合も復帰できる）
4. 片方でプロジェクトを切り替えても、もう片方のエージェント起動先が変わらない

## リスクと緩和策

- **Windowsでのウィンドウ生成のデッドロック**: メニューイベントなど同期コンテキストからの生成はリスクがあるため、別スレッドでウィンドウ生成を行う
- **capabilities不足でIPC失敗**: 追加ウィンドウのラベルを `project-*` に統一し、capabilities の windows に glob を追加する
- **フォーカス特定の曖昧さ**: `is_focused` を利用してメニューイベントの配送先ウィンドウを決定し、見つからない場合は `main` をフォールバックとする
- **非表示ウィンドウが一覧に残る**: `is_visible` を利用して非表示ウィンドウを Window メニューから除外する
