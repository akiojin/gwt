# Node.js 22 (LTS) ベースイメージ
FROM node:22-bookworm

RUN apt-get update && apt-get install -y \
    jq \
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
    @anthropic-ai/claude-code@latest \
    @openai/codex@latest \
    @google/gemini-cli@latest

# Install uv/uvx
RUN curl -fsSL https://astral.sh/uv/install.sh | bash
ENV PATH="/root/.cargo/bin:${PATH}"

# エントリーポイントスクリプトをコピー
COPY scripts/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

WORKDIR /claude-worktree
ENTRYPOINT ["/entrypoint.sh"]
CMD ["bash"]
