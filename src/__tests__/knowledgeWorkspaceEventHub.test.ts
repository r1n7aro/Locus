import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  resetKnowledgeWorkspaceEventHubForTests,
  subscribeKnowledgeWorkspaceEvents,
} from "../services/knowledgeWorkspaceEventHub";

const eventMocks = vi.hoisted(() => ({
  listen: vi.fn(),
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
  releases: [] as Array<ReturnType<typeof vi.fn>>,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: eventMocks.listen,
}));

beforeEach(() => {
  resetKnowledgeWorkspaceEventHubForTests();
  eventMocks.handlers.clear();
  eventMocks.releases = [];
  eventMocks.listen.mockReset();
  eventMocks.listen.mockImplementation(
    async (name: string, handler: (event: { payload: unknown }) => void) => {
      eventMocks.handlers.set(name, handler);
      const release = vi.fn();
      eventMocks.releases.push(release);
      return release;
    },
  );
});

describe("knowledgeWorkspaceEventHub", () => {
  it("shares native listeners across subscribers", async () => {
    const firstWorkspace = vi.fn();
    const secondWorkspace = vi.fn();
    const firstPlugins = vi.fn();
    const secondPlugins = vi.fn();

    const releaseFirst = await subscribeKnowledgeWorkspaceEvents(
      firstWorkspace,
      firstPlugins,
    );
    const releaseSecond = await subscribeKnowledgeWorkspaceEvents(
      secondWorkspace,
      secondPlugins,
    );

    expect(eventMocks.listen).toHaveBeenCalledTimes(2);
    eventMocks.handlers.get("locus://workspace-event")?.({
      payload: { eventName: "knowledge-changed" },
    });
    eventMocks.handlers.get("plugins-changed")?.({ payload: undefined });
    expect(firstWorkspace).toHaveBeenCalledTimes(1);
    expect(secondWorkspace).toHaveBeenCalledTimes(1);
    expect(firstPlugins).toHaveBeenCalledTimes(1);
    expect(secondPlugins).toHaveBeenCalledTimes(1);

    releaseFirst();
    expect(eventMocks.releases.every((release) => !release.mock.calls.length)).toBe(true);
    releaseSecond();
    expect(eventMocks.releases.every((release) => release.mock.calls.length === 1)).toBe(true);
  });

  it("isolates a failing subscriber from the remaining panes", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const secondWorkspace = vi.fn();
    const releaseFirst = await subscribeKnowledgeWorkspaceEvents(
      () => { throw new Error("pane failed"); },
      vi.fn(),
    );
    const releaseSecond = await subscribeKnowledgeWorkspaceEvents(secondWorkspace, vi.fn());

    eventMocks.handlers.get("locus://workspace-event")?.({
      payload: { eventName: "knowledge-changed" },
    });
    expect(secondWorkspace).toHaveBeenCalledTimes(1);
    expect(consoleError).toHaveBeenCalledTimes(1);

    releaseFirst();
    releaseSecond();
    consoleError.mockRestore();
  });

  it("releases a partially-created native listener when startup fails", async () => {
    const releaseWorkspace = vi.fn();
    eventMocks.listen.mockImplementation(async (name: string) => {
      if (name === "plugins-changed") throw new Error("plugins listener failed");
      return releaseWorkspace;
    });

    await expect(subscribeKnowledgeWorkspaceEvents(vi.fn(), vi.fn()))
      .rejects.toThrow("plugins listener failed");
    expect(releaseWorkspace).toHaveBeenCalledTimes(1);
  });
});
