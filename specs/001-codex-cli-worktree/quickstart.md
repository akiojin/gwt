# クイックスタートガイド: Codex CLI対応

**機能**: claude-worktree起動時のツール選択  
**バージョン**: 1.0.0  
**最終更新**: 2025-01-06

## 概要

このガイドでは、claude-worktreeコマンドの新しいツール選択機能を素早く試す方法を説明します。

## 前提条件

- Node.js 18+ がインストールされていること
- npm または yarn が利用可能であること
- ターミナル環境（bash, zsh, PowerShell など）

## インストール

```bash
# リポジトリのクローン
git clone <repository-url>
cd claude-worktree

# 依存関係のインストール
npm install

# グローバルインストール（オプション）
npm link
```

## 基本的な使用方法

### 1. 対話型選択（推奨）

```bash
# claude-worktreeを起動
claude-worktree

# 以下のような選択画面が表示されます：
? Which AI tool would you like to use? (Use arrow keys, press 'q' to cancel)
❯ Claude Code - Anthropic's AI coding assistant
  Codex CLI - OpenAI's code generation tool
```

- 矢印キーで選択
- Enterキーで確定
- qキーでキャンセル

### 2. 直接指定

```bash
# Claude Codeを直接起動
claude-worktree --tool claude

# Codex CLIを直接起動
claude-worktree --tool codex
```

### 3. ツール固有オプション付き起動

```bash
# Claude Codeをリジュームモードで起動
claude-worktree --tool claude -- -r

# Claude Codeをコンテキスト継続モードで起動
claude-worktree --tool claude -- -c

# Codex CLIを継続モードで起動
claude-worktree --tool codex -- --continue

# Codex CLIをレジュームモードで起動
claude-worktree --tool codex -- --resume
```

### 4. ヘルプの表示

```bash
# 使用可能なオプションを確認
claude-worktree --help
```

## 動作確認テスト

### テスト1: 初回起動

```bash
# 設定をリセット
rm -rf ~/.claude-worktree

# claude-worktreeを起動
claude-worktree

# 期待される動作:
# 1. ツール選択画面が表示される（qキーでキャンセル可能）
# 2. Claude Codeを選択
# 3. "✓ Claude Code workspace initialized" が表示される
```

### テスト1.1: キャンセル操作

```bash
# claude-worktreeを起動
claude-worktree

# qキーを押す

# 期待される動作:
# "Operation cancelled by user (q)" が表示される
# 終了コード 130 で終了
```

### テスト2: 選択の記憶

```bash
# 1回目: Claude Codeを選択
claude-worktree
# Claude Codeを選択

# 2回目: 前回の選択が記憶されている
claude-worktree
# 期待: Claude Codeがデフォルトで選択されている状態
```

### テスト3: Codex CLI未インストール時

```bash
# Codex CLIが未インストールの環境で実行
claude-worktree

# 期待される動作:
# Codex CLIの選択肢が無効化されている
# または選択時に警告メッセージが表示される
```

### テスト4: コマンドラインオプション

```bash
# バージョン確認
claude-worktree --version
# 期待: バージョン番号が表示される

# デフォルト設定のリセット
claude-worktree --reset-default
# 期待: "Default tool selection has been reset" が表示される

# 静音モード
claude-worktree --tool claude --quiet
# 期待: 最小限の出力でClaude Codeが起動

# ツール固有オプションの動作確認
claude-worktree --tool claude -- -r
# 期待: Claude Codeが -r オプション付きで起動

claude-worktree --tool codex -- --continue
# 期待: Codex CLIが --continue オプション付きで起動
```

### テスト5: エラーハンドリング

```bash
# 無効なツール指定
claude-worktree --tool invalid
# 期待: エラーメッセージ "Error: Invalid tool 'invalid'"

# 権限エラーのシミュレーション
chmod 000 ~/.claude-worktree/config.json
claude-worktree
# 期待: 権限エラーメッセージと回復方法の提示
chmod 644 ~/.claude-worktree/config.json
```

## 設定ファイル

設定は `~/.claude-worktree/config.json` に保存されます：

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
nano ~/.claude-worktree/config.json

# または環境変数で制御
export CLAUDE_WORKTREE_DEFAULT_TOOL=codex
claude-worktree
```

## トラブルシューティング

### 問題: ツール選択画面が表示されない

**解決方法**:
```bash
# ターミナルがインタラクティブモードか確認
echo $PS1
# 空の場合は非インタラクティブモード

# 強制的にインタラクティブモードで実行
bash -i -c "claude-worktree"
```

### 問題: 色が表示されない

**解決方法**:
```bash
# カラー出力を無効化
claude-worktree --no-color

# または設定で無効化
echo '{"preferences":{"colorOutput":false}}' > ~/.claude-worktree/config.json
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
# test-claude-worktree.sh

echo "=== Claude-Worktree Tool Selection Test Suite ==="

# テスト1: ヘルプ
echo "Test 1: Help"
claude-worktree --help || exit 1

# テスト2: バージョン
echo "Test 2: Version"
claude-worktree --version || exit 1

# テスト3: Claude直接指定
echo "Test 3: Direct Claude"
claude-worktree --tool claude --quiet || exit 1

# テスト4: 設定リセット
echo "Test 4: Reset Default"
claude-worktree --reset-default || exit 1

echo "=== All tests passed! ==="
```

## パフォーマンステスト

```bash
# 起動時間測定
time claude-worktree --tool claude --quiet

# 期待: < 100ms
```

## フィードバック

問題や改善提案がある場合は、GitHubのIssueを作成してください。

---
*このガイドは機能仕様 FR-001〜FR-010 の実装確認用です*