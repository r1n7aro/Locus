// @vitest-environment jsdom
import { afterAll, describe, expect, it } from "vitest";
import {
  acknowledgeSessionPromotions, attentionLayoutKey, isSessionUnread, pendingSessionPromotions, sessionAttention,
  updateSessionAttention, type SessionAttentionSnapshot,
} from "../services/sessionAttention";
import type { ProjectExplorerSnapshot } from "../types/workbench";
import { createSessionAttentionServer } from "./fixtures/sessionAttentionServer";

let deliver!: (snapshot: SessionAttentionSnapshot) => void;
const server = createSessionAttentionServer({
  snapshot: () => new Promise((resolve) => { deliver = resolve; }),
});
afterAll(() => server.uninstall());

const tree: ProjectExplorerSnapshot = {
  projectId: "p", presetId: "default", presetName: "Default", revision: 1, manifestPath: "tree.json", presets: [],
  nodes: ["a", "b"].map((id, position) => ({
    nodeId: id, projectId: "p", nodeKind: "resource", resourceKind: "session", resourceId: id, hidden: false, position,
  })),
};

describe("session attention before the first snapshot", () => {
  it("initializes a new layout only once the desktop's results are known", async () => {
    // Starts synchronization; the snapshot is still on its way.
    expect(isSessionUnread("a")).toBe(false);
    const initialized = updateSessionAttention((state) => {
      if (!state.promoted[attentionLayoutKey(tree)]) {
        acknowledgeSessionPromotions(state, tree, pendingSessionPromotions(state, tree));
      }
    });
    deliver({
      revision: 2,
      sequence: 2,
      sessions: {
        a: { sequence: 1, readSequence: 0, kind: "done", runId: "r1", completionSequence: 1, revision: 1, updatedAt: 0 },
        b: { sequence: 2, readSequence: 2, kind: "done", runId: "r2", completionSequence: 2, revision: 2, updatedAt: 0 },
      },
    });
    await initialized;
    expect(isSessionUnread("a")).toBe(true);
    // Completions from before this layout existed are not promoted later.
    expect(pendingSessionPromotions(sessionAttention.value, tree)).toEqual([]);
  });
});
