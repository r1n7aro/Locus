import { describe, expect, it } from "vitest";
import { workspaceRelativeFolder } from "../services/markdownImage";

describe("workspaceRelativeFolder", () => {
  it("returns the folder relative to the workspace root", () => {
    expect(workspaceRelativeFolder("F:/Projects/Game", "F:\\Projects\\Game\\Docs\\images")).toBe("Docs/images");
    expect(workspaceRelativeFolder("F:\\Projects\\Game\\", "f:/projects/game/Docs/")).toBe("Docs");
    expect(workspaceRelativeFolder("/home/me/game", "/home/me/game/assets")).toBe("assets");
  });

  it("rejects the root itself and folders outside the workspace", () => {
    expect(workspaceRelativeFolder("F:/Projects/Game", "F:/Projects/Game")).toBeNull();
    expect(workspaceRelativeFolder("F:/Projects/Game", "F:/Projects/GameAssets/images")).toBeNull();
    expect(workspaceRelativeFolder("F:/Projects/Game", "E:/Other")).toBeNull();
    // POSIX paths keep their case.
    expect(workspaceRelativeFolder("/home/me/game", "/home/me/Game/assets")).toBeNull();
  });
});
