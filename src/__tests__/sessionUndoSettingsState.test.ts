import { describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  getEnabled: vi.fn(),
  setEnabled: vi.fn(),
  subscribe: vi.fn(),
  eventHandler: null as ((payload: { enabled: boolean }) => void) | null,
}));

vi.mock("../services/system", () => ({
  getSessionUndoEnabled: mocks.getEnabled,
  setSessionUndoEnabled: mocks.setEnabled,
}));

vi.mock("../services/locusRuntime", () => ({
  getLocusRuntime: () => ({
    kind: "tauri",
    invoke: vi.fn(),
    subscribe: mocks.subscribe,
  }),
}));

import {
  SESSION_UNDO_ENABLED_CHANGED_EVENT,
  useSessionUndoSettings,
} from "../composables/useSessionUndoSettings";

describe("shared session undo settings", () => {
  it("loads persisted state, synchronizes runtime events, and saves changes", async () => {
    mocks.getEnabled.mockResolvedValue(false);
    mocks.setEnabled.mockResolvedValue(undefined);
    mocks.subscribe.mockImplementation(async (
      eventName: string,
      handler: (payload: { enabled: boolean }) => void,
    ) => {
      expect(eventName).toBe(SESSION_UNDO_ENABLED_CHANGED_EVENT);
      mocks.eventHandler = handler;
      return vi.fn();
    });
    const { state, load, setEnabled } = useSessionUndoSettings();

    await expect(load(true)).resolves.toBe(false);
    expect(state.enabled).toBe(false);
    expect(state.ready).toBe(true);
    expect(mocks.subscribe).toHaveBeenCalledOnce();

    mocks.eventHandler?.({ enabled: true });
    expect(state.enabled).toBe(true);

    await setEnabled(false);
    expect(mocks.setEnabled).toHaveBeenCalledWith(false);
    expect(state.enabled).toBe(false);
    expect(state.busy).toBe(false);
  });
});
