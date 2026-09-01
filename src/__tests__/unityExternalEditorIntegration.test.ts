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

  it("registers Locus with Unity and routes source files to editable workbench tabs", () => {
    const editor = read("locus_unity/Editor/LocusExternalCodeEditor.cs");
    const sourceExtensions = editor.slice(
      editor.indexOf("LocusSourceExtensions"),
      editor.indexOf("LocusSourceFileNames"),
    );
    const bootstrap = read("src/composables/useAppBootstrap.ts");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const fileEditor = read("src/components/workbench/WorkspaceFilePreview.vue");
    const service = read("src/services/workspaceExplorer.ts");
    const command = read("src-tauri/src/commands/workspace_explorer.rs");

    expect(editor).toContain("IExternalCodeEditor");
    expect(editor).toContain("CodeEditor.Register");
    expect(editor).toContain('OpenScriptEvent = "locus-open-script"');
    expect(editor).toContain('"--locus-open-script"');
    expect(editor).toContain('".asmdef"');
    expect(editor).toContain('".cs"');
    expect(editor).toContain('".shader"');
    expect(editor).toContain("EditorSettings.projectGenerationUserExtensions");
    expect(editor).toMatch(/if \(!ShouldOpenInLocus\(assetPath\)\)\s+return false;/);
    expect(sourceExtensions).not.toContain('".asset"');
    expect(sourceExtensions).not.toContain('".fbx"');
    expect(sourceExtensions).not.toContain('".png"');
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

  it("leaves Unity-native assets to their default double-click behavior", () => {
    const editor = read("locus_unity/Editor/LocusExternalCodeEditor.cs");
    const unityFallback = editor.indexOf("if (ShouldUseUnityDefaultOpen(assetPath))");
    const sourceBoundary = editor.indexOf("if (!ShouldOpenInLocus(assetPath))");
    const locusDispatch = editor.indexOf("if (LocusBridge.HasConnectedDesktopClient())");

    expect(editor).toContain('path.EndsWith(".unity", StringComparison.OrdinalIgnoreCase)');
    expect(editor).toContain('path.EndsWith(".prefab", StringComparison.OrdinalIgnoreCase)');
    expect(editor).toContain("AssetDatabase.LoadMainAssetAtPath(path)");
    expect(editor).toContain("#if UNITY_6000_5_OR_NEWER");
    expect(editor).toContain("AssetDatabase.CanOpenAssetInEditor(asset.GetEntityId())");
    expect(editor).toContain("AssetDatabase.CanOpenAssetInEditor(asset.GetInstanceID())");
    expect(editor).toMatch(/if \(ShouldUseUnityDefaultOpen\(assetPath\)\)\s+return false;/);
    expect(unityFallback).toBeGreaterThan(-1);
    expect(unityFallback).toBeLessThan(locusDispatch);
    expect(sourceBoundary).toBeGreaterThan(unityFallback);
    expect(sourceBoundary).toBeLessThan(locusDispatch);
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
