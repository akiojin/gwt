#!/bin/bash
set -e

# Git設定（Gitがインストールされている場合）
if command -v git &> /dev/null; then
    # グローバルGit設定（安全なディレクトリを追加）
    git config --global --add safe.directory /claude-worktree
    
    # ユーザー名とメールの設定（環境変数から）
    if [ -n "$GITHUB_USERNAME" ]; then
        git config --global user.name "$GITHUB_USERNAME"
    fi
    
    if [ -n "$GIT_USER_EMAIL" ]; then
        git config --global user.email "$GIT_USER_EMAIL"
    fi
fi

# SSH設定（SSHキーが存在する場合）
if [ -d "/root/.ssh" ]; then
    chmod 700 /root/.ssh
    if [ -f "/root/.ssh/id_rsa" ]; then
        chmod 600 /root/.ssh/id_rsa
    fi
    if [ -f "/root/.ssh/id_ed25519" ]; then
        chmod 600 /root/.ssh/id_ed25519
    fi
fi

# GitHub CLIの認証（GITHUB_TOKENが設定されている場合）
if [ -n "$GITHUB_TOKEN" ] && command -v gh &> /dev/null; then
    echo "$GITHUB_TOKEN" | gh auth login --with-token 2>/dev/null || true
fi

# プロジェクトディレクトリに移動
cd /claude-worktree

# package.jsonが存在する場合、依存関係をインストール
if [ -f "package.json" ]; then
    echo "📦 Installing dependencies..."
    if command -v pnpm &> /dev/null; then
        pnpm install --frozen-lockfile 2>/dev/null || pnpm install
    elif command -v npm &> /dev/null; then
        npm ci 2>/dev/null || npm install
    fi
    
    # TypeScriptプロジェクトの場合、ビルドを実行
    if [ -f "tsconfig.json" ]; then
        echo "🔨 Building TypeScript project..."
        npm run build 2>/dev/null || true
    fi
fi

echo "🚀 Claude Worktree Docker environment is ready!"
echo ""

# コマンドの実行（デフォルトはbash）
exec "$@"