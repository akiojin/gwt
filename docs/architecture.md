# Architecture

gwt は二つの Rust バイナリ — `gwt` (GUI フロントドア) と `gwtd`
(CLI / 常駐ランタイム) — として配布されます。両者は同じワークスペース
からビルドされ、`gwtd daemon` が macOS / Linux 上ではバックグラウンドで
プロジェクト単位の runtime daemon としても動作します (SPEC-2077)。

## Components

- `crates/gwt/`
  - **`gwt`**: Wry + Tao ベースのネイティブ GUI、ローカル
    HTTP/WebSocket サーバー、WebView / ブラウザ共通のキャンバス UI、
    フックの outward-facing 入口 (`gwt hook ...`)
  - **`gwtd`**: 同じクレート内で生成される CLI バイナリ
    (`gwtd issue ...` / `gwtd pr ...` / `gwtd board ...` /
    `gwtd actions ...` / `gwtd daemon ...`)。`gwtd daemon start` は
    Unix ドメインソケット経由でプロジェクト単位の runtime daemon を
    bootstrap する (SPEC-2077 Phase H1)
- `crates/gwt-core/`
  - Git / worktree / 設定 / ログ / coordination / index ランタイム、
    daemon 契約 (`DaemonEndpoint` / `ClientFrame` / `DaemonFrame` の
    ワイヤスキーマ)
- `crates/gwt-github/`
  - SPEC Issue の取得・更新・ローカルキャッシュ
- `crates/gwt-agent/`
  - エージェントセッション状態とランタイムメタデータ
- `crates/gwt-terminal/`
  - PTY とプロセスウィンドウ管理

## Data Flow

1. `gwt` を GUI モードで起動すると、ネイティブウィンドウとローカル
   HTTP/WebSocket サーバーが立ち上がる。 macOS / Linux では同時に
   `prepare_daemon_front_door_for_path` がプロジェクトの daemon
   endpoint メタデータを書き込む (`bind: "internal://gwt-front-door"`
   sentinel) — ただしこれは endpoint ファイルの記録だけで、 IPC
   サーバーは起動しない
2. WebView は同じサーバーへ接続し、キャンバス上のウィンドウ状態を
   同期する
3. `Shell` / `Agent` ウィンドウは PTY を通してプロセスを実行する
4. `Branches` / `File Tree` / `Issue` などのウィンドウは Rust 側で
   データを集約し、フロントエンドへ送る
5. multi-instance 同期 (例: Board 投稿の即時反映) を有効にするには、
   ユーザーが明示的に `gwtd daemon start` を実行して常駐 IPC
   サーバーを立ち上げる。 daemon が live な間、 GUI ハンドラは
   `daemon_publisher::publish_event` で `ClientFrame::Publish {
   channel: "board", .. }` を発火し、 同じプロジェクトの他
   インスタンスが `DaemonFrame::Event` を購読していれば polling
   遅延なく UI に反映される (SPEC-2077 Phase H1)。 daemon が起動
   していない場合は publish が `Err("daemon not running")` を返し、
   ローカルの workspace-state が引き続き source of truth となる
6. `gwtd issue ...` などの CLI サブコマンドは GUI を起動せず、
   同じバイナリ内で完結する。 Windows では daemon は未実装で
   `gwtd daemon start` が "not yet implemented" を返すため、
   multi-instance fan-out は利用できない (`gwtd daemon status` は
   常に `stopped` を返す)

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
  - `~/.gwt/projects/<repo-hash>/logs/gwt.log.YYYY-MM-DD`
- Runtime daemon endpoint (macOS / Linux):
  - `~/.gwt/projects/<repo-hash>/runtime/daemon/<worktree-hash>.json`
    (filename comes from `RuntimeScope::endpoint_path` in
    `crates/gwt-core/src/daemon.rs:101`)
  - クラッシュした daemon の stale エントリは次回の `gwtd daemon
    start` / GUI 起動時に `resolve_bootstrap_action` が `is_alive`
    チェックでクリーンアップする
