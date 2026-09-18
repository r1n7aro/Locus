import { defineConfig } from "vite";
import path from "node:path";
import vue from "@vitejs/plugin-vue";

export default defineConfig({
  plugins: [vue()],
  server: { fs: { allow: [path.resolve(import.meta.dirname, "../..")] } },
  build: { target: "chrome105", sourcemap: false, manifest: true,
    rollupOptions: { input: { univer: path.resolve(import.meta.dirname, "index.html"), tabulator: path.resolve(import.meta.dirname, "baseline.html") } } },
});
