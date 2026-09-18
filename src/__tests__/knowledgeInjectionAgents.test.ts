// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createApp, h, nextTick, ref, type App } from "vue";
import { createPinia } from "pinia";
import KnowledgeInjectionAgents from "../components/knowledge/KnowledgeInjectionAgents.vue";
import KnowledgePreview from "../components/knowledge/KnowledgePreview.vue";
import BaseDropdown from "../components/ui/BaseDropdown.vue";
import type { AgentInfo, KnowledgeDocument, KnowledgeDocumentPatch } from "../types";
import * as agentService from "../services/agent";

vi.mock("../services/agent", () => ({
  listAgents: vi.fn(), listSubagentDefs: vi.fn(),
  listWorkspaceAgents: vi.fn(), listWorkspaceSubagentDefs: vi.fn(),
}));
vi.mock("../components/ui/BaseMarkdownEditor.vue", () => ({
  default: { render: () => h("div", { class: "base-markdown-editor" }) },
}));

const unity = { id: "unity", name: "Unity" } as AgentInfo;
const reviewer = { id: "reviewer", name: "Reviewer" } as AgentInfo;
const workspaceRef = { checkoutId: "checkout-a", expectedGeneration: 1 };
let app: App | undefined;
let host: HTMLDivElement;
async function flush() { await Promise.resolve(); await Promise.resolve(); await nextTick(); }

beforeEach(() => {
  vi.resetAllMocks();
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { configurable: true, value: vi.fn() });
  vi.mocked(agentService.listAgents).mockResolvedValue([unity]);
  vi.mocked(agentService.listSubagentDefs).mockResolvedValue([unity, reviewer]);
  vi.mocked(agentService.listWorkspaceAgents).mockResolvedValue([unity]);
  vi.mocked(agentService.listWorkspaceSubagentDefs).mockResolvedValue([unity, reviewer]);
  host = document.createElement("div");
  document.body.append(host);
});
afterEach(() => { app?.unmount(); app = undefined; host.remove(); });

function checkbox(name: string) {
  return document.querySelector<HTMLButtonElement>(`[role="checkbox"][aria-label="${name}"]`)!;
}

it("defaults to Unity and saves each Agent selection including an empty list", async () => {
  const selected = ref<string[] | undefined>();
  app = createApp({ render: () => h(KnowledgeInjectionAgents, {
    modelValue: selected.value, workspaceRef,
    "onUpdate:modelValue": (value) => { selected.value = value; },
  }) });
  app.mount(host);
  await flush();
  expect(agentService.listWorkspaceAgents).toHaveBeenCalledWith(workspaceRef);
  expect(document.querySelectorAll('[role="checkbox"]')).toHaveLength(2);
  expect(checkbox("Unity").getAttribute("aria-checked")).toBe("true");
  expect(checkbox("Reviewer").getAttribute("aria-checked")).toBe("false");
  checkbox("Reviewer").click(); await nextTick();
  expect(selected.value).toEqual(["unity", "reviewer"]);
  checkbox("Unity").click(); await nextTick();
  expect(selected.value).toEqual(["reviewer"]);
  checkbox("Reviewer").click(); await nextTick();
  expect(selected.value).toEqual([]);
});

it("preserves unavailable Agents and respects read-only or pending saves", async () => {
  const update = vi.fn();
  app = createApp(KnowledgeInjectionAgents, {
    modelValue: ["custom-offline"], disabled: true, "onUpdate:modelValue": update,
  });
  app.mount(host); await flush();
  expect(checkbox("custom-offline").getAttribute("aria-checked")).toBe("true");
  checkbox("Unity").click();
  expect(update).not.toHaveBeenCalled();
});

it("ignores stale responses when switching workspaces", async () => {
  let resolveOld!: (value: AgentInfo[]) => void;
  vi.mocked(agentService.listWorkspaceAgents).mockReturnValueOnce(new Promise((resolve) => { resolveOld = resolve; }));
  const scope = ref(workspaceRef);
  app = createApp({ render: () => h(KnowledgeInjectionAgents, { modelValue: [], workspaceRef: scope.value }) });
  app.mount(host);
  scope.value = { checkoutId: "checkout-b", expectedGeneration: 2 };
  await nextTick(); await flush();
  resolveOld([{ id: "stale", name: "Stale" } as AgentInfo]); await flush();
  expect(checkbox("Stale")).toBeNull();
  expect(checkbox("Reviewer")).not.toBeNull();
});

it("allows retry after a failed Agent list request", async () => {
  vi.mocked(agentService.listAgents).mockRejectedValueOnce(new Error("unavailable"));
  app = createApp(KnowledgeInjectionAgents);
  app.mount(host); await flush();
  expect(document.querySelectorAll('[role="checkbox"]')).toHaveLength(0);
  host.querySelector<HTMLButtonElement>(".base-button")!.click(); await flush();
  expect(checkbox("Unity")).not.toBeNull();
});

it("connects the document editor's Agent panel to metadata updates", async () => {
  const current = ref<KnowledgeDocument>({
    id: "kd_injection", type: "memory", path: "agents.md", title: "agents",
    injectMode: "full", effectiveInjectMode: "full", readOnly: false,
    aiMaintained: false, effectiveAiMaintained: false, body: "Knowledge body",
    summary: null, maintenanceRules: null, effectiveMaintenanceRules: null, modifiedAt: 1,
  });
  const update = vi.fn((patch: KnowledgeDocumentPatch) => {
    current.value = { ...current.value, ...patch } as KnowledgeDocument;
  });
  app = createApp({ render: () => h(KnowledgePreview, {
    document: current.value, loading: false, saveLoading: false, workspaceRef,
    onUpdateMeta: update,
  }) }).use(createPinia());
  app.mount(host); await flush();
  host.querySelector<HTMLButtonElement>('.meta-dropdown .base-dropdown-trigger')!.click();
  await flush();
  expect(checkbox("Unity").getAttribute("aria-checked")).toBe("true");
  checkbox("Reviewer").click(); await nextTick();
  expect(update).toHaveBeenLastCalledWith({ injectAgents: ["unity", "reviewer"] });
  expect(current.value.body).toBe("Knowledge body");
  current.value = { ...current.value, injectMode: "excerpt", effectiveInjectMode: "excerpt" };
  await nextTick();
  expect(document.querySelector(".injection-agents")).toBeNull();
});

describe("injection menu companion panel", () => {
  it("keeps the teleported menu open and lets checkboxes own Space and Escape", async () => {
    const mode = ref("excerpt");
    const selected = ref<string[]>(["unity"]);
    app = createApp({ render: () => h(BaseDropdown, {
      modelValue: mode.value,
      options: [{ value: "excerpt", label: "L1" }, { value: "full", label: "L2" }],
      closeOnSelect: false, teleport: true,
      "onUpdate:modelValue": (value) => { mode.value = value; },
    }, mode.value === "full" ? { aside: () => h(KnowledgeInjectionAgents, {
      modelValue: selected.value,
      "onUpdate:modelValue": (value) => { selected.value = value; },
    }) } : {}) });
    app.mount(host);
    host.querySelector<HTMLButtonElement>(".base-dropdown-trigger")!.click(); await nextTick();
    document.querySelectorAll<HTMLButtonElement>('[role="option"]')[1].click(); await flush();
    expect(mode.value).toBe("full");
    expect(document.querySelector(".base-dropdown-menu.with-aside")).not.toBeNull();
    const target = checkbox("Reviewer");
    target.focus();
    const space = new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true });
    target.dispatchEvent(space);
    expect(space.defaultPrevented).toBe(false);
    target.click(); await nextTick();
    expect(selected.value).toEqual(["unity", "reviewer"]);
    expect(document.querySelector(".base-dropdown-menu")).not.toBeNull();
    target.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); await nextTick();
    await vi.waitFor(() => expect(document.querySelector(".base-dropdown-menu")).toBeNull());
    expect(document.activeElement).toBe(host.querySelector(".base-dropdown-trigger"));
  });
});
