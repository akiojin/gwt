# 実装計画: シンプルターミナルタブ

**仕様ID**: `SPEC-f490dded` | **日付**: 2026-02-13 | **仕様書**: `specs/SPEC-f490dded/spec.md`

## 目的

- Tools > New Terminal メニューおよび Ctrl+` ショートカットで、素のシェル（bash/zsh）ターミナルタブを起動可能にする
- 既存のエージェントタブと統一された UX を提供しつつ、エージェント起動フローをバイパスする

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`, `crates/gwt-core/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **ターミナル**: xterm.js v6（TerminalView.svelte）
- **テスト**: cargo test / vitest
- **前提**: PaneManager の `launch_agent()` はシェルコマンドでも動作する（command/args 差し替え）

## 実装方針

### Phase 1: バックエンド - PTY 生成とメニュー

#### 1-1. Tauri コマンド `spawn_shell` の追加

`crates/gwt-tauri/src/commands/terminal.rs` に新しいコマンドを追加する。

- 引数: `working_dir: Option<String>`（省略時はホームディレクトリ）
- `$SHELL` 環境変数からシェルを解決、未設定なら `/bin/sh`
- PaneManager の `launch_agent()` を呼び出し（command=シェル、args=空、agent_name="terminal"）
- I/O リーダースレッドを起動（既存の `start_pty_reader` を再利用）
- pane_id を返却

#### 1-2. AgentColor に Terminal 用カラーの追加

`crates/gwt-core/src/terminal/mod.rs` の `AgentColor` enum を確認し、グレー表現が可能か確認。既存の `White` を流用するか、フロントエンド側で `--text-muted` にマッピングする。

#### 1-3. Tools メニューに "New Terminal" 追加

`crates/gwt-tauri/src/menu.rs`:

- 定数 `MENU_ID_TOOLS_NEW_TERMINAL` を追加
- MenuItem を作成（label: "New Terminal"、accelerator: Ctrl+` を試行）
- Tools サブメニューの先頭に配置（Launch Agent... の前）
- `app.rs` のメニューイベントハンドラにディスパッチ追加

### Phase 2: バックエンド - OSC 7 パースと cwd 通知

#### 2-1. OSC 7 パーサーの実装

`crates/gwt-core/src/terminal/osc.rs`（新規ファイル）:

- PTY 出力のバイトストリームから `ESC ] 7 ; file://hostname/path ST` を検出
- ST（String Terminator）は `ESC \` または `BEL (0x07)` の両方に対応
- URL デコード（%XX）を処理してパスを取得
- パフォーマンス: バイトスキャンで `0x1b` を検出した場合のみパースロジックに入る

#### 2-2. PTY リーダーループへの OSC 7 検出の組み込み

`crates/gwt-tauri/src/commands/terminal.rs` の I/O リーダースレッド:

- 既存の `terminal-output` イベント発行ループに OSC 7 チェックを追加
- cwd が変化した場合のみ `terminal-cwd-changed` イベントを発行（pane_id, cwd）
- pane 単位で最後の cwd を保持し、重複イベントを抑制

### Phase 3: フロントエンド - タブタイプとUI

#### 3-1. Tab 型に `terminal` タイプを追加

`gwt-gui/src/lib/types.ts`:

- `Tab.type` に `"terminal"` を追加
- `Tab` に `cwd?: string` フィールドを追加

#### 3-2. App.svelte にターミナルタブ管理ロジックを追加

- `menu-action` ハンドラに `"new-terminal"` アクションを追加
- `spawn_shell` コマンドを呼び出し、返却された pane_id で新しい Tab を作成
- `terminal-cwd-changed` イベントのリスナーを追加し、対応するタブの cwd とラベルを更新
- `terminal-closed` イベントハンドラを拡張し、terminal タイプのタブも処理
- プロジェクトクローズ時のクリーンアップに terminal タブの PTY kill を追加

#### 3-3. MainArea.svelte のタブバーレンダリング拡張

- `terminal` タイプのタブにグレードット（`--text-muted`）を表示
- ラベルは `tab.cwd` の basename を表示（cwd 未設定時は "Terminal"）
- タブにホバー時、フルパスをツールチップ（title 属性）で表示
- × ボタンで `close_terminal` を呼び出し（既存のエージェントタブと同じフロー）
- `terminal` タイプのタブでも TerminalView をレンダリング（既存のターミナルレイヤーを利用、全機能有効）

### Phase 4: 永続化と復元

#### 4-1. ターミナルタブの永続化

`gwt-gui/src/lib/agentTabsPersistence.ts` を拡張:

- `StoredAgentTab` に `type?: "terminal"` と `cwd?: string` フィールドを追加
- terminal タブの保存時に最新の cwd を含める
- 復元時: terminal タブは新しい PTY を `spawn_shell(cwd)` で生成して復元

#### 4-2. Window メニューへの統合

`crates/gwt-tauri/src/commands/window_tabs.rs` の `SyncWindowAgentTabsRequest`:

- `WindowAgentTabEntry` に `tab_type?: String` フィールドを追加（省略時は "agent"）
- `menu.rs` の Window メニュー描画で、terminal タブも含めて一覧表示

## テスト

### バックエンド

- OSC 7 パーサーのユニットテスト（正常パス、ST のバリエーション、不正入力、URL デコード）
- `spawn_shell` コマンドの統合テスト（PTY 生成、I/O、クローズ）

### フロントエンド

- Tab 型の terminal タイプのレンダリングテスト（ドット色、ラベル表示）
- cwd 変更時のラベル更新テスト
- ターミナルタブの永続化・復元テスト
