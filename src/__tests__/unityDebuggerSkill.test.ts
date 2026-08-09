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
});
