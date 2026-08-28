import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useAgentStore } from "../stores/agent";

const agentServiceMocks = vi.hoisted(() => ({
  listAgents: vi.fn(),
  listSubagentDefs: vi.fn(),
  listWorkspaceAgents: vi.fn(),
  listWorkspaceSubagentDefs: vi.fn(),
}));

vi.mock("../services/agent", () => agentServiceMocks);

describe("Agent store legacy aliases", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    agentServiceMocks.listAgents.mockResolvedValue([
      {
        id: "dev",
        name: "Unity",
        description: "Unity development",
        projectTypes: ["unity"],
        isDefault: true,
        source: "app",
      },
    ]);
    agentServiceMocks.listSubagentDefs.mockResolvedValue([]);
    agentServiceMocks.listWorkspaceSubagentDefs.mockResolvedValue([]);
  });

  it("routes retired built-in ids to Unity for historical sessions", async () => {
    const store = useAgentStore();
    await store.loadAgents();

    for (const id of ["git", "knowledge", "runtime_debugger", "doc", "wiki"]) {
      store.selectAgent(id);
      expect(store.selectedAgentId).toBe("dev");
    }
  });

  it("selects the workspace-compatible default when the checkout changes", async () => {
    agentServiceMocks.listWorkspaceAgents.mockResolvedValue([
      {
        id: "simple",
        name: "Simple",
        description: "General development",
        projectTypes: ["generic"],
        isDefault: true,
        source: "app",
      },
      {
        id: "dev",
        name: "Unity",
        description: "Unity development",
        projectTypes: ["unity"],
        isDefault: false,
        source: "app",
      },
    ]);
    const store = useAgentStore();
    await store.loadAgents();

    await store.loadWorkspaceAgents({ checkoutId: "checkout-generic" });
    expect(store.selectedAgentId).toBe("simple");

    store.selectAgent("dev");
    await store.loadWorkspaceAgents({ checkoutId: "checkout-generic" });
    expect(store.selectedAgentId).toBe("dev");

    await store.loadWorkspaceAgents({ checkoutId: "checkout-other" });
    expect(store.selectedAgentId).toBe("simple");
  });
});
