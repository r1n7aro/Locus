import { describe, expect, it } from "vitest";
import { newSessionInsertionIndex, type SessionPlacementNode } from "../stores/workspaceSessionPlacement";

type Node = SessionPlacementNode & { pinned?: boolean };

function layout(...specs: string[]): Node[] {
  return specs.map((spec) => {
    const [kind, id, flag] = spec.split(":");
    return {
      nodeId: id!,
      resourceKind: kind === "folder" ? null : kind,
      resourceId: kind === "folder" ? null : id,
      hidden: flag === "hidden",
      pinned: flag === "pinned",
    };
  });
}

const rules = {
  isSessionAnchor: (node: Node) => node.resourceKind === "session" && !node.hidden && !node.pinned,
  isShownEntry: (node: Node) => !node.hidden,
};

function insertionIndex(nodes: Node[]): number {
  return newSessionInsertionIndex(nodes, nodes.findIndex((node) => node.resourceId === "newSession"), rules);
}

describe("new session insertion", () => {
  it("joins the session list below the entries instead of sessions left above them", () => {
    const nodes = layout(
      "system:newSession", "session:stray-a", "session:stray-b", "system:collaboration", "system:assets",
      "system:views", "system:agents", "system:archived:hidden", "session:main-a", "session:main-b",
    );
    expect(nodes[insertionIndex(nodes)]?.nodeId).toBe("main-a");
  });

  it("skips a session moved between two entries", () => {
    const nodes = layout("system:newSession", "system:collaboration", "session:island", "system:assets", "session:main");
    expect(nodes[insertionIndex(nodes)]?.nodeId).toBe("main");
  });

  it("treats a hidden entry below the list as part of the list", () => {
    const nodes = layout("system:newSession", "system:collaboration", "session:a", "system:archived:hidden", "session:b");
    expect(nodes[insertionIndex(nodes)]?.nodeId).toBe("a");
  });

  it("keeps sessions on top when every entry was moved below them", () => {
    const nodes = layout("system:newSession", "session:a", "session:b", "system:collaboration", "system:assets");
    expect(nodes[insertionIndex(nodes)]?.nodeId).toBe("a");
  });

  it("skips pinned and hidden sessions as anchors", () => {
    const nodes = layout("system:newSession", "system:collaboration", "session:pinned:pinned", "session:archived:hidden", "session:a");
    expect(nodes[insertionIndex(nodes)]?.nodeId).toBe("a");
  });

  it("starts an empty list after the leading entries and knowledge documents", () => {
    const nodes = layout("system:newSession", "system:collaboration", "system:knowledge", "knowledge:plan", "folder:notes");
    expect(insertionIndex(nodes)).toBe(4);
    expect(insertionIndex(layout("system:newSession", "system:collaboration"))).toBe(2);
    expect(newSessionInsertionIndex([], -1, rules)).toBe(0);
  });
});
