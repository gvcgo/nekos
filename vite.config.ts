import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// Tauri expects a fixed dev port; leave host 127.0.0.1 (loopback only).
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: "127.0.0.1",
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2022",
    minify: "esbuild",
    sourcemap: false,
  },
});
