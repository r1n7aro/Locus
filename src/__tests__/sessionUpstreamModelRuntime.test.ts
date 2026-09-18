// @vitest-environment jsdom
import { createApp, nextTick, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SessionContextUsageWindow from "../components/SessionContextUsageWindow.vue";
import type { SessionContextUsageReport, StreamEvent } from "../types";
import zh from "../language/zh.json";

const mocks = vi.hoisted(() => ({ getReport: vi.fn(), subscribe: vi.fn() }));
vi.mock("../services/session", () => ({ getSessionContextUsageReport: mocks.getReport }));
vi.mock("../services/sessionStreamEventHub", () => ({ subscribeSessionStreamEvents: mocks.subscribe }));
vi.mock("../i18n", () => ({
  t: (key: string, ...args: unknown[]) => (zh[key as keyof typeof zh] ?? key)
    .replace(/\{(\d+)\}/g, (_, index: string) => String(args[Number(index)])),
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

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  report = {
    sessionId: "session", sessionTitle: "模型测试", agentId: "unity", modelId: "openai/local-alias",
    upstreamModel: "actual-upstream-model", contextTokens: 100, contextLimit: 1000,
    rawEstimatedContextTokens: 100, reportedContextTokens: 100,
    breakdown: { systemPromptTokens: 0, environmentTokens: 0, rulesTokens: 0, knowledgeTokens: 0,
      runtimeInjectionTokens: 0, conversationTokens: 100, toolDefinitionTokens: 0, activeToolResultTokens: 0 },
    tools: [], cacheInvalidations: [],
    usage: { totalInputTokens: 100, totalOutputTokens: 10, totalCacheReadTokens: 0, totalCacheWriteTokens: 0,
      timedOutputTokens: 10, modelActiveDurationMs: 1000, totalCostUsd: 0, pricedRounds: 0,
      contextTokens: 100, contextLimit: 1000 },
    timing: { remoteOutputDurationMs: null, localToolDurationMs: null, totalDurationMs: null },
  };
  mocks.getReport.mockImplementation(async () => structuredClone(report));
  mocks.subscribe.mockImplementation((listener: typeof onStream) => { onStream = listener; return () => {}; });
  root = document.createElement("div");
  document.body.append(root);
});

afterEach(() => { app?.unmount(); app = undefined; root.remove(); vi.useRealTimers(); });

describe("session upstream model", () => {
  it("shows the local selection and upstream identity together without replacing context limits", async () => {
    await mount();
    const overview = root.querySelector(".context-overview")!;
    expect(overview.textContent).toContain("openai/local-alias · unity");
    expect(overview.querySelector(".context-upstream-model")?.textContent).toContain("最近响应模型：actual-upstream-model");
    expect(overview.querySelector(".context-upstream-model")?.getAttribute("title")).toBe("actual-upstream-model");
    expect(overview.querySelector(".context-overview-value")?.textContent).toContain("100 / 1.0k");
  });

  it.each([null, undefined, ""])("labels unavailable server identity explicitly (%s)", async (model) => {
    report.upstreamModel = model;
    await mount();
    expect(root.querySelector(".context-upstream-model")?.textContent).toContain("最近响应模型：未返回");
  });

  it("refreshes after a response and clears a previous upstream identity when headers are missing", async () => {
    await mount();
    for (const model of ["new-upstream-model", null]) {
      report.upstreamModel = model;
      onStream({ event: { type: "done", sessionId: "session", runId: "run", messageId: "m", fullText: "" } });
      await vi.advanceTimersByTimeAsync(300);
      await settle();
      expect(root.querySelector(".context-upstream-model")?.textContent).toContain(model ?? "未返回");
    }
  });
});
