import { resetWorkspaceEventHubForTests } from "../services/workspaceEventHub";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { snapshotWorkspaceEvents } from "../services/workspaceEventHub";
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
    resetWorkspaceEventHubForTests();
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
    expect(eventMocks.releases.reduce((count, release) => count + release.mock.calls.length, 0)).toBe(1);
    expect(snapshotWorkspaceEvents().nativeListenerCount).toBe(1);
    expect(snapshotWorkspaceEvents().subscribers).toHaveLength(0);

    const releaseAgain = await subscribeKnowledgeWorkspaceEvents(vi.fn(), vi.fn());
    expect(eventMocks.listen.mock.calls.filter(([event]) => event === "locus://workspace-event")).toHaveLength(1);
    releaseAgain();
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

  it("releases the business subscription but keeps the window listener when plugin startup fails", async () => {
    const releaseWorkspace = vi.fn();
    eventMocks.listen.mockImplementation(async (name: string) => {
      if (name === "plugins-changed") throw new Error("plugins listener failed");
      return releaseWorkspace;
    });

    await expect(subscribeKnowledgeWorkspaceEvents(vi.fn(), vi.fn()))
      .rejects.toThrow("plugins listener failed");
    expect(releaseWorkspace).not.toHaveBeenCalled();
    expect(snapshotWorkspaceEvents().subscribers).toHaveLength(0);
  });
});
