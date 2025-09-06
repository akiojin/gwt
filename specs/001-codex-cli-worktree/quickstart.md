# クイックスタートガイド: Codex CLI対応

**機能**: worktree起動時のツール選択  
**バージョン**: 1.0.0  
**最終更新**: 2025-01-06

## 概要

このガイドでは、worktreeコマンドの新しいツール選択機能を素早く試す方法を説明します。

## 前提条件

- Node.js 18+ がインストールされていること
- npm または yarn が利用可能であること
- ターミナル環境（bash, zsh, PowerShell など）

## インストール

```bash
# リポジトリのクローン
git clone <repository-url>
cd worktree-project

# 依存関係のインストール
npm install

# グローバルインストール（オプション）
npm link
```

## 基本的な使用方法

### 1. 対話型選択（推奨）

```bash
# worktreeを起動
worktree

# 以下のような選択画面が表示されます：
? Which AI tool would you like to use? (Use arrow keys)
❯ Claude Code - Anthropic's AI coding assistant
  Codex CLI - OpenAI's code generation tool
```

矢印キーで選択し、Enterキーで確定します。

### 2. 直接指定

```bash
# Claude Codeを直接起動
worktree --tool claude

# Codex CLIを直接起動
worktree --tool codex
```

### 3. ヘルプの表示

```bash
# 使用可能なオプションを確認
worktree --help
```

## 動作確認テスト

### テスト1: 初回起動

```bash
# 設定をリセット
rm -rf ~/.worktree

# worktreeを起動
worktree

# 期待される動作:
# 1. ツール選択画面が表示される
# 2. Claude Codeを選択
# 3. "✓ Claude Code workspace initialized" が表示される
```

### テスト2: 選択の記憶

```bash
# 1回目: Claude Codeを選択
worktree
# Claude Codeを選択

# 2回目: 前回の選択が記憶されている
worktree
# 期待: Claude Codeがデフォルトで選択されている状態
```

### テスト3: Codex CLI未インストール時

```bash
# Codex CLIが未インストールの環境で実行
worktree

# 期待される動作:
# Codex CLIの選択肢が無効化されている
# または選択時に警告メッセージが表示される
```

### テスト4: コマンドラインオプション

```bash
# バージョン確認
worktree --version
# 期待: バージョン番号が表示される

# デフォルト設定のリセット
worktree --reset-default
# 期待: "Default tool selection has been reset" が表示される

# 静音モード
worktree --tool claude --quiet
# 期待: 最小限の出力でClaude Codeが起動
```

### テスト5: エラーハンドリング

```bash
# 無効なツール指定
worktree --tool invalid
# 期待: エラーメッセージ "Error: Invalid tool 'invalid'"

# 権限エラーのシミュレーション
chmod 000 ~/.worktree/config.json
worktree
# 期待: 権限エラーメッセージと回復方法の提示
chmod 644 ~/.worktree/config.json
```

## 設定ファイル

設定は `~/.worktree/config.json` に保存されます：

```json
{
  "version": "1.0.0",
  "defaultTool": "claude",
  "lastSelection": "claude",
  "preferences": {
    "rememberSelection": true,
    "showWelcomeMessage": true,
    "colorOutput": true
  }
}
```

### 設定の変更

```bash
# 設定ファイルを直接編集
nano ~/.worktree/config.json

# または環境変数で制御
export WORKTREE_DEFAULT_TOOL=codex
worktree
```

## トラブルシューティング

### 問題: ツール選択画面が表示されない

**解決方法**:
```bash
# ターミナルがインタラクティブモードか確認
echo $PS1
# 空の場合は非インタラクティブモード

# 強制的にインタラクティブモードで実行
bash -i -c "worktree"
```

### 問題: 色が表示されない

**解決方法**:
```bash
# カラー出力を無効化
worktree --no-color

# または設定で無効化
echo '{"preferences":{"colorOutput":false}}' > ~/.worktree/config.json
```

### 問題: Codex CLIが見つからない

**解決方法**:
```bash
# Codex CLIのインストール状況確認
which codex

# インストールされていない場合
npm install -g @openai/codex-cli
```

## 統合テストスクリプト

すべての機能を自動テストするスクリプト：

```bash
#!/bin/bash
# test-worktree.sh

echo "=== Worktree Tool Selection Test Suite ==="

# テスト1: ヘルプ
echo "Test 1: Help"
worktree --help || exit 1

# テスト2: バージョン
echo "Test 2: Version"
worktree --version || exit 1

# テスト3: Claude直接指定
echo "Test 3: Direct Claude"
worktree --tool claude --quiet || exit 1

# テスト4: 設定リセット
echo "Test 4: Reset Default"
worktree --reset-default || exit 1

echo "=== All tests passed! ==="
```

## パフォーマンステスト

```bash
# 起動時間測定
time worktree --tool claude --quiet

# 期待: < 100ms
```

## フィードバック

問題や改善提案がある場合は、GitHubのIssueを作成してください。

---
*このガイドは機能仕様 FR-001〜FR-010 の実装確認用です*