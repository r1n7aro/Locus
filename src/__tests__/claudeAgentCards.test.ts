import { beforeEach, describe, expect, it, vi } from "vitest";
import { computed } from "vue";

const mocks = vi.hoisted(() => ({
  ipcInvoke: vi.fn(),
  subscribe: vi.fn(),
  handlers: new Map<string, (payload: unknown) => unknown>(),
}));

vi.mock("../services/ipc", () => ({ ipcInvoke: mocks.ipcInvoke }));
vi.mock("../services/locusRuntime", () => ({
  getLocusRuntime: () => ({ subscribe: mocks.subscribe }),
}));

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("native Agent cards", () => {
  beforeEach(() => {
    vi.resetModules();
    mocks.ipcInvoke.mockReset();
    mocks.handlers.clear();
    mocks.subscribe.mockReset();
    mocks.subscribe.mockImplementation(async (eventName: string, handler: (payload: unknown) => unknown) => {
      mocks.handlers.set(eventName, handler);
      return () => mocks.handlers.delete(eventName);
    });
  });

  it("neither loads nor listens until a card asks", async () => {
    await import("../services/claudeAgentCards");
    await flush();
    expect(mocks.subscribe).not.toHaveBeenCalled();
    expect(mocks.ipcInvoke).not.toHaveBeenCalled();
  });

  it("loads a conversation's stored cards once and follows live changes", async () => {
    const service = await import("../services/claudeAgentCards");
    mocks.ipcInvoke.mockResolvedValueOnce({ toolu_old: { model: "claude-sonnet-4-5", recordSessionId: "rec-old" } });
    const old = computed(() => service.claudeAgentCard("s1", "toolu_old"));
    const live = computed(() => service.claudeAgentCard("s1", "toolu_new"));
    expect(old.value).toBeNull();
    await flush();
    expect(mocks.ipcInvoke).toHaveBeenCalledWith("get_claude_agent_cards", { sessionId: "s1" });
    expect(old.value).toEqual({ model: "claude-sonnet-4-5", recordSessionId: "rec-old" });
    expect(live.value).toBeNull();

    // The record arrives when the agent starts, its model with its first request.
    const emit = mocks.handlers.get(service.CLAUDE_AGENT_CARD_EVENT)!;
    emit({ sessionId: "s1", toolCallId: "toolu_new", recordSessionId: "rec-new" });
    expect(live.value).toEqual({ model: undefined, recordSessionId: "rec-new" });
    emit({ sessionId: "s1", toolCallId: "toolu_new", model: "claude-opus-5-5" });
    expect(live.value).toEqual({ model: "claude-opus-5-5", recordSessionId: "rec-new" });
    // Cards of another conversation stay apart; a loaded one is not asked again.
    expect(service.claudeAgentCard("s2", "toolu_new")).toBeNull();
    expect(service.claudeAgentCard("s1", "toolu_old")?.model).toBe("claude-sonnet-4-5");
    await flush();
    expect(mocks.ipcInvoke).toHaveBeenCalledTimes(2);
    expect(mocks.ipcInvoke).toHaveBeenLastCalledWith("get_claude_agent_cards", { sessionId: "s2" });
    expect(mocks.subscribe).toHaveBeenCalledTimes(1);
  });

  it("keeps a live change that arrives before the stored cards", async () => {
    const service = await import("../services/claudeAgentCards");
    let answer: (value: Record<string, object>) => void = () => {};
    mocks.ipcInvoke.mockReturnValueOnce(new Promise((resolve) => { answer = resolve; }));
    expect(service.claudeAgentCard("s1", "toolu_1")).toBeNull();
    await flush();
    mocks.handlers.get(service.CLAUDE_AGENT_CARD_EVENT)?.({ sessionId: "s1", toolCallId: "toolu_1", model: "claude-opus-5-5" });
    answer({ toolu_1: { model: "claude-sonnet-4-5", recordSessionId: "rec-1" } });
    await flush();
    expect(service.claudeAgentCard("s1", "toolu_1")).toEqual({ model: "claude-opus-5-5", recordSessionId: "rec-1" });
  });

  it("asks again after a failed query", async () => {
    const service = await import("../services/claudeAgentCards");
    mocks.ipcInvoke.mockRejectedValueOnce(new Error("offline"));
    expect(service.claudeAgentCard("s1", "toolu_1")).toBeNull();
    await flush();
    mocks.ipcInvoke.mockResolvedValueOnce({ toolu_1: { model: "claude-opus-5-5" } });
    service.claudeAgentCard("s1", "toolu_1");
    await flush();
    expect(service.claudeAgentCard("s1", "toolu_1")?.model).toBe("claude-opus-5-5");
    expect(service.claudeAgentCard(null, "toolu_1")).toBeNull();
  });
});
