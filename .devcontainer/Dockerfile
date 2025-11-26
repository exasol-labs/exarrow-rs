# exarrow-rs Development Container

FROM debian:bookworm-slim

ARG TZ
ENV TZ="${TZ:-UTC}"

ARG CLAUDE_CODE_VERSION=latest

# Install development tools, networking, and firewall dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    # Build essentials for Rust
    build-essential \
    pkg-config \
    libssl-dev \
    # Version control and tools
    git \
    gh \
    # Shell and utilities
    bash \
    curl \
    wget \
    ca-certificates \
    unzip \
    less \
    procps \
    sudo \
    fzf \
    man-db \
    nano \
    vim \
    jq \
    # Firewall and networking
    iptables \
    ipset \
    iproute2 \
    dnsutils \
    aggregate \
    && apt-get clean && rm -rf /var/lib/apt/lists/*

# Create non-root user for development
ARG USERNAME=coder
ARG USER_UID=1000
ARG USER_GID=$USER_UID

RUN groupadd --gid $USER_GID $USERNAME \
    && useradd --uid $USER_UID --gid $USER_GID -m -s /bin/bash $USERNAME \
    && echo "$USERNAME ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/$USERNAME \
    && chmod 0440 /etc/sudoers.d/$USERNAME

# Persist shell history
RUN mkdir /commandhistory \
    && touch /commandhistory/.bash_history \
    && chown -R $USERNAME /commandhistory

# Set DEVCONTAINER environment variable
ENV DEVCONTAINER=true

# Create workspace and config directories
RUN mkdir -p /workspace /home/$USERNAME/.claude /home/$USERNAME/.cargo \
    && chown -R $USERNAME:$USERNAME /workspace /home/$USERNAME/.claude /home/$USERNAME/.cargo

WORKDIR /workspace

# Install git-delta for better diffs
ARG GIT_DELTA_VERSION=0.18.2
RUN ARCH=$(dpkg --print-architecture) && \
    wget -q "https://github.com/dandavison/delta/releases/download/${GIT_DELTA_VERSION}/git-delta_${GIT_DELTA_VERSION}_${ARCH}.deb" && \
    dpkg -i "git-delta_${GIT_DELTA_VERSION}_${ARCH}.deb" && \
    rm "git-delta_${GIT_DELTA_VERSION}_${ARCH}.deb"

# Switch to non-root user for remaining installations
USER $USERNAME
WORKDIR /home/$USERNAME

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y \
    && . $HOME/.cargo/env \
    && rustup component add rustfmt clippy rust-analyzer

# Set up Rust environment
ENV PATH="/home/$USERNAME/.cargo/bin:$PATH"
ENV CARGO_HOME="/home/$USERNAME/.cargo"
ENV RUSTUP_HOME="/home/$USERNAME/.rustup"

# Install Node.js (required for Claude Code CLI and OpenSpec)
USER root
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y nodejs \
    && apt-get clean && rm -rf /var/lib/apt/lists/*

# Set up npm global directory for non-root user
RUN mkdir -p /usr/local/share/npm-global \
    && chown -R $USERNAME:$USERNAME /usr/local/share/npm-global

USER $USERNAME

# Configure npm to use user-writable global prefix
ENV NPM_CONFIG_PREFIX=/usr/local/share/npm-global
ENV PATH=$PATH:/usr/local/share/npm-global/bin

# Install Claude Code CLI
RUN npm install -g @anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}
RUN mkdir -p /home/$USERNAME/.claude

# Install OpenSpec CLI
RUN npm install -g @fission-ai/openspec@latest

# Set shell and editor preferences
ENV SHELL=/bin/bash
ENV EDITOR=vim
ENV VISUAL=vim

# Copy and set up firewall script
COPY init-firewall.sh /usr/local/bin/
USER root
RUN chmod +x /usr/local/bin/init-firewall.sh \
    && echo "$USERNAME ALL=(root) NOPASSWD: /usr/local/bin/init-firewall.sh" > /etc/sudoers.d/$USERNAME-firewall \
    && chmod 0440 /etc/sudoers.d/$USERNAME-firewall

USER $USERNAME
WORKDIR /workspace
