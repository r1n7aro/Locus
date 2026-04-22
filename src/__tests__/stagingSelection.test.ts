import { describe, expect, it } from "vitest";
import { resolveStagingFileSelection } from "../components/collab/stagingSelection";

describe("stagingSelection", () => {
  it("keeps a plain click as the shift-range anchor without turning it into multi-select", () => {
    const result = resolveStagingFileSelection({
      visiblePaths: ["Assets/A.cs", "Assets/B.cs", "Assets/C.cs"],
      selectedPaths: new Set(),
      lastClickedPath: null,
      clickedPath: "Assets/A.cs",
      shiftKey: false,
      ctrlKey: false,
      metaKey: false,
    });

    expect([...result.nextSelectedPaths]).toEqual([]);
    expect(result.nextLastClickedPath).toBe("Assets/A.cs");
    expect(result.shouldActivateFile).toBe(true);
  });

  it("selects the full range on the first shift-click after a plain click", () => {
    const firstClick = resolveStagingFileSelection({
      visiblePaths: ["Assets/A.cs", "Assets/B.cs", "Assets/C.cs"],
      selectedPaths: new Set(),
      lastClickedPath: null,
      clickedPath: "Assets/A.cs",
      shiftKey: false,
      ctrlKey: false,
      metaKey: false,
    });

    const shiftClick = resolveStagingFileSelection({
      visiblePaths: ["Assets/A.cs", "Assets/B.cs", "Assets/C.cs"],
      selectedPaths: firstClick.nextSelectedPaths,
      lastClickedPath: firstClick.nextLastClickedPath,
      clickedPath: "Assets/C.cs",
      shiftKey: true,
      ctrlKey: false,
      metaKey: false,
    });

    expect([...shiftClick.nextSelectedPaths]).toEqual([
      "Assets/A.cs",
      "Assets/B.cs",
      "Assets/C.cs",
    ]);
    expect(shiftClick.nextLastClickedPath).toBe("Assets/C.cs");
    expect(shiftClick.shouldActivateFile).toBe(false);
  });

  it("continues to toggle individual files for ctrl-click selection", () => {
    const result = resolveStagingFileSelection({
      visiblePaths: ["Assets/A.cs", "Assets/B.cs", "Assets/C.cs"],
      selectedPaths: new Set(["Assets/A.cs"]),
      lastClickedPath: "Assets/A.cs",
      clickedPath: "Assets/B.cs",
      shiftKey: false,
      ctrlKey: true,
      metaKey: false,
    });

    expect([...result.nextSelectedPaths]).toEqual(["Assets/A.cs", "Assets/B.cs"]);
    expect(result.nextLastClickedPath).toBe("Assets/B.cs");
    expect(result.shouldActivateFile).toBe(false);
  });

  it("falls back to single selection when the previous anchor path disappeared", () => {
    const result = resolveStagingFileSelection({
      visiblePaths: ["Assets/B.cs", "Assets/C.cs"],
      selectedPaths: new Set(),
      lastClickedPath: "Assets/A.cs",
      clickedPath: "Assets/C.cs",
      shiftKey: true,
      ctrlKey: false,
      metaKey: false,
    });

    expect([...result.nextSelectedPaths]).toEqual(["Assets/C.cs"]);
    expect(result.nextLastClickedPath).toBe("Assets/C.cs");
    expect(result.shouldActivateFile).toBe(false);
  });
});
