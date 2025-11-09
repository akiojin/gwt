# クイックスタートガイド: Worktree内でのコマンド実行制限機能

**日付**: 2025-11-09
**仕様ID**: SPEC-eae13040

## 概要

このガイドでは、Worktree内でのコマンド実行制限機能の開発、テスト、デバッグ方法を説明します。

## 前提条件

### 必須

- Bash 4.0以上
- jq 1.5以上
- git 2.0以上

### 推奨

- Python 3.6以上 (堅牢なコマンド解析のため)
- realpath (シンボリックリンク解決のため、coreutilsに含まれる)
- ShellCheck (コード品質チェックのため)

### インストール方法

**macOS**:
```bash
brew install jq coreutils python3 shellcheck
```

**Ubuntu/Debian**:
```bash
sudo apt-get install jq coreutils python3 shellcheck
```

## セットアップ

### 1. リポジトリのクローン

```bash
git clone https://github.com/your-org/claude-worktree.git
cd claude-worktree
```

### 2. Worktreeの作成

```bash
# 既存のWorktree設定を使用
claude-worktree feature/disallow-bash-command
cd .worktrees/feature-disallow-bash-command
```

### 3. 依存関係の確認

```bash
# jqのバージョン確認
jq --version  # jq-1.5 以上

# gitのバージョン確認
git --version  # 2.0以上

# Python3の確認
python3 --version  # 3.6以上(推奨)

# realpathの確認
command -v realpath && echo "realpath available"
```

## 開発ワークフロー

### 1. フックスクリプトの編集

フックスクリプトは`.claude/hooks/`ディレクトリに配置されています。

```bash
# cdコマンド制限フック
vim .claude/hooks/block-cd-command.sh

# gitブランチ操作制限フック
vim .claude/hooks/block-git-branch-ops.sh

# ファイル操作制限フック(新規作成予定)
vim .claude/hooks/block-file-ops.sh
```

### 2. フックのテスト

#### 手動テスト

フックスクリプトに直接JSON入力を渡してテスト:

```bash
# cdコマンドのテスト(ブロックされるべき)
echo '{"tool_name":"Bash","tool_input":{"command":"cd /tmp"}}' | \
  .claude/hooks/block-cd-command.sh
echo $?  # 2 (ブロック)

# git branch --listのテスト(許可されるべき)
echo '{"tool_name":"Bash","tool_input":{"command":"git branch --list"}}' | \
  .claude/hooks/block-git-branch-ops.sh
echo $?  # 0 (許可)
```

#### 自動テスト(Bats)

Batsテストフレームワークを使用:

```bash
# Batsのインストール
brew install bats-core  # macOS
sudo apt-get install bats  # Ubuntu

# テストの実行
bats tests/hooks/test-cd-command.bats
bats tests/hooks/test-git-branch-ops.bats
```

### 3. ShellCheckによる静的解析

```bash
# block-cd-command.shの解析
shellcheck .claude/hooks/block-cd-command.sh

# block-git-branch-ops.shの解析
shellcheck .claude/hooks/block-git-branch-ops.sh
```

警告が出た場合は修正してください。特に以下の警告に注意:
- SC2155: 変数宣言と代入を分離
- SC2269: 不要な変数代入

### 4. エンドツーエンドテスト

実際のClaude Code環境でテスト:

```bash
# Claude Codeを起動
claude-code

# テストコマンドを実行(Bash

ツール経由)
# ブロックされるコマンド
cd /tmp  # → ブロックされる
git checkout main  # → ブロックされる

# 許可されるコマンド
cd ./src  # → 許可される(Worktree内)
git branch --list  # → 許可される(参照系)
```

## よくある操作

### 新しいコマンドパターンの追加

#### 1. **正規表現パターンを定義**

```bash
# block-git-branch-ops.sh の 148 行目付近に追加
if echo "$trimmed_segment" | grep -qE '^git\s+新しいコマンド\b'; then
    # ブロック処理
fi
```

#### 2. **エラーメッセージを定義**

```bash
cat <<EOF
{
  "decision": "block",
  "reason": "🚫 新しいコマンドは許可されていません",
  "stopReason": "理由の詳細説明\n\nBlocked command: $command"
}
EOF
```

#### 3. **テストケースを追加**

```bash
# tests/hooks/test-git-branch-ops.bats に追加
@test "新しいコマンドがブロックされる" {
  run echo '{"tool_name":"Bash","tool_input":{"command":"git 新しいコマンド"}}' | \
    .claude/hooks/block-git-branch-ops.sh
  [ "$status" -eq 2 ]
}
```

### デバッグ方法

#### 1. stderrログの確認

フックスクリプトはstderrにログを出力します:

```bash
# stderrを確認
echo '{"tool_name":"Bash","tool_input":{"command":"cd /tmp"}}' | \
  .claude/hooks/block-cd-command.sh 2>&1 | grep "🚫"
```

#### 2. デバッグ出力の追加

フックスクリプトに一時的にデバッグ出力を追加:

```bash
# is_within_worktree()関数内に追加
echo "DEBUG: target_path=$target_path" >&2
echo "DEBUG: abs_path=$abs_path" >&2
echo "DEBUG: WORKTREE_ROOT=$WORKTREE_ROOT" >&2
```

#### 3. シェルトレースの有効化

```bash
# フックスクリプトの先頭に追加
set -x  # トレースモード有効化

# または、実行時に環境変数で指定
BASH_XTRACEFD=2 bash -x .claude/hooks/block-cd-command.sh < input.json
```

### Worktree境界判定のテスト

`is_within_worktree()`関数を個別にテスト:

```bash
# block-cd-command.shを直接実行
source .claude/hooks/block-cd-command.sh

# Worktree内のパスをテスト
if is_within_worktree "./src"; then
  echo "Worktree内"
else
  echo "Worktree外"
fi

# Worktree外のパスをテスト
if is_within_worktree "/tmp"; then
  echo "Worktree内"
else
  echo "Worktree外"
fi
```

## トラブルシューティング

### 問題: jqコマンドが見つからない

**症状**:
```
.claude/hooks/block-cd-command.sh: line 57: jq: command not found
```

**解決策**:

```bash
# macOS
brew install jq

# Ubuntu/Debian
sudo apt-get install jq
```

### 問題: realpathコマンドが見つからない

**症状**:

```
realpath: command not found
```

**解決策**:

```bash
# macOS
brew install coreutils

# Ubuntu/Debian
# デフォルトでインストール済み
```

**代替案**:
Python3がインストールされていれば、フォールバック実装が自動的に使用されます。

### 問題: Python3のshlex.split()がエラー

**症状**:
```
python3: No module named shlex
```

**解決策**:
shlexは標準ライブラリのため、通常はインストール不要。Python3が正しくインストールされているか確認:

```bash
python3 --version
python3 -c "import shlex; print('OK')"
```

### 問題: 複合コマンドがブロックされない

**症状**:
```bash
echo "test" && git checkout main
# ブランチが切り替わってしまう
```

**原因**:
フックスクリプトの複合コマンド分割ロジックに問題がある可能性。

**デバッグ**:
```bash
# コマンド分割のデバッグ
command="echo test && git checkout main"
command_segments=$(printf '%s\n' "$command" | sed -E 's/\|&/\n/g; s/\|\|/\n/g; s/&&/\n/g; s/[;|&]/\n/g')
echo "$command_segments"
```

**解決策**:
セグメント分割ロジックを確認し、必要に応じてPython shlex.split()を使用。

### 問題: git checkout -- fileがブロックされる

**症状**:
```bash
git checkout -- file.txt
# ブロックされてしまう
```

**原因**:
`git checkout -- file`のパターンマッチングが未実装。

**解決策**:
block-git-branch-ops.shの148行目付近に以下を追加:

```bash
# git checkout -- file はファイル復元なので許可
if echo "$trimmed_segment" | grep -qE '^git\s+checkout\s+--\s'; then
    continue
fi
```

## 次のステップ

1. `/speckit.tasks` を実行してタスク生成
2. `/speckit.implement` で実装開始
3. テストケースを追加
4. CI/CDパイプラインに統合

## 参考資料

- [機能仕様書](../../SPEC-eae13040/spec.md)
- [実装計画](plan.md)
- [調査レポート](research.md)
- [データモデル](data-model.md)
- [Batsドキュメント](https://bats-core.readthedocs.io/)
- [ShellCheckドキュメント](https://www.shellcheck.net/)
