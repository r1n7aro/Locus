import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  hasRuntime: vi.fn(),
  ipcInvoke: vi.fn(),
  listen: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("../services/ipc", () => ({ ipcInvoke: mocks.ipcInvoke }));
vi.mock("../services/tauriRuntime", () => ({ hasTauriWindowRuntime: mocks.hasRuntime }));

import { PRIMARY_WINDOW_CHANGED_EVENT, watchPrimaryWindow } from "../services/primaryWindow";

let announce: (event: { payload: string | null }) => void;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.hasRuntime.mockReturnValue(true);
  mocks.listen.mockImplementation(async (name: string, handler: typeof announce) => {
    expect(name).toBe(PRIMARY_WINDOW_CHANGED_EVENT);
    announce = handler;
    return vi.fn();
  });
});

describe("primary application window", () => {
  it("is primary outside the desktop runtime", async () => {
    mocks.hasRuntime.mockReturnValue(false);
    const onChange = vi.fn();
    await watchPrimaryWindow("main", onChange);
    expect(onChange).toHaveBeenCalledExactlyOnceWith(true);
    expect(mocks.listen).not.toHaveBeenCalled();
  });

  it("reports the backend's current answer", async () => {
    mocks.ipcInvoke.mockResolvedValue("workspace-app-a");
    const onChange = vi.fn();
    await watchPrimaryWindow("workspace-app-b", onChange);
    expect(onChange).toHaveBeenCalledExactlyOnceWith(false);
  });

  it("takes over when the previous primary window closes", async () => {
    mocks.ipcInvoke.mockResolvedValue("main");
    const onChange = vi.fn();
    await watchPrimaryWindow("workspace-app-b", onChange);
    announce({ payload: "workspace-app-b" });
    expect(onChange.mock.calls.map(([primary]) => primary)).toEqual([false, true]);
  });

  it("ignores a query answer that an announced change already superseded", async () => {
    let answer: (label: string | null) => void = () => {};
    mocks.ipcInvoke.mockReturnValue(new Promise<string | null>((resolve) => { answer = resolve; }));
    const onChange = vi.fn();
    const watching = watchPrimaryWindow("workspace-app-b", onChange);
    await vi.waitFor(() => expect(mocks.ipcInvoke).toHaveBeenCalled());
    announce({ payload: "workspace-app-b" });
    answer("main");
    await watching;
    expect(onChange.mock.calls.map(([primary]) => primary)).toEqual([true]);
  });

  it("keeps listening when the query fails", async () => {
    mocks.ipcInvoke.mockRejectedValue(new Error("unavailable"));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const onChange = vi.fn();
    await watchPrimaryWindow("main", onChange);
    expect(onChange).not.toHaveBeenCalled();
    announce({ payload: "main" });
    expect(onChange).toHaveBeenCalledExactlyOnceWith(true);
    warn.mockRestore();
  });
});
