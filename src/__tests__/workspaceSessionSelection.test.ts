import { describe, expect, it } from "vitest";
import {
  resolveWorkspaceSessionContextIds,
  resolveWorkspaceSessionSelection,
} from "../components/workbench/workspaceSessionSelection";

const visibleSessionIds = ["s1", "s2", "s3", "s4"];

describe("workspace session selection", () => {
  it("seeds ctrl selection with the active session and toggles clicked sessions", () => {
    const selected = resolveWorkspaceSessionSelection({
      visibleSessionIds,
      selectedSessionIds: new Set(),
      anchorSessionId: null,
      activeSessionId: "s1",
      clickedSessionId: "s3",
      shiftKey: false,
      ctrlKey: true,
      metaKey: false,
    });

    expect(Array.from(selected.nextSelectedSessionIds)).toEqual(["s1", "s3"]);
    expect(selected.nextAnchorSessionId).toBe("s3");
    expect(selected.shouldActivateSession).toBe(false);

    const toggled = resolveWorkspaceSessionSelection({
      visibleSessionIds,
      selectedSessionIds: selected.nextSelectedSessionIds,
      anchorSessionId: selected.nextAnchorSessionId,
      activeSessionId: "s1",
      clickedSessionId: "s3",
      shiftKey: false,
      ctrlKey: true,
      metaKey: false,
    });

    expect(Array.from(toggled.nextSelectedSessionIds)).toEqual(["s1"]);
  });

  it("selects the visible session range in both shift directions", () => {
    const forward = resolveWorkspaceSessionSelection({
      visibleSessionIds,
      selectedSessionIds: new Set(),
      anchorSessionId: "s2",
      activeSessionId: "s1",
      clickedSessionId: "s4",
      shiftKey: true,
      ctrlKey: false,
      metaKey: false,
    });
    expect(Array.from(forward.nextSelectedSessionIds)).toEqual(["s2", "s3", "s4"]);
    expect(forward.nextAnchorSessionId).toBe("s2");

    const backward = resolveWorkspaceSessionSelection({
      visibleSessionIds,
      selectedSessionIds: forward.nextSelectedSessionIds,
      anchorSessionId: "s4",
      activeSessionId: "s1",
      clickedSessionId: "s2",
      shiftKey: true,
      ctrlKey: false,
      metaKey: false,
    });
    expect(Array.from(backward.nextSelectedSessionIds)).toEqual(["s2", "s3", "s4"]);
  });

  it("uses the active session as the first shift anchor", () => {
    const result = resolveWorkspaceSessionSelection({
      visibleSessionIds,
      selectedSessionIds: new Set(),
      anchorSessionId: null,
      activeSessionId: "s2",
      clickedSessionId: "s4",
      shiftKey: true,
      ctrlKey: false,
      metaKey: false,
    });

    expect(Array.from(result.nextSelectedSessionIds)).toEqual(["s2", "s3", "s4"]);
    expect(result.shouldActivateSession).toBe(false);
  });

  it("returns to activation on a plain click", () => {
    const result = resolveWorkspaceSessionSelection({
      visibleSessionIds,
      selectedSessionIds: new Set(["s1", "s3"]),
      anchorSessionId: "s3",
      activeSessionId: "s1",
      clickedSessionId: "s2",
      shiftKey: false,
      ctrlKey: false,
      metaKey: false,
    });

    expect(result.nextSelectedSessionIds.size).toBe(0);
    expect(result.nextAnchorSessionId).toBe("s2");
    expect(result.shouldActivateSession).toBe(true);
  });

  it("targets the whole selection only when right-clicking inside it", () => {
    expect(resolveWorkspaceSessionContextIds({
      visibleSessionIds,
      selectedSessionIds: new Set(["s1", "s3"]),
      targetSessionId: "s3",
    })).toEqual(["s1", "s3"]);

    expect(resolveWorkspaceSessionContextIds({
      visibleSessionIds,
      selectedSessionIds: new Set(["s1", "s3"]),
      targetSessionId: "s2",
    })).toEqual(["s2"]);
  });
});
