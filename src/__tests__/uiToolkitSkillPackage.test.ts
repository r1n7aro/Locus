import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string): string {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("UI Toolkit DevTools Skill package", () => {
  it("registers the editor and Play Mode inspection workflow", () => {
    const manifest = JSON.parse(read("skills/ui-toolkit/skill.json")) as {
      id: string;
      command: { trigger: string };
      tools: Array<{
        name: string;
        runtime: string;
        path: string;
        entryType: string;
        method: string;
        requestEditorStatus: string;
        mutatesWorkspace: boolean;
      }>;
    };

    expect(manifest.id).toBe("ui-toolkit");
    expect(manifest.command.trigger).toBe("/ui-toolkit");
    expect(manifest.tools.map((tool) => tool.name)).toEqual([
      "unity-ui-list-panels",
      "unity-ui-inspect",
      "unity-ui-style",
      "unity-ui-action",
      "unity-ui-wait",
      "unity-ui-highlight",
    ]);
    for (const tool of manifest.tools) {
      expect(tool).toMatchObject({
        runtime: "unity",
        path: "unity/Editor/UIToolkitDevTools.cs",
        entryType: "Locus.Skills.UIToolkitDevTools",
        requestEditorStatus: "any",
      });
    }
    expect(manifest.tools.find((tool) => tool.method === "Action")?.mutatesWorkspace).toBe(true);
  });

  it("documents source edits, viewport capture, and runtime UIDocuments", () => {
    const skill = read("skills/ui-toolkit/SKILL.md");
    const implementation = read("skills/ui-toolkit/unity/Editor/UIToolkitDevTools.cs");

    expect(skill).toContain("Play Mode");
    expect(skill).toContain("Runtime `UIDocument`");
    expect(skill).toContain("unity_capture_viewport");
    expect(skill).toContain("matchedRules[].fullPath");
    expect(skill).toContain("requestEditorStatus");
    expect(skill).not.toContain("view_snapshot");
    expect(implementation).toContain("Resources.FindObjectsOfTypeAll<UIDocument>()");
    expect(implementation).toContain("MatchedRulesExtractor");
    expect(implementation).toContain("EditorApplication.update");
    expect(implementation).toContain("MouseDownEvent.GetPooled");
    expect(implementation).toContain("ClickEvent.GetPooled");
    expect(implementation).toContain('"playing_paused"');
  });
});
