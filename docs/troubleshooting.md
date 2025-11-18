# Troubleshooting

## Branch Cleanup が動作しない場合

ブランチクリーンアップ（マージ済みPRやベースブランチと差分がないブランチの整理）が期待通りに動作しない場合は、以下のデバッグ手順を試してください。

### デバッグモードの有効化

環境変数 `DEBUG_CLEANUP=1` を設定してデバッグ情報を表示できます：

```bash
DEBUG_CLEANUP=1 gwt
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

## ReleaseブランチのAuto Mergeが進まない場合

### Requiredチェックの確認

1. `gh pr checks <PR番号>` で `lint` / `test` / `semantic-release` の3ジョブが成功しているか確認します。
2. 失敗したジョブがある場合は GitHub Actions から `release` ワークフローを再実行します（`Re-run all jobs`）。
3. semantic-release の CHANGELOG/公開処理で失敗した場合はコミットメッセージ（Conventional Commits準拠）とトークン権限を見直し、修正後に再度 `/release` を実行して release ブランチを更新します。

### Branch Protection チェックリスト

- [ ] `main` ブランチに「Require status checks」を設定し、`lint` / `test` / `semantic-release` だけを Required にしている
- [ ] `main` への直接 push を禁止しており、release→main PR 経由でのみ更新している（管理者 push を許可する場合も release フロー前提）
- [ ] 「Allow auto-merge」が `main` で有効になっている
- [ ] `release` ブランチでは GitHub Actions (semantic-release) が push できるようにしており、保護ルールでブロックしていない
- [ ] `SEMANTIC_RELEASE_TOKEN` が `repo` スコープを持ち、Release ワークフローに `GH_TOKEN` / `GITHUB_TOKEN` として渡されている
- [ ] release→main PR に `release` / `auto-merge` ラベルが付与され、`gh pr merge --auto --merge` 成功時のログが記録されている

1つでも未達の場合、Auto Merge は pending のままとなります。設定を整えた後に `/release` を再実行して release ブランチを最新化してください。

## ブランチ一覧の表示順がおかしい場合

ブランチ一覧画面で期待と異なる並び順になっている場合は、次の手順で原因を切り分けてください。

1. `bun run build` の後に `bun run start -- --help` を実行し、最新のビルドで CLI が起動しているか確認する
2. `bun run test tests/unit/ui/table.test.ts` を実行し、ソートのユニットテストがすべて成功することを確認する
3. worktree が優先されない場合は `git worktree list` で対象ブランチの worktree エントリが存在するか確認する
4. ローカル／リモートの優先度が逆転している場合は `git fetch --all --prune` を実行し、古いリモートブランチを整理する

上記でも問題が解決しない場合は、確認結果とともに Issue を作成してください。

## Windows環境での実行

### bunx実行時のエラー

Windows環境でbunxを使用して実行する際に以下のエラーが発生する場合があります：

**症状：**

```
Error [ERR_MODULE_NOT_FOUND]: Cannot find module 'E:\gwt\dist\index.js'
```

**原因：**

- パッケージがインストールされた際にビルドが実行されていない
- TypeScriptがグローバルにインストールされていない

**解決方法：**

1. **初回実行時**

   ```bash
   # パッケージを一度アンインストール
   bun remove -g @akiojin/gwt

   # 再インストール（prepareスクリプトが自動実行される）
   bun add -g @akiojin/gwt
   ```

2. **ローカル開発時**

   ```bash
   # 依存関係のインストール
   bun install

   # ビルドの実行
   bun run build

   # 実行
   bun run start
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
# bunx経由でTypeScriptを実行
bunx tsc

# または、package.jsonのスクリプトを使用
bun run build
```

### パッケージマネージャの警告の対処

**原因：**
各パッケージマネージャ固有の設定ファイルが残っている場合に、警告が表示されることがあります。

**解決方法：**

- プロジェクト直下に残存する設定ファイル（例: `.npmrc`、`.yarnrc`、`.pnpmfile.cjs` など）を見直し/削除
- 本プロジェクトでは bun を前提とします
