import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { fileURLToPath, URL } from "node:url";

// Tauri 前端开发配置：
// - dev server 固定 5173 端口，与 src-tauri/tauri.conf.json 的 devUrl 对应
// - clearScreen:false 保留 Tauri CLI 的终端输出
export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "chrome105",
    outDir: "dist",
  },
});
