// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, h, nextTick, reactive, ref, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { UnityConnectionStatus, UnitySemanticState } from "../types";

type Handler = (event: unknown) => void;
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), handlers: new Set<(event: unknown) => void>() }));
vi.mock("../services/ipc", () => ({ ipcInvoke: mocks.invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("../composables/useClaudeConnectionStatus", () => ({
  useClaudeConnectionStatus: () => ({ visible: ref(false), phase: ref("unknown"), status: ref(null), error: ref("") }),
}));
vi.mock("../composables/useClaudePrewarm", () => ({ useClaudePrewarm: () => {} }));
vi.mock("../services/locusRuntime", () => ({ getLocusRuntime: () => ({
  invoke: mocks.invoke,
  subscribe: async (_name: string, handler: Handler) => {
    mocks.handlers.add(handler);
    return () => mocks.handlers.delete(handler);
  },
}) }));

import ChatStatusIndicators from "../components/chat/ChatStatusIndicators.vue";
import { t } from "../i18n";

const scope = { checkoutId: "poll-a", expectedGeneration: 1, expectedMaterializationEpoch: 0 };
let app: App | undefined;
let host: HTMLDivElement;

async function flush() { for (let index = 0; index < 25; index += 1) await nextTick(); }
function semanticCalls(): number {
  return mocks.invoke.mock.calls.filter(([command]) => command === "get_unity_semantic_state").length;
}
function semantic(phase: string, checkedAtMs: number): UnitySemanticState {
  return {
    phase, source: "pipe", confidence: "high", transient: false, checkedAtMs,
    process: { state: phase === "quit" ? "not_running" : "running" },
    channel: { controlPipe: "ready" },
    domain: { phase: "none", source: "pipe", confidence: "high" },
    editorMode: { value: "editing", source: "pipe", confidence: "high" },
    mainThread: { state: "responsive", cpuActive: false, quiescentForMs: 0 },
    safety: { canCallUnityApi: phase === "editing", canModifyAssetsSafely: phase === "editing", recommendedAction: "proceed" },
    statePlane: { observer: "cache", dataPlane: "native", nativeBroker: "ready", nativeHook: "patched", historySamples: 1 },
    editorLog: { source: "unavailable", safeMode: false },
  } as UnitySemanticState;
}
function push(state: UnitySemanticState, checkoutId = scope.checkoutId) {
  for (const handler of [...mocks.handlers]) {
    handler({ eventName: "unity-semantic-state", checkoutId, workspaceGeneration: 1, payload: state });
  }
}
function labels(): string[] {
  return [...host.querySelectorAll<HTMLButtonElement>("button")]
    .map(button => button.getAttribute("aria-label") ?? "")
    .filter(label => label.startsWith(`${t("chat.status.unity.title")}:`));
}

beforeEach(() => {
  vi.useFakeTimers();
  mocks.handlers.clear();
  mocks.invoke.mockReset().mockImplementation(async (command: string) => {
    if (command === "get_unity_semantic_state") return semantic("editing", 1);
    if (command === "mcp_get_status" || command === "list_models") return [];
    return null;
  });
  host = document.createElement("div");
  document.body.appendChild(host);
});
afterEach(() => { app?.unmount(); app = undefined; host.remove(); vi.useRealTimers(); });

async function mount(bars: Array<{ pollingPaused: boolean }>) {
  const props = bars.map(bar => reactive({
    workspaceRef: scope,
    projectServices: ["unity"],
    workingDir: "F:/PollA",
    unityConnected: true,
    unityConnectionStatus: { checkedAtMs: 1 } as UnityConnectionStatus,
    pollingPaused: bar.pollingPaused,
  }));
  app = createApp(() => h("div", props.map(item => h(ChatStatusIndicators, item))));
  app.use(createPinia());
  app.mount(host);
  await flush();
  return props;
}

describe("Unity semantic state push", () => {
  it("fetches one snapshot per scope and never polls", async () => {
    await mount([{ pollingPaused: false }, { pollingPaused: false }]);
    expect(semanticCalls()).toBe(1);
    await vi.advanceTimersByTimeAsync(10_000);
    await flush();
    expect(semanticCalls()).toBe(1);
  });

  it("applies pushed states to visible bars and ignores other checkouts", async () => {
    await mount([{ pollingPaused: false }]);
    expect(labels()[0]).toContain(t("chat.unity.semantic.editing"));

    push(semantic("quit", 5), "other-checkout");
    await flush();
    expect(labels()[0]).toContain(t("chat.unity.semantic.editing"));

    push(semantic("quit", 5));
    await flush();
    expect(labels()[0]).toContain(t("chat.unity.semantic.quit"));

    // A sample older than the shown one (a late snapshot) does not regress it.
    push(semantic("editing", 2));
    await flush();
    expect(labels()[0]).toContain(t("chat.unity.semantic.quit"));
  });

  it("holds hidden bars and catches them up from the shared snapshot when shown", async () => {
    const [visible, hidden] = await mount([{ pollingPaused: false }, { pollingPaused: true }]);
    push(semantic("quit", 5));
    await flush();
    expect(labels()[0]).toContain(t("chat.unity.semantic.quit"));
    expect(labels()[1]).not.toContain(t("chat.unity.semantic.quit"));

    hidden.pollingPaused = false;
    await flush();
    expect(labels()[1]).toContain(t("chat.unity.semantic.quit"));
    expect(semanticCalls()).toBe(1);

    // The last visible consumer leaving closes the scope; showing it again
    // fetches a fresh snapshot.
    visible.pollingPaused = true;
    hidden.pollingPaused = true;
    await flush();
    hidden.pollingPaused = false;
    await flush();
    expect(semanticCalls()).toBe(2);
  });
});
