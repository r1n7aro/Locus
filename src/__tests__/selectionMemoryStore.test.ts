// @vitest-environment jsdom
import { createPinia, setActivePinia } from "pinia";
import { reactive } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useNewSessionSettings } from "../composables/useNewSessionSettings";
import { useSelectionMemoryStore } from "../stores/selectionMemory";
import type { WorkspaceSelectionSnapshot } from "../services/selectionMemory";
import { clampEffortToLevels } from "../stores/newSessionSelection";
import type { EffortLevel } from "../types";

const service = vi.hoisted(() => ({
  getWorkspaceSelection: vi.fn(),
  saveWorkspaceSelection: vi.fn(),
}));
vi.mock("../services/selectionMemory", () => service);
vi.mock("../services/agent", () => ({ setWorkspaceDefaultAgent: vi.fn() }));

const modelStore = reactive({
  availableModels: [{ id: "m-a" }, { id: "m-b", supportedEfforts: ["low", "high"] }],
  modelDefaults: { mainModel: "" },
  agentModelPreferences: {} as Record<string, { modelId: string; effort: string }>,
  lastModelId: "",
  hasUserDefaultEffort: false,
  defaultEffort: "high",
  defaultCodexFastMode: false,
  effortsForModel: (id: string) => (id === "m-b" ? ["low", "high"] : []),
  clampEffortForModel: (id: string, level: EffortLevel) => clampEffortToLevels(id === "m-b" ? ["low", "high"] : [], level),
  noteRememberedSelection: vi.fn(),
});
vi.mock("../stores/model", () => ({ useModelStore: () => modelStore }));

function snapshot(projectId: string, agentId = "", modelId = ""): WorkspaceSelectionSnapshot {
  return {
    projectId,
    memory: { agentId, modelId, effort: "", agents: {}, updatedAtMs: 0 },
    lastAgentId: agentId,
    workspaceDefaultAgentId: null,
  };
}

beforeEach(() => {
  setActivePinia(createPinia());
  useNewSessionSettings().reset();
  vi.clearAllMocks();
  service.saveWorkspaceSelection.mockResolvedValue(undefined);
});

describe("selection memory store", () => {
  it("shares a remembered choice with other checkouts of the same project", async () => {
    service.getWorkspaceSelection.mockImplementation(async () => snapshot("project"));
    const store = useSelectionMemoryStore();
    await store.load({ checkoutId: "main" });
    await store.load({ checkoutId: "worktree" });
    await store.remember({ checkoutId: "main" }, { agentId: "simple", modelId: "m-b", effort: "low" });
    expect(store.snapshotFor("worktree")?.memory).toMatchObject({
      agentId: "simple",
      modelId: "m-b",
      agents: { simple: { modelId: "m-b", effort: "low" } },
    });
    expect(service.saveWorkspaceSelection).toHaveBeenCalledWith({ checkoutId: "main" }, {
      agentId: "simple",
      modelId: "m-b",
      effort: "low",
    });
    expect(modelStore.noteRememberedSelection).toHaveBeenCalledWith("simple", "m-b", "low");
  });

  it("keeps a choice made while an older load is still in flight", async () => {
    service.getWorkspaceSelection.mockResolvedValueOnce(snapshot("project"));
    const store = useSelectionMemoryStore();
    await store.load({ checkoutId: "main" });
    let finishLoad: (value: WorkspaceSelectionSnapshot) => void = () => undefined;
    service.getWorkspaceSelection.mockReturnValueOnce(new Promise((resolve) => { finishLoad = resolve; }));
    const pending = store.load({ checkoutId: "main" });
    await store.remember({ checkoutId: "main" }, { agentId: "simple", modelId: "m-b" });
    finishLoad(snapshot("project", "unity", "m-a"));
    await pending;
    expect(store.snapshotFor("main")?.memory.modelId).toBe("m-b");
  });

  it("starts a new session from memory, falling back to the Agent's default effort", async () => {
    service.getWorkspaceSelection.mockResolvedValue(snapshot("project", "simple", "m-b"));
    const store = useSelectionMemoryStore();
    await store.load({ checkoutId: "main" });
    const agents = [
      { id: "unity", isDefault: true, defaultEffort: "high" as const },
      { id: "simple", isDefault: false, defaultEffort: "low" as const },
    ];
    expect(store.resolveNewSession("main", agents)).toEqual({
      agentId: "simple",
      modelId: "m-b",
      effort: "low",
      fastMode: false,
    });
    useNewSessionSettings().setAgentMode("workspace");
    expect(store.resolveNewSession("main", agents).agentId).toBe("unity");
  });
});
