// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, h, nextTick, reactive, ref, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { UnitySemanticState } from "../types";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), confirm: vi.fn(), handlers: new Set<(event: unknown) => void>() }));
vi.mock("../services/ipc", () => ({ ipcInvoke: mocks.invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: mocks.confirm }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("../composables/useClaudeConnectionStatus", () => ({
  useClaudeConnectionStatus: () => ({ visible: ref(false), phase: ref("unknown"), status: ref(null), error: ref("") }),
}));
vi.mock("../composables/useClaudePrewarm", () => ({ useClaudePrewarm: () => {} }));
vi.mock("../services/locusRuntime", () => ({ getLocusRuntime: () => ({
  invoke: mocks.invoke,
  subscribe: async (_name: string, handler: (event: unknown) => void) => {
    mocks.handlers.add(handler);
    return () => mocks.handlers.delete(handler);
  },
}) }));

import ChatStatusIndicators from "../components/chat/ChatStatusIndicators.vue";
import { t } from "../i18n";

let app: App | undefined;
let host: HTMLDivElement;
let semantic: UnitySemanticState;
const scope = { checkoutId: "recovery-a", expectedGeneration: 1, expectedMaterializationEpoch: 0 };
async function flush() { for (let i = 0; i < 25; i++) await nextTick(); }
/** The backend observer pushes every semantic state change. */
async function push(next: UnitySemanticState) {
  semantic = { ...next, checkedAtMs: semantic.checkedAtMs + 1 };
  for (const handler of [...mocks.handlers]) {
    handler({ eventName: "unity-semantic-state", checkoutId: scope.checkoutId, workspaceGeneration: 1, payload: semantic });
  }
  await flush();
}
function restartCalls() { return mocks.invoke.mock.calls.filter(([command]) => command === "restart_unresponsive_unity_project"); }
function recoveryButton() { return [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.trim() === t("chat.status.unity.restart")); }
async function mount(extra: Record<string, unknown> = {}) {
  const props = reactive({ workspaceRef: scope, projectServices: ["unity"], workingDir: "F:/RecoveryA", unityConnected: true, ...extra });
  app = createApp(() => h(ChatStatusIndicators, props));
  app.use(createPinia()); app.mount(host); await flush();
  [...host.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => button.getAttribute("aria-label")?.startsWith(`${t("chat.status.unity.title")}:`))!.click();
  await flush();
  return props;
}

beforeEach(() => {
  mocks.handlers.clear();
  semantic = {
    phase: "unresponsive", source: "native_broker", confidence: "high", transient: false, checkedAtMs: 1,
    process: { state: "running", pid: 42, pidCreatedAtMs: 1000 },
    channel: { controlPipe: "ready", lastLatencyMs: 0 },
    domain: { phase: "none", source: "native_broker", confidence: "high" },
    editorMode: { value: "editing", source: "last_known", confidence: "low" },
    mainThread: { state: "unresponsive_busy", cpuActive: true, quiescentForMs: 0, heartbeatAgeMs: 45_000, activeExecutions: 1 },
    safety: { canCallUnityApi: false, canModifyAssetsSafely: false, recommendedAction: "diagnose_hang" },
    statePlane: { observer: "cache", dataPlane: "native", nativeBroker: "ready", nativeHook: "patched", historySamples: 1 },
    editorLog: { source: "unavailable", safeMode: false },
  };
  mocks.confirm.mockReset().mockResolvedValue(true);
  mocks.invoke.mockReset().mockImplementation(async (command: string) => {
    if (command === "get_unity_semantic_state") return semantic;
    if (command === "mcp_get_status" || command === "list_models") return [];
    return null;
  });
  host = document.createElement("div"); document.body.appendChild(host);
});
afterEach(() => { app?.unmount(); app = undefined; host.remove(); vi.useRealTimers(); });

describe("Unity recovery panel", () => {
  it("shows the failure and permits recovery despite an in-flight recompile", async () => {
    await mount({ unityRecompiling: true, unityConnected: false });
    expect(host.querySelector(".chat-status-popover-summary")?.classList.contains("tone-danger")).toBe(true);
    expect(host.textContent).toContain(t("chat.status.unity.seconds", 45));
    expect(recoveryButton()?.disabled).toBe(false);
    recoveryButton()!.click(); await flush();
    expect(mocks.confirm).toHaveBeenCalledOnce();
    expect(restartCalls()).toEqual([["restart_unresponsive_unity_project", {
      workspaceRef: scope, expectedProcessId: 42, expectedCreatedAtMs: 1000,
    }, expect.anything()]]);
  });

  it("does not restart after cancellation or a workspace change during confirmation", async () => {
    mocks.confirm.mockResolvedValueOnce(false);
    const props = await mount();
    recoveryButton()!.click(); await flush();
    expect(restartCalls()).toHaveLength(0);
    let accept!: (value: boolean) => void;
    mocks.confirm.mockImplementationOnce(() => new Promise<boolean>(resolve => { accept = resolve; }));
    recoveryButton()!.click(); await flush();
    props.workspaceRef = { ...scope, checkoutId: "recovery-b" }; await flush();
    accept(true); await flush();
    expect(restartCalls()).toHaveLength(0);
  });

  it("shows restart errors and removes recovery when the main thread recovers", async () => {
    const implementation = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation((command, args, options) => command === "restart_unresponsive_unity_project"
      ? Promise.reject(new Error("Restart failed")) : implementation(command, args, options));
    await mount();
    recoveryButton()!.click(); await flush();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("Restart failed");
    await push({ ...semantic, phase: "editing", safety: { canCallUnityApi: true, canModifyAssetsSafely: true, recommendedAction: "proceed" } });
    expect(recoveryButton()).toBeUndefined();
  });

  it("shows the brief hang diagnosis and keeps the stack in the diagnostics section", async () => {
    semantic.hangDiagnosis = {
      status: "running", processId: 42, startedAtMs: 1, verdict: "unknown", sampleCount: 0,
      summary: "Analyzing", frames: [],
    };
    await mount();
    expect(host.textContent).toContain(t("chat.status.unity.hang.running"));

    const location = {
      index: 2, kind: "managed", origin: "agent", ip: "0x1",
      display: "__LocusSnippet:Run () (at Snippet.cs:3)",
    };
    await push({ ...semantic, hangDiagnosis: {
      status: "complete", processId: 42, startedAtMs: 1, source: "native_broker", verdict: "busy",
      cpuPercent: 97.6, stackStable: true, sampleCount: 3, location, locationReason: "agent_code",
      summary: "busy", notes: ["Mono note"],
      frames: [
        { index: 0, kind: "native", origin: "native", ip: "0x2", display: "Unity.dll!LeakDetection::Record+0xA7" },
        { index: 1, kind: "unknown", origin: "unknown", ip: "0x3", display: "0x3 (JIT or dynamic code)" },
        location,
      ],
    } });
    const main = host.querySelector(".chat-status-detail-list")!.textContent!;
    expect(main).toContain(t("chat.status.unity.hang.location"));
    expect(main).toContain(location.display);
    expect(main).toContain(t("chat.status.unity.hang.cpu", 98));
    expect(main).toContain(t("chat.status.unity.hang.origin.agent"));
    expect(main).not.toContain("LeakDetection");
    const diagnostics = host.querySelector(".chat-status-diagnostics")!;
    const stack = [...diagnostics.querySelectorAll("dd.is-multiline")].map((node) => node.textContent);
    expect(stack[0]).toBe("#0 Unity.dll!LeakDetection::Record+0xA7\n#1 0x3 (JIT or dynamic code)\n#2 __LocusSnippet:Run () (at Snippet.cs:3)");
    expect(diagnostics.textContent).toContain(t("chat.status.unity.hang.samples", 3));
    expect(diagnostics.textContent).toContain("Mono note");
  });

  it("does not offer forced restart for a modal dialog or an unknown process identity", async () => {
    semantic.safety.recommendedAction = "resolve_dialog";
    await mount();
    expect(recoveryButton()).toBeUndefined();
    app!.unmount(); app = undefined;
    semantic.safety.recommendedAction = "diagnose_hang";
    semantic.process.pidCreatedAtMs = undefined;
    await mount();
    expect(recoveryButton()).toBeUndefined();
  });
});
