# 機能仕様: TUI→Tauri GUI完全移行 Phase 1: 基盤構築

**仕様ID**: `SPEC-d6210238`
**作成日**: 2026-02-08
**ステータス**: ドラフト
**カテゴリ**: GUI

**入力**: gwt を ratatui TUI から Tauri v2 + Svelte 5 + xterm.js GUI に完全移行する Phase 1

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - gwt-core ライブラリの TUI 依存除去 (優先度: P1)

gwt-core ライブラリから TUI フレームワーク固有の依存（ratatui, crossterm, vt100）を完全に除去し、
バックエンドライブラリとして GUI フレームワークに依存しない形にする。
ターミナルエミュレーション（vt100）は xterm.js に置換されるため不要となる。
PTY 管理、スクロールバック、IPC は引き続き gwt-core が担当する。

**この優先度の理由**: gwt-core は全てのフロントエンド（旧TUI、新GUI）の基盤であり、
TUI 依存を除去しないと新しい GUI バックエンド（gwt-tauri）が gwt-core を利用できない。

**独立したテスト**: `cargo build -p gwt-core` でビルド成功、`cargo test -p gwt-core` で全テスト通過、
`cargo clippy -p gwt-core -- -D warnings` で警告なしを確認。

**受け入れシナリオ**:

1. **前提条件** gwt-core が ratatui, vt100 に依存している、**操作** 依存除去後に `cargo build -p gwt-core` を実行する、**期待結果** ビルドが成功し ratatui/vt100/crossterm が依存ツリーに含まれない
2. **前提条件** gwt-core に renderer.rs, emulator.rs が存在する、**操作** 両ファイルを削除し mod.rs から除去する、**期待結果** ビルドが成功し、PTY管理・スクロールバック・IPC の既存テストが全通過する
3. **前提条件** BuiltinLaunchConfig と PaneConfig が `ratatui::style::Color` を使用している、**操作** 独自の色型に置換する、**期待結果** gwt-core 内で ratatui への参照が一切なくなる

---

### ユーザーストーリー 2 - Tauri バックエンドプロジェクトの初期化 (優先度: P1)

Tauri v2 ベースの GUI デスクトップアプリケーションのバックエンド（crates/gwt-tauri/）を作成し、
gwt-core を依存として利用する。空のウィンドウが表示できる最小構成を構築する。

**この優先度の理由**: GUI アプリケーションの骨格がなければ、以降の全フェーズの開発が進められない。

**独立したテスト**: `cargo build -p gwt-tauri` でビルド成功、`cargo tauri dev` でウィンドウが表示される。

**受け入れシナリオ**:

1. **前提条件** crates/gwt-tauri/ が存在しない、**操作** Tauri v2 プロジェクトを作成し `cargo build -p gwt-tauri` を実行する、**期待結果** ビルドが成功する
2. **前提条件** gwt-tauri が gwt-core を依存として宣言している、**操作** `cargo build` をワークスペースルートで実行する、**期待結果** gwt-core と gwt-tauri の両方がビルドに成功する

---

### ユーザーストーリー 3 - Svelte 5 フロントエンドの初期化 (優先度: P1)

Svelte 5 + TypeScript + Vite ベースのフロントエンドプロジェクト（gwt-gui/）を作成し、
Tauri バックエンドと統合する。ダークテーマの基本ウィンドウが表示できる状態にする。

**この優先度の理由**: フロントエンドがなければウィンドウに何も表示できず、GUI としての最小動作確認ができない。

**独立したテスト**: `npm run tauri dev`（gwt-gui/ 内）でアプリケーションウィンドウが表示される。

**受け入れシナリオ**:

1. **前提条件** gwt-gui/ が存在しない、**操作** Svelte 5 + Vite プロジェクトを作成する、**期待結果** `npm run dev` でフロントエンドの開発サーバーが起動する
2. **前提条件** Tauri と Svelte が統合されている、**操作** `npm run tauri dev` を実行する、**期待結果** ダークテーマのデスクトップウィンドウが表示される

---

### ユーザーストーリー 4 - ワークスペース構成の更新 (優先度: P2)

Cargo ワークスペースから旧クレート（gwt-cli, gwt-web, gwt-frontend）を除外し、
新クレート（gwt-tauri）を追加する。package.json も Tauri 開発用に更新する。

**この優先度の理由**: ワークスペース構成が正しくないと CI やビルドが破綻するが、
gwt-core クリーンアップと Tauri 初期化が先に完了している必要がある。

**独立したテスト**: `cargo build` がワークスペースルートで成功し、
旧クレートのビルドが試みられないことを確認。

**受け入れシナリオ**:

1. **前提条件** Cargo.toml に gwt-cli, gwt-web, gwt-frontend が members に含まれている、**操作** これらを members から除外し gwt-tauri を追加する、**期待結果** `cargo build` が gwt-core と gwt-tauri のみをビルドし成功する
2. **前提条件** package.json が npm 配布用の構成になっている、**操作** Tauri 開発用に更新する、**期待結果** `npm run tauri dev` が利用可能になる

---

### ユーザーストーリー 5 - VS Code/Cursor風レイアウト表示 (優先度: P2)

VS Code / Cursor スタイルの GUI レイアウトを構築する。
メニューバー、サイドバー（ブランチリスト）、タブ付きメインエリア、ステータスバーで構成される。

レイアウト構成:

```text
[Menu Bar: File | Edit | View | Window | Settings | Help]
+--Sidebar-----------+--Main Area (tabbed)------------------+
| [Local|Remote|All]  | [Session Summary] [Agent1] [Agent2]  |
| branch-1           |                                       |
| branch-2 (active)  | (content of selected tab)             |
| branch-3           |                                       |
| ...                |                                       |
+--------------------+---------------------------------------+
| Status Bar                                                  |
+-------------------------------------------------------------+
```

- サイドバー: ブランチリスト + Local/Remote/All フィルタ
- メインエリア: タブ切替（セッション要約タブ + エージェントタブ）
- エージェントタブ: エージェント起動のたびにタブが追加される
- メニューバー: File / Edit / View / Window / Settings / Help
- Settings: メニューから設定画面を開く

**この優先度の理由**: Phase 2 以降でブランチリストやターミナルを配置するための
UIの「箱」が必要だが、先にフレームワーク統合が完了している必要がある。

**独立したテスト**: アプリケーション起動時にメニューバー、サイドバー、タブ付きメインエリア、
ステータスバーが視覚的に表示されることを確認。

**受け入れシナリオ**:

1. **前提条件** Svelte + Tauri が統合済み、**操作** アプリケーションを起動する、**期待結果** メニューバー、左側にサイドバー領域、右側にタブ付きメインエリア、下部にステータスバーが表示される
2. **前提条件** アプリケーションが起動している、**操作** ウィンドウをリサイズする、**期待結果** サイドバーとメインエリアがレスポンシブに追従する
3. **前提条件** アプリケーションが起動している、**操作** メニューバーの各メニューをクリックする、**期待結果** ドロップダウンメニューが表示される
4. **前提条件** Agent タブが存在する、**操作** Session Summary/Settings から Agent タブへ切り替える、**期待結果** ターミナルが自動でフォーカスされ、キーボード入力が即反映される

---

---

### ユーザーストーリー 6 - プロジェクト選択（Open Project） (優先度: P2)

TUIではターミナルのカレントディレクトリが暗黙のプロジェクトルートだったが、
GUIアプリはOSから起動されるためプロジェクトを明示的に選択する機能が必要。
VS Code の「Open Folder」と同様の体験を提供する。

**この優先度の理由**: プロジェクトが選択されないとブランチリストもエージェントも
機能しないが、Phase 1 ではUIの骨格のみで実データ表示は Phase 2 のため。

**独立したテスト**: アプリ起動時にプロジェクト未選択状態で Open Project 画面が
表示され、フォルダ選択後にメインレイアウトに遷移することを確認。

**受け入れシナリオ**:

1. **前提条件** プロジェクトが未選択、**操作** アプリケーションを起動する、**期待結果** Open Project 画面（フォルダ選択ボタン + 最近のプロジェクト一覧）が表示される
2. **前提条件** Open Project 画面が表示されている、**操作** フォルダを選択する、**期待結果** メインレイアウト（サイドバー + メインエリア）に遷移する
3. **前提条件** プロジェクトを一度開いたことがある、**操作** アプリケーションを再起動する、**期待結果** 前回のプロジェクトが自動で開かれメインレイアウトが表示される

---

### エッジケース

- gwt-core の Color 型置換後、既存の gwt-cli（旧コード）との互換性は不要（TUI は完全廃止のため）
- Tauri v2 のクロスプラットフォームビルド差異（macOS/Windows/Linux）は Phase 4 で対応
- gwt-core テストで PTY を生成するテストは OS 依存のため、CI 環境での挙動に注意が必要
- Open Project で、Gitリポジトリそのものではなく gwt の作業ルート（bareリポジトリの親ディレクトリ）を選択した場合でも、プロジェクトとして開けること（TUI互換）

## 要件 *(必須)*

### 機能要件

- **FR-200**: gwt-core の Cargo.toml から ratatui, vt100, crossterm の依存を完全に除去**しなければならない**
- **FR-201**: gwt-core の terminal/emulator.rs（vt100 ラッパー）を削除**しなければならない**
- **FR-202**: gwt-core の terminal/renderer.rs（vt100→ratatui 変換）を削除**しなければならない**
- **FR-203**: gwt-core の terminal/mod.rs から emulator, renderer モジュール宣言を除去**しなければならない**
- **FR-204**: gwt-core の `BuiltinLaunchConfig.agent_color` と `PaneConfig.agent_color` を ratatui::style::Color から独自の色型に置換**しなければならない**
- **FR-205**: gwt-core の terminal/pane.rs から `render()` メソッド（ratatui::Buffer, Rect 依存）を削除**しなければならない**
- **FR-206**: gwt-core の terminal/pane.rs から `screen()` メソッド（vt100::Screen 依存）を削除**しなければならない**
- **FR-207**: gwt-core の terminal/pane.rs から TerminalEmulator フィールドとその利用箇所を除去**しなければならない**
- **FR-208**: gwt-core の terminal/pane.rs の `process_bytes()` からエミュレータ処理を除去し、スクロールバック書き込みと PTY データ転送のみを残さ**なければならない**
- **FR-209**: gwt-core の terminal/pane.rs の `mouse_protocol_enabled()` メソッド（vt100 依存）を削除**しなければならない**
- **FR-210**: gwt-core の既存テスト（pty.rs, scrollback.rs, pane.rs, manager.rs, ipc.rs）が全て通過**しなければならない**
- **FR-211**: crates/gwt-tauri/ に Tauri v2 バックエンドクレートを作成**しなければならない**
- **FR-212**: gwt-tauri は gwt-core を依存として利用**しなければならない**
- **FR-213**: gwt-gui/ に Svelte 5 + TypeScript + Vite フロントエンドプロジェクトを作成**しなければならない**
- **FR-214**: Tauri と Svelte フロントエンドが統合され、ネイティブウィンドウが表示**されなければならない**
- **FR-215**: メインウィンドウはダークテーマ（暗色背景）で表示**されなければならない**
- **FR-216**: メインウィンドウはサイドバー（左）とタブ付きメインエリア（右）の2カラムレイアウトを持た**なければならない**
- **FR-228**: アプリケーション起動時にプロジェクトが未選択の場合、「Open Project」画面（フォルダ選択 + 最近のプロジェクト一覧）を表示**しなければならない**
- **FR-229**: File メニューに「Open Project...」（フォルダ選択ダイアログ）と「Recent Projects」（履歴一覧）を提供**しなければならない**
- **FR-230**: 選択されたプロジェクトパスを永続化し、次回起動時に自動で開く**ことができなければならない**
- **FR-222**: メインウィンドウはメニューバー（File/Edit/View/Window/Settings/Help）を持た**なければならない**
- **FR-223**: メインエリアはタブ切替方式で、セッション要約タブとエージェントタブを表示**しなければならない**
- **FR-231**: システムは、エージェントタブがアクティブになった時点でターミナル（xterm.js）へフォーカスを移し、ユーザーが即座に入力できる状態にしなければ**ならない**
- **FR-224**: エージェント起動のたびにメインエリアにエージェントタブが追加**されなければならない**
- **FR-225**: サイドバーにはブランチリストと Local/Remote/All フィルタを表示**しなければならない**
- **FR-226**: メインウィンドウ下部にステータスバーを表示**しなければならない**
- **FR-227**: Settings はメニューバーから開く設定画面として提供**しなければならない**
- **FR-217**: ワークスペースの Cargo.toml から gwt-cli, gwt-web, gwt-frontend メンバーを除外**しなければならない**
- **FR-218**: ワークスペースの Cargo.toml に gwt-tauri メンバーを追加**しなければならない**
- **FR-219**: `cargo build` がワークスペースルートで成功**しなければならない**
- **FR-220**: `cargo test` が gwt-core の全テストで通過**しなければならない**
- **FR-221**: `cargo clippy --all-targets --all-features -- -D warnings` が警告なしで通過**しなければならない**

### 主要エンティティ

- **AgentColor**: ratatui::style::Color を置換する gwt-core 独自の色型。RGB, Indexed, Named（Green, Blue 等）の表現を持つ
- **gwt-tauri**: Tauri v2 バックエンドバイナリクレート。AppState を管理し、Tauri Commands/Events を提供する
- **gwt-gui**: Svelte 5 フロントエンドプロジェクト。Tauri バックエンドと IPC で通信する

## 成功基準 *(必須)*

### 測定可能な成果

- **SC-001**: `cargo build` がワークスペースルートで 0 エラーで完了する
- **SC-002**: `cargo test -p gwt-core` の全テストが通過する
- **SC-003**: `cargo clippy --all-targets --all-features -- -D warnings` が 0 警告で完了する
- **SC-004**: gwt-core の依存ツリーに ratatui, vt100, crossterm が含まれない（`cargo tree -p gwt-core` で確認）
- **SC-005**: `npm run tauri dev`（または同等コマンド）でデスクトップウィンドウが表示される

## 制約と仮定 *(該当する場合)*

### 制約

- 現在のブランチ（feature/multi-terminal）で作業を完結する（新規ブランチ作成禁止）
- gwt-core の PTY 管理（portable-pty）、スクロールバック、IPC 機能は維持する
- Tauri v2 の安定版を使用する

### 仮定

- macOS 環境で開発・検証を行い、Windows/Linux 対応は Phase 4 で実施する
- Node.js >= 18 が開発環境にインストールされている
- Rust stable toolchain が利用可能である
- xterm.js によるターミナルエミュレーションは Phase 2 で統合するため、Phase 1 では不要

## 範囲外 *(必須)*

次の項目は、この機能の範囲外です：

- ターミナルエミュレーション統合（xterm.js + PTY streaming）— Phase 2
- ブランチリストの実データ表示（Tauri Command 連携）— Phase 2
- セッション要約タブの実データ表示 — Phase 2
- エージェント起動フォーム — Phase 2
- 設定画面の実装（メニューから開く画面の中身）— Phase 3
- 旧コード（gwt-cli, gwt-web, gwt-frontend）の物理削除 — Phase 4
- GitHub Actions CI/CD 更新 — Phase 4
- クロスプラットフォームビルド検証 — Phase 4

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- Tauri v2 の CSP（Content Security Policy）を適切に設定し、任意のスクリプト実行を防止する
- Tauri IPC の allowlist を最小権限で設定する

## 依存関係 *(該当する場合)*

- Tauri v2 CLI (`@tauri-apps/cli`)
- Svelte 5 + Vite
- portable-pty（gwt-core が引き続き使用）

## 参考資料 *(該当する場合)*

- [Tauri v2 公式ドキュメント](https://v2.tauri.app/)
- [Svelte 5 公式ドキュメント](https://svelte.dev/docs)
- [xterm.js 公式](https://xtermjs.org/)（Phase 2 で使用）
