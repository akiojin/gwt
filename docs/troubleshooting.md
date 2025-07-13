# Troubleshooting

## Clean up merged PRs が動作しない場合

マージ済みPRのクリーンアップが期待通りに動作しない場合は、以下のデバッグ手順を試してください。

### デバッグモードの有効化

環境変数 `DEBUG_CLEANUP=1` を設定してデバッグ情報を表示できます：

```bash
DEBUG_CLEANUP=1 claude-worktree
```

デバッグモードでは以下の情報が表示されます：

- 利用可能なworktreeのリスト
- GitHubから取得したマージ済みPRのリスト
- ブランチ名のマッチング結果
- GitHub API呼び出しの詳細

### よくある問題と解決方法

#### 1. ブランチ名のマッチング問題

worktreeのブランチ名とGitHub PRのブランチ名が一致しない場合があります。

**症状：**
- マージ済みPRが存在するのにクリーンアップ対象として表示されない

**解決方法：**
- デバッグモードでブランチ名を確認
- 必要に応じて手動でブランチを削除

#### 2. GitHub CLI認証エラー

GitHub CLIが認証されていない場合、PR情報を取得できません。

**症状：**
```
Error: Failed to fetch merged pull requests
Details: authentication required
```

**解決方法：**
```bash
gh auth login
```

#### 3. リモートリポジトリの同期問題

ローカルのリモートブランチ情報が古い場合があります。

**症状：**
- 最近マージされたPRが表示されない

**解決方法：**
```bash
git fetch --all --prune
```

#### 4. GitHub CLIが利用できない

GitHub CLIがインストールされていない場合、PR機能は動作しません。

**症状：**
- PR関連のメニューが表示されない

**解決方法：**
GitHub CLIをインストールしてください：
- macOS: `brew install gh`
- Windows: `winget install GitHub.CLI`
- Linux: 各ディストリビューションのパッケージマネージャーを使用

### 手動でのクリーンアップ

自動クリーンアップが失敗する場合は、手動で削除できます：

```bash
# worktreeの削除
git worktree remove /path/to/worktree --force

# ブランチの削除
git branch -D branch-name
```

### サポート

問題が解決しない場合は、以下の情報を含めてIssueを作成してください：

1. デバッグモードの出力
2. 使用しているOS
3. Gitとgh CLIのバージョン
4. 実行したコマンドと期待した結果