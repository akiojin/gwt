# クイックスタートガイド: Clean up merged PRs 機能修正

## 概要
このガイドでは、メインメニューの「Clean up merged PRs」機能の表示不整合を修正する手順を説明します。

## 前提条件
- Node.js 18以上
- Git
- bun（パッケージマネージャー）

## セットアップ手順

### 1. リポジトリのクローン
```bash
git clone <repository-url>
cd claude-worktree
```

### 2. 依存関係のインストール
```bash
bun install
```

### 3. ビルド
```bash
bun run build
```

## 動作確認

### 修正前の動作確認
1. アプリケーションを起動
   ```bash
   bunx .
   ```

2. メインメニューが表示される
   - 「Actions: (n) Create new branch, (m) Manage worktrees, (c) Clean up merged PRs, (a) Account management, (q) Exit」と表示される
   - 'a'キーを押しても何も起こらない（バグ）

### 修正後の動作確認
1. アプリケーションを起動
   ```bash
   bunx .
   ```

2. メインメニューが表示される
   - 「Actions: (n) Create new branch, (m) Manage worktrees, (c) Clean up merged PRs, (q) Exit」と表示される
   - 'a' への言及が削除されている

3. 各キーの動作を確認
   - 'n'キー: 新しいブランチ作成
   - 'm'キー: worktree管理
   - 'c'キー: マージ済みPRのクリーンアップ
   - 'q'キー: 終了

## テストシナリオ

### シナリオ1: メニュー表示の確認
1. アプリケーションを起動
2. メニューに「(a) Account management」が表示されていないことを確認
3. 他のメニュー項目が正しく表示されることを確認

### シナリオ2: 'c'キーの動作確認
1. アプリケーションを起動
2. 'c'キーを押す
3. 「Clean up merged PRs」機能が起動することを確認
4. GitHub CLIが未インストールの場合、適切なエラーメッセージが表示されることを確認

### シナリオ3: キーボードナビゲーション
1. アプリケーションを起動
2. 上下矢印キーまたは'j'/'k'キーでメニュー項目を移動できることを確認
3. Enterキーで選択できることを確認

## トラブルシューティング

### 問題: ビルドエラーが発生する
**解決策**: 
```bash
bun install
bun run build
```

### 問題: GitHub CLIエラーが表示される
**解決策**: GitHub CLIをインストール
```bash
# macOS
brew install gh

# Windows
winget install --id GitHub.cli

# Linux
curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | sudo dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg
echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | sudo tee /etc/apt/sources.list.d/github-cli.list > /dev/null
sudo apt update
sudo apt install gh
```

## 成功基準
- [ ] メニューに「(a) Account management」が表示されない
- [ ] 'n', 'm', 'c', 'q'キーがすべて正常に動作する
- [ ] エラーメッセージが適切に表示される
- [ ] キーボードナビゲーションが正常に動作する