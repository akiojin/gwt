# Node.js 22 (LTS) ベースイメージ
FROM node:22-bookworm

ARG ZIG_VERSION=0.15.2
ARG ZIG_SHA256=02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239
ARG PNPM_VERSION=10.29.2

# 開発/CIで必要になる基盤ツール + Tauri/Linux 依存をイメージに同梱
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    gnupg \
    jq \
    patchelf \
    pkg-config \
    python3 \
    ripgrep \
    vim \
    libgtk-3-dev \
    libwebkit2gtk-4.1-dev \
    libjavascriptcoregtk-4.1-dev \
    libsoup-3.0-dev \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# Install Zig
RUN curl -fsSL "https://ziglang.org/download/${ZIG_VERSION}/zig-x86_64-linux-${ZIG_VERSION}.tar.xz" -o /tmp/zig.tar.xz && \
    echo "${ZIG_SHA256}  /tmp/zig.tar.xz" | sha256sum -c - && \
    tar -C /opt -xf /tmp/zig.tar.xz && \
    ln -s "/opt/zig-x86_64-linux-${ZIG_VERSION}/zig" /usr/local/bin/zig && \
    rm /tmp/zig.tar.xz

# Global tools (minimal - other tools are in devDependencies)
RUN npm add -g bun@latest
RUN corepack enable && corepack prepare pnpm@${PNPM_VERSION} --activate

# Install Rust
RUN /bin/bash -c "set -o pipefail && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
ENV PATH="/root/.cargo/bin:${PATH}"

# Install uv/uvx
RUN /bin/bash -c "set -o pipefail && curl -fsSL https://astral.sh/uv/install.sh | bash"

# GitHub CLIのインストール
RUN curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | gpg --dearmor -o /usr/share/keyrings/githubcli-archive-keyring.gpg && \
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" > /etc/apt/sources.list.d/github-cli.list && \
    apt-get update && \
    apt-get install -y gh && \
    rm -rf /var/lib/apt/lists/*

# エントリーポイントスクリプトをコピー
COPY scripts/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
CMD ["bash"]
