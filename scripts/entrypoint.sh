#!/bin/bash

set -euo pipefail

# Git設定（node:22-bookwormにはGitが含まれている）
# グローバルGit設定（安全なディレクトリを追加）
# Worktreeの.gitがホストパスを参照するため、CWD依存を避けて実行する
git -C / config --global --add safe.directory /gwt

# ユーザー名とメールの設定（環境変数から）
if [ -n "${GITHUB_USERNAME:-}" ]; then
  git -C / config --global user.name "$GITHUB_USERNAME"
fi

if [ -n "${GIT_USER_EMAIL:-}" ]; then
  git -C / config --global user.email "$GIT_USER_EMAIL"
fi

# Git認証ファイルを環境変数から作成
if [ -n "${GITHUB_USERNAME:-}" ] && [ -n "${GITHUB_PERSONAL_ACCESS_TOKEN:-}" ]; then
  printf '%s\n' "https://${GITHUB_USERNAME}:${GITHUB_PERSONAL_ACCESS_TOKEN}@github.com" > /root/.git-credentials
  chmod 600 /root/.git-credentials
  git -C / config --global credential.helper store
fi

# GitHub CLIの認証（GITHUB_TOKENが設定されている場合）
if [ -n "${GITHUB_TOKEN:-}" ] && command -v gh &> /dev/null; then
  if echo "$GITHUB_TOKEN" | gh auth login --with-token; then
    echo "✅ GitHub CLI authenticated"
  else
    echo "⚠️ GitHub CLI authentication failed (non-fatal)" >&2
  fi
fi

# .codexディレクトリのセットアップ
# auth.jsonをホストと同期（クロスプラットフォーム対応）
mkdir -p /root/.codex
if [ -f /root/.codex-host/auth.json ]; then
  # auth.jsonが誤ってディレクトリとして作成されている場合は削除
  if [ -d /root/.codex/auth.json ]; then
    echo "⚠️ Removing incorrectly created auth.json directory"
    rm -rf /root/.codex/auth.json
  fi

  # ホストのauth.jsonが存在しない、または空、またはホスト側が新しい場合はコピー
  if [ ! -f /root/.codex/auth.json ] || [ ! -s /root/.codex/auth.json ] || [ /root/.codex-host/auth.json -nt /root/.codex/auth.json ]; then
    cp /root/.codex-host/auth.json /root/.codex/auth.json
    chmod 600 /root/.codex/auth.json
    echo "✅ Codex auth.json synced from host"
  else
    echo "✅ Codex auth.json is up to date"
  fi
else
  echo "ℹ️ INFO: Codex auth.json not found on host (optional)"
fi

# .claudeディレクトリのセットアップ
# 認証・設定をホストと同期（クロスプラットフォーム対応）
mkdir -p /root/.claude
if [ -d /root/.claude-host ] && [ -n "$(find /root/.claude-host -mindepth 1 -maxdepth 1 2>/dev/null)" ]; then
  cp -R /root/.claude-host/. /root/.claude/
  echo "✅ Claude config synced from host"
else
  echo "ℹ️ INFO: Claude config not found on host (optional)"
fi

echo "🚀 Docker development environment is ready!"
echo "   You can now build the project with: bun run build"
echo "   Or start development with: bun run dev"
echo ""

# コマンドの実行（デフォルトはbash）
exec "$@"
