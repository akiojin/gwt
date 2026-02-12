# Troubleshooting

## アプリが起動できない / 白画面になる

1. ログを確認します。

- `~/.gwt/logs/`

1. 開発環境の場合は、フロントエンド依存関係と Tauri の起動順を確認します。

```bash
cd gwt-gui
npm ci

cd ..
cargo tauri dev
```

## 設定が壊れている/読み込めない

設定ファイルを一時退避して、起動確認します。

- `~/.gwt/config.toml`
- `~/.gwt/profiles.yaml`

## プロジェクトを開けない（Git リポジトリとして認識されない）

- 選択したディレクトリが Git リポジトリであることを確認してください。
- bare リポジトリを直接開くのではなく、worktree（作業ツリー）側のディレクトリを選択してください。
