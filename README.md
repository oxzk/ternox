# Ternox

> A minimal self-hosted web terminal.

Ternox 是一个轻量级、自托管的 Web Terminal。

## 快速开始

```bash
docker pull ghcr.io/oxzk/ternox:latest

# 配置登录账号（AUTH_PASSWORD 禁止默认弱密码，禁止与用户名相同）
cp .env.example .env && vim .env

docker run -d --name ternox \
  -p 7681:7681 \
  --env-file .env \
  -v ternox-home:/root \
  ghcr.io/oxzk/ternox:latest
```

## 本地开发

```bash
# 前端（vite dev server，/api 与 /ws 代理到 7681）
cd frontend && npm install && npm run dev

# 后端（自动读取 backend/.env，可 cp .env.example backend/.env）
cd backend && cargo run
```