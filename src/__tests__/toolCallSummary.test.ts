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

  it("keeps compact file summaries for regular file tools", () => {
    expect(buildToolCallArgsSummary("read", JSON.stringify({
      file_path: "Assets/Scripts/Gameplay/PlayerController.cs",
    }))).toBe("…/Gameplay/PlayerController.cs");
  });

  it("shows url summaries for web_fetch", () => {
    expect(buildToolCallArgsSummary("web_fetch", JSON.stringify({
      url: "https://example.com/docs",
      format: "markdown",
    }))).toBe("https://example.com/docs");
  });
});
