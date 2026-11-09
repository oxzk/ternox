# ---------- 前端构建 ----------
FROM node:22-alpine AS frontend

WORKDIR /app

COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci

COPY frontend/ ./
RUN npm run build


# ---------- 后端构建 ----------
FROM rust:1-bookworm AS backend

WORKDIR /app

COPY backend/ ./

# 前端产物覆盖到 backend/static，与 rust-embed 的 $CARGO_MANIFEST_DIR/static 一致
COPY --from=frontend /app/dist ./static

RUN cargo build --release


# ---------- 运行镜像 ----------
FROM debian:bookworm-slim

# sudo：root 用户免密可用（容器内终端按 root 设计）
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        ca-certificates \
        curl \
        sudo \
        tzdata \
        vim \
        procps \
    && rm -rf /var/lib/apt/lists/*

COPY --from=backend \
    /app/target/release/ternox \
    /usr/local/bin/ternox

# 运行环境（不设 HOME：容器内为 root，HOME=/root 即终端默认目录）
ENV HOST=0.0.0.0 \
    PORT=7681 \
    TERM=xterm-256color \
    TERMINAL_SHELL=/bin/bash

# uv（Python 包管理器）+ 预装 Python 3.12
# 安装期显式 HOME=/root，与运行时（passwd 默认）一致，保证默认目录可被发现
ENV UV_INSTALL_DIR=/usr/local/bin
RUN export HOME=/root \
    && curl -LsSf https://astral.sh/uv/install.sh | sh \
    && uv python install 3.12

EXPOSE 7681

HEALTHCHECK --interval=30s --start-period=5s --retries=3 \
    CMD curl -fsS http://127.0.0.1:7681/healthz || exit 1

ENTRYPOINT ["/usr/local/bin/ternox"]
