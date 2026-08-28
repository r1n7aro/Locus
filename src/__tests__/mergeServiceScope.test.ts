import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceRef } from "../services/project";

const eventMocks = vi.hoisted(() => {
  const handlers = new Map<string, Array<(event: { payload: unknown }) => void>>();
  return {
    handlers,
    listen: vi.fn(async (eventName: string, handler: (event: { payload: unknown }) => void) => {
      const current = handlers.get(eventName) ?? [];
      current.push(handler);
      handlers.set(eventName, current);
      return () => {
        handlers.set(
          eventName,
          (handlers.get(eventName) ?? []).filter(candidate => candidate !== handler),
        );
      };
    }),
  };
});

vi.mock("@tauri-apps/api/event", () => ({
  listen: eventMocks.listen,
}));

vi.mock("../services/ipc", () => ({
  ipcInvoke: vi.fn(),
}));

import { ipcInvoke } from "../services/ipc";
import {
  listenMergeProgress,
  mergeSemanticApply,
  mergeSemanticSession,
  mergeSemanticTarget,
  mergeSemanticValidate,
  type MergeProgressEvent,
} from "../services/merge";
import { WORKSPACE_EVENT_NAME } from "../services/project";

const mockedInvoke = vi.mocked(ipcInvoke);
const checkoutA: WorkspaceRef = {
  checkoutId: "checkout-a",
  expectedGeneration: 3,
};
const checkoutB: WorkspaceRef = {
  checkoutId: "checkout-b",
  expectedGeneration: 7,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

function emitEvent(eventName: string, payload: unknown) {
  for (const handler of eventMocks.handlers.get(eventName) ?? []) {
    handler({ payload });
  }
}

const sessionRequest = {
  filePath: "Assets/Scene.unity",
  baseOid: "base",
  leftOid: "left",
  rightOid: "right",
};

const applyRequest = {
  mergeKey: "merge-key",
  filePath: "Assets/Scene.unity",
  resolutions: {},
};

describe("merge service workspace scope", () => {
  beforeEach(() => {
    eventMocks.handlers.clear();
    eventMocks.listen.mockClear();
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue(undefined);
  });

  it("forwards WorkspaceRef through every semantic merge IPC", async () => {
    await mergeSemanticSession(sessionRequest, checkoutA);
    await mergeSemanticTarget({ mergeKey: "merge-key", targetId: "target" }, checkoutA);
    await mergeSemanticValidate(applyRequest, checkoutA);
    await mergeSemanticApply(applyRequest, checkoutA);

    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "git_merge_semantic_session", {
      request: sessionRequest,
      workspaceRef: checkoutA,
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "git_merge_semantic_target", {
      request: { mergeKey: "merge-key", targetId: "target" },
      workspaceRef: checkoutA,
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(3, "git_merge_semantic_validate", {
      request: applyRequest,
      workspaceRef: checkoutA,
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(4, "git_merge_semantic_apply", {
      request: applyRequest,
      workspaceRef: checkoutA,
    });
  });

  it("ignores bare progress and routes only matching scoped workspace events", async () => {
    const progressA = vi.fn();
    const progressB = vi.fn();
    const unlistenA = await listenMergeProgress(progressA, checkoutA);
    const unlistenB = await listenMergeProgress(progressB, checkoutB);
    const requestA = deferred<any>();
    const requestB = deferred<any>();
    mockedInvoke
      .mockImplementationOnce(() => requestA.promise)
      .mockImplementationOnce(() => requestB.promise);

    const pendingA = mergeSemanticSession(sessionRequest, checkoutA);
    const bareProgress: MergeProgressEvent = {
      requestKey: "merge:Assets/Scene.unity",
      phase: "parseYaml",
      current: 1,
      total: 4,
      elapsedMs: 5,
    };
    emitEvent("merge-progress", bareProgress);
    expect(progressA).not.toHaveBeenCalled();
    expect(progressB).not.toHaveBeenCalled();

    const pendingB = mergeSemanticSession(sessionRequest, checkoutB);
    emitEvent("merge-progress", bareProgress);
    expect(progressA).not.toHaveBeenCalled();
    expect(progressB).not.toHaveBeenCalled();

    emitEvent(WORKSPACE_EVENT_NAME, {
      eventName: "merge-progress",
      streamRevision: 8,
      projectId: "project",
      checkoutId: checkoutB.checkoutId,
      workspaceGeneration: 6,
      payload: bareProgress,
    });
    expect(progressB).not.toHaveBeenCalled();

    emitEvent(WORKSPACE_EVENT_NAME, {
      eventName: "merge-progress",
      streamRevision: 9,
      projectId: "project",
      checkoutId: checkoutB.checkoutId,
      workspaceGeneration: checkoutB.expectedGeneration,
      payload: bareProgress,
    });
    expect(progressB).toHaveBeenCalledWith(bareProgress);
    expect(progressA).not.toHaveBeenCalled();

    requestA.resolve({});
    requestB.resolve({});
    await Promise.all([pendingA, pendingB]);
    unlistenA();
    unlistenB();
  });
});
