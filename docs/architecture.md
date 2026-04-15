# Architecture

gwt は単一の Rust バイナリで GUI と CLI を提供します。

## Components

- `crates/gwt/`
  - Wry + Tao ベースのネイティブ GUI
  - ローカル HTTP/WebSocket サーバー
  - WebView / ブラウザ共通のキャンバス UI
  - `gwt issue ...` / `gwt pr ...` / `gwt hook ...` の CLI 入口
- `crates/gwt-core/`
  - Git / worktree / 設定 / ログ / coordination / index ランタイム
- `crates/gwt-github/`
  - SPEC Issue の取得・更新・ローカルキャッシュ
- `crates/gwt-agent/`
  - エージェントセッション状態とランタイムメタデータ
- `crates/gwt-terminal/`
  - PTY とプロセスウィンドウ管理

## Data Flow

1. `gwt` を GUI モードで起動すると、ネイティブウィンドウとローカル HTTP/WebSocket サーバーが立ち上がる
2. WebView は同じサーバーへ接続し、キャンバス上のウィンドウ状態を同期する
3. `Shell` / `Agent` ウィンドウは PTY を通してプロセスを実行する
4. `Branches` / `File Tree` / `Issue` などのウィンドウは Rust 側でデータを集約し、フロントエンドへ送る
5. `gwt issue ...` などの CLI サブコマンドは GUI を起動せず、同じバイナリ内で完結する

## Persistence

- GUI 状態:
  - `~/.gwt/workspace-state.json`
- 設定:
  - `~/.gwt/config.toml`
- プロファイル:
  - `~/.gwt/profiles.yaml`
- Issue / SPEC キャッシュ:
  - `~/.gwt/cache/issues/<repo-hash>/`
- ログ:
  - `~/.gwt/logs/`
