# Node.js 22 (LTS) ベースイメージ
FROM node:22-bookworm

RUN apt-get update && apt-get install -y \
    jq \
    vim \
    ripgrep \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# Global tools with bun
RUN npm i -g \
    npm@latest \
    bun@latest \
    typescript@latest \
    eslint@latest \
    prettier@latest \
    @openai/codex@latest \
    @google/gemini-cli@latest

# Install uv/uvx
RUN curl -fsSL https://astral.sh/uv/install.sh | bash
ENV PATH="/root/.cargo/bin:${PATH}"

# GitHub CLIのインストール
RUN curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | gpg --dearmor -o /usr/share/keyrings/githubcli-archive-keyring.gpg && \
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" > /etc/apt/sources.list.d/github-cli.list && \
    apt-get update && \
    apt-get install -y gh && \
    rm -rf /var/lib/apt/lists/*

# エントリーポイントスクリプトをコピー
COPY scripts/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

WORKDIR /claude-worktree
ENTRYPOINT ["/entrypoint.sh"]
CMD ["bash"]
