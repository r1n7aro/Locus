import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Python runtime settings", () => {
  it("adds Python runtime selection to general settings", () => {
    const source = read("src/components/settings/GeneralSettings.vue");
    const service = read("src/services/system.ts");

    expect(source).toContain('import BaseDropdown from "../ui/BaseDropdown.vue"');
    expect(source).toContain("getPythonRuntimeState");
    expect(source).toContain("savePythonRuntimeSelection");
    expect(source).toContain('t("settings.general.pythonRuntime")');
    expect(source).toContain("<BaseDropdown");
    expect(source).toContain('menu-align="start"');
    expect(source).toContain("void refreshPythonRuntimeState()");
    expect(source).toContain("@update:model-value=\"selectPythonRuntime\"");
    expect(service).toContain('ipcInvoke<PythonRuntimeState>("get_python_runtime_state")');
    expect(service).toContain('ipcInvoke<PythonRuntimeState>("save_python_runtime_selection"');
  });

  it("loads Python runtime discovery off the blocking command path", () => {
    const command = read("src-tauri/src/commands/system.rs");
    const runtime = read("src-tauri/src/python_runtime.rs");

    expect(command).toContain("spawn_blocking");
    expect(runtime).toContain("command_output_with_timeout");
    expect(runtime).toContain("PY_RUNTIME_PROBE_TIMEOUT");
  });

  it("defines localized Python runtime labels", () => {
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(zh).toContain('"settings.general.pythonRuntime": "Python 运行时"');
    expect(zh).toContain('"settings.general.pythonManaged": "托管 Python"');
    expect(zh).toContain('"settings.general.pythonSystem": "系统 Python"');
    expect(en).toContain('"settings.general.pythonRuntime": "Python Runtime"');
    expect(en).toContain('"settings.general.pythonManaged": "Managed Python"');
    expect(en).toContain('"settings.general.pythonSystem": "System Python"');
  });

  it("adds Git runtime status to general settings", () => {
    const source = read("src/components/settings/GeneralSettings.vue");
    const gitService = read("src/services/git.ts");
    const gitCommands = read("src-tauri/src/commands/git.rs");
    const lib = read("src-tauri/src/lib.rs");

    expect(source).toContain("gitRuntimeState");
    expect(source).toContain("gitSaveRuntimeSelection");
    expect(source).toContain('t("settings.general.gitRuntime")');
    expect(source).toContain("<BaseDropdown");
    expect(source).toContain("void refreshGitRuntimeState()");
    expect(source).toContain("@update:model-value=\"selectGitRuntime\"");
    expect(source).toContain("gitRuntimePath");
    expect(gitService).toContain('ipcInvoke<GitRuntimeState>("git_runtime_state")');
    expect(gitService).toContain('ipcInvoke<GitRuntimeState>("git_save_runtime_selection"');
    expect(gitCommands).toContain("pub struct GitRuntimeState");
    expect(gitCommands).toContain("discover_git_runtimes(false)");
    expect(gitCommands).toContain("git_runtime_state");
    expect(gitCommands).toContain("git_save_runtime_selection");
    expect(lib).toContain("commands::git_runtime_state");
  });

  it("bundles managed Git with the desktop package", () => {
    const pkg = read("package.json");
    const tauriConfig = read("src-tauri/tauri.with_embed_python_git.conf.json");
    const processUtil = read("src-tauri/src/process_util.rs");
    const lib = read("src-tauri/src/lib.rs");
    const script = read("scripts/prepare-managed-git.mjs");

    expect(pkg).toContain('"git:bundle": "bun run scripts/prepare-managed-git.mjs"');
    expect(pkg).toContain("bun run git:bundle");
    expect(tauriConfig).toContain('"./gen/managed-git": "managed-git/"');
    expect(processUtil).toContain("GitDiscoverySource::Managed");
    expect(processUtil).toContain("resolve_git_from_managed_resource");
    expect(processUtil).toContain("git_runtime_key");
    expect(processUtil).toContain("push_git_registry_candidates");
    expect(processUtil).toContain(`resolve_git_from_env()
        .or_else(resolve_git_from_path)
        .or_else(resolve_git_from_common_locations)
        .or_else(resolve_git_from_managed_resource)`);
    expect(lib).toContain("set_managed_git_resource_dir");
    expect(script).toContain("PortableGit");
  });

  it("defines release installer flavors for embedded and no-embed packages", () => {
    const pkg = read("package.json");
    const releaseScript = read("scripts/build-release-installers.mjs");
    const tauriConfig = read("src-tauri/tauri.conf.json");
    const runTauri = read("scripts/run-tauri.mjs");
    const withoutEmbedConfig = read("src-tauri/tauri.without_embed_python_git.conf.json");

    expect(pkg).toContain('"release:installers": "bun run scripts/build-release-installers.mjs"');
    expect(pkg).toContain('"build:tauri": "bun run build:tauri:with_embed_python_git"');
    expect(pkg).toContain('"build:tauri:without_embed_python_git"');
    expect(tauriConfig).not.toContain("managed-python");
    expect(tauriConfig).not.toContain("managed-git");
    expect(runTauri).toContain("tauri.with_embed_python_git.conf.json");
    expect(runTauri).toContain("shouldInjectDefaultReleaseFlavor");
    expect(releaseScript).toContain('"default"');
    expect(releaseScript).toContain('"with_embed_python_git"');
    expect(releaseScript).toContain('"without_embed_python_git"');
    expect(releaseScript).toContain("Windows x64 - without_embed_python_git");
    expect(releaseScript).toContain("`-${suffix}-setup.exe`");
    expect(withoutEmbedConfig).toContain('"beforeBuildCommand": "bun run build:tauri:without_embed_python_git"');
    expect(withoutEmbedConfig).not.toContain("managed-python");
    expect(withoutEmbedConfig).not.toContain("managed-git");
  });
});
