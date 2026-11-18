# Architecture

gwt は同じ Rust workspace からビルドする `gwt`（GUI フロントドア）と
`gwtd`（JSON envelope CLI / 常駐 runtime）を配布します。

operation の使い方・件数・命名規則・互換性と daemon の起動方法・保存先は、
[README.md](../README.md#gwtd-operations-and-daemon) /
[README.ja.md](../README.ja.md#gwtd-operations-and-daemon) を正本とします。
このファイルは構成概要だけを扱います。

## Components

- `crates/gwt/`: Wry + Tao の GUI、HTTP/WebSocket サーバー、WebView / ブラウザ共通 UI、
  `gwtd` の JSON envelope dispatch と project runtime daemon。
- `crates/gwt-core/`: 設定・ログ・coordination・index と daemon の endpoint / IPC 契約。
- `crates/gwt-git/`: Git / worktree 操作。
- `crates/gwt-github/`: GitHub Issue / SPEC の取得・更新・キャッシュ。
- `crates/gwt-agent/`: エージェントセッション状態と runtime metadata。
- `crates/gwt-terminal/`: PTY とプロセス管理。

## Runtime

GUI はローカル HTTP/WebSocket サーバーで UI を提供し、プロジェクト daemon を監督します。
daemon は macOS / Linux の Unix domain socket と Windows の named pipe に対応し、
プロジェクトの実行 owner と IPC によるイベント配信を担います。
通常の `gwtd` 操作は stdin JSON envelope を使います。managed hook は専用 argv transport
を使います。具体例とプラットフォーム別の契約は上記 README の節を参照してください。
