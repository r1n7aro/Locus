import { describe, expect, it } from "vitest";
import { classifyUnitySceneObjectError } from "../services/unity";

describe("classifyUnitySceneObjectError", () => {
  it("detects unloaded scene errors", () => {
    expect(classifyUnitySceneObjectError(new Error("Scene is not loaded in the editor: Assets/Scenes/Main.unity")))
      .toBe("sceneNotLoaded");
  });

  it("detects missing object errors", () => {
    expect(classifyUnitySceneObjectError({ message: "GameObject was not found: Assets/Scenes/Main.unity/Root/Deleted" }))
      .toBe("objectMissing");
  });

  it("falls back to unknown for unrelated errors", () => {
    expect(classifyUnitySceneObjectError("Unity disconnected")).toBe("unknown");
  });
});
