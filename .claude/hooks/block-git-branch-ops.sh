#!/bin/bash

# Claude Code PreToolUse Hook: Block git branch operations
# このスクリプトは git checkout, git switch, git branch, git worktree コマンドをブロックします

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

# ブランチ切り替え/作成/worktreeコマンドをチェック
if echo "$command" | grep -qE '\bgit\s+(checkout|switch|branch|worktree)\b'; then
    # JSON応答を返す
    cat <<EOF
{
  "decision": "block",
  "reason": "🚫 ブランチ切り替え・作成・worktreeコマンドは禁止されています / Branch switching, creation, and worktree commands are not allowed",
  "stopReason": "Worktreeは起動したブランチで作業を完結させる設計です。git checkout、git switch、git branch、git worktree 等の操作は実行できません。\n\nReason: Worktree is designed to complete work on the launched branch. Branch operations such as git checkout, git switch, git branch, and git worktree cannot be executed.\n\nBlocked command: $command"
}
EOF

    # stderrにもメッセージを出力
    echo "🚫 ブロック: $command" >&2
    echo "理由: Worktreeは起動したブランチで作業を完結させる設計です。" >&2

    exit 2  # ブロック
fi

# 許可
exit 0
