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
    expect(source).toContain("void refreshPythonRuntimeState(false, false)");
    expect(source).toContain('t("settings.general.pythonSearching")');
    expect(source).toContain('@open="openPythonRuntimeOptions"');
    expect(source).toContain('@click="refreshPythonRuntimeState(true, true)"');
    expect(source).toContain("@update:model-value=\"selectPythonRuntime\"");
    expect(service).toContain("let pythonRuntimeStateCache");
    expect(service).toContain("let currentPythonRuntimeStateCache");
    expect(service).toContain('ipcInvoke<PythonRuntimeState>("get_python_runtime_state", { refresh, discover })');
    expect(service).toContain('ipcInvoke<PythonRuntimeState>("save_python_runtime_selection"');
  });

  it("loads Python runtime discovery off the blocking command path without repeating cached scans", () => {
    const command = read("src-tauri/src/commands/system.rs");
    const runtime = read("src-tauri/src/python_runtime.rs");
    const processUtil = read("src-tauri/src/process_util.rs");

    expect(command).toContain("spawn_blocking");
    expect(command).toContain("refresh.unwrap_or(false)");
    expect(command).toContain("discover.unwrap_or(true)");
    expect(runtime).toContain("command_output_with_timeout");
    expect(runtime).toContain("PY_RUNTIME_PROBE_TIMEOUT");
    expect(runtime).toContain("discover_python_runtimes_cached");
    expect(runtime).toContain("current_python_runtime_state");
    expect(runtime).toContain("suppress_command_window(&mut command)");
    expect(runtime).toContain("managed_python_package_dir");
    expect(runtime).toContain("write_pip_module_shim");
    expect(runtime).toContain("PIP_ZIPAPP");
    expect(processUtil).toContain("pub fn suppress_command_window");
    expect(processUtil).toContain("cmd.creation_flags(CREATE_NO_WINDOW)");
  });

  it("defines localized Python runtime labels", () => {
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(zh).toContain('"settings.general.pythonRuntime": "Python 运行时"');
    expect(zh).toContain("托管 Python 的依赖安装到数据目录");
    expect(zh).toContain('"settings.general.pythonManaged": "托管 Python"');
    expect(zh).toContain('"settings.general.pythonSystem": "系统 Python"');
    expect(zh).toContain('"settings.general.pythonSearching": "搜索中.."');
    expect(en).toContain('"settings.general.pythonRuntime": "Python Runtime"');
    expect(en).toContain("Managed Python installs packages into the data directory");
    expect(en).toContain('"settings.general.pythonManaged": "Managed Python"');
    expect(en).toContain('"settings.general.pythonSystem": "System Python"');
    expect(en).toContain('"settings.general.pythonSearching": "Searching.."');
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
    expect(source).toContain("void refreshGitRuntimeState(false)");
    expect(source).toContain('@click="refreshGitRuntimeState(true)"');
    expect(source).toContain("@update:model-value=\"selectGitRuntime\"");
    expect(source).toContain("gitRuntimePath");
    expect(gitService).toContain("let gitRuntimeStateCache");
    expect(gitService).toContain('ipcInvoke<GitRuntimeState>("git_runtime_state", { refresh })');
    expect(gitService).toContain('ipcInvoke<GitRuntimeState>("git_save_runtime_selection"');
    expect(gitCommands).toContain("pub struct GitRuntimeState");
    expect(gitCommands).toContain("discover_git_runtimes_cached");
    expect(gitCommands).toContain("discover_git_runtimes(false)");
    expect(gitCommands).toContain("git_runtime_state");
    expect(gitCommands).toContain("spawn_blocking");
    expect(gitCommands).toContain("refresh.unwrap_or(false)");
    expect(gitCommands).toContain("git_save_runtime_selection");
    expect(lib).toContain("commands::git_runtime_state");
  });

  it("bundles managed Git with the desktop package", () => {
    const pkg = read("package.json");
    const tauriConfig = read("src-tauri/tauri.with_embed_python_git.conf.json");
    const installer = read("src-tauri/nsis/installer.nsi");
    const processUtil = read("src-tauri/src/process_util.rs");
    const lib = read("src-tauri/src/lib.rs");
    const script = read("scripts/prepare-managed-git.mjs");

    expect(pkg).toContain('"git:bundle": "bun run scripts/prepare-managed-git.mjs"');
    expect(pkg).toContain("bun run git:bundle");
    expect(tauriConfig).toContain('"./gen/managed-git": "managed-git/"');
    expect(installer).toContain("Function LocusDetectSystemGit");
    expect(installer).toContain("SearchPath $LocusGitProbePath \"git.exe\"");
    expect(installer).toContain("System Git is available; skipping bundled Git resources.");
    expect(installer).toContain("StrCpy $R0 \"{{this.[1]}}\" 11");
    expect(installer).toContain("$R0 != \"managed-git\"");
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
    expect(script).toContain("findCachedGitArchive");
    expect(script).toContain("Falling back to cached managed Git archive");
  });

  it("routes managed Python pip installs into the app data directory", () => {
    const script = read("scripts/prepare-managed-python.mjs");
    const runtime = read("src-tauri/src/python_runtime.rs");
    const shell = read("src-tauri/src/tool/builtins/shell.rs");
    const bashTool = read("tools/bash.json");

    expect(script).toContain("PIP_ZIPAPP_URL");
    expect(script).toContain("pip.pyz");
    expect(script).toContain("rmSync(path.join(targetDir, \"python313._pth\")");
    expect(runtime).toContain("managed_python_package_dir");
    expect(runtime).toContain(".join(\"site-packages\")");
    expect(runtime).toContain("write_pip_module_shim");
    expect(shell).toContain("PIP_TARGET");
    expect(shell).toContain("PYTHONHOME");
    expect(shell).toContain("managed_python_path_env");
    expect(bashTool).toContain("Managed Python installs pip packages into the Locus data directory by default");
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
