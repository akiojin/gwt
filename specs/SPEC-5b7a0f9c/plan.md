# 実装計画: Terminal ANSI Diagnostics（GUI）

## 目的

- アクティブなエージェントターミナルについて「ANSI/SGR/カラーSGR が実際に流れているか」を GUI 内で可視化する。
- branch->paneId インデックスの保存先をリポジトリ単位に分離し、プロジェクト横断の衝突を防ぐ。

## 実装方針

### バックエンド（gwt-core / gwt-tauri）

- `gwt-core`:
  - スクロールバックログの末尾 N bytes を読むユーティリティを追加（ログ全読を避ける）。
  - terminal index（branch->paneId）保存先を `repo_root` 単位に分離する。
- `gwt-tauri`:
  - `probe_terminal_ansi(pane_id)` を追加し、ログ末尾を解析してカウント/フラグを返す。
  - 解析は CSI の最終バイト判定で SGR（`...m`）のみを対象とし、色を伴う SGR を別カウントする。

### フロントエンド（gwt-gui）

- `Agent` メニューに `Terminal Diagnostics` を追加。
- アクティブタブが `agent` の場合に `probe_terminal_ansi` を呼び出して結果をオーバーレイ表示する。
- 結果に応じて、以下を表示する:
  - 色SGRなし: 対処コマンド（`git -c color.ui=always ...`、`rg --color=always ...`）を提示
  - 色SGRあり: 表示経路問題の可能性を示し、次の調査（xterm 書き込み API、renderer 等）を案内

## テスト

- `gwt-core`:
  - 末尾読み取りユーティリティのテスト（サイズ超過・未満・空ファイル）。
  - repo_root 分離後の index 保存/読み取りが衝突しないテスト。
- `gwt-tauri`:
  - ANSI 解析のユニットテスト（色SGRあり/なし、非SGR CSI、reset/italic 等）。

