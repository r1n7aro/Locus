import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

function read(path: string): string {
  return readFileSync(path, "utf8");
}

describe("runtime data isolation", () => {
  it("honors an explicit runtime data directory before persistent storage", () => {
    const storage = read("src-tauri/src/commands/storage.rs");
    const fileLog = read("src-tauri/src/file_log.rs");

    expect(storage).toContain(
      'RUNTIME_STORAGE_DIR_ENV: &str = "LOCUS_RUNTIME_DATA_DIR"',
    );
    expect(storage).toContain(
      "if let Some(runtime) = runtime_storage_dir_from_env()?",
    );
    expect(fileLog).toContain("runtime_storage_dir_from_env()?");
    expect(fileLog).toContain('data_dir.join("logs")');
  });

  it("isolates database, config, logs, workspace, and WebView state", () => {
    const runtimePaths = read("src-tauri/src/runtime_paths.rs");
    const workspace = read("src-tauri/src/commands/workspace.rs");
    const fileLog = read("src-tauri/src/file_log.rs");
    const lib = read("src-tauri/src/lib.rs");

    expect(runtimePaths).toContain(
      'RUNTIME_DATA_DIR_ENV: &str = "LOCUS_RUNTIME_DATA_DIR"',
    );
    expect(runtimePaths).toContain(
      'RUNTIME_CONFIG_DIR_ENV: &str = "LOCUS_RUNTIME_CONFIG_DIR"',
    );
    expect(runtimePaths).toContain(
      'RUNTIME_LOG_DIR_ENV: &str = "LOCUS_RUNTIME_LOG_DIR"',
    );
    expect(runtimePaths).toContain(
      'RUNTIME_WORKSPACE_DIR_ENV: &str = "LOCUS_RUNTIME_WORKSPACE_DIR"',
    );
    expect(runtimePaths).toContain(
      'WEBVIEW_DATA_DIR_ENV: &str = "WEBVIEW2_USER_DATA_FOLDER"',
    );
    expect(runtimePaths).toContain('"--locus-isolated"');
    expect(runtimePaths).toContain('"--locus-database-dir"');
    expect(runtimePaths).toContain('println!("LOCUS_RUNTIME_JSON {value}")');

    expect(workspace).toContain("runtime_config_dir_from_env()?");
    expect(fileLog).toContain("runtime_log_dir_from_env()?");
    expect(lib).toContain("RuntimeLaunchOptions::configure_from_env_args()");
    expect(lib).toContain("runtime_workspace_for_setup");
  });

  it("provides a bun dev-mcp entry that creates temporary directories by default", () => {
    const packageJson = read("package.json");
    const launcher = read("scripts/run-tauri.mjs");

    expect(packageJson).toContain(
      '"locus:test:app": "bun run tauri dev-mcp-isolated"',
    );
    expect(launcher).toContain(
      'const DEV_WITH_MCP_ISOLATED_COMMAND = "dev-mcp-isolated";',
    );
    expect(launcher).toContain(
      'mkdtempSync(path.join(tmpdir(), "locus-app-test-"))',
    );
    expect(launcher).toContain(
      "env.LOCUS_RUNTIME_DATA_DIR = manifest.databaseDir",
    );
    expect(launcher).toContain(
      "env.LOCUS_RUNTIME_CONFIG_DIR = manifest.configDir",
    );
    expect(launcher).toContain("env.LOCUS_RUNTIME_LOG_DIR = manifest.logDir");
    expect(launcher).toContain(
      "env.LOCUS_RUNTIME_WORKSPACE_DIR = manifest.workspace",
    );
    expect(launcher).toContain(
      "env.WEBVIEW2_USER_DATA_FOLDER = manifest.webviewDataDir",
    );
    expect(launcher).toContain("LOCUS_RUNTIME_JSON");
  });

  it("locks the resolved data directory before opening the session store", () => {
    const lib = read("src-tauri/src/lib.rs");
    const lock = read("src-tauri/src/runtime_data_lock.rs");

    expect(lib.indexOf("RuntimeDataDirLock::acquire(&data_dir)")).toBeLessThan(
      lib.indexOf("SessionStore::new_with_tool_results_root"),
    );
    expect(lock).toContain("try_lock_exclusive");
    expect(lock).toContain("already in use by another process");
  });
});
