import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string): string {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("UI Toolkit C# API Skill package", () => {
  it("loads a unity_execute API capability without package tools", () => {
    const manifest = JSON.parse(read("skills/ui-toolkit/skill.json")) as {
      id: string;
      version: string;
      command: { trigger: string };
      tools?: unknown[];
      capabilities: {
        unity: Array<{ name: string; path: string; api: string }>;
      };
    };

    expect(manifest).toMatchObject({
      id: "ui-toolkit",
      version: "1.0.0",
      command: { trigger: "/ui-toolkit" },
    });
    expect(manifest.tools ?? []).toEqual([]);
    expect(manifest.capabilities.unity).toEqual([
      {
        name: "ui-toolkit-csharp-api",
        path: "unity/Editor/UIToolkitApi.cs",
        api: "unity_execute",
      },
    ]);
  });

  it("exposes composable C# handles for Editor and Play Mode panels", () => {
    const skill = read("skills/ui-toolkit/SKILL.md");
    const api = read("skills/ui-toolkit/unity/Editor/UIToolkitApi.cs");
    const runtime = read("skills/ui-toolkit/unity/Editor/UIToolkitDevTools.cs");

    expect(skill).toContain("UIToolkitApi.Open()");
    expect(skill).toContain("Runtime `UIDocument`");
    expect(skill).toContain("unity_execute");
    expect(skill).toContain("unity_capture_viewport");
    expect(skill).not.toContain("unity_ui_");
    expect(skill).not.toContain("view_snapshot");

    expect(api).toContain("public static class UIToolkitApi");
    expect(api).toContain("public sealed class UIToolkitSession");
    expect(api).toContain("public sealed class UIPanel");
    expect(api).toContain("public sealed class UIElement");
    expect(api).toContain("public Task<UIWaitResult> WaitAsync");
    expect(api).toContain("public UIStylePreview SetStyles");
    expect(api).toContain("public static string Json(object value)");
    expect(runtime).toContain("internal static class UIToolkitDevTools");
    expect(runtime).toContain("Resources.FindObjectsOfTypeAll<UIDocument>()");
    expect(runtime).toContain("MatchedRulesExtractor");
    expect(runtime).toContain("EditorApplication.update");
    expect(runtime).toContain("ClickEvent.GetPooled");
  });

  it("keeps inspection bounded and styles opt-in", () => {
    const api = read("skills/ui-toolkit/unity/Editor/UIToolkitApi.cs");
    const runtime = read("skills/ui-toolkit/unity/Editor/UIToolkitDevTools.cs");

    expect(api).toContain("public int Depth { get; set; } = 2");
    expect(api).toContain("public int MaxElements { get; set; } = 80");
    expect(api).toContain("public bool IncludeComputedStyle { get; set; }");
    expect(api).toContain("public bool IncludeMatchedRules { get; set; }");
    expect(runtime).not.toContain('result["fullType"]');
    expect(runtime).not.toContain('result["childIds"]');
    expect(runtime).toContain("CompactTextLimit = 160");
  });

  it("reuses one session panel snapshot and validates only the selected panel", () => {
    const api = read("skills/ui-toolkit/unity/Editor/UIToolkitApi.cs");
    const runtime = read("skills/ui-toolkit/unity/Editor/UIToolkitDevTools.cs");

    expect(api).toContain("private List<UIPanel> _panelSnapshot;");
    expect(api).toContain("public IList<UIPanel> RefreshPanels(bool includeEmpty = false)");
    expect(api).toContain("new UIToolkitDevTools.ListPanelsRequest { includeEmpty = true }");
    expect(runtime).toContain("private static PanelRecord RefreshPanelSnapshot(PanelRecord record)");
    expect(runtime).toContain("!ReferenceEquals(record.root.panel, record.panel)");
    expect(runtime.match(/DiscoverPanels\(\)/g)).toHaveLength(2);
    expect(runtime).not.toMatch(/RequirePanel\(string panelId\)[\s\S]*?\{[\s\S]*?DiscoverPanels\(\)/);
  });

  it("bounds JSON serialization and skips value-type property explosions", () => {
    const skill = read("skills/ui-toolkit/SKILL.md");
    const api = read("skills/ui-toolkit/unity/Editor/UIToolkitApi.cs");

    expect(skill).toContain("值类型只读取公开字段");
    expect(api).toContain("private const int MaxNodes = 4096;");
    expect(api).toContain("private const int MaxMembersPerObject = 64;");
    expect(api).toContain("private const int MaxCollectionItems = 512;");
    expect(api).toContain("private const int MaxOutputChars = 256 * 1024;");
    expect(api).toContain("if (!type.IsValueType)");
    expect(api).toContain('"<" + property.Name + ">k__BackingField"');
    expect(api).toContain("if (backingField == null && !trustedGetters)");
    expect(api).toContain('WriteTruncationMember(state, ref first, "<max-members>")');
    expect(api).toContain('StopWithValue(state, "<max-output>")');
  });
});
