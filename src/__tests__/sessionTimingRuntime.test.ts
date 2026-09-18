// @vitest-environment jsdom
import { createApp, nextTick, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SessionContextUsageWindow from "../components/SessionContextUsageWindow.vue";
import type { SessionContextUsageReport, StreamEvent } from "../types";
import zh from "../language/zh.json";

const mocks = vi.hoisted(() => ({
  getReport: vi.fn(),
  subscribe: vi.fn(),
  unsubscribe: vi.fn(),
}));

vi.mock("../services/session", () => ({ getSessionContextUsageReport: mocks.getReport }));
vi.mock("../services/sessionStreamEventHub", () => ({ subscribeSessionStreamEvents: mocks.subscribe }));
vi.mock("../i18n", () => ({
  t: (key: string, ...args: unknown[]) => {
    const text = zh[key as keyof typeof zh] ?? key;
    return text.replace(/\{(\d+)\}/g, (_, index: string) => String(args[Number(index)]));
  },
}));

let app: App | undefined;
let root: HTMLDivElement;
let report: SessionContextUsageReport;
let onStream: (dispatch: { event: StreamEvent }) => void;

async function settle() {
  for (let index = 0; index < 8; index++) await Promise.resolve();
  await nextTick();
}

async function mount() {
  app = createApp(SessionContextUsageWindow, { sessionId: "session", tokenUsage: report.usage });
  app.mount(root);
  await settle();
}

function values() {
  return Array.from(root.querySelectorAll(".context-timing-metrics strong"), (element) => element.textContent);
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  report = {
    sessionId: "session", sessionTitle: "计时测试", agentId: "unity", modelId: "model",
    contextTokens: 100, contextLimit: 1000, rawEstimatedContextTokens: 100, reportedContextTokens: 100,
    breakdown: { systemPromptTokens: 0, environmentTokens: 0, rulesTokens: 0, knowledgeTokens: 0,
      runtimeInjectionTokens: 0, conversationTokens: 100, toolDefinitionTokens: 0, activeToolResultTokens: 0 },
    tools: [], cacheInvalidations: [],
    usage: { totalInputTokens: 100, totalOutputTokens: 10, totalCacheReadTokens: 0, totalCacheWriteTokens: 0,
      timedOutputTokens: 10, modelActiveDurationMs: 47000, totalCostUsd: 0, pricedRounds: 0,
      contextTokens: 100, contextLimit: 1000 },
    timing: { remoteOutputDurationMs: 47000, localToolDurationMs: 92000, totalDurationMs: 3723000 },
  };
  mocks.getReport.mockImplementation(async () => structuredClone(report));
  mocks.subscribe.mockImplementation((listener: typeof onStream) => {
    onStream = listener;
    return mocks.unsubscribe;
  });
  root = document.createElement("div");
  document.body.append(root);
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  root.remove();
  vi.useRealTimers();
});

describe("session timing statistics", () => {
  it("shows all three durations immediately below token usage with their definitions", async () => {
    await mount();
    const timing = root.querySelector(".context-timing-section")!;
    expect(timing.previousElementSibling?.getAttribute("aria-label")).toBe("Token 消耗");
    expect(timing.getAttribute("aria-label")).toBe("会话耗时统计");
    expect(values()).toEqual(["47 秒", "1 分 32 秒", "1 小时 2 分 3 秒"]);
    expect(timing.querySelectorAll("[title]")).toHaveLength(3);
  });

  it("distinguishes missing historical records from zero and subsecond durations", async () => {
    report.timing = { remoteOutputDurationMs: null, localToolDurationMs: 0, totalDurationMs: 120 };
    await mount();
    expect(values()).toEqual(["—", "0 秒", "< 1 秒"]);
  });

  it("refreshes after tool completion without a token change and unsubscribes on close", async () => {
    await mount();
    onStream({ event: { type: "done", sessionId: "other-session", runId: "run", messageId: "m", fullText: "" } });
    await vi.advanceTimersByTimeAsync(300);
    expect(mocks.getReport).toHaveBeenCalledTimes(1);
    report.timing.localToolDurationMs = 100_000;
    onStream({ event: { type: "toolCallDone", sessionId: "session", runId: "run", toolCallId: "tool",
      toolName: "bash", output: "done", outcome: "done" } });
    await vi.advanceTimersByTimeAsync(300);
    await settle();
    expect(values()[1]).toBe("1 分 40 秒");
    expect(mocks.getReport).toHaveBeenCalledTimes(2);
    app!.unmount();
    app = undefined;
    expect(mocks.unsubscribe).toHaveBeenCalledOnce();
  });
});
