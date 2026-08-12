import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("Unity debugger protocol", () => {
  it("ships an excerpt-injected debugger skill with the cooperative protocol", () => {
    const skill = read("knowledge/skill/debugger.md");

    expect(skill).toContain("injectMode: excerpt");
    expect(skill).toContain("skillSurface: both");
    expect(skill).toContain("commandTrigger: /debug");
    expect(skill).toContain("summary: >-");
    expect(skill).not.toContain("## Summary");
    expect(skill).not.toContain("## Content");
    expect(skill).toContain("ctx.ListTickSystems()");
    expect(skill).toContain("ctx.SwitchToThreadPool()");
    expect(skill).toContain("ctx.BreakWhen");
    expect(skill).toContain("ctx.StepFrame()");
    expect(skill).toContain("ctx.ResumeGame()");
    expect(skill).toContain("ends that `unity_execute` invocation");
    expect(skill).toContain("  - bash");
    expect(skill).toContain("Get-Command cdb, windbg, windbgx");
    expect(skill).toContain("injected `windows-native-debuggers` runtime context");
    expect(skill).toContain("PATH, Windows Kits, or WinDbg app execution aliases");
    expect(skill).toContain("`refreshAfterSeconds`");
    expect(skill).toContain("`signatureStatus: not_checked`");
    expect(skill).toContain("recommend installing Microsoft WinDbg or Debugging Tools for Windows");
    expect(skill).toContain("never embed machine-specific discovery results in this Skill");
    expect(skill).toContain("finish the command script with `qd`");
    expect(skill).not.toContain("Program Files (x86)\\Windows Kits");
    expect(skill).not.toMatch(/[A-Z]:\\[^\s`]+cdb\.exe/i);
  });

  it("exposes dynamic tick, thread and pending-await diagnostics in the plugin", () => {
    const tickDebugger = read("locus_unity/Editor/LocusBridge.TickDebugger.cs");
    const execute = read(
      "locus_unity/Editor/ExecuteCodeAsync/LocusBridge.ExecuteCodeAsync.cs",
    );
    const runStates = read("locus_unity/Editor/LocusBridge.RunStates.cs");

    expect(tickDebugger).toContain("UnityTickSystemSnapshot ListTickSystems()");
    expect(tickDebugger).toContain("SwitchToThreadPool()");
    expect(tickDebugger).toContain("ExecuteCodeBreakpointReachedException");
    expect(execute).toContain("sourceText = SourceLineText(sourceLine)");
    expect(execute).toContain('source = "await"');
    expect(runStates).toContain("SetTickSystem(UnityTickSystemInfo system");
    expect(runStates).toContain("SetTickPoint(UnityLoopPoint point)");
  });

  it("injects a read-only native debugger snapshot for command and read activation", () => {
    const runtime = read("src-tauri/src/skill_runtime_context.rs");
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const readFile = read("src-tauri/src/agent/instance/read_file.rs");
    const rustApp = read("src-tauri/src/lib.rs");

    expect(rustApp).toContain("mod skill_runtime_context;");
    expect(runtime).toContain('provider: "windows-native-debuggers"');
    expect(runtime).toContain("augment_path_with_registry_paths");
    expect(runtime).toContain("Windows Kits\\Installed Roots");
    expect(runtime).toContain("AppModel\\Repository\\Packages");
    expect(runtime).toContain('.join("WindowsApps")');
    expect(runtime).toContain("installed: bool");
    expect(runtime).toContain("available: bool");
    expect(runtime).toContain('signature_status: "not_checked".to_string()');
    expect(runtime).toContain("Duration::from_secs(30)");
    expect(runtime).toContain("refresh_after_seconds: u64");
    expect(runtime).toContain('const DEBUGGER_LOGICAL_PATH: &str = "skill/debugger.md"');
    expect(runtime).not.toContain("LEGACY_DEBUGGER");
    expect(runtime).not.toContain('"skill/builtin/debugger.md"');
    expect(agent).toContain("SkillRuntimeContextTrigger::Command");
    expect(agent).toContain("SkillRuntimeContextTrigger::KnowledgeRead");
    expect(readFile).toContain("SkillRuntimeContextTrigger::Read");
  });
});
