#!/bin/bash

# Claude Code PreToolUse Hook: Block cd command
# このスクリプトは cd コマンドをブロックします（Worktree環境での安全性のため）

# stdinからJSON入力を読み取り
json_input=$(cat)

# ツール名を確認
tool_name=$(echo "$json_input" | jq -r '.tool_name // empty')

# Bashツール以外は許可
if [ "$tool_name" != "Bash" ]; then
    exit 0
fi

# コマンドを取得
command=$(echo "$json_input" | jq -r '.tool_input.command // empty')

# 演算子で連結された各コマンドを個別にチェックするために分割
# &&, ||, ;, |, |&, &, 改行などで区切って先頭トークンを判定する
command_segments=$(printf '%s\n' "$command" | sed -E 's/\|&/\n/g; s/\|\|/\n/g; s/&&/\n/g; s/[;|&]/\n/g')

while IFS= read -r segment; do
    # リダイレクトやheredoc以降を落としてトリミング
    trimmed_segment=$(echo "$segment" | sed 's/[<>].*//; s/<<.*//' | xargs)

    # 空行はスキップ
    if [ -z "$trimmed_segment" ]; then
        continue
    fi

    # cdコマンドをチェック（cd、builtin cd、command cdなど）
    if echo "$trimmed_segment" | grep -qE '^(builtin[[:space:]]+)?cd\b'; then
        # JSON応答を返す
        cat <<EOF
{
  "decision": "block",
  "reason": "🚫 cdコマンドは禁止されています / cd command is not allowed",
  "stopReason": "Worktreeは起動したディレクトリで作業を完結させる設計です。cdコマンドによるディレクトリ移動は実行できません。\n\nReason: Worktree is designed to complete work in the launched directory. Directory navigation using cd command cannot be executed.\n\nBlocked command: $command\n\n代わりに、絶対パスを指定してコマンドを実行してください。例: 'git -C /path/to/repo status' や '/path/to/script.sh'"
}
EOF

        # stderrにもメッセージを出力
        echo "🚫 ブロック: $command" >&2
        echo "理由: Worktreeは起動したディレクトリで作業を完結させる設計です。" >&2

        exit 2  # ブロック
    fi
done <<< "$command_segments"

# 許可
exit 0
