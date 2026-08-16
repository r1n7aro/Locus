import { describe, expect, it } from "vitest";
import { buildToolCallArgsSummary } from "../components/toolCallSummary";

describe("toolCallSummary", () => {
  it("shows unity_yaml_read file and object path before detail mode", () => {
    const summary = buildToolCallArgsSummary("unity_yaml_read", JSON.stringify({
      detail: "components",
      file_path: "Assets/Gameplay/Combat/Prefabs/DestructibleBlocks/DestructibleBlock_OrangeYellow.prefab",
      max_array_items: 20,
      object_path: "DestructibleBlock_OrangeYellow",
    }));

    expect(summary).toBe(
      "Assets/Gameplay/Combat/Prefabs/DestructibleBlocks/DestructibleBlock_OrangeYellow.prefab/DestructibleBlock_OrangeYellow",
    );
  });

  it("shows unity_yaml_read file path for document reads", () => {
    expect(buildToolCallArgsSummary("unity_yaml_read", JSON.stringify({
      detail: "document",
      file_path: "Assets/Materials/Ground.mat",
    }))).toBe("Assets/Materials/Ground.mat");
  });

  it("keeps project-relative file summaries from Assets", () => {
    expect(buildToolCallArgsSummary("read", JSON.stringify({
      file_path: "Assets/Scripts/Gameplay/PlayerController.cs",
    }))).toBe("Assets/Scripts/Gameplay/PlayerController.cs");
  });

  it("shows asset-qualified Property Tree paths for read and search", () => {
    const path = "Assets/Actions/LightNormalAttack1.asset/hitTrack/clips/4";
    expect(buildToolCallArgsSummary("unity_yaml_read", JSON.stringify({ path, depth: 2 })))
      .toBe(path);
    expect(buildToolCallArgsSummary("unity_yaml_search", JSON.stringify({ path, query: "damage" })))
      .toBe(path);
  });

  it("makes absolute workspace paths project-relative", () => {
    expect(buildToolCallArgsSummary("read", JSON.stringify({
      file_path: "F:\\SampleGame\\Assets\\Scripts\\ECS\\Runtime\\Time\\TimelineTypes.cs",
    }), {
      workingDir: "f:/samplegame/",
    })).toBe("Assets/Scripts/ECS/Runtime/Time/TimelineTypes.cs");
  });

  it("keeps other Unity project roots project-relative", () => {
    expect(buildToolCallArgsSummary("read", JSON.stringify({
      file_path: "F:/SampleGame/Packages/com.example.tool/Editor/Tool.cs",
    }), {
      workingDir: "F:/SampleGame",
    })).toBe("Packages/com.example.tool/Editor/Tool.cs");
  });

  it("uses the attached directory name as the external root", () => {
    expect(buildToolCallArgsSummary("read", JSON.stringify({
      file_path: "D:/SharedTools/Scripts/Build.cs",
    }), {
      workingDir: "F:/SampleGame",
      extraWorkdirs: ["D:/SharedTools"],
    })).toBe("SharedTools/Scripts/Build.cs");
  });

  it("keeps a system root for unregistered paths containing Assets", () => {
    expect(buildToolCallArgsSummary("read", JSON.stringify({
      file_path: "D:/OtherProject/Assets/Scripts/Runtime/Time/TimelineTypes.cs",
    }), {
      workingDir: "F:/SampleGame",
    })).toBe("D:/OtherProject/Assets/…/Time/TimelineTypes.cs");
  });

  it("matches workspace roots on complete path segments", () => {
    expect(buildToolCallArgsSummary("read", JSON.stringify({
      file_path: "F:/SampleGame-copy/Assets/Scripts/Player.cs",
    }), {
      workingDir: "F:/SampleGame",
    })).toBe("F:/SampleGame-copy/Assets/Scripts/Player.cs");
  });

  it("shows url summaries for web_fetch", () => {
    expect(buildToolCallArgsSummary("web_fetch", JSON.stringify({
      url: "https://example.com/docs",
      format: "markdown",
    }))).toBe("https://example.com/docs");
  });

});
