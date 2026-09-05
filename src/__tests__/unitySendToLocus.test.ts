import { beforeEach, describe, expect, it, vi } from "vitest";

const runtime = vi.hoisted(() => ({
  handler: null as ((payload: unknown) => void) | null,
  subscribe: vi.fn(),
}));

vi.mock("../services/ipc", () => ({
  ipcInvoke: vi.fn(),
}));

vi.mock("../services/locusRuntime", () => ({
  getLocusRuntime: () => ({
    kind: "tauri",
    invoke: vi.fn(),
    subscribe: runtime.subscribe,
  }),
}));

import { subscribeUnitySendToLocus } from "../services/unity";

beforeEach(() => {
  runtime.handler = null;
  runtime.subscribe.mockReset();
  runtime.subscribe.mockImplementation(async (_eventName, handler) => {
    runtime.handler = handler;
    return () => {};
  });
});

describe("Unity Send to Locus events", () => {
  it("forwards native broker attachments with their workspace scope", async () => {
    const received: unknown[] = [];
    await subscribeUnitySendToLocus((payload) => received.push(payload));

    runtime.handler?.({
      eventName: "unity-editor-update",
      checkoutId: "checkout-a",
      workspaceGeneration: 5,
      payload: {},
    });
    runtime.handler?.({
      eventName: "unity-send-to-locus",
      checkoutId: "checkout-a",
      workspaceGeneration: 5,
      payload: {
        files: [{
          path: "C:/tmp/replay.dereplay",
          name: "replay.dereplay",
          typeLabel: "DustEcho Replay",
          isDir: false,
          source: "replay-timeline",
        }],
      },
    });

    expect(received).toEqual([{
      files: [{
        path: "C:/tmp/replay.dereplay",
        name: "replay.dereplay",
        typeLabel: "DustEcho Replay",
        isDir: false,
        source: "replay-timeline",
      }],
      workspaceRef: {
        checkoutId: "checkout-a",
        expectedGeneration: 5,
      },
    }]);
  });
});
