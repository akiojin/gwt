# 実装計画: gwt GUI マルチウィンドウ + Native Windowメニュー

**仕様ID**: `SPEC-4470704f` | **日付**: 2026-02-09（更新: 2026-02-17） | **仕様書**: `specs/SPEC-4470704f/spec.md`
**入力**: `specs/SPEC-4470704f/spec.md` からの機能仕様

## 概要

本機能は以下を実現する：

- ネイティブメニューバーへ操作導線を統合し、in-app menubar（WebView上の疑似メニューUI）を廃止する
- `File > New Window` で新規ウィンドウを開ける
- `Window` メニューで、プロジェクトを開いているウィンドウへ切り替え（再表示 + フォーカス）できる
- プロジェクト状態はウィンドウ単位で保持し、別ウィンドウの操作に干渉しない
- 非表示（閉じて hide）になったウィンドウは `Window` メニューに表示しない
- 同一 canonical path のプロジェクトを別ウィンドウで開こうとした場合は既存ウィンドウへフォーカスし、重複オープンを防止する
- 同名ディレクトリでも canonical path が異なる場合は別プロジェクトとして開ける

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript + Vite（`gwt-gui/`）
- **権限**: Tauri v2 capabilities（`crates/gwt-tauri/capabilities/`）
- **テスト**: `cargo test -p gwt-tauri`（ユニットテスト）、`cd gwt-gui && pnpm exec svelte-check --tsconfig ./tsconfig.json`（型/静的チェック）、`cd gwt-gui && pnpm exec playwright test e2e/open-project-smoke.spec.ts --project=chromium`（E2Eスモーク）
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
- `crates/gwt-tauri/src/commands/project.rs`（2026-02-17 追加）
  - `open_project` の返却を `OpenProjectResult { info, action, focused_window_label }` に変更
  - canonical path による重複判定を追加し、同一実体の既存ウィンドウがあれば `FocusedExisting` を返して show+focus
  - `create_project` は後方互換として `ProjectInfo` を返すため、`open_project(...).map(|result| result.info)` を利用
- `crates/gwt-tauri/src/state.rs`（2026-02-17 追加）
  - `window_project_identities`（window label -> canonical path）を追加
  - `find_window_by_project_identity` を追加し、重複判定を state 経由で解決
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
  - `open_project` の結果を `OpenProjectResult` で受け、`action === "opened"` のときのみ現在ウィンドウの `projectPath` を更新する
- `gwt-gui/src/lib/components/MenuBar.svelte`
  - 未使用化するため削除（または参照を完全に外す）
- `gwt-gui/src/lib/components/OpenProject.svelte`（2026-02-17 追加）
  - `open_project` の戻り値を `OpenProjectResult` として扱い、`opened` 時のみ `onOpen` を呼ぶ
- `gwt-gui/src/lib/types.ts`（2026-02-17 追加）
  - `OpenProjectResult` 型を追加
- `gwt-gui/e2e/support/tauri-mock.ts`（2026-02-17 追加）
  - `open_project` のモック戻り値を新API形式に追従

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
- ウィンドウラベルごとの canonical identity 参照/更新/検索（除外ウィンドウ指定含む）が正しく動くこと
- Window menu の項目ラベル生成（同名ディレクトリの区別）が安定していること
- Window menu 用の menu-id のパースが正しく動くこと
- `canonical_project_identity` が `..` / symlink 等の実体同一パスを同一キーとして扱えること
- `open_project` の戻り値シリアライズで `action` が `focusedExisting/opened` として表現されること

### 手動受け入れ（GUI）

1. `File > New Window` でウィンドウが増える
2. 2ウィンドウで別プロジェクトを開く
3. `Window` メニューで切替できる（非表示になっている場合も復帰できる）
4. 片方でプロジェクトを切り替えても、もう片方のエージェント起動先が変わらない
5. ウィンドウAで `/work/a` を開いた状態で、ウィンドウBから `/work/a` を開くと A が前面化される
6. ウィンドウAで `/work/x/repo`、ウィンドウBで `/work/y/repo`（同名ディレクトリ）を同時に開いて利用できる

## リスクと緩和策

- **Windowsでのウィンドウ生成のデッドロック**: メニューイベントなど同期コンテキストからの生成はリスクがあるため、別スレッドでウィンドウ生成を行う
- **capabilities不足でIPC失敗**: 追加ウィンドウのラベルを `project-*` に統一し、capabilities の windows に glob を追加する
- **フォーカス特定の曖昧さ**: `is_focused` を利用してメニューイベントの配送先ウィンドウを決定し、見つからない場合は `main` をフォールバックとする
- **非表示ウィンドウが一覧に残る**: `is_visible` を利用して非表示ウィンドウを Window メニューから除外する
