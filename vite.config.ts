import path from "node:path";
import { defineConfig, normalizePath } from "vite";
import vue from "@vitejs/plugin-vue";
import { viteStaticCopy } from "vite-plugin-static-copy";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vitejs.dev/config/
export default defineConfig(async () => ({
  plugins: [
    vue(),
    viteStaticCopy({
      targets: [
        {
          src: normalizePath(path.resolve(__dirname, "node_modules/vditor/dist/**/*")),
          dest: "vendor/vditor",
          rename: {
            stripBase: 2,
          },
        },
      ],
    }),
  ],

  resolve: {
    alias: [
      {
        find: /^vue$/,
        replacement: "vue/dist/vue.esm-bundler.js",
      },
    ],
  },

  test: {
    setupFiles: ["src/__tests__/setupVitest.ts"],
    include: ["src/__tests__/**/*.test.ts"],
    exclude: ["ref/**"],
  },

  build: {
    chunkSizeWarningLimit: 800, // three.js chunk ~725KB, already lazy-loaded
    rollupOptions: {
      output: {
        manualChunks: {
          vendor: ["vue", "pinia", "marked", "highlight.js"],
          "binary-preview": ["ag-psd"],
          "three-preview": ["three"],
        },
      },
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 14901,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. Tell Vite's file watcher to ignore large / churny trees that don't
      //    need HMR. On Windows each watched directory costs one
      //    ReadDirectoryChangesW handle; build churn (src-tauri/obj, dotnet,
      //    parallel-agent worktrees) leaks them, and left unbounded the dev
      //    server accrued ~46k handles within minutes of startup. The compile
      //    -server apphost.exe in obj/ can also be locked by dotnet while
      //    Tauri rebuilds, making Node's fs watcher exit on EBUSY.
      //    node_modules & .git are already ignored by Vite's defaults; the
      //    rest mirrors .gitignore.
      ignored: [
        // native / .NET build outputs (locked & churny during tauri dev)
        "**/src-tauri/**", // also covers src-tauri/gen managed runtimes (~1.4k dirs)
        "**/locus_compile_server/**/bin/**",
        "**/locus_compile_server/**/obj/**",
        // workspace-only trees (see .gitignore)
        "**/.claude/**", // parallel-agent worktrees + session data (~9k dirs)
        "**/testproject/**",
        "**/ref/**",
        "**/plans/**",
        "**/experiments/**",
        "**/docs/**", // separate Mintlify site with its own node_modules
        // caches / temp / artifacts
        "**/.cache/**",
        "**/.tmp/**",
        "**/tmp/**",
        "**/debug/**",
        "**/.codex/**",
        "**/codex-artifacts/**",
        "**/.venv/**",
        "**/.venv-docs/**",
        // build output
        "**/dist/**",
        "**/dist-ssr/**",
        "**/site/**",
        // editor / logs
        "**/.vscode/**",
        "**/.idea/**",
        "**/logs/**",
        "**/*.log",
      ],
    },
  },
}));
