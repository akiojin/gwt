# タスク一覧: マルチターミナル（gwt内蔵ターミナルエミュレータ）

**仕様ID**: `SPEC-1d6dd9fc`
**作成日**: 2026-02-08

## Layer 1: 基盤

### T001: VT100 crateの選定と検証

- **依存**: なし
- **対象ファイル**: 検証用コード（一時）、spec.md（結果反映）
- **内容**:
  - alacritty_terminal crateの評価（API、依存サイズ、ratatuiセル変換の容易さ）
  - vt100 crateの評価（同上）
  - 両者でANSIカラー、カーソル制御、フルスクリーンアプリ（vim相当）の出力をパースし、ratatuiバッファに変換するベンチマーク
  - 選定結果をspec.md FR-005に記載
- **TDD**: ベンチマークコードで性能を計測。選定基準を定量的に記録
- **受け入れ条件**: 選定crateが決定し、spec.mdに反映されている

### T002: エラー型定義（error.rs）

- **依存**: なし
- **対象ファイル**: `crates/gwt-core/src/terminal/error.rs`, `crates/gwt-core/src/terminal/mod.rs`
- **内容**:
  - TerminalError enum定義（PtyCreationFailed, PtyIoError, EmulatorError, ScrollbackError, IpcError, PaneLimitReached）
  - thiserror派生
  - gwt-core/src/terminal/mod.rs モジュール登録
- **TDD**: エラー型のDisplay、From変換のテスト
- **受け入れ条件**: cargo test通過、clippy警告なし

### T003: PTY管理モジュール（pty.rs）

- **依存**: T002
- **対象ファイル**: `crates/gwt-core/src/terminal/pty.rs`
- **内容**:
  - PtyHandle構造体（portable-pty crateまたはOS固有API）
  - PTY作成（コマンド、作業ディレクトリ、環境変数設定）
  - 非同期I/O（tokio）：読み込み（PTY→アプリ）、書き込み（アプリ→PTY）
  - WINSIZEリサイズ
  - SIGTERM送信
  - PTYクリーンアップ（drop時）
  - 環境変数設定（TERM, GWT_PANE_ID, GWT_BRANCH, GWT_AGENT, GWT_SOCKET_PATH, GWT_SESSION_ID, GWT_WORKTREE_PATH, GWT_LOG_FILE）
- **TDD**:
  - PTY作成と`echo hello`実行→出力読み取りのテスト
  - 環境変数が正しく設定されているかのテスト
  - WINSIZEリサイズのテスト
  - プロセス終了検出のテスト
- **受け入れ条件**: PTYで簡単なコマンドを実行し、出力を読み取れる

## Layer 2: コアエンジン

### T004: VT100エミュレータラッパー（emulator.rs）

- **依存**: T001, T002
- **対象ファイル**: `crates/gwt-core/src/terminal/emulator.rs`
- **内容**:
  - TerminalEmulator trait定義（process_input, get_cell, get_cursor_pos, resize, get_size）
  - T001で選定したcrateのラッパー実装
  - ANSIカラー（256色+TrueColor）処理
  - カーソル制御処理
  - BEL文字（\x07）検出とコールバック
  - マウスイベント処理
- **TDD**:
  - ANSIカラーシーケンス入力→セルカラー検証
  - カーソル移動シーケンス→カーソル位置検証
  - BEL文字検出テスト
  - 画面クリアシーケンステスト
  - リサイズテスト
- **受け入れ条件**: 基本的なANSIシーケンスが正しく解釈される

### T005: ファイルベーススクロールバック（scrollback.rs）

- **依存**: T002
- **対象ファイル**: `crates/gwt-core/src/terminal/scrollback.rs`
- **内容**:
  - ScrollbackFile構造体
  - 非同期ファイル書き込み（BufWriter + tokio::fs）
  - ファイルからの行範囲読み込み（スクロールバック用）
  - ファイルパス生成（`~/.gwt/terminals/{pane-id}.log`）
  - ディレクトリ作成（`~/.gwt/terminals/`）
  - gwt終了時のクリーンアップ（ディレクトリ内全ファイル削除）
  - ディスク容量不足時の警告
- **TDD**:
  - 書き込み→読み込み一致テスト
  - 大量書き込み（10000行）→任意行範囲読み込みテスト
  - クリーンアップテスト
  - 同時書き込みと読み込みの非干渉テスト
- **受け入れ条件**: 10000行以上の書き込みと任意位置読み込みが正常動作

### T006: VT100→ratatuiレンダラー（renderer.rs）

- **依存**: T004
- **対象ファイル**: `crates/gwt-core/src/terminal/renderer.rs`
- **内容**:
  - render_to_buffer関数（VT100セルバッファ→ratatui::buffer::Buffer変換）
  - セル属性変換（fg/bg色、bold/italic/underline等）
  - カーソル位置情報の提供
  - （初期は毎フレーム全変換、パフォーマンス問題発生時にdirty領域最適化）
- **TDD**:
  - 空画面の変換テスト
  - ANSIカラーセルの変換正確性テスト
  - 属性（bold, italic等）の変換テスト
  - 全画面サイズ（例: 120x40）の変換パフォーマンステスト（16ms以内）
- **受け入れ条件**: VT100バッファの内容がratatuiバッファに正確に反映される

## Layer 3: ペイン管理

### T007: TerminalPane構造体（pane.rs）

- **依存**: T003, T004, T005, T006
- **対象ファイル**: `crates/gwt-core/src/terminal/pane.rs`
- **内容**:
  - TerminalPane構造体（pane_id, pty, emulator, scrollback, branch_name, agent_name, agent_color, status, started_at, log_file_path）
  - PaneStatus enum（Running, Completed(i32), Error(String)）
  - 非同期I/Oループ（PTY出力→VT100パース→スクロールバック書き込み→描画通知）
  - PTYへの入力送信
  - プロセス終了検出→ステータス更新
  - リサイズ処理（VT100+PTY同時更新）
- **TDD**:
  - ペイン作成→コマンド実行→出力がエミュレータに反映されるテスト
  - 入力送信→PTYに到達するテスト
  - プロセス終了→ステータスがCompleted/Errorに変化するテスト
  - リサイズ→VT100とPTYのサイズが一致するテスト
- **受け入れ条件**: TerminalPaneがコマンドを実行し、出力を表示し、終了を検出できる

### T008: PaneManager（manager.rs）

- **依存**: T007
- **対象ファイル**: `crates/gwt-core/src/terminal/manager.rs`
- **内容**:
  - PaneManager構造体（panes, active_index, max_panes=4, is_fullscreen）
  - create_pane（コマンド、ブランチ名、エージェント名等）→TerminalPane作成
  - close_pane（pane_id）→SIGTERM送信→ペイン削除
  - next_tab / prev_tab
  - get_active_pane / get_pane_by_id
  - 4ペイン上限チェック
  - 自動クローズ処理（プロセス終了検出→ペイン削除）
  - 全ペイン閉鎖検出（全画面復帰のトリガー）
  - フルスクリーントグル
  - 全ペインへのSIGTERM送信（gwt終了時）
  - SIGWINCHハンドラ（全ペインリサイズ）
- **TDD**:
  - ペイン作成→一覧に追加されるテスト
  - 4ペイン作成→5つ目がエラーになるテスト
  - タブ切り替え（next/prev）のインデックス管理テスト
  - ペイン自動クローズ→アクティブインデックス調整テスト
  - 全ペイン閉鎖検出テスト
  - フルスクリーントグルテスト
- **受け入れ条件**: 複数ペインのライフサイクルが正しく管理される

## Layer 4: UI統合

### T009: ターミナルペインWidget（terminal_pane.rs）

- **依存**: T006, T008
- **対象ファイル**: `crates/gwt-cli/src/tui/screens/terminal_pane.rs`
- **内容**:
  - TerminalPaneWidget（ratatui Widget trait実装）
  - ステータスバー描画：ブランチ名 + エージェント名（SPEC-3b0ed29bカラー） + ステータス + 経過時間
  - タブバー描画：全タブ一覧 + アクティブハイライト
  - フォーカスインジケータ（ボーダーカラー変更等）
  - VT100バッファのratatuiレンダリング
- **TDD**:
  - Widgetレンダリング→期待されるバッファ内容のテスト
  - ステータスバーの各要素が正しく表示されるテスト
  - タブバーのアクティブハイライトテスト
  - フォーカスインジケータのテスト
- **受け入れ条件**: ターミナルペインがratatuiウィジェットとして正しく描画される

### T010: レイアウト改修（split_layout.rs）

- **依存**: T009
- **対象ファイル**: `crates/gwt-cli/src/tui/screens/split_layout.rs`
- **内容**:
  - 左右50:50分割レイアウトの実装
  - ターミナルペインの有無による動的レイアウト切り替え（全画面↔分割）
  - フルスクリーンモード（右側ペインのみ全画面）
  - 80列未満フォールバック（フルスクリーンターミナル）
  - 横幅変更時のフォールバック復帰
- **TDD**:
  - ペインなし→全画面ブランチリストのレイアウトテスト
  - ペインあり→50:50分割のレイアウトテスト
  - フルスクリーンモード→全画面ペインのレイアウトテスト
  - 79列→フルスクリーンフォールバックのテスト
  - 80列→50:50復帰のテスト
- **受け入れ条件**: レイアウトが条件に応じて正しく切り替わる

### T011: イベント処理改修（app.rs, event.rs）

- **依存**: T008, T010
- **対象ファイル**: `crates/gwt-cli/src/tui/app.rs`, `crates/gwt-cli/src/tui/event.rs`
- **内容**:
  - PrefixCommand enum定義（NextTab, PrevTab, CopyMode, Paste, ClosePane, FullscreenToggle, Cancel, SendPrefix）
  - フォーカス状態管理（GwtUi / TerminalPane）
  - Ctrl+G処理：プレフィックスモード切り替え
  - プレフィックスモード中のコマンド解釈（n,p,[,],x,z,Esc,Ctrl+G）
  - TerminalPaneフォーカス時のPTY透過入力
  - マウスクリックによるフォーカス切り替え
  - SIGWINCHシグナル処理→PaneManagerリサイズ通知
- **TDD**:
  - Ctrl+G→プレフィックスモード移行テスト
  - プレフィックスモード→n→NextTabコマンド発行テスト
  - プレフィックスモード→Esc→キャンセルテスト
  - Ctrl+G→Ctrl+G→Ctrl+Gリテラル送信テスト
  - TerminalPaneフォーカス時のキー透過テスト
  - マウスクリックフォーカス切り替えテスト
- **受け入れ条件**: 全プレフィックスコマンドが正しく動作し、フォーカス管理が正常

### T012: エージェント起動フロー改修

- **依存**: T008, T011
- **対象ファイル**: `crates/gwt-core/src/tmux/launcher.rs`（改修）、関連ファイル
- **内容**:
  - 内蔵ターミナルモードの起動パス追加
  - PaneManager経由でTerminalPaneを作成
  - エージェントコマンドの構築（既存build_agent_command流用）
  - worktreeディレクトリの設定
  - tmuxモードとの設定ベース切り替え
- **TDD**:
  - 内蔵ターミナルモードでエージェント起動→ペイン作成テスト
  - 環境変数設定の正確性テスト
  - 作業ディレクトリ設定テスト
- **受け入れ条件**: ブランチリストからエージェントを起動し、内蔵ターミナルで表示される

## Layer 5: 高度な機能

### T013: ペイン間通信（ipc.rs）

- **依存**: T008
- **対象ファイル**: `crates/gwt-core/src/terminal/ipc.rs`
- **内容**:
  - PaneIpc構造体
  - Unixドメインソケットサーバー（`~/.gwt/gwt.sock`、パーミッション0700）
  - send_keys（pane_id, keys）→対象ペインのPTYに入力送信
  - pipe_output（source_pane_id）→出力ストリームの提供
  - SharedChannel（名前付きチャネル）の作成、書き込み、読み取り、削除
  - ソケット作成失敗時のフォールバック（IPC無効化、基本機能は動作）
  - gwt終了時のソケットクリーンアップ
- **TDD**:
  - send_keys→対象ペインのPTY入力に反映されるテスト
  - pipe_output→出力ストリームが正しく転送されるテスト
  - SharedChannel作成→書き込み→読み取りテスト
  - ソケット作成失敗→フォールバック動作テスト
  - クリーンアップテスト
- **受け入れ条件**: ペイン間でキー送信、出力パイプ、チャネル通信が動作する

### T014: コピー&ペースト

- **依存**: T004, T011
- **対象ファイル**: `crates/gwt-cli/src/tui/screens/terminal_pane.rs`（改修）、新規コピーモード処理
- **内容**:
  - コピーモード（Ctrl+G → [）
    - VT100バッファ上のカーソル移動（矢印キー、PageUp/Down）
    - 選択開始（Space）、選択範囲ハイライト
    - 選択確定（Enter）→arboard経由でクリップボード
    - コピーモード離脱（Esc/q）
  - ペースト（Ctrl+G → ]）→arboard読み取り→PTY送信
  - Shift+マウスドラッグのパススルー
- **TDD**:
  - コピーモード突入→カーソル移動テスト
  - 選択開始→範囲選択→確定→クリップボード内容テスト
  - ペースト→PTY入力テスト
- **受け入れ条件**: テキスト選択→コピー→ペーストの完全フローが動作する

### T015: Docker統合

- **依存**: T007, T012
- **対象ファイル**: `crates/gwt-core/src/docker/manager.rs`（改修）
- **内容**:
  - docker exec PTY対応（`docker exec -it <container> <command>`形式でPTY作成）
  - 既存DockerManagerのコンテナ起動フローとの統合
  - コンテナ停止検出→ペイン自動クローズ
- **TDD**:
  - docker exec PTY作成テスト（Docker環境が必要なため統合テスト）
  - コンテナ停止→ペインクローズテスト
- **受け入れ条件**: Dockerコンテナ内のエージェントが内蔵ターミナルで動作する

### T016: ホストターミナルリサイズ統合

- **依存**: T008, T010
- **対象ファイル**: `crates/gwt-cli/src/tui/app.rs`（改修）
- **内容**:
  - SIGWINCH処理パイプラインの実装
  - crossterm resize event → PaneManager全ペインリサイズ
  - レイアウト再計算（50:50の各ペインサイズ更新）
  - フォールバック閾値（80列）の動的チェック
- **TDD**:
  - リサイズイベント→VT100サイズ更新テスト
  - リサイズイベント→PTY WINSIZEテスト
  - 80列未満→フォールバックテスト
  - 80列以上→復帰テスト
- **受け入れ条件**: ホストリサイズ時にペイン内アプリが正しく再描画される

### T017: tmux統合の段階的廃止

- **依存**: T012
- **対象ファイル**: `crates/gwt-core/src/tmux/launcher.rs`（改修）、設定ファイル
- **内容**:
  - 設定項目追加（terminal_mode: "builtin" | "tmux"、デフォルト: "builtin"）
  - tmuxモード選択時の既存フロー維持
  - 内蔵ターミナルモード選択時のtmux依存スキップ
  - tmux関連コードへのdeprecated注記
- **TDD**:
  - terminal_mode=builtin→内蔵ターミナル使用テスト
  - terminal_mode=tmux→tmux使用テスト
  - デフォルト値がbuiltinであるテスト
- **受け入れ条件**: 設定で内蔵ターミナル/tmuxを切り替えられる

## 依存関係グラフ

```text
T001 ──┐
       ├──→ T004 ──→ T006 ──→ T007 ──→ T008 ──→ T011 ──→ T012 ──→ T015
T002 ──┤                              ↗         ↗  ↑       ↗       T017
       ├──→ T003 ─────────────────────          │  │      │
       └──→ T005 ──────────────────────────────→│  │      │
                                                   │      │
T009 ←── T006 + T008                               │      │
T010 ←── T009                                      │      │
                                                   │      │
T013 ←── T008                                      │      │
T014 ←── T004 + T011                               │      │
T016 ←── T008 + T010                               │      │
```

## 並列化可能なタスク

- T001 + T002: 完全に独立
- T003 + T005: T002完了後、並列実行可能
- T004 + T005: T001完了後（T004）、T002完了後（T005）で並列実行可能
- T013 + T014 + T015 + T016 + T017: Layer 4完了後、すべて並列実行可能
