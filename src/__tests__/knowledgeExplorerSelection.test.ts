import { describe, expect, it } from "vitest";
import {
  pruneKnowledgeDeleteTargets,
  pruneKnowledgeDragNodes,
  resolveKnowledgeContextSelection,
  resolveKnowledgeExplorerSelection,
} from "../components/knowledge/knowledgeExplorerSelection";

describe("knowledgeExplorerSelection", () => {
  it("seeds ctrl selection with the currently opened item", () => {
    const result = resolveKnowledgeExplorerSelection({
      visiblePaths: ["design/a", "design/b", "design/c.md"],
      selectedPaths: new Set(),
      lastAnchorPath: null,
      clickedPath: "design/c.md",
      shiftKey: false,
      ctrlKey: true,
      metaKey: false,
      seedPath: "design/a",
    });

    expect(Array.from(result.nextSelectedPaths)).toEqual(["design/a", "design/c.md"]);
    expect(result.nextLastAnchorPath).toBe("design/c.md");
    expect(result.shouldHandleAsPlainClick).toBe(false);
  });

  it("builds a contiguous shift range from the last anchor", () => {
    const result = resolveKnowledgeExplorerSelection({
      visiblePaths: ["design/a", "design/b", "design/c.md", "design/d.md"],
      selectedPaths: new Set(["design/c.md"]),
      lastAnchorPath: "design/b",
      clickedPath: "design/d.md",
      shiftKey: true,
      ctrlKey: false,
      metaKey: false,
      seedPath: null,
    });

    expect(Array.from(result.nextSelectedPaths)).toEqual([
      "design/b",
      "design/c.md",
      "design/d.md",
    ]);
    expect(result.nextLastAnchorPath).toBe("design/b");
    expect(result.shouldHandleAsPlainClick).toBe(false);
  });

  it("uses the whole selection when right-click hits an already selected item", () => {
    const paths = resolveKnowledgeContextSelection({
      visiblePaths: ["design/a", "design/b", "design/c.md", "design/d.md"],
      selectedPaths: new Set(["design/b", "design/d.md"]),
      targetPath: "design/d.md",
    });

    expect(paths).toEqual(["design/b", "design/d.md"]);
  });

  it("reduces nested folder and document deletes to the outer folder", () => {
    const targets = pruneKnowledgeDeleteTargets([
      { kind: "document", path: "systems/core-loop.md" },
      { kind: "folder", path: "systems" },
      { kind: "folder", path: "systems/rendering" },
      { kind: "document", path: "ai/brain.md" },
    ]);

    expect(targets).toEqual([
      { kind: "folder", path: "systems" },
      { kind: "document", path: "ai/brain.md" },
    ]);
  });

  it("prunes multi-drag sets to their outermost nodes by tree path", () => {
    const nodes = pruneKnowledgeDragNodes([
      { path: "design/systems/core-loop.md" },
      { path: "design/systems" },
      { path: "design/systems/rendering" },
      { path: "design/ai/brain.md" },
      // duplicate entries collapse to one
      { path: "design/ai/brain.md" },
    ]);

    expect(nodes.map((node) => node.path)).toEqual([
      "design/systems",
      "design/ai/brain.md",
    ]);
  });
});
