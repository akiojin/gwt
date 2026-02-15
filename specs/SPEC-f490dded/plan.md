# 実装計画: シンプルターミナルタブ

**仕様ID**: `SPEC-f490dded` | **日付**: 2026-02-14 | **仕様書**: `specs/SPEC-f490dded/spec.md`

## 目的

- Tools > New Terminal メニューおよび Ctrl+` ショートカットで、素のシェル（bash/zsh）ターミナルタブを起動可能にする
- 既存のエージェントタブと統一された UX を提供しつつ、エージェント起動フローをバイパスする
- タブ並び替え D&D を Tauri WebView でも安定動作させ、タブバー外 pointermove ケースで no-op にならないようにする

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`, `crates/gwt-core/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **ターミナル**: xterm.js v6（TerminalView.svelte）
- **テスト**: cargo test / vitest
- **前提**: PaneManager に `spawn_shell()` メソッドを新設し、`launch_agent()` のブランチマッピング保存をスキップする

## 実装方針

### Phase 1: バックエンド - PTY 生成とメニュー

#### 1-1. PaneManager に `spawn_shell()` メソッドを追加

`crates/gwt-core/src/terminal/manager.rs` に新メソッドを追加する。

- `launch_agent()` のロジックを再利用しつつ、`save_branch_mapping()` をスキップ
- 引数: `config: BuiltinLaunchConfig, rows: u16, cols: u16`（repo_root 不要）
- pane_id 生成、PaneConfig 構築、TerminalPane::new() は同一フロー

#### 1-2. Tauri コマンド `spawn_shell` の追加

`crates/gwt-tauri/src/commands/terminal.rs` に新しいコマンドを追加する。

- 引数: `working_dir: Option<String>`（省略時はホームディレクトリ）
- `$SHELL` 環境変数からシェルを解決、未設定なら `/bin/sh`
- PaneManager の `spawn_shell()` を呼び出し（command=シェル、args=空、agent_name="terminal"）
- I/O リーダースレッドを起動（既存の `stream_pty_output()` を再利用）
- pane_id を返却

#### 1-3. AgentColor の利用方針

- バックエンドでは `AgentColor::White` を使用（既存 enum に追加不要）
- フロントエンドの MainArea.svelte で `tab.type === "terminal"` の場合にドットカラーを `--text-muted` にオーバーライド

#### 1-4. Tools メニューに "New Terminal" 追加

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

## リスク

| ID | リスク | 影響 | 軽減策 |
|---|---|---|---|
| RISK-001 | Ctrl+` が Tauri accelerator で非対応 | US2 が動作しない | フロントエンド keydown フォールバックで代替 |
| RISK-002 | OSC 7 がバッファ境界で分断 | cwd 更新が欠落 | pane 単位の不完全シーケンスバッファ |
| RISK-003 | bash が OSC 7 を送出しない | bash ユーザーの cwd 追従不可 | グレースフルデグレード（起動時 cwd を固定表示） |
| RISK-004 | stream_pty_output() の変更が既存エージェントタブに影響 | 既存機能の退行 | agent_name=="terminal" の pane のみ OSC 7 処理を適用 |

## 依存関係

- Phase 1 の `spawn_shell()` メソッドが Phase 2〜4 の全てのベース
- Phase 2 の OSC 7 パーサーは Phase 3 の cwd ラベル更新に必要
- Phase 3 の Tab 型拡張は Phase 4 の永続化に必要
- Phase 4 の Window メニュー統合は既存の `sync_window_agent_tabs` に依存

## マイルストーン

| マイルストーン | 内容 | 完了条件 |
|---|---|---|
| M1: 最小動作 | メニューからシェル起動・入出力可能 | SC-001 |
| M2: UI 完成 | グレードット・cwd ラベル・ツールチップ | US3 全シナリオ |
| M3: cwd 追従 | OSC 7 パースによるリアルタイム更新 | SC-002 |
| M4: ライフサイクル | exit 自動クローズ・プロジェクト連動 | SC-003 |
| M5: 永続化 | アプリ再起動後の復元 | SC-004 |
| M6: メニュー統合 | Window メニューにターミナルタブ表示 | SC-005 |

## テスト

### バックエンド

- OSC 7 パーサーのユニットテスト（正常パス、ST のバリエーション、不正入力、URL デコード）
- `spawn_shell` コマンドの統合テスト（PTY 生成、I/O、クローズ）

### フロントエンド

- Tab 型の terminal タイプのレンダリングテスト（ドット色、ラベル表示）
- cwd 変更時のラベル更新テスト
- ターミナルタブの永続化・復元テスト
- タブ並び替え D&D の回帰テスト（`window` 配信 pointermove ケース）

## 追加修正（2026-02-14）: タブ D&D no-op 対策

### 目的

- Tauri 実行時に HTML DragEvent が安定しない環境でも、タブ順序の変更を pointer イベントだけで完結させる

### 実装

1. `MainArea.svelte` のドラッグ追跡をタブバー要素依存から `window` リスナー依存へ変更する
2. ドラッグ開始時に `window` の `pointermove / pointerup / pointercancel` を購読し、終了時に必ず解除する
3. close ボタン押下時はドラッグ状態を開始しないガードを追加する
4. ネイティブ DragEvent 依存を弱めるためタブの `draggable` を無効化し、pointer ベース挙動を主経路にする

### 検証

- `MainArea.test.ts` に `window` へ dispatch された pointermove で `onTabReorder` が呼ばれる再現テストを追加（RED→GREEN）
