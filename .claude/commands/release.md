# Release Command

`/release` コマンドは develop ブランチを release ブランチへ同期し、release ブランチに対して semantic-release ワークフローを起動しつつ、release→main の PR を自動マージ待機状態に設定します。

## 実行手順

1. develop ブランチを最新化し、リリース対象コミットが揃っていることを確認します。
2. Claude Code で `/release` コマンドを実行するか、CLI で `gh workflow run release-trigger.yml --ref develop -f confirm=release` を実行します。
3. release ブランチへの push が完了すると、自動的に release ワークフロー (`.github/workflows/release.yml`) が起動し、`lint` → `test` → `semantic-release` の Required チェックが走ります。
4. release→main PR が作成（または更新）され Auto Merge が有効化されます。Required チェックが全て成功すると人手なしで main へマージされます。

## 実行内容

release-trigger ワークフローは以下を自動化します:

1. develop ブランチをチェックアウトし、release ブランチへ fast-forward push（`git push origin develop:release --force-with-lease`）。
2. release と main の diff を記録し、リカバリー用ログを Step Summary に保存します。
3. release→main の PR を作成または更新し、`release` / `auto-merge` ラベルと release ワークフローの監視リンクを付与します。
4. `gh pr merge --auto --merge` を実行し、Required チェック（lint/test/semantic-release）のみを条件とした Auto Merge を設定します。

## 注意事項

- release ワークフローは release ブランチの push と `workflow_dispatch` のみで起動します。main への直接 push は Branch Protection で禁止されています。
- Required チェックは `lint`、`test`、`semantic-release` の 3 つのみです。これらが失敗した場合は GitHub Actions から再実行し、必要に応じて `/release` を再実行してください。
- release→main PR は Auto Merge 前提のためレビュー必須にはしていません。監査が必要な場合は一時的に Auto Merge を解除してください。
- npm / GitHub Release の公開は semantic-release job が担当します。main には PR が自動マージされた時点で変更が反映されます。
