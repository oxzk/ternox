import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// 后端地址；开发时代理需同时改写 Host 与 Origin 以通过后端同源校验
const backend = process.env.TERNOX_BACKEND ?? "http://127.0.0.1:7681";
const wsBackend = backend.replace(/^http/, "ws");

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": {
        target: backend,
        changeOrigin: true,
        headers: { origin: backend },
      },
      "/ws": {
        target: wsBackend,
        ws: true,
        changeOrigin: true,
        // Origin 规范上只允许 http/https 形式（浏览器 WS 握手亦然），
        // 后端 same_origin 会拒绝 ws:// 前缀，因此这里同样改写为 http 形式
        headers: { origin: backend },
      },
    },
  },
  build: {
    outDir: "dist",
    sourcemap: false,
  },
});
