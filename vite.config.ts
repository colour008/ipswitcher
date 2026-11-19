import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { fileURLToPath, URL } from "node:url";
import { readFileSync } from "node:fs";

// 构建时把 package.json 版本号注入前端全局常量，打包后界面版本自动同步
const pkg = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8"));

// Tauri 固定端口，dev 阶段由 tauri.conf.json 的 devUrl 指向
export default defineConfig({
  plugins: [vue()],
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    watch: {
      // Tauri 自身目录不参与前端热更新
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
    emptyOutDir: false,
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL("./index.html", import.meta.url)),
        widget: fileURLToPath(new URL("./widget.html", import.meta.url)),
      },
    },
  },
});
