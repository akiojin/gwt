# 実装計画: マルチターミナル（gwt内蔵ターミナルエミュレータ）

**仕様ID**: `SPEC-1d6dd9fc`
**作成日**: 2026-02-08

## 実装アプローチ

一括実装。全ユーザーストーリー（US1-US10）を単一フェーズで実装する。
ただし、依存関係に基づいた実装順序は厳守する。

## モジュール構成

### gwt-core側（新規モジュール: `terminal/`）

```text
crates/gwt-core/src/terminal/
├── mod.rs              # モジュールエクスポート
├── pty.rs              # PTY管理（作成、I/O、リサイズ、クリーンアップ）
├── emulator.rs         # VT100エミュレータラッパー（crate選定結果を抽象化）
├── pane.rs             # TerminalPane構造体とライフサイクル管理
├── manager.rs          # PaneManager（複数ペイン管理、タブ切り替え）
├── scrollback.rs       # ファイルベーススクロールバック
├── ipc.rs              # ペイン間通信（Unixドメインソケット、チャネル）
├── renderer.rs         # VT100バッファ→ratatui Buffer変換
└── error.rs            # ターミナル関連エラー型
```

### gwt-cli側（既存モジュール改修 + 新規）

```text
crates/gwt-cli/src/tui/
├── app.rs              # 改修: SplitLayout状態管理、フォーカス制御
├── event.rs            # 改修: プレフィックスキー処理、入力ルーティング
├── screens/
│   ├── split_layout.rs # 改修: 左右50:50レイアウト描画
│   └── terminal_pane.rs# 新規: ターミナルペイン描画Widget
└── widgets/
    └── tab_bar.rs      # 新規: タブバーWidget
```

### 既存tmux統合（段階的廃止）

```text
crates/gwt-core/src/tmux/
├── launcher.rs         # 改修: 内蔵ターミナルモードの分岐追加
└── ...                 # 他: 当面はそのまま維持
```

## 実装順序（依存関係に基づく）

### Layer 1: 基盤（依存なし）

1. **VT100 crateの選定と検証**
   - alacritty_terminal, vt100 crateの比較検証
   - ratatuiへの変換パフォーマンス測定
   - 選定結果をspec.mdに反映

2. **PTY管理モジュール（pty.rs）**
   - portable-pty crateまたはOS固有API
   - PTY作成、I/O読み書き、WINSIZE更新、SIGTERM送信
   - 環境変数設定（拡張セット）

3. **エラー型定義（error.rs）**

### Layer 2: コアエンジン（Layer 1に依存）

1. **VT100エミュレータラッパー（emulator.rs）**
   - 選定crateの抽象化レイヤー
   - 入力処理、出力パース、セルバッファ管理
   - BEL文字のホスト転送

2. **ファイルベーススクロールバック（scrollback.rs）**
   - 非同期ファイル書き込み（tokio::fs）
   - ファイルからの読み込み（スクロールバックモード用）
   - gwt終了時のクリーンアップ

3. **VT100→ratatuiレンダラー（renderer.rs）**
   - VT100セルバッファ→ratatui Bufferの変換
   - パフォーマンス計測と最適化判断

### Layer 3: ペイン管理（Layer 2に依存）

1. **TerminalPane構造体（pane.rs）**
   - PTY + VT100エミュレータ + スクロールバックの統合
   - ステータス管理（Running/Completed/Error）
   - 非同期I/Oループ

2. **PaneManager（manager.rs）**
   - 複数ペインのライフサイクル管理
   - タブ切り替え、アクティブペイン管理
   - 最大4ペイン制限
   - フルスクリーントグル状態管理

### Layer 4: UI統合（Layer 3に依存）

1. **ターミナルペインWidget（terminal_pane.rs）**
   - ratatui Widget traitの実装
   - ステータスバー描画（ブランチ名、エージェント名カラー、ステータス、経過時間）
   - タブバー描画
   - フォーカスインジケータ

2. **レイアウト改修（split_layout.rs）**
    - 左右50:50分割
    - フルスクリーンモード
    - 80列未満フォールバック

3. **イベント処理改修（app.rs, event.rs）**
    - プレフィックスキー（Ctrl+G）処理
    - フォーカス管理（左側UI ↔ 右側ペイン）
    - マウスクリックによるフォーカス切り替え
    - PTYへの透過的キー入力送信

4. **エージェント起動フロー改修（launcher.rs等）**
    - 内蔵ターミナルモードの追加
    - tmuxモードとの切り替え（設定ベース）

### Layer 5: 高度な機能（Layer 4に依存）

1. **ペイン間通信（ipc.rs）**
    - Unixドメインソケットサーバー
    - send-keys、pipe-pane、共有チャネル
    - フォールバック（ソケット作成失敗時）

2. **コピー&ペースト**
    - コピーモード（Ctrl+G → [）
    - テキスト選択、arboardクリップボード
    - ペースト（Ctrl+G → ]）
    - Shift+マウスパススルー

3. **Docker統合**
    - docker exec PTY対応
    - 既存DockerManagerとの統合

4. **ホストターミナルリサイズ**
    - SIGWINCH処理
    - VT100+PTYサイズ更新
    - フォールバック閾値処理

## 技術的リスクと対策

| リスク | 影響 | 対策 |
| ------ | ---- | ---- |
| VT100 crateのratatuiとの互換性 | 描画品質低下 | Layer 1で比較検証。最悪自前パーサ |
| PTY I/Oのレイテンシ | 描画遅延 | tokio非同期I/O + バッファリング |
| バッファ変換のパフォーマンス | フレームレート低下 | Dirty領域最適化を必要に応じて適用 |
| Unixソケットのパーミッション問題 | IPC不可 | IPC無効化フォールバック |
| 大量出力時のファイルI/O負荷 | ディスクI/Oボトルネック | BufWriterによるバッファリング |

## テスト戦略

### ユニットテスト

- VT100エミュレータ: ANSIシーケンス解釈の正確性
- PTY管理: PTY作成、I/O、リサイズ
- スクロールバック: ファイル書き込み、読み込み、クリーンアップ
- PaneManager: タブ切り替え、ライフサイクル、上限制御
- レンダラー: VT100→ratatuiバッファ変換の正確性
- IPC: send-keys、pipe-pane、共有チャネル

### 統合テスト

- エージェント起動→出力表示→終了→ペインクローズの完全フロー
- 複数ペインのタブ切り替え
- フォーカス切り替えと入力ルーティング
- コピー&ペーストフロー
- Docker環境でのエージェント実行

### 手動テスト（VT100エミュレーション検証）

- vim起動・編集・保存
- htop表示
- ANSIカラー出力確認
- リサイズ時の再描画確認
