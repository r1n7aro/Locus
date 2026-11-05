import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  listen: vi.fn(),
  invoke: vi.fn(),
  channelHandler: null as null | ((message: unknown) => void),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
  Channel: class {
    constructor(handler: (message: unknown) => void) {
      mocks.channelHandler = handler;
    }
  },
}));
// Inside the Locus Remote app the page also runs under a Tauri runtime.
vi.mock("../services/tauriRuntime", () => ({
  hasTauriWindowRuntime: () => true,
  getCurrentTauriWindowLabel: () => "main",
}));

beforeEach(() => {
  vi.resetModules();
  mocks.channelHandler = null;
  mocks.listen.mockReset().mockResolvedValue(vi.fn());
  mocks.invoke.mockReset().mockResolvedValue(null);
});

function routed(sessionId: string) {
  return {
    eventName: "stream-event",
    streamRevision: 4,
    projectId: "p",
    checkoutId: "c",
    workspaceGeneration: 1,
    payload: { type: "done", sessionId, runId: "r" },
  };
}

describe("remote client transport", () => {
  it("routes interests and workspace events through the remote transport, never Tauri", async () => {
    const { installRemoteClientTransport } = await import("../services/remoteClient");
    const setInterests = vi.fn().mockResolvedValue(undefined);
    let emit!: (event: unknown) => void;
    installRemoteClientTransport({
      setInterests,
      subscribeWorkspaceEvents(handler) {
        emit = handler as (event: unknown) => void;
        return () => {};
      },
    });
    const hub = await import("../services/workspaceEventHub");
    const { flushWindowEventInterests } = await import("../services/windowEventInterests");
    const received = vi.fn();
    await hub.listenWorkspaceEvent("mobile", (event) => received(event.payload), {
      interests: [{ sessionId: "s1" }],
    });
    await flushWindowEventInterests();
    expect(setInterests).toHaveBeenLastCalledWith({ sessions: ["s1"], checkouts: [], streamCheckouts: [] });
    expect(mocks.listen).not.toHaveBeenCalled();
    expect(mocks.invoke).not.toHaveBeenCalled();

    emit(routed("s1"));
    expect(received).toHaveBeenCalledWith(routed("s1"));
    hub.resetWorkspaceEventHubForTests();
  });

  it("refuses commands the desktop did not grant and forwards granted ones", async () => {
    mocks.invoke.mockImplementation(async (command: string, args?: { method?: string }) =>
      command === "remote_invoke" && args?.method === "get_device_info"
        ? { fullControl: false, methods: ["load_session_view", "get_device_info"] }
        : null);
    const { connectAppBridge, RemoteUnavailableError } = await import("../mobile/bridge");
    const bridge = await connectAppBridge();
    expect(mocks.invoke).toHaveBeenCalledWith("remote_subscribe", { onMessage: expect.anything() });
    // The grant list arrives with the first device info.
    await bridge.deviceInfo();
    expect(bridge.canCall("open_workspace")).toBe(false);

    await expect(bridge.runtime.invoke("open_workspace", { path: "C:/x" })).rejects.toBeInstanceOf(RemoteUnavailableError);
    expect(mocks.invoke).not.toHaveBeenCalledWith("remote_invoke", expect.objectContaining({ method: "open_workspace" }));

    await bridge.runtime.invoke("load_session_view", { sessionId: "s1", messageLimit: null });
    expect(mocks.invoke).toHaveBeenLastCalledWith("remote_invoke", {
      method: "load_session_view",
      params: { sessionId: "s1", messageLimit: null },
    });
  });

  it("rethrows a command's own error value", async () => {
    const appError = { code: "workspace.missing", message: "gone" };
    mocks.invoke.mockImplementation(async (command: string, args?: { method?: string }) => {
      if (command === "remote_invoke" && args?.method === "list_dir_entries") {
        throw { code: "failed", message: "gone", data: appError };
      }
      if (command === "remote_invoke" && args?.method === "git_commit") {
        throw { code: "forbidden", message: "needs full control" };
      }
      return null;
    });
    const { connectAppBridge } = await import("../mobile/bridge");
    const bridge = await connectAppBridge();
    await expect(bridge.runtime.invoke("list_dir_entries", {})).rejects.toEqual(appError);
    await expect(bridge.runtime.invoke("git_commit", {})).rejects.toEqual({ code: "forbidden", message: "needs full control" });
  });

  it("delivers channel messages to the matching listeners", async () => {
    const { connectAppBridge } = await import("../mobile/bridge");
    const bridge = await connectAppBridge();
    const events = vi.fn();
    const statuses = vi.fn();
    const resyncs = vi.fn();
    bridge.transport.subscribeWorkspaceEvents(events);
    bridge.onStatus(statuses);
    bridge.onResync(resyncs);

    mocks.channelHandler!({ kind: "event", event: routed("s2") });
    mocks.channelHandler!({ kind: "resync", reason: "overflow" });
    mocks.channelHandler!({ kind: "status", status: { state: "connected", desktopName: "PC", deviceName: "Pixel" } });
    mocks.channelHandler!({ kind: "closing", reason: "revoked" });

    expect(events).toHaveBeenCalledWith(routed("s2"));
    expect(resyncs).toHaveBeenCalledWith("overflow");
    expect(statuses).toHaveBeenNthCalledWith(1, { state: "connected", desktopName: "PC", deviceName: "Pixel" });
    expect(statuses).toHaveBeenNthCalledWith(2, expect.objectContaining({ state: "revoked" }));

    bridge.dispose();
    mocks.channelHandler!({ kind: "event", event: routed("s3") });
    expect(events).toHaveBeenCalledTimes(1);
  });
});
