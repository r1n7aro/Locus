import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string): string {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Graphics Debugger Skill package", () => {
  it("loads both graphics-debugging backends through existing tools", () => {
    const manifest = JSON.parse(read("skills/graphics-debugger/skill.json")) as {
      id: string;
      version: string;
      command: { trigger: string };
      ignoredMarkdownFiles: string[];
      tools?: unknown[];
      capabilities: {
        unity: Array<{ name: string; path: string; api: string }>;
        python: Array<{ name: string; path: string; module?: string }>;
      };
    };

    expect(manifest.id).toBe("graphics-debugger");
    expect(manifest.version).toBe("1.0.0");
    expect(manifest.command.trigger).toBe("/graphics-debugger");
    expect(manifest.ignoredMarkdownFiles).toEqual(["runtime/**/LICENSE.md"]);
    expect(manifest.tools).toBeUndefined();
    expect(manifest.capabilities.unity).toEqual([
      {
        name: "frame-debugger-csharp-api",
        path: "unity/Editor/FrameDebuggerApi.cs",
        api: "unity_execute",
      },
      {
        name: "renderdoc-csharp-api",
        path: "unity/Editor/RenderDocCapture.cs",
        api: "unity_execute",
      },
    ]);
    expect(manifest.capabilities.python).toEqual([
      {
        name: "renderdoc-python-api",
        path: "scripts/locus_renderdoc.py",
        module: "locus_renderdoc",
      },
      {
        name: "texture-analysis-python-api",
        path: "scripts/locus_texture_analysis.py",
        module: "locus_texture_analysis",
      },
    ]);
  });

  it("routes lightweight inspection and deep replay through one Skill", () => {
    const root = "skills/graphics-debugger";
    const skill = read(`${root}/SKILL.md`);
    const frameDebugger = read(`${root}/unity/Editor/FrameDebuggerApi.cs`);
    const capture = read(`${root}/unity/Editor/RenderDocCapture.cs`);
    const pythonApi = read(`${root}/scripts/locus_renderdoc.py`);
    const textureAnalysis = read(`${root}/scripts/locus_texture_analysis.py`);
    const worker = read(`${root}/scripts/renderdoc_worker.py`);
    const pythonRuntime = read("src-tauri/src/python_runtime.rs");
    const skillRuntime = read("src-tauri/src/commands/skill.rs");
    const agentRuntime = read("src-tauri/src/agent/instance/mod.rs");
    const packageJson = read("package.json");

    expect(skill).toContain("unity_execute");
    expect(skill).toContain("unity_run_states");
    expect(skill).toContain("summary: >-");
    expect(skill).not.toContain("## L1");
    expect(skill).toContain("FrameDebuggerApi.CaptureAsync");
    expect(skill).toContain("InitializeAsync(@\"<Root>\")");
    expect(skill).toContain("ReplayValidation");
    expect(skill).toContain("OpenCapture");
    expect(skill).toContain("import locus_renderdoc as lrd");
    expect(skill).toContain('isinstance(result["data"], bytes)');
    expect(skill).toContain("import locus_texture_analysis as lta");
    expect(frameDebugger).toContain("public static class FrameDebuggerApi");
    expect(frameDebugger).toContain("UnityEngine.FrameDebugger.enabled");
    expect(frameDebugger).toContain(
      'FindType("UnityEditorInternal.FrameDebuggerInternal.FrameDebuggerUtility")',
    );
    expect(frameDebugger).toContain('Invoke("GetFrameEvents")');
    expect(frameDebugger).toContain('Invoke("GetFrameEventData", index, data)');
    expect(frameDebugger).toContain("public static Task<FrameTextureExportResult>");
    expect(frameDebugger).toContain("BlitToRenderTexture");
    expect(capture).toContain("public static class RenderDocCaptureApi");
    expect(capture).toContain("RenderDocInitializeResult");
    expect(capture).toContain("RenderDocBeginCaptureResult");
    expect(capture).toContain("RenderDocTriggerCaptureResult");
    expect(capture).toContain("RenderDocEndCaptureResult");
    expect(capture).toContain("RenderDocCaptureLookupResult");
    expect(capture).toContain("RenderDocCaptureOnceResult");
    expect(capture).toContain("CaptureOnceAsync(");
    expect(capture).toContain("api.MaskOverlayBits(0, 0)");
    expect(capture).toContain("ResolveNativeWindowHandle(window)");
    expect(capture).toContain("GetFocus()");
    expect(capture).toContain("UnityGUIViewWndClass");
    expect(capture).toContain("EnumWindows(");
    expect(capture).toContain("StartFrameCapture(IntPtr.Zero, active.NativeWindowHandle)");
    expect(capture).toContain("EndFrameCapture(IntPtr.Zero, active.NativeWindowHandle)");
    expect(capture).not.toContain("UnityEditorInternal.RenderDoc.BeginCaptureRenderDoc");
    expect(capture).not.toContain("UnityEditorInternal.RenderDoc.EndCaptureRenderDoc");
    expect(capture).toContain("DiscardFrameCapture(IntPtr.Zero, IntPtr.Zero)");
    expect(capture).not.toContain("LaunchReplayUI");
    expect(pythonApi).toContain("def inspect_capture(");
    expect(pythonApi).toContain("def compute_bindings(");
    expect(pythonApi).toContain("def buffer_data(");
    expect(pythonApi).toContain("data = bytes(raw)");
    expect(pythonApi).toContain("def unpack_buffer(");
    expect(pythonApi).toContain("def _invoke_worker(");
    expect(pythonApi).toContain('environment.pop("PYTHONHOME", None)');
    expect(pythonApi).toContain('"--python=" + worker');
    expect(pythonApi).toContain('getattr(subprocess, "CREATE_NO_WINDOW", 0)');
    expect(pythonApi).toContain("creationflags=creation_flags");
    expect(worker).toContain("locus_renderdoc._worker_dispatch(");
    expect(worker).toContain('os.environ["LOCUS_RENDERDOC_MODULE_ROOT"]');
    expect(textureAnalysis).toContain("def analyze_texture(");
    expect(textureAnalysis).toContain("def compare_textures(");
    expect(textureAnalysis).toContain('"edgeDensity"');
    expect(textureAnalysis).toContain('"psnrDb"');
    expect(textureAnalysis).toContain('"changedPixelRatio"');
    expect(pythonRuntime).toContain("register_skill_python_modules");
    expect(pythonRuntime).toContain("active_skill_python_modules()");
    expect(skillRuntime).toContain("pub struct SkillPackagePythonCapability");
    expect(skillRuntime).toContain("pub module: Option<String>");
    expect(skillRuntime).toContain(
      "skill_package_python_modules_for_package_sync_for_working_dir",
    );
    expect(agentRuntime).toContain(
      "crate::python_runtime::register_skill_python_modules(modules)",
    );
    expect(packageJson).toContain('"renderdoc:bundle"');
    expect(read("scripts/prepare-renderdoc-runtime.mjs")).toContain(
      '"skills", "graphics-debugger", "runtime"',
    );
  });

  it("restores active Skill assemblies before compiling unity_run_states", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const runStates = agent.slice(agent.indexOf("async fn execute_unity_run_states"));
    const restoreIndex = runStates.indexOf(
      "self.ensure_active_skill_package_unity_runtimes().await",
    );
    const compileIndex = runStates.indexOf(
      "compile_run_states_with_non_public_access",
    );

    expect(restoreIndex).toBeGreaterThan(0);
    expect(compileIndex).toBeGreaterThan(restoreIndex);
  });
});
