import { defineConfig } from "vite";

// Launcher page bundled into the Locus Remote app (desktop pages are served
// by each paired desktop instead).
// @ts-expect-error process is a Node global
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  clearScreen: false,
  server: {
    port: 1430,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1431 } : undefined,
  },
  build: {
    target: "chrome105",
    outDir: "dist",
    emptyOutDir: true,
  },
});
