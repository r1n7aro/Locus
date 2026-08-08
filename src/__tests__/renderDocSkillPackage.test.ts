import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string): string {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("RenderDoc Skill package", () => {
  it("loads one capture-only Unity tool", () => {
    const manifest = JSON.parse(read("skills/renderdoc/skill.json")) as {
      id: string;
      command: { trigger: string };
      ignoredMarkdownFiles: string[];
      tools: Array<{
        name: string;
        runtime: string;
        path: string;
        method: string;
        requestEditorStatus: string;
        parameters: { required: string[] };
      }>;
    };

    expect(manifest.id).toBe("renderdoc");
    expect(manifest.command.trigger).toBe("/renderdoc");
    expect(manifest.ignoredMarkdownFiles).toEqual(["runtime/**/LICENSE.md"]);
    expect(manifest.tools).toHaveLength(1);
    expect(manifest.tools[0]).toMatchObject({
      name: "renderdoc-capture-frame",
      runtime: "unity",
      path: "unity/Editor/RenderDocCapture.cs",
      method: "CaptureFrame",
      requestEditorStatus: "any",
    });
    expect(manifest.tools[0].parameters.required).toEqual(["target"]);
  });

  it("keeps frame inspection on RenderDoc embedded Python", () => {
    const skill = read("skills/renderdoc/SKILL.md");
    const capture = read("skills/renderdoc/unity/Editor/RenderDocCapture.cs");
    const inspect = read("skills/renderdoc/scripts/inspect_capture.py");
    const exportTexture = read("skills/renderdoc/scripts/export_texture.py");
    const packageJson = read("package.json");

    expect(skill).toContain("qrenderdoc");
    expect(skill).toContain("import renderdoc as rd");
    expect(skill).toContain("LOCUS_RENDERDOC_PIPELINE_SNAPSHOTS");
    expect(capture).toContain("StartFrameCapture(IntPtr.Zero, IntPtr.Zero)");
    expect(capture).toContain("EndFrameCapture(IntPtr.Zero, IntPtr.Zero)");
    expect(capture).not.toContain("LaunchReplayUI");
    expect(inspect).toContain("controller.GetRootActions()");
    expect(inspect).toContain("controller.GetPipelineState()");
    expect(exportTexture).toContain("controller.SaveTexture");
    expect(packageJson).toContain('"renderdoc:bundle"');
  });
});
