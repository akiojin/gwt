# Troubleshooting

## アプリが起動できない / 白画面になる

1. ログを確認します。

- `~/.gwt/logs/`
- `stderr` に出力される `http://127.0.0.1:<port>/` の URL を通常のブラウザで開き、WebView 固有の問題かどうかを切り分けます。

1. 開発環境では GUI を直接起動して確認します。

```bash
cargo run -p gwt
```

## 設定が壊れている/読み込めない

設定ファイルを一時退避して、起動確認します。

- `~/.gwt/config.toml`
- `~/.gwt/profiles.yaml`

## プロジェクトを開けない（Git リポジトリとして認識されない）

- 選択したディレクトリが Git リポジトリであることを確認してください。
- bare リポジトリを直接開くのではなく、worktree（作業ツリー）側のディレクトリを選択してください。

## CLI 実行で GUI が開く

- `gwt issue ...` / `gwt pr ...` / `gwt actions ...` / `gwt board ...` / `gwt hook ...` は GUI を起動しません。
- GUI が開いてしまう場合は、実行しているバイナリが古い可能性があります。`which gwt` と `gwt --help` 相当の配置を確認し、最新の `gwt` バイナリへ置き換えてください。

## Windows: Host OS 起動でタブが空白になる / 入力できない

以下は Issue #1029 の再発確認手順です。

1. 対象プロジェクトを開き、Agent 起動時の Runtime を `Host OS` に設定します。
2. 起動を実行し、失敗ケースを再現します（旧バージョンからのマイグレーション済みプロジェクトを含む）。
3. 失敗時に terminal タブへ `PTY stream error` を含むメッセージと `Press Enter to close this tab.` が表示されることを確認します。
4. terminal タブをアクティブにして `Enter` を押し、タブが閉じることを確認します。
5. 表示されない / 閉じられない場合は `~/.gwt/logs/` の該当時刻ログを採取し、起動条件（branch / profile / runtime）とあわせて記録します。
