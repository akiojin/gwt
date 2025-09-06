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

## Windows環境での実行

### npx実行時のエラー

Windows環境でnpxを使用して実行する際に以下のエラーが発生する場合があります：

**症状：**
```
Error [ERR_MODULE_NOT_FOUND]: Cannot find module 'E:\claude-worktree\dist\index.js'
```

**原因：**
- パッケージがインストールされた際にビルドが実行されていない
- TypeScriptがグローバルにインストールされていない

**解決方法：**

1. **初回実行時**
   ```bash
   # パッケージを一度アンインストール
   pnpm uninstall -g @akiojin/claude-worktree
   
   # 再インストール（prepareスクリプトが自動実行される）
   pnpm add -g @akiojin/claude-worktree
   ```

2. **ローカル開発時**
   ```bash
   # 依存関係のインストール
   pnpm install
   
   # ビルドの実行
   pnpm run build
   
   # 実行
   pnpm start
   ```

### TypeScriptコンパイルエラー

**症状：**
```
This is not the tsc command you are looking for
```

**原因：**
Windowsの`tsc`コマンドが別のプログラムを参照している

**解決方法：**
```bash
# npx経由でTypeScriptを実行
npx tsc

# または、package.jsonのスクリプトを使用
pnpm run build
```

### パッケージマネージャの警告の対処

**症状：**
```
pnpm warn Unknown project config "shamefully-hoist"...
```

**原因：**
pnpm / yarn / npm 固有の設定ファイルが存在する

**解決方法：**
- これらの警告は無視して問題ありません
- 必要に応じて、`.npmrc`、`.yarnrc`、`.pnpmfile.cjs` などの設定ファイルを見直し/削除
