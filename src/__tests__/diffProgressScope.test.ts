import { beforeEach, describe, expect, it, vi } from "vitest";

const eventMocks = vi.hoisted(() => ({
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: eventMocks.listen,
}));

import {
  computeRequestKey,
  listenDiffProgress,
  type DiffProgressEvent,
} from "../services/diff";
import { WORKSPACE_EVENT_NAME } from "../services/project";

describe("diff progress workspace routing", () => {
  beforeEach(() => {
    eventMocks.listen.mockReset();
  });

  it("accepts only the exact checkout generation", async () => {
    let listener: ((event: { payload: unknown }) => void) | null = null;
    eventMocks.listen.mockImplementation(async (_name, handler) => {
      listener = handler;
      return vi.fn();
    });
    const received: DiffProgressEvent[] = [];
    await listenDiffProgress(
      () => ({ checkoutId: "checkout-a", expectedGeneration: 7 }),
      (event) => received.push(event),
    );
    expect(eventMocks.listen).toHaveBeenCalledWith(
      WORKSPACE_EVENT_NAME,
      expect.any(Function),
    );

    const progress: DiffProgressEvent = {
      requestKey: "request-a",
      phase: "textDiff",
      current: 1,
      total: 4,
      elapsedMs: 12,
    };
    const emit = (checkoutId: string, workspaceGeneration: number) => listener?.({
      payload: {
        eventName: "diff-progress",
        streamRevision: 1,
        projectId: "project",
        checkoutId,
        workspaceGeneration,
        payload: progress,
      },
    });
    emit("checkout-b", 7);
    emit("checkout-a", 6);
    emit("checkout-a", 7);

    expect(received).toEqual([progress]);
  });

  it("includes generation in the request identity", () => {
    const request = {
      source: "gitUnstaged" as const,
      filePath: "Assets/Shared.cs",
      detail: "full" as const,
    };
    expect(computeRequestKey(request, {
      checkoutId: "checkout-a",
      expectedGeneration: 7,
    })).not.toBe(computeRequestKey(request, {
      checkoutId: "checkout-a",
      expectedGeneration: 8,
    }));
  });
});
