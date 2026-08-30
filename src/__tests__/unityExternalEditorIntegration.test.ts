import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();
const read = (path: string) => readFileSync(resolve(cwd, path), "utf8");

describe("Locus Unity external editor integration", () => {
  it("keeps automatic default selection opt-in", () => {
    const config = read("src-tauri/src/config.rs");
    const settings = read("src/components/settings/UnityConnectionSettings.vue");
    const system = read("src/services/system.ts");

    expect(config).toContain("default_unity_external_editor_default_enabled");
    expect(config).toContain("AtomicBool::new(false)");
    expect(settings).toContain("getUnityExternalEditorDefaultEnabled");
    expect(settings).toContain("setUnityExternalEditorDefaultEnabled");
    expect(system).toContain('"set_unity_external_editor_default_enabled"');
  });

  it("registers Locus with Unity and routes C# files to editable workbench tabs", () => {
    const editor = read("locus_unity/Editor/LocusExternalCodeEditor.cs");
    const bootstrap = read("src/composables/useAppBootstrap.ts");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const fileEditor = read("src/components/workbench/WorkspaceFilePreview.vue");
    const service = read("src/services/workspaceExplorer.ts");
    const command = read("src-tauri/src/commands/workspace_explorer.rs");

    expect(editor).toContain("IExternalCodeEditor");
    expect(editor).toContain("CodeEditor.Register");
    expect(editor).toContain('OpenScriptEvent = "locus-open-script"');
    expect(editor).toContain('"--locus-open-script"');
    expect(editor).not.toContain('path.EndsWith(".cs"');
    expect(bootstrap).toContain('"locus-open-script"');
    expect(bootstrap).toContain("stageExternalScriptOpen");
    expect(bootstrap).toContain('setPage("development")');
    expect(workbench).toContain("revealPendingExternalScriptOpen");
    expect(workbench).toContain('kind: "workspaceFile"');
    expect(workbench).toContain("revealPosition(");
    expect(fileEditor).toContain("workspaceFilePreview(");
    expect(fileEditor).toContain("workspaceFileWrite(");
    expect(fileEditor).toContain("EditorView.scrollIntoView");
    expect(service).toContain('"workspace_file_preview"');
    expect(service).toContain('"workspace_file_write"');
    expect(command).toContain("pub async fn workspace_file_preview(");
    expect(command).toContain("pub async fn workspace_file_write(");
  });

  it("generates Unity project files through the Locus plugin", () => {
    const projectFiles = read("locus_unity/Editor/LocusProjectFiles.cs");
    const generator = read("locus_unity/Editor/LocusProjectFileGenerator.cs");
    const bridge = read("locus_unity/Editor/LocusBridge.cs");
    const lsp = read("src-tauri/src/csharp_lsp/mod.rs");

    expect(projectFiles).toContain("LocusProjectFilesAssetPostprocessor");
    expect(projectFiles).toContain("ProjectGeneration");
    expect(projectFiles).toContain("_syncInProgress");
    expect(projectFiles).toContain("GeneratorVersion = 1");
    expect(projectFiles).toContain("LocusProjectFileGenerator.Generate()");
    expect(projectFiles).toContain("LocusProjectFileGeneratorCommand");
    expect(projectFiles).not.toContain("UnityEditor.SyncVS");
    expect(generator).toContain("CompilationPipeline");
    expect(generator).toContain("AssembliesType.Editor");
    expect(generator).toContain("OnGeneratedCSProject");
    expect(generator).toContain("OnGeneratedSlnSolution");
    expect(generator).toContain("WriteIfChanged");
    expect(bridge).toContain('case "sync_project_files"');
    expect(lsp).toContain("sync_project_files(&workspace)");
  });
});
