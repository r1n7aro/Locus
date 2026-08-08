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

  it("makes absolute workspace paths project-relative", () => {
    expect(buildToolCallArgsSummary("read", JSON.stringify({
      file_path: "F:\\SampleGame-6.5\\Assets\\Scripts\\ECS\\Runtime\\Time\\TimelineTypes.cs",
    }), {
      workingDir: "f:/samplegame-6.5/",
    })).toBe("Assets/Scripts/ECS/Runtime/Time/TimelineTypes.cs");
  });

  it("keeps other Unity project roots project-relative", () => {
    expect(buildToolCallArgsSummary("read", JSON.stringify({
      file_path: "F:/SampleGame-6.5/Packages/com.example.tool/Editor/Tool.cs",
    }), {
      workingDir: "F:/SampleGame-6.5",
    })).toBe("Packages/com.example.tool/Editor/Tool.cs");
  });

  it("uses the attached directory name as the external root", () => {
    expect(buildToolCallArgsSummary("read", JSON.stringify({
      file_path: "D:/SharedTools/Scripts/Build.cs",
    }), {
      workingDir: "F:/SampleGame-6.5",
      extraWorkdirs: ["D:/SharedTools"],
    })).toBe("SharedTools/Scripts/Build.cs");
  });

  it("keeps a system root for unregistered paths containing Assets", () => {
    expect(buildToolCallArgsSummary("read", JSON.stringify({
      file_path: "D:/OtherProject/Assets/Scripts/Runtime/Time/TimelineTypes.cs",
    }), {
      workingDir: "F:/SampleGame-6.5",
    })).toBe("D:/OtherProject/Assets/…/Time/TimelineTypes.cs");
  });

  it("matches workspace roots on complete path segments", () => {
    expect(buildToolCallArgsSummary("read", JSON.stringify({
      file_path: "F:/SampleGame-6.5-copy/Assets/Scripts/Player.cs",
    }), {
      workingDir: "F:/SampleGame-6.5",
    })).toBe("F:/SampleGame-6.5-copy/Assets/Scripts/Player.cs");
  });

  it("shows url summaries for web_fetch", () => {
    expect(buildToolCallArgsSummary("web_fetch", JSON.stringify({
      url: "https://example.com/docs",
      format: "markdown",
    }))).toBe("https://example.com/docs");
  });

});
