import { describe, expect, it } from "vitest";
import { buildSessionTree } from "../components/chat/sessionTree";
import type { SessionSummary } from "../types";

function makeSession(overrides: Partial<SessionSummary> & Pick<SessionSummary, "id" | "title" | "sessionType" | "updatedAt">): SessionSummary {
  return {
    agentId: null,
    parentSessionId: null,
    ...overrides,
  };
}

describe("buildSessionTree", () => {
  it("sorts root sessions by latest update time", () => {
    const sessions = [
      makeSession({ id: "older", title: "Older", sessionType: "chat", updatedAt: 100 }),
      makeSession({ id: "newer", title: "Newer", sessionType: "chat", updatedAt: 180 }),
      makeSession({ id: "middle", title: "Middle", sessionType: "chat", updatedAt: 140 }),
    ];

    const tree = buildSessionTree({ sessions });

    expect(tree.map((node) => node.sourceSessionId)).toEqual(["newer", "middle", "older"]);
  });

  it("keeps folder sessions explicit instead of grouping all doc sessions together", () => {
    const sessions = [
      makeSession({ id: "folder-1", title: "Doc Batch 1", sessionType: "folder", updatedAt: 10 }),
      makeSession({ id: "doc-1", title: "Knowledge: Input System", sessionType: "knowledge", updatedAt: 9, parentSessionId: "folder-1" }),
      makeSession({ id: "doc-2", title: "Knowledge: Camera", sessionType: "knowledge", updatedAt: 8 }),
    ];

    const tree = buildSessionTree({ sessions });

    expect(tree).toHaveLength(2);
    expect(tree[0].kind).toBe("folder");
    if (tree[0].kind === "folder") {
      expect(tree[0].label).toBe("Doc Batch 1");
      expect(tree[0].children).toHaveLength(1);
      expect(tree[0].children[0].kind).toBe("session");
      if (tree[0].children[0].kind === "session") {
        expect(tree[0].children[0].title).toBe("Input System");
      }
    }
    expect(tree[1].kind).toBe("session");
    if (tree[1].kind === "session") {
      expect(tree[1].title).toBe("Camera");
    }
  });

  it("nests subagent sessions under their parent session", () => {
    const sessions = [
      makeSession({ id: "root-1", title: "Main Task", sessionType: "chat", updatedAt: 10 }),
      makeSession({ id: "child-1", title: "sub:inspect code", sessionType: "chat", updatedAt: 11, parentSessionId: "root-1", agentId: "explorer" }),
    ];

    const tree = buildSessionTree({ sessions });

    expect(tree).toHaveLength(1);
    expect(tree[0].kind).toBe("session");
    if (tree[0].kind === "session") {
      expect(tree[0].children).toHaveLength(1);
      expect(tree[0].children[0].kind).toBe("session");
      if (tree[0].children[0].kind === "session") {
        expect(tree[0].children[0].title).toBe("inspect code");
        expect(tree[0].children[0].parentSessionId).toBe("root-1");
      }
    }
  });

  it("uses runtime status for queued knowledge sessions", () => {
    const sessions = [
      makeSession({
        id: "doc-queued",
        title: "Knowledge: Event System",
        sessionType: "knowledge",
        updatedAt: 12,
        runtimeStatus: "queued",
      }),
    ];

    const tree = buildSessionTree({ sessions });

    expect(tree).toHaveLength(1);
    expect(tree[0].kind).toBe("session");
    if (tree[0].kind === "session") {
      expect(tree[0].title).toBe("Event System");
      expect(tree[0].status).toBe("queued");
    }
  });

  it("keeps legacy docgen sessions readable", () => {
    const sessions = [
      makeSession({ id: "doc-legacy", title: "Doc: Combat Loop", sessionType: "docgen", updatedAt: 7 }),
      makeSession({ id: "wiki-legacy", title: "Wiki: AI State Machine", sessionType: "docgen", updatedAt: 6 }),
    ];

    const tree = buildSessionTree({ sessions });

    expect(tree).toHaveLength(2);
    expect(tree[0].kind).toBe("session");
    expect(tree[1].kind).toBe("session");
    if (tree[0].kind === "session") {
      expect(tree[0].title).toBe("Combat Loop");
    }
    if (tree[1].kind === "session") {
      expect(tree[1].title).toBe("AI State Machine");
    }
  });
});
