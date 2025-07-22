# Node.js 22 (LTS) ベースイメージ
FROM node:22-bookworm

# 追加パッケージのインストール
# node:22-bookwormには既にcurl、wget、git、python3が含まれている
RUN apt-get update && apt-get install -y \
    unzip \
    jq \
    gh \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*


# Corepackを有効化してpnpmを利用可能にする
RUN corepack enable

# pnpmの環境変数設定
ENV PNPM_HOME=/pnpm
ENV PATH="$PNPM_HOME:$PATH"

# Claude Codeのインストール
RUN npm install -g @anthropic-ai/claude-code@latest

# グローバルNode.jsツールのインストール（pnpm使用）
RUN pnpm add -g \
    typescript@latest \
    eslint@latest \
    prettier@latest

# pnpmのグローバルストアを設定（コンテナ内でキャッシュ）
RUN pnpm config set store-dir /pnpm-store

# エントリーポイントスクリプトをコピー
COPY .docker/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

WORKDIR /claude-worktree
ENTRYPOINT ["/entrypoint.sh"]
CMD ["bash"]
