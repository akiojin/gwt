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
   HTTP/WebSocket サーバーが立ち上がる。 同時に macOS / Linux では
   プロジェクトに紐づく `gwtd daemon` が auto-bootstrap し、
   Unix ドメインソケットで待ち受ける
2. WebView は同じサーバーへ接続し、キャンバス上のウィンドウ状態を
   同期する
3. `Shell` / `Agent` ウィンドウは PTY を通してプロセスを実行する
4. `Branches` / `File Tree` / `Issue` などのウィンドウは Rust 側で
   データを集約し、フロントエンドへ送る
5. Board に投稿が起きると、 GUI ハンドラが local の workspace-state を
   更新したあと daemon に `ClientFrame::Publish { channel: "board", .. }`
   を発火する。 同じプロジェクトの全 `gwt` インスタンスが
   `DaemonFrame::Event` を購読していれば、 polling 遅延なく UI に
   反映される (SPEC-2077 Phase H1)
6. `gwtd issue ...` などの CLI サブコマンドは GUI を起動せず、
   同じバイナリ内で完結する。 Windows では daemon は未実装のため
   multi-instance fan-out は行われず、 `gwtd daemon status` は
   常に `stopped` を返す

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
  - `~/.gwt/projects/<repo-hash>/runtime/daemon/endpoint.json`
  - クラッシュした daemon の stale エントリは次回の `gwtd daemon
    start` / GUI 起動時に自動でクリーンアップされる
